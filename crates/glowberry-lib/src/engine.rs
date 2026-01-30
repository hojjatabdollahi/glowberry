// SPDX-License-Identifier: MPL-2.0

use crate::{
    fragment_canvas, gpu, img_source,
    upower::{start_power_monitor, PowerMonitorHandle},
    user_context::{EnvGuard, UserContext},
    wallpaper::Wallpaper,
};
use cosmic_config::{calloop::ConfigWatchSource, CosmicConfigEntry};
use eyre::{eyre, Context};
use glowberry_config::{
    power_saving::{OnBatteryAction, PowerSavingConfig},
    state::State,
    Config,
};
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputInfo, OutputState},
    reexports::{
        calloop,
        calloop_wayland_source::WaylandSource,
        client::{
            delegate_noop,
            globals::registry_queue_init,
            protocol::{
                wl_output::{self, WlOutput},
                wl_surface,
            },
            Connection, Dispatch, Proxy, QueueHandle, Weak,
        },
        protocols::wp::{
            fractional_scale::v1::client::{
                wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
            },
            viewporter::client::{wp_viewport, wp_viewporter},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use std::thread;
use tracing::error;

/// Access glibc malloc tunables.
#[cfg(target_env = "gnu")]
mod malloc {
    use std::os::raw::c_int;
    const M_MMAP_THRESHOLD: c_int = -3;

    unsafe extern "C" {
        fn malloc_trim(pad: usize);
        fn mallopt(param: c_int, value: c_int) -> c_int;
    }

    /// Prevents glibc from hoarding memory via memory fragmentation.
    pub fn limit_mmap_threshold() {
        unsafe {
            mallopt(M_MMAP_THRESHOLD, 65536);
        }
    }

    /// Asks glibc to trim malloc arenas.
    pub fn trim() {
        unsafe {
            malloc_trim(0);
        }
    }
}

/// GPU state for shader-based live wallpapers.
pub struct GpuLayerState {
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    canvas: fragment_canvas::FragmentCanvas,
}

// Manual Debug impl since wgpu types don't implement Debug
impl std::fmt::Debug for GpuLayerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuLayerState")
            .field("surface_config", &self.surface_config)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct EngineConfig {
    pub enable_wayland: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            enable_wayland: true,
        }
    }
}

#[derive(Debug)]
pub struct BackgroundEngine;

impl BackgroundEngine {
    #[allow(clippy::too_many_lines)]
    pub fn run(config: EngineConfig) -> eyre::Result<()> {
        Self::run_with_stop(config, None)
    }

