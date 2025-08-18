// SPDX-License-Identifier: MPL-2.0

mod colored;
mod draw;
mod img_source;
mod scaler;
mod wallpaper;

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

use cosmic_bg_config::{Config, state::State};
use cosmic_config::{CosmicConfigEntry, calloop::ConfigWatchSource};
use eyre::Context;
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputInfo, OutputState},
    reexports::{
        calloop,
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, Dispatch, Proxy, QueueHandle, Weak, delegate_noop,
            globals::registry_queue_init,
            protocol::{
                wl_output::{self, WlOutput},
                wl_surface,
            },
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
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};

use tracing::error;
use tracing_subscriber::prelude::*;
use wallpaper::Wallpaper;

#[derive(Debug)]
pub struct CosmicBgLayer {
    layer: LayerSurface,
    viewport: wp_viewport::WpViewport,
    wl_output: WlOutput,
    output_info: OutputInfo,
    pool: Option<SlotPool>,
    needs_redraw: bool,
    size: Option<(u32, u32)>,
    fractional_scale: Option<u32>,
}

#[allow(clippy::too_many_lines)]
fn main() -> color_eyre::Result<()> {
    // Prevents glibc from hoarding memory via memory fragmentation.
    #[cfg(target_env = "gnu")]
    malloc::limit_mmap_threshold();

    color_eyre::install()?;

    if std::env::var("RUST_SPANTRACE").is_err() {
        unsafe {
            std::env::set_var("RUST_SPANTRACE", "0");
        }
    }

    init_logger();

    let conn = Connection::connect_to_env().wrap_err("wayland client connection failed")?;

    let mut event_loop: calloop::EventLoop<'static, CosmicBg> =
        calloop::EventLoop::try_new().wrap_err("failed to create event loop")?;

    let (globals, event_queue) =
        registry_queue_init(&conn).wrap_err("failed to initialize registry queue")?;

    let qh = event_queue.handle();

    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .map_err(|err| err.error)
        .wrap_err("failed to insert main EventLoop into WaylandSource")?;

    let config_context = cosmic_bg_config::context();

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
                            cosmic_bg_config::BACKGROUNDS => {
                                tracing::debug!("updating backgrounds");
                                state.config.load_backgrounds(&conf_context);
                                changes_applied = true;
                            }

                            cosmic_bg_config::DEFAULT_BACKGROUND => {
                                tracing::debug!("updating default background");
                                let entry = conf_context.default_background();

                                if state.config.default_background != entry {
                                    state.config.default_background = entry;
                                    changes_applied = true;
                                }
                            }

                            cosmic_bg_config::SAME_ON_ALL => {
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

    let source_tx = img_source::img_source(&event_loop.handle());

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
    };

    loop {
        event_loop.dispatch(None, &mut bg_state)?;

        if bg_state.exit {
            break;
        }
    }

    Ok(())
}

#[derive(Debug)]
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
    wallpapers: Vec<Wallpaper>,
    config: Config,
    active_outputs: Vec<WlOutput>,
}

impl CosmicBg {
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
            for wallpaper in &mut self.wallpapers {
                if let Some(layer) = wallpaper
                    .layers
                    .iter_mut()
                    .find(|layer| layer.layer.wl_surface() == surface)
                {
                    layer.fractional_scale = Some(new_factor as u32 * 120);
                    wallpaper.draw();
                    break;
                }
            }
        }
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
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
            for wallpaper in &mut self.wallpapers {
                if let Some(layer) = wallpaper
                    .layers
                    .iter_mut()
                    .find(|layer| layer.wl_output == output)
                {
                    layer.fractional_scale = Some(output_info.scale_factor as u32 * 120);
                    wallpaper.draw();
                    break;
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

        for wallpaper in &mut self.wallpapers {
            let (w, h) = configure.new_size;
            if let Some(w_layer) = wallpaper.layers.iter_mut().find(|l| &l.layer == layer) {
                w_layer.size = Some((w, h));
                w_layer.needs_redraw = true;

                if let Some(pool) = w_layer.pool.as_mut() {
                    if let Err(why) = pool.resize(w as usize * h as usize * 4) {
                        tracing::error!(?why, "failed to resize pool");
                        continue;
                    }
                } else {
                    match SlotPool::new(w as usize * h as usize * 4, &self.shm_state) {
                        Ok(pool) => {
                            w_layer.pool.replace(pool);
                        }

                        Err(why) => {
                            tracing::error!(?why, "failed to create pool");
                            continue;
                        }
                    }
                }

                wallpaper.draw();

                break;
            }
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
                    for wallpaper in &mut state.wallpapers {
                        if let Some(layer) = wallpaper
                            .layers
                            .iter_mut()
                            .find(|layer| layer.layer.wl_surface() == &surface)
                        {
                            layer.fractional_scale = Some(scale);
                            wallpaper.draw();
                            break;
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

fn init_logger() {
    let log_level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|level| level.parse::<tracing::Level>().ok())
        .unwrap_or(tracing::Level::INFO);

    let log_format = tracing_subscriber::fmt::format()
        .pretty()
        .without_time()
        .with_line_number(true)
        .with_file(true)
        .with_target(false)
        .with_thread_names(true);

    let log_filter = tracing_subscriber::fmt::Layer::default()
        .with_writer(std::io::stderr)
        .event_format(log_format)
        .with_filter(tracing_subscriber::filter::filter_fn(move |metadata| {
            metadata.level() == &tracing::Level::ERROR
                || (metadata.target().starts_with("cosmic_bg") && metadata.level() <= &log_level)
        }));

    tracing_subscriber::registry().with(log_filter).init();
}