    #[allow(clippy::too_many_lines)]
    fn run_with_stop(
        config: EngineConfig,
        stop_rx: Option<calloop::channel::Channel<()>>,
    ) -> eyre::Result<()> {
        if !config.enable_wayland {
            return Ok(());
        }

        // Prevents glibc from hoarding memory via memory fragmentation.
        #[cfg(target_env = "gnu")]
        malloc::limit_mmap_threshold();

        let conn = Connection::connect_to_env().wrap_err("wayland client connection failed")?;
        // Clone the connection for use in CosmicBg state (needed for GPU surface creation)
        let conn_for_state = conn.clone();

        let mut event_loop: calloop::EventLoop<'static, CosmicBg> =
            calloop::EventLoop::try_new().wrap_err("failed to create event loop")?;

        let (globals, event_queue) =
            registry_queue_init(&conn).wrap_err("failed to initialize registry queue")?;

        let qh = event_queue.handle();

        WaylandSource::new(conn, event_queue)
            .insert(event_loop.handle())
            .map_err(|err| err.error)
            .wrap_err("failed to insert main EventLoop into WaylandSource")?;

        if let Some(stop_rx) = stop_rx {
            event_loop
                .handle()
                .insert_source(stop_rx, |event, _, state| match event {
                    calloop::channel::Event::Msg(()) | calloop::channel::Event::Closed => {
                        state.exit = true;
                    }
                })
                .map_err(|err| eyre!("failed to insert stop channel into event loop: {err}"))?;
        }

        let config_context = glowberry_config::context();

        let config = match config_context {
            Ok(config_context) => {
                let source = ConfigWatchSource::new(&config_context.0)
                    .expect("failed to create ConfigWatchSource");

                let conf_context = config_context.clone();
                event_loop
                    .handle()
                    .insert_source(source, move |(_config, keys), (), state| {
                        let mut changes_applied = false;

                        for key in &keys {
                            match key.as_str() {
                                glowberry_config::BACKGROUNDS => {
                                    tracing::debug!("updating backgrounds");
                                    state.config.load_backgrounds(&conf_context);
                                    changes_applied = true;
                                }

                                glowberry_config::DEFAULT_BACKGROUND => {
                                    tracing::debug!("updating default background");
                                    let entry = conf_context.default_background();

                                    if state.config.default_background != entry {
                                        state.config.default_background = entry;
                                        changes_applied = true;
                                    }
                                }

                                glowberry_config::SAME_ON_ALL => {
                                    tracing::debug!("updating same_on_all");
                                    state.config.same_on_all = conf_context.same_on_all();

                                    if state.config.same_on_all {
                                        state.config.outputs.clear();
                                    } else {
                                        state.config.load_backgrounds(&conf_context);
                                    }
                                    state.config.outputs.clear();
                                    changes_applied = true;
                                }

                                // Power saving config keys
                                glowberry_config::power_saving::PAUSE_ON_FULLSCREEN
                                | glowberry_config::power_saving::PAUSE_ON_COVERED
                                | glowberry_config::power_saving::COVERAGE_THRESHOLD
                                | glowberry_config::power_saving::ADJUST_ON_BATTERY
                                | glowberry_config::power_saving::ON_BATTERY_ACTION
                                | glowberry_config::power_saving::PAUSE_ON_LOW_BATTERY
                                | glowberry_config::power_saving::LOW_BATTERY_THRESHOLD
                                | glowberry_config::power_saving::PAUSE_ON_LID_CLOSED => {
                                    tracing::debug!(key, "power saving config changed");
                                    state.power_saving_config = conf_context.power_saving_config();
                                    tracing::info!(config = ?state.power_saving_config, "Updated power saving config");
                                }

                                _ => {
                                    tracing::debug!(key, "key modified");
                                    if let Some(output) = key.strip_prefix("output.") {
                                        if let Ok(new_entry) = conf_context.entry(key) {
                                            if let Some(existing) = state.config.entry_mut(output) {
                                                *existing = new_entry;
                                                changes_applied = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if changes_applied {
                            state.apply_backgrounds();

                            #[cfg(target_env = "gnu")]
                            malloc::trim();

                            tracing::debug!(
                                same_on_all = state.config.same_on_all,
                                outputs = ?state.config.outputs,
                                backgrounds = ?state.config.backgrounds,
                                default_background = ?state.config.default_background.source,
                                "new state"
                            );
                        }
                    })
                    .expect("failed to insert config watching source into event loop");

                Config::load(&config_context).unwrap_or_else(|why| {
                    tracing::error!(?why, "Config file error, falling back to defaults");
                    Config::default()
                })
            }
            Err(why) => {
                tracing::error!(?why, "Config file error, falling back to defaults");
                Config::default()
            }
        };

        // Load power saving configuration
        let power_saving_config = glowberry_config::context()
            .map(|ctx| ctx.power_saving_config())
            .unwrap_or_default();
        tracing::info!(?power_saving_config, "Loaded power saving config");

        // Start power monitor for battery/lid state tracking
        let power_monitor = start_power_monitor();
        if power_monitor.is_some() {
            tracing::info!("Power monitor started successfully");
        } else {
            tracing::warn!("Failed to start power monitor, power saving features will be disabled");
        }

        let source_tx = img_source::img_source(&event_loop.handle(), |state, source, event| {
            use notify::event::{ModifyKind, RenameMode};

            match event.kind {
                notify::EventKind::Create(_)
                | notify::EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                    for w in state
                        .wallpapers
                        .iter_mut()
                        .filter(|w| w.entry.output == source)
                    {
                        for p in &event.paths {
                            if !w.image_queue.contains(p) {
                                w.image_queue.push_front(p.into());
                            }
                        }
                        w.image_queue.retain(|p| !event.paths.contains(p));
                        // TODO maybe resort or shuffle at some point?
                    }
                }
                notify::EventKind::Remove(_)
                | notify::EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                    for w in state
                        .wallpapers
                        .iter_mut()
                        .filter(|w| w.entry.output == source)
                    {
                        w.image_queue.retain(|p| !event.paths.contains(p));
                    }
                }
                _ => {}
            }
        });

        // initial setup with all images
        let wallpapers = {
            let mut wallpapers = Vec::with_capacity(config.backgrounds.len() + 1);

            wallpapers.extend({
                config.backgrounds.iter().map(|bg| {
                    Wallpaper::new(
                        bg.clone(),
                        qh.clone(),
                        event_loop.handle(),
                        source_tx.clone(),
                    )
                })
            });

            wallpapers.sort_by(|a, b| a.entry.output.cmp(&b.entry.output));

            wallpapers.push(Wallpaper::new(
                config.default_background.clone(),
                qh.clone(),
                event_loop.handle(),
                source_tx.clone(),
            ));

            wallpapers
        };

        // Check if any wallpaper uses a shader source
        let has_shader_source = config
            .backgrounds
            .iter()
            .any(|bg| matches!(bg.source, glowberry_config::Source::Shader(_)))
            || matches!(
                config.default_background.source,
                glowberry_config::Source::Shader(_)
            );

        // Lazily initialize GPU renderer only if needed
        let gpu_renderer = if has_shader_source {
            tracing::info!("Initializing GPU renderer for shader wallpapers");
            Some(gpu::GpuRenderer::new())
        } else {
            None
        };

        let mut bg_state = CosmicBg {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            compositor_state: CompositorState::bind(&globals, &qh).unwrap(),
            shm_state: Shm::bind(&globals, &qh).unwrap(),
            layer_state: LayerShell::bind(&globals, &qh).unwrap(),
            viewporter: globals.bind(&qh, 1..=1, ()).unwrap(),
            fractional_scale_manager: globals.bind(&qh, 1..=1, ()).ok(),
            qh,
            source_tx,
            loop_handle: event_loop.handle(),
            exit: false,
            wallpapers,
            config,
            active_outputs: Vec::new(),
            gpu_renderer,
            connection: conn_for_state,
            power_monitor,
            power_saving_config,
        };

        loop {
            event_loop.dispatch(None, &mut bg_state)?;

            if bg_state.exit {
                break;
            }
        }

        Ok(())
    }
}

pub struct BackgroundHandle {
    stop_tx: calloop::channel::Sender<()>,
    join: Option<thread::JoinHandle<()>>,
    env_guard: Option<EnvGuard>,
}

impl BackgroundHandle {
    pub fn spawn(user: UserContext, config: EngineConfig) -> Self {
        // Environment variables are process-wide, so keep the guard for the handle lifetime.
        let env_guard = user.apply();
        let (stop_tx, stop_rx) = calloop::channel::channel();
        let join = thread::spawn(move || {
            if let Err(err) = BackgroundEngine::run_with_stop(config, Some(stop_rx)) {
                tracing::error!(?err, "background engine exited with error");
            }
        });

        Self {
            stop_tx,
            join: Some(join),
            env_guard: Some(env_guard),
        }
    }

    pub fn stop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        self.env_guard.take();
    }
}

impl Drop for BackgroundHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug)]
pub struct CosmicBgLayer {
    pub(crate) layer: LayerSurface,
    pub(crate) viewport: wp_viewport::WpViewport,
    pub(crate) wl_output: WlOutput,
    pub(crate) output_info: OutputInfo,
    pub(crate) pool: Option<SlotPool>,
    pub(crate) needs_redraw: bool,
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) fractional_scale: Option<u32>,
    /// GPU state for shader wallpapers (None for static wallpapers).
    pub(crate) gpu_state: Option<GpuLayerState>,
}

pub struct CosmicBg {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm_state: Shm,
    layer_state: LayerShell,
    viewporter: wp_viewporter::WpViewporter,
    fractional_scale_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    qh: QueueHandle<CosmicBg>,
    source_tx: calloop::channel::SyncSender<(String, notify::Event)>,
    loop_handle: calloop::LoopHandle<'static, CosmicBg>,
    exit: bool,
    pub(crate) wallpapers: Vec<Wallpaper>,
    config: Config,
    active_outputs: Vec<WlOutput>,
    /// GPU renderer for shader wallpapers (lazily initialized).
    gpu_renderer: Option<gpu::GpuRenderer>,
    /// Wayland connection for creating GPU surfaces.
    connection: Connection,
    /// Power monitor handle for battery/lid state.
    power_monitor: Option<PowerMonitorHandle>,
    /// Power saving configuration.
    power_saving_config: PowerSavingConfig,
}

// Manual Debug impl since wgpu types don't implement Debug
impl std::fmt::Debug for CosmicBg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CosmicBg")
            .field("exit", &self.exit)
            .field("wallpapers", &self.wallpapers)
            .field("config", &self.config)
            .field("active_outputs", &self.active_outputs)
            .field("gpu_renderer", &self.gpu_renderer.is_some())
            .field("power_monitor", &self.power_monitor.is_some())
            .finish_non_exhaustive()
    }
}

impl CosmicBg {
    /// Check if shader animation should be paused based on current power state.
    /// Returns true if animation should be paused.
    fn should_pause_animation(&self) -> bool {
        let Some(ref power_monitor) = self.power_monitor else {
            return false; // No power monitor, don't pause
        };

        let power_state = power_monitor.current();
        let config = &self.power_saving_config;

        // Check lid closed (pause on internal displays)
        if config.pause_on_lid_closed && power_state.lid_is_closed {
            tracing::debug!("Pausing animation: lid is closed");
            return true;
        }

        // Check low battery
        if config.pause_on_low_battery {
            if let Some(percentage) = power_state.battery_percentage {
                if percentage <= config.low_battery_threshold as f64 {
                    tracing::debug!(
                        percentage,
                        threshold = config.low_battery_threshold,
                        "Pausing animation: low battery"
                    );
                    return true;
                }
            }
        }

        // Check on battery action
        if power_state.on_battery {
            match config.on_battery_action {
                OnBatteryAction::Pause => {
                    tracing::debug!("Pausing animation: on battery (pause action)");
                    return true;
                }
                OnBatteryAction::Nothing
                | OnBatteryAction::ReduceTo15Fps
                | OnBatteryAction::ReduceTo10Fps
                | OnBatteryAction::ReduceTo5Fps => {
                    // Don't pause, but frame rate may be reduced (handled elsewhere)
                }
            }
        }

        false
    }

    /// Get the effective frame rate based on power state.
    /// Returns None if using the shader's configured frame rate.
    fn effective_frame_rate(&self) -> Option<u8> {
        let Some(ref power_monitor) = self.power_monitor else {
            return None;
        };

        let power_state = power_monitor.current();

        if power_state.on_battery {
            self.power_saving_config.on_battery_action.frame_rate()
        } else {
            None
        }
    }

    fn shader_physical_size(
        layer_size: Option<(u32, u32)>,
        fractional_scale: Option<u32>,
        output_mode_dims: Option<(u32, u32)>,
    ) -> (u32, u32) {
        if let Some((w, h)) = layer_size {
            let scale = fractional_scale.unwrap_or(120);
            return (w * scale / 120, h * scale / 120);
        }

        if let Some((w, h)) = output_mode_dims {
            return (w, h);
        }

        let (w, h) = (1920, 1080);
        let scale = fractional_scale.unwrap_or(120);
        (w * scale / 120, h * scale / 120)
    }

    fn shader_layer_physical_size(layer: &CosmicBgLayer) -> (u32, u32) {
        let output_mode_dims = layer
            .output_info
            .modes
            .iter()
            .find(|m| m.current)
            .map(|m| (m.dimensions.0 as u32, m.dimensions.1 as u32));

        Self::shader_physical_size(layer.size, layer.fractional_scale, output_mode_dims)
    }

    fn update_shader_layer_surface(
        gpu: &gpu::GpuRenderer,
        qh: &QueueHandle<Self>,
        layer: &mut CosmicBgLayer,
    ) {
        let (physical_w, physical_h) = Self::shader_layer_physical_size(layer);
        let Some(gpu_state) = layer.gpu_state.as_mut() else {
            return;
        };

        gpu_state.surface_config =
            gpu.configure_surface(&gpu_state.surface, physical_w, physical_h);
        gpu_state
            .canvas
            .update_resolution(gpu.queue(), physical_w, physical_h);

        // Set viewport destination to logical size so compositor scales correctly
        if let Some((logical_w, logical_h)) = layer.size {
            layer
                .viewport
                .set_destination(logical_w as i32, logical_h as i32);
        }

        let wl_surface = layer.layer.wl_surface();
        wl_surface.frame(qh, wl_surface.clone());
        layer.layer.commit();
    }

    fn apply_backgrounds(&mut self) {
        self.wallpapers.clear();

        let mut all_wallpaper = Wallpaper::new(
            self.config.default_background.clone(),
            self.qh.clone(),
            self.loop_handle.clone(),
            self.source_tx.clone(),
        );

        let mut backgrounds = self.config.backgrounds.clone();
        backgrounds.sort_by(|a, b| a.output.cmp(&b.output));

        'outer: for output in &self.active_outputs {
            let Some(output_info) = self.output_state.info(output) else {
                continue;
            };

            let o_name = output_info.name.clone().unwrap_or_default();
            for background in &backgrounds {
                if background.output == o_name {
                    let mut new_wallpaper = Wallpaper::new(
                        background.clone(),
                        self.qh.clone(),
                        self.loop_handle.clone(),
                        self.source_tx.clone(),
                    );

                    new_wallpaper
                        .layers
                        .push(self.new_layer(output.clone(), output_info));
                    _ = new_wallpaper.save_state();
                    self.wallpapers.push(new_wallpaper);

                    continue 'outer;
                }
            }

            all_wallpaper
                .layers
                .push(self.new_layer(output.clone(), output_info));
        }

        _ = all_wallpaper.save_state();
        self.wallpapers.push(all_wallpaper);
    }

    #[must_use]
    pub fn new_layer(&self, output: WlOutput, output_info: OutputInfo) -> CosmicBgLayer {
        let surface = self.compositor_state.create_surface(&self.qh);

        let layer = self.layer_state.create_layer_surface(
            &self.qh,
            surface.clone(),
            Layer::Background,
            "wallpaper".into(),
            Some(&output),
        );

        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        surface.commit();

        let viewport = self.viewporter.get_viewport(&surface, &self.qh, ());

        let fractional_scale = if let Some(mngr) = self.fractional_scale_manager.as_ref() {
            mngr.get_fractional_scale(&surface, &self.qh, surface.downgrade());
            None
        } else {
            (self.compositor_state.wl_compositor().version() < 6)
                .then_some(output_info.scale_factor as u32 * 120)
        };

        CosmicBgLayer {
            layer,
            viewport,
            wl_output: output,
            output_info,
            size: None,
            fractional_scale,
            needs_redraw: false,
            pool: None,
            gpu_state: None,
        }
    }

    /// Initialize GPU state for a shader wallpaper layer (internal version using indices).
    fn init_gpu_layer_internal(
        &mut self,
        wallpaper_idx: usize,
        layer_idx: usize,
        shader_source: &glowberry_config::ShaderSource,
    ) {
        // Ensure GPU renderer is initialized
        if self.gpu_renderer.is_none() {
            tracing::info!("Lazily initializing GPU renderer for shader wallpaper");
            self.gpu_renderer = Some(gpu::GpuRenderer::new());
        }

        let gpu = self.gpu_renderer.as_ref().unwrap();

        // Get layer info needed for surface creation
        let layer = &self.wallpapers[wallpaper_idx].layers[layer_idx];
        let wl_surface = layer.layer.wl_surface().clone();
        let output_name = layer.output_info.name.clone();

        // Get native resolution from the current output mode
        let (physical_width, physical_height) = layer
            .output_info
            .modes
            .iter()
            .find(|m| m.current)
            .map(|m| (m.dimensions.0 as u32, m.dimensions.1 as u32))
            .unwrap_or_else(|| {
                // Fallback to layer size with scale if no mode info
                let (w, h) = layer.size.unwrap_or((1920, 1080));
                let scale = layer.fractional_scale.unwrap_or(120);
                (w * scale / 120, h * scale / 120)
            });

        tracing::debug!(
            output = ?output_name,
            physical_width,
            physical_height,
            "GPU layer dimensions (native resolution)"
        );

        // Create GPU surface
        let surface = unsafe { gpu.create_surface(&self.connection, &wl_surface) };

        // Configure surface at native resolution
        let surface_config = gpu.configure_surface(&surface, physical_width, physical_height);

        // Create fragment canvas
        match fragment_canvas::FragmentCanvas::new(gpu, shader_source, surface_config.format) {
            Ok(mut canvas) => {
                canvas.update_resolution(gpu.queue(), physical_width, physical_height);

                // Render the first frame immediately to avoid showing default wallpaper
                if let Ok(surface_texture) = surface.get_current_texture() {
                    let view = surface_texture
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    canvas.render(gpu, &view);
                    surface_texture.present();
                    canvas.mark_frame_rendered();
                    tracing::debug!(output = ?output_name, "Rendered initial shader frame");
                }

                let layer = &mut self.wallpapers[wallpaper_idx].layers[layer_idx];
                layer.gpu_state = Some(GpuLayerState {
                    surface,
                    surface_config,
                    canvas,
                });

                // Set viewport destination to logical size so compositor scales correctly
                if let Some((logical_w, logical_h)) = layer.size {
                    layer
                        .viewport
                        .set_destination(logical_w as i32, logical_h as i32);
                }

                // Request first frame callback to continue animation
                wl_surface.frame(&self.qh, wl_surface.clone());
                layer.layer.commit();

                tracing::info!(
                    output = ?output_name,
                    "Initialized GPU layer for shader wallpaper"
                );
            }
            Err(err) => {
                tracing::error!(
                    ?err,
                    "Failed to create fragment canvas for shader wallpaper"
                );
            }
        }
    }
}

impl CompositorHandler for CosmicBg {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if self.fractional_scale_manager.is_none() {
            let mut target: Option<(usize, usize, bool)> = None;
            for (wallpaper_idx, wallpaper) in self.wallpapers.iter().enumerate() {
                if let Some(layer_idx) = wallpaper
                    .layers
                    .iter()
                    .position(|layer| layer.layer.wl_surface() == surface)
                {
                    target = Some((wallpaper_idx, layer_idx, wallpaper.is_shader()));
                    break;
                }
            }

            if let Some((wallpaper_idx, layer_idx, is_shader)) = target {
                let qh = self.qh.clone();
                let gpu = self.gpu_renderer.as_ref();
                let wallpaper = &mut self.wallpapers[wallpaper_idx];
                let layer = &mut wallpaper.layers[layer_idx];
                layer.fractional_scale = Some(new_factor as u32 * 120);
                if is_shader {
                    if let Some(gpu) = gpu {
                        Self::update_shader_layer_surface(gpu, &qh, layer);
                    }
                } else {
                    wallpaper.draw();
                }
            }
        }
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // Check if animation should be paused due to power state
        let should_pause = self.should_pause_animation();

        // Find the wallpaper and layer for this surface
        for wallpaper in &mut self.wallpapers {
            if let Some(layer) = wallpaper
                .layers
                .iter_mut()
                .find(|l| l.layer.wl_surface() == surface)
            {
                // Check if this is a shader wallpaper with GPU state
                if let Some(gpu_state) = &mut layer.gpu_state {
                    // Skip rendering if paused, but still request frame callback
                    // so we can resume when power state changes
                    if !should_pause {
                        // Check if we should render this frame (frame rate limiting)
                        if gpu_state.canvas.should_render() {
                            if let Some(gpu) = &self.gpu_renderer {
                                // Get current texture
                                match gpu_state.surface.get_current_texture() {
                                    Ok(surface_texture) => {
                                        let view = surface_texture
                                            .texture
                                            .create_view(&wgpu::TextureViewDescriptor::default());

                                        // Update resolution for this specific layer's surface
                                        let width = gpu_state.surface_config.width;
                                        let height = gpu_state.surface_config.height;

                                        tracing::trace!(
                                            output = ?layer.output_info.name,
                                            width,
                                            height,
                                            "Rendering shader frame"
                                        );

                                        gpu_state.canvas.update_resolution(
                                            gpu.queue(),
                                            width,
                                            height,
                                        );

                                        // Render the shader
                                        gpu_state.canvas.render(gpu, &view);

                                        // Present
                                        surface_texture.present();

                                        gpu_state.canvas.mark_frame_rendered();
                                    }
                                    Err(wgpu::SurfaceError::Timeout) => {
                                        tracing::warn!("GPU surface timeout");
                                    }
                                    Err(
                                        wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated,
                                    ) => {
                                        let width = gpu_state.surface_config.width;
                                        let height = gpu_state.surface_config.height;
                                        gpu_state.surface_config = gpu.configure_surface(
                                            &gpu_state.surface,
                                            width,
                                            height,
                                        );
                                        gpu_state.canvas.update_resolution(
                                            gpu.queue(),
                                            width,
                                            height,
                                        );
                                        tracing::warn!(
                                            "GPU surface lost or outdated; reconfigured surface"
                                        );
                                    }
                                    Err(wgpu::SurfaceError::OutOfMemory) => {
                                        tracing::error!("GPU out of memory");
                                    }
                                    Err(err) => {
                                        tracing::warn!(?err, "GPU surface error");
                                    }
                                }
                            }
                        }
                    }

                    // Request next frame callback to continue animation
                    // We always request this so we can resume when unpaused
                    surface.frame(qh, surface.clone());
                    layer.layer.commit();
                }
                break;
            }
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // TODO
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &WlOutput,
    ) {
    }
}

impl OutputHandler for CosmicBg {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        wl_output: wl_output::WlOutput,
    ) {
        self.active_outputs.push(wl_output.clone());
        let Some(output_info) = self.output_state.info(&wl_output) else {
            return;
        };

        if let Some(pos) = self
            .wallpapers
            .iter()
            .position(|w| match w.entry.output.as_str() {
                "all" => !w.layers.iter().any(|l| l.wl_output == wl_output),
                name => {
                    Some(name) == output_info.name.as_deref()
                        && !w.layers.iter().any(|l| l.wl_output == wl_output)
                }
            })
        {
            let layer = self.new_layer(wl_output, output_info);
            self.wallpapers[pos].layers.push(layer);
            if let Err(err) = self.wallpapers[pos].save_state() {
                tracing::error!("{err}");
            }
        }
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.fractional_scale_manager.is_none()
            && self.compositor_state.wl_compositor().version() < 6
        {
            let Some(output_info) = self.output_state.info(&output) else {
                return;
            };
            let output_info = output_info.clone();
            let mut target: Option<(usize, usize, bool)> = None;
            for (wallpaper_idx, wallpaper) in self.wallpapers.iter().enumerate() {
                if let Some(layer_idx) = wallpaper
                    .layers
                    .iter()
                    .position(|layer| layer.wl_output == output)
                {
                    target = Some((wallpaper_idx, layer_idx, wallpaper.is_shader()));
                    break;
                }
            }

            if let Some((wallpaper_idx, layer_idx, is_shader)) = target {
                let qh = self.qh.clone();
                let gpu = self.gpu_renderer.as_ref();
                let wallpaper = &mut self.wallpapers[wallpaper_idx];
                let layer = &mut wallpaper.layers[layer_idx];
                layer.output_info = output_info;
                layer.fractional_scale = Some(layer.output_info.scale_factor as u32 * 120);
                if is_shader {
                    if let Some(gpu) = gpu {
                        Self::update_shader_layer_surface(gpu, &qh, layer);
                    }
                } else {
                    wallpaper.draw();
                }
            }
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.active_outputs.retain(|o| o != &output);
        let Some(output_info) = self.output_state.info(&output) else {
            return;
        };

        // state cleanup
        if let Ok(state_helper) = State::state() {
            let mut state = State::get_entry(&state_helper).unwrap_or_default();
            state
                .wallpapers
                .retain(|(o_name, _source)| Some(o_name) != output_info.name.as_ref());
            if let Err(err) = state.write_entry(&state_helper) {
                error!("{err}");
            }
        }

        let Some(output_wallpaper) =
            self.wallpapers
                .iter_mut()
                .find(|w| match w.entry.output.as_str() {
                    "all" => true,
                    name => Some(name) == output_info.name.as_deref(),
                })
        else {
            return;
        };

        let Some(layer_position) = output_wallpaper
            .layers
            .iter()
            .position(|bg_layer| bg_layer.wl_output == output)
        else {
            return;
        };

        output_wallpaper.layers.remove(layer_position);
    }
}

impl LayerShellHandler for CosmicBg {
    fn closed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        dropped_layer: &LayerSurface,
    ) {
        for wallpaper in &mut self.wallpapers {
            wallpaper
                .layers
                .retain(|layer| &layer.layer != dropped_layer);
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let span = tracing::debug_span!("<CosmicBg as LayerShellHandler>::configure");
        let _handle = span.enter();

        let (w, h) = configure.new_size;

        // Find the wallpaper and layer index for this surface
        let mut found_info: Option<(usize, usize, bool, Option<glowberry_config::ShaderSource>)> =
            None;

        for (wp_idx, wallpaper) in self.wallpapers.iter_mut().enumerate() {
            if let Some(layer_idx) = wallpaper.layers.iter().position(|l| &l.layer == layer) {
                let is_shader = wallpaper.is_shader();
                let shader_source = wallpaper.shader_source().cloned();
                found_info = Some((wp_idx, layer_idx, is_shader, shader_source));

                // Update layer state
                let w_layer = &mut wallpaper.layers[layer_idx];
                w_layer.size = Some((w, h));
                w_layer.needs_redraw = true;
                break;
            }
        }

        let Some((wp_idx, layer_idx, is_shader, shader_source)) = found_info else {
            return;
        };

        if is_shader {
            // Initialize or update GPU state for shader wallpapers
            if let Some(shader_source) = shader_source {
                let w_layer = &mut self.wallpapers[wp_idx].layers[layer_idx];

                if w_layer.gpu_state.is_none() {
                    // Initialize GPU state
                    self.init_gpu_layer_internal(wp_idx, layer_idx, &shader_source);
                } else {
                    let qh = self.qh.clone();
                    if let Some(gpu) = self.gpu_renderer.as_ref() {
                        let layer = &mut self.wallpapers[wp_idx].layers[layer_idx];
                        Self::update_shader_layer_surface(gpu, &qh, layer);
                    }
                }
            }
        } else {
            // Static wallpaper - use SHM buffer pool
            let w_layer = &mut self.wallpapers[wp_idx].layers[layer_idx];

            if let Some(pool) = w_layer.pool.as_mut() {
                if let Err(why) = pool.resize(w as usize * h as usize * 4) {
                    tracing::error!(?why, "failed to resize pool");
                    return;
                }
            } else {
                match SlotPool::new(w as usize * h as usize * 4, &self.shm_state) {
                    Ok(pool) => {
                        w_layer.pool.replace(pool);
                    }
                    Err(why) => {
                        tracing::error!(?why, "failed to create pool");
                        return;
                    }
                }
            }

            self.wallpapers[wp_idx].draw();
        }
    }
}

impl ShmHandler for CosmicBg {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

delegate_compositor!(CosmicBg);
delegate_output!(CosmicBg);
delegate_shm!(CosmicBg);
delegate_layer!(CosmicBg);
delegate_registry!(CosmicBg);
delegate_noop!(CosmicBg: wp_viewporter::WpViewporter);
delegate_noop!(CosmicBg: wp_viewport::WpViewport);
delegate_noop!(CosmicBg: wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, Weak<wl_surface::WlSurface>>
    for CosmicBg
{
    fn event(
        state: &mut CosmicBg,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        surface: &Weak<wl_surface::WlSurface>,
        _: &Connection,
        _: &QueueHandle<CosmicBg>,
    ) {
        match event {
            wp_fractional_scale_v1::Event::PreferredScale { scale } => {
                if let Ok(surface) = surface.upgrade() {
                    let mut target: Option<(usize, usize, bool)> = None;
                    for (wallpaper_idx, wallpaper) in state.wallpapers.iter().enumerate() {
                        if let Some(layer_idx) = wallpaper
                            .layers
                            .iter()
                            .position(|layer| layer.layer.wl_surface() == &surface)
                        {
                            target = Some((wallpaper_idx, layer_idx, wallpaper.is_shader()));
                            break;
                        }
                    }

                    if let Some((wallpaper_idx, layer_idx, is_shader)) = target {
                        let qh = state.qh.clone();
                        let gpu = state.gpu_renderer.as_ref();
                        let wallpaper = &mut state.wallpapers[wallpaper_idx];
                        let layer = &mut wallpaper.layers[layer_idx];
                        layer.fractional_scale = Some(scale);
                        if is_shader {
                            if let Some(gpu) = gpu {
                                CosmicBg::update_shader_layer_surface(gpu, &qh, layer);
                            }
                        } else {
                            wallpaper.draw();
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
    }
}

impl ProvidesRegistryState for CosmicBg {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

#[cfg(test)]
mod tests {
    use super::CosmicBg;

    #[test]
    fn shader_physical_size_prefers_layer_size_over_mode() {
        let size = Some((100, 50));
        let scale = Some(150);
        let mode = Some((1920, 1080));

        let result = CosmicBg::shader_physical_size(size, scale, mode);

        assert_eq!(result, (125, 62));
    }

    #[test]
    fn shader_physical_size_uses_mode_when_size_missing() {
        let result = CosmicBg::shader_physical_size(None, Some(150), Some((1280, 720)));

        assert_eq!(result, (1280, 720));
    }

    #[test]
    fn shader_physical_size_defaults_scale_to_120() {
        let result = CosmicBg::shader_physical_size(Some((1200, 800)), None, Some((640, 480)));

        assert_eq!(result, (1200, 800));
    }
}
