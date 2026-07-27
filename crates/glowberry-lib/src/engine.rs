// SPDX-License-Identifier: MPL-2.0

use crate::{
    fragment_canvas, gpu, img_source,
    screensaver::{FadePhase, ScreensaverState},
    upower::{PowerMonitorHandle, PowerStateChanged, start_power_monitor},
    wallpaper::Wallpaper,
};
use cosmic_config::{CosmicConfigEntry, calloop::ConfigWatchSource};
use eyre::Context;
use glowberry_config::{
    Config, Source,
    power_saving::{OnBatteryAction, PowerSavingConfig},
    screensaver::{DismissEvent, ScreensaverConfig},
    state::State,
};
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputInfo, OutputState},
    reexports::{
        calloop,
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, Dispatch, Proxy, QueueHandle, Weak, delegate_noop,
            globals::registry_queue_init,
            protocol::{
                wl_buffer, wl_keyboard,
                wl_output::{self, WlOutput},
                wl_pointer, wl_seat, wl_surface,
            },
        },
        protocols::{
            ext::idle_notify::v1::client::{ext_idle_notification_v1, ext_idle_notifier_v1},
            wp::{
                alpha_modifier::v1::client::{wp_alpha_modifier_surface_v1, wp_alpha_modifier_v1},
                fractional_scale::v1::client::{
                    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
                },
                single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1,
                viewporter::client::{wp_viewport, wp_viewporter},
            },
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::time::Duration;
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
    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) surface_config: wgpu::SurfaceConfiguration,
    pub(crate) canvas: fragment_canvas::FragmentCanvas,
    /// Resolution scale (0.25-1.0): the buffer is rendered at this fraction of
    /// the output's physical size and upscaled by the compositor via viewport.
    pub(crate) render_scale: f32,
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
    /// Show the screensaver immediately at startup, bypassing both the
    /// `enabled` config flag and the idle timeout.
    ///
    /// This exists for testing a shader without waiting out the timeout. It
    /// starts a full daemon, so it should not be run alongside the one
    /// `cosmic-session` launched — the two would fight over the wallpaper layer.
    pub force_screensaver: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            enable_wayland: true,
            force_screensaver: false,
        }
    }
}

#[derive(Debug)]
pub struct BackgroundEngine;

impl BackgroundEngine {
    #[allow(clippy::too_many_lines)]
    pub fn run(config: EngineConfig) -> eyre::Result<()> {
        if !config.enable_wayland {
            return Ok(());
        }

        // Captured before `config` is shadowed by the loaded wallpaper config.
        let force_screensaver = config.force_screensaver;

        // Prevents glibc from hoarding memory via memory fragmentation.
        #[cfg(target_env = "gnu")]
        malloc::limit_mmap_threshold();

        let conn = Connection::connect_to_env().wrap_err("wayland client connection failed")?;
        // Clone the connection for use in GlowBerry state (needed for GPU surface creation)
        let conn_for_state = conn.clone();

        let mut event_loop: calloop::EventLoop<'static, GlowBerry> =
            calloop::EventLoop::try_new().wrap_err("failed to create event loop")?;

        let (globals, event_queue) =
            registry_queue_init(&conn).wrap_err("failed to initialize registry queue")?;

        let qh = event_queue.handle();

        WaylandSource::new(conn, event_queue)
            .insert(event_loop.handle())
            .map_err(|err| err.error)
            .wrap_err("failed to insert main EventLoop into WaylandSource")?;

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
                                glowberry_config::power_saving::ADJUST_ON_BATTERY
                                | glowberry_config::power_saving::ON_BATTERY_ACTION
                                | glowberry_config::power_saving::PAUSE_ON_LOW_BATTERY
                                | glowberry_config::power_saving::LOW_BATTERY_THRESHOLD
                                | glowberry_config::power_saving::PAUSE_ON_LID_CLOSED => {
                                    tracing::debug!(key, "power saving config changed");
                                    let was_paused = state.should_pause_animation();
                                    state.power_saving_config = conf_context.power_saving_config();
                                    tracing::info!(config = ?state.power_saving_config, "Updated power saving config");
                                    // Force reapply frame rates with new config
                                    state.reapply_frame_rates();
                                    // Resume animation if we were paused and now we're not
                                    let is_paused = state.should_pause_animation();
                                    if was_paused && !is_paused {
                                        tracing::info!("Resuming shader animation after config change");
                                        state.request_frame_callbacks();
                                    }
                                }

                                // Screensaver config keys
                                glowberry_config::screensaver::ENABLED
                                | glowberry_config::screensaver::IDLE_TIMEOUT_SECS
                                | glowberry_config::screensaver::SOURCE
                                | glowberry_config::screensaver::FADE_IN_MS
                                | glowberry_config::screensaver::FADE_OUT_MS
                                | glowberry_config::screensaver::DISMISS_ON
                                | glowberry_config::screensaver::MAX_RUNTIME_SECS
                                | glowberry_config::screensaver::ON_BATTERY => {
                                    tracing::debug!(key, "screensaver config changed");
                                    state.screensaver_config = conf_context.screensaver_config();
                                    tracing::info!(
                                        config = ?state.screensaver_config,
                                        "Updated screensaver config"
                                    );

                                    // A disabled or reconfigured screensaver must
                                    // not stay on screen showing the old settings.
                                    if !state.screensaver_config.enabled {
                                        state.teardown_screensaver();
                                    }
                                    state.refresh_idle_notification();
                                }

                                _ => {
                                    tracing::debug!(key, "key modified");
                                    if let Some(output) = key.strip_prefix("output.")
                                        && let Ok(new_entry) = conf_context.entry(key)
                                            && let Some(existing) = state.config.entry_mut(output) {
                                                *existing = new_entry;
                                                changes_applied = true;
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

        // Create channel for power state change notifications
        let (power_notify_tx, power_notify_rx) = calloop::channel::channel();

        // Start power monitor for battery/lid state tracking
        let power_monitor = start_power_monitor(Some(power_notify_tx));
        if power_monitor.is_some() {
            tracing::info!("Power monitor started successfully");
        } else {
            tracing::warn!("Failed to start power monitor, power saving features will be disabled");
        }

        // Insert power state change notification source into event loop
        event_loop
            .handle()
            .insert_source(power_notify_rx, |event, _, state| {
                if let calloop::channel::Event::Msg(PowerStateChanged) = event {
                    tracing::debug!("Received power state change notification");
                    state.on_power_state_changed();
                }
            })
            .expect("failed to insert power notification channel into event loop");

        let source_tx = img_source::img_source(&event_loop.handle(), |state, source, event| {
            use notify::event::{ModifyKind, RenameMode};

            match event.kind {
                // Shader file content changed — hot-reload
                notify::EventKind::Modify(ModifyKind::Data(_)) => {
                    for (idx, w) in state.wallpapers.iter().enumerate() {
                        if w.entry.output != source {
                            continue;
                        }
                        if matches!(w.entry.source, Source::Shader(_)) {
                            tracing::debug!(
                                output = source,
                                "Shader file modified, triggering hot-reload"
                            );
                            state.reload_shader(idx);
                            return;
                        }
                    }
                }

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
            match gpu::GpuRenderer::new() {
                Ok(renderer) => Some(renderer),
                Err(err) => {
                    tracing::error!(
                        ?err,
                        "GPU initialization failed — shader wallpapers will fall back to static color"
                    );
                    None
                }
            }
        } else {
            None
        };

        // Load screensaver configuration
        let mut screensaver_config = glowberry_config::context()
            .map(|ctx| ctx.screensaver_config())
            .unwrap_or_default();

        if force_screensaver {
            tracing::info!("--screensaver given; forcing the screensaver on for this run");
            screensaver_config.enabled = true;
            // Bypassing the timeout means bypassing the battery gate too:
            // an explicit request should be honoured.
            screensaver_config.on_battery = true;
        }

        tracing::info!(?screensaver_config, "Loaded screensaver config");

        // `ext-idle-notify-v1` is staging and not universally implemented; a
        // missing global disables the screensaver rather than failing startup.
        let idle_notifier = globals
            .bind::<ext_idle_notifier_v1::ExtIdleNotifierV1, _, _>(&qh, 1..=2, ())
            .ok();
        if idle_notifier.is_none() && screensaver_config.enabled {
            tracing::warn!(
                "Compositor does not advertise ext-idle-notify-v1; screensaver disabled"
            );
        }

        // Optional: without the alpha modifier the screensaver still works, it
        // just appears and disappears without fading.
        let alpha_modifier = globals
            .bind::<wp_alpha_modifier_v1::WpAlphaModifierV1, _, _>(&qh, 1..=1, ())
            .ok();
        if alpha_modifier.is_none() && screensaver_config.enabled {
            tracing::info!(
                "Compositor does not advertise wp_alpha_modifier_v1; screensaver fades disabled"
            );
        }

        let single_pixel_buffer = globals
            .bind::<wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1, _, _>(
                &qh,
                1..=1,
                (),
            )
            .ok();

        let mut bg_state = GlowBerry {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            compositor_state: CompositorState::bind(&globals, &qh).unwrap(),
            shm_state: Shm::bind(&globals, &qh).unwrap(),
            layer_state: LayerShell::bind(&globals, &qh).unwrap(),
            viewporter: globals.bind(&qh, 1..=1, ()).unwrap(),
            fractional_scale_manager: globals.bind(&qh, 1..=1, ()).ok(),
            qh: qh.clone(),
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
            current_frame_rate_override: None,
            was_animation_paused: false,
            seat_state: SeatState::new(&globals, &qh),
            keyboard: None,
            pointer: None,
            screensaver_config,
            idle_notifier,
            idle_notification: None,
            armed_idle_timeout_ms: None,
            alpha_modifier,
            single_pixel_buffer,
            screensaver: None,
        };

        bg_state.refresh_idle_notification();

        // Outputs arrive asynchronously, so a forced screensaver cannot be
        // raised until at least one wl_output has been advertised.
        let mut pending_force_activate = force_screensaver;

        loop {
            event_loop.dispatch(None, &mut bg_state)?;

            if pending_force_activate && !bg_state.active_outputs.is_empty() {
                pending_force_activate = false;
                bg_state.activate_screensaver();
            }

            if bg_state.exit {
                break;
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct GlowBerryLayer {
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

pub struct GlowBerry {
    registry_state: RegistryState,
    pub(crate) output_state: OutputState,
    pub(crate) compositor_state: CompositorState,
    shm_state: Shm,
    pub(crate) layer_state: LayerShell,
    pub(crate) viewporter: wp_viewporter::WpViewporter,
    fractional_scale_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    pub(crate) qh: QueueHandle<GlowBerry>,
    source_tx: calloop::channel::SyncSender<(String, notify::Event)>,
    pub(crate) loop_handle: calloop::LoopHandle<'static, GlowBerry>,
    exit: bool,
    pub(crate) wallpapers: Vec<Wallpaper>,
    config: Config,
    pub(crate) active_outputs: Vec<WlOutput>,
    /// GPU renderer for shader wallpapers (lazily initialized).
    pub(crate) gpu_renderer: Option<gpu::GpuRenderer>,
    /// Wayland connection for creating GPU surfaces.
    connection: Connection,
    /// Power monitor handle for battery/lid state.
    pub(crate) power_monitor: Option<PowerMonitorHandle>,
    /// Power saving configuration.
    power_saving_config: PowerSavingConfig,
    /// Currently applied frame rate override (None = using configured rates).
    current_frame_rate_override: Option<u8>,
    /// Whether animation was paused in the last frame (for detecting resume).
    was_animation_paused: bool,
    /// Seat tracking, needed for the idle notification and screensaver input.
    seat_state: SeatState,
    /// Keyboard on the first seat that reported one, used to dismiss the
    /// screensaver. `None` until the capability arrives.
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Pointer on the first seat that reported one.
    pointer: Option<wl_pointer::WlPointer>,
    /// Screensaver configuration.
    pub(crate) screensaver_config: ScreensaverConfig,
    /// `ext-idle-notify-v1` manager, absent on compositors without the protocol.
    idle_notifier: Option<ext_idle_notifier_v1::ExtIdleNotifierV1>,
    /// The live idle notification, recreated when the timeout changes.
    idle_notification: Option<ext_idle_notification_v1::ExtIdleNotificationV1>,
    /// Timeout the live notification was created with, so redundant config
    /// events do not rebuild it and restart the compositor's idle countdown.
    armed_idle_timeout_ms: Option<u32>,
    /// Compositor-side alpha multiplier manager, used for screensaver fades.
    pub(crate) alpha_modifier: Option<wp_alpha_modifier_v1::WpAlphaModifierV1>,
    /// Single-pixel buffer manager, used to paint black screensaver surfaces
    /// without allocating a full-size buffer.
    pub(crate) single_pixel_buffer:
        Option<wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1>,
    /// Live screensaver state; `None` whenever the screensaver is not showing.
    pub(crate) screensaver: Option<ScreensaverState>,
}

// Manual Debug impl since wgpu types don't implement Debug
impl std::fmt::Debug for GlowBerry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlowBerry")
            .field("exit", &self.exit)
            .field("wallpapers", &self.wallpapers)
            .field("config", &self.config)
            .field("active_outputs", &self.active_outputs)
            .field("gpu_renderer", &self.gpu_renderer.is_some())
            .field("power_monitor", &self.power_monitor.is_some())
            .finish_non_exhaustive()
    }
}

impl GlowBerry {
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

        // Check low battery (only when on battery, not when plugged in)
        if config.pause_on_low_battery
            && power_state.on_battery
            && let Some(percentage) = power_state.battery_percentage
            && percentage <= config.low_battery_threshold as f64
        {
            tracing::debug!(
                percentage,
                threshold = config.low_battery_threshold,
                "Pausing animation: low battery"
            );
            return true;
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

    /// Create, replace, or drop the idle notification to match current config.
    ///
    /// Notification objects carry their timeout immutably, so a timeout change
    /// means destroying the old object and creating a new one. Destroying the
    /// old one first matters: leaving both alive would have the compositor
    /// report idle twice.
    ///
    /// Rebuilding restarts the compositor's countdown, so this is a no-op when
    /// the wanted timeout already matches the armed one. cosmic-config delivers
    /// several change events for a single user action, and without that guard
    /// each one would push the screensaver further away.
    pub(crate) fn refresh_idle_notification(&mut self) {
        let wanted = self.screensaver_config.enabled;
        let wanted_timeout_ms = wanted.then(|| self.screensaver_config.idle_timeout_ms());

        if wanted_timeout_ms == self.armed_idle_timeout_ms && self.idle_notification.is_some() {
            tracing::debug!("Idle notification already armed with this timeout; leaving it alone");
            return;
        }

        if let Some(notification) = self.idle_notification.take() {
            notification.destroy();
        }
        self.armed_idle_timeout_ms = None;

        if !wanted {
            if self.screensaver.is_some() {
                self.teardown_screensaver();
            }
            return;
        }

        let Some(notifier) = self.idle_notifier.as_ref() else {
            return;
        };

        // The seat arrives asynchronously; if it is not here yet, SeatHandler
        // calls back into this function once it is.
        let Some(seat) = self.seat_state.seats().next() else {
            tracing::debug!("No seat yet; deferring idle notification setup");
            return;
        };

        let timeout_ms = self.screensaver_config.idle_timeout_ms();

        // `get_idle_notification` respects idle inhibitors, unlike the v2
        // `get_input_idle_notification`. That is what keeps the screensaver from
        // appearing over a fullscreen video.
        self.idle_notification =
            Some(notifier.get_idle_notification(timeout_ms, &seat, &self.qh, ()));
        self.armed_idle_timeout_ms = Some(timeout_ms);

        tracing::info!(timeout_ms, "Idle notification armed for screensaver");

        if let Some(true) = self.screensaver_config.exceeds_screen_off() {
            tracing::warn!(
                timeout_secs = self.screensaver_config.idle_timeout_secs,
                "Screensaver timeout is at or beyond cosmic-idle's screen_off_time; \
                 lower it so the screensaver is visible before the screen blanks"
            );
        }
    }

    /// Render one screensaver frame for `surface` and advance its fade.
    ///
    /// Returns the delay until the next frame is due, or `None` when the surface
    /// is static and needs no further callbacks (fully faded in with nothing
    /// animating, or already released by the runtime cap).
    fn render_screensaver_frame(&mut self, surface: &wl_surface::WlSurface) -> Option<Duration> {
        let runtime_expired = self.screensaver_runtime_expired();
        let fade_in = Duration::from_millis(self.screensaver_config.fade_in_ms);
        let fade_out = Duration::from_millis(self.screensaver_config.fade_out_ms);
        // The wallpaper's pause rules (lid closed, low battery) apply to the
        // screensaver too — a closed lid should not render either.
        let should_pause = self.should_pause_animation();

        // Step aside before cosmic-idle's own fade surface goes up. Removing
        // ourselves is more robust than relying on which of two overlay-layer
        // surfaces the compositor happens to stack on top.
        if self.screensaver_past_screen_off_deadline() {
            tracing::info!(
                "Reached cosmic-idle's screen-off time; removing screensaver so the \
                 fade-to-black and lock proceed on a clean surface"
            );
            self.teardown_screensaver();
            return None;
        }

        let layer_idx = self
            .screensaver
            .as_ref()?
            .layers
            .iter()
            .position(|l| l.wl_surface() == surface)?;

        let screensaver = self.screensaver.as_mut()?;
        let (alpha, phase_done) = screensaver.phase.alpha(fade_in, fade_out);

        // Fade-out completion is the actual teardown trigger.
        if phase_done && matches!(screensaver.phase, FadePhase::FadingOut { .. }) {
            tracing::info!("Screensaver dismissed");
            self.teardown_screensaver();
            return None;
        }

        if phase_done && matches!(screensaver.phase, FadePhase::FadingIn { .. }) {
            screensaver.phase = FadePhase::Active;
        }

        let fading = !matches!(screensaver.phase, FadePhase::Active);
        let layer = &mut screensaver.layers[layer_idx];

        if !layer.is_configured() {
            return None;
        }

        layer.set_alpha(alpha);

        // Past the runtime cap, swap the shader for a black buffer: nobody is
        // watching, and this is the common case when the screensaver timeout is
        // far below cosmic-idle's screen-off time. The swap has to be explicit —
        // just dropping gpu_state would leave the surface referencing swapchain
        // buffers that no longer exist.
        if runtime_expired && layer.gpu_state.is_some() {
            let output = layer.output_info.name.clone();
            let max_runtime_secs = self.screensaver_config.max_runtime_secs;
            tracing::info!(
                ?output,
                ?max_runtime_secs,
                "Screensaver runtime cap reached; releasing renderer and going black"
            );
            self.attach_black_buffer(layer_idx);
            if let Some(screensaver) = self.screensaver.as_mut() {
                screensaver.render_released = true;
            }
            return fading.then(|| Duration::from_millis(16));
        }

        let has_gpu = layer.gpu_state.is_some();

        if !has_gpu {
            // Black layers only need a commit to latch the new alpha; the
            // single-pixel buffer already attached stays put.
            layer.layer.commit();
            // Keep waking only while a fade is in flight.
            return fading.then(|| Duration::from_millis(16));
        }

        if should_pause {
            // Still commit so the fade completes, but do not render.
            layer.layer.commit();
            return fading.then(|| Duration::from_millis(16));
        }

        let gpu_state = layer.gpu_state.as_mut()?;
        if !gpu_state.canvas.should_render() {
            return Some(gpu_state.canvas.next_frame_delay());
        }

        let gpu = self.gpu_renderer.as_ref()?;

        match gpu_state.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(surface_texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => {
                let view = surface_texture
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                tracing::trace!(alpha, "Rendering screensaver frame");
                gpu_state.canvas.render(gpu, &view);
                // present() commits the surface, which is also what latches the
                // alpha multiplier set above.
                surface_texture.present();
                gpu_state.canvas.mark_frame_rendered();
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                let width = gpu_state.surface_config.width;
                let height = gpu_state.surface_config.height;
                gpu_state.surface_config = gpu.configure_surface(&gpu_state.surface, width, height);
                gpu_state
                    .canvas
                    .update_resolution(gpu.queue(), width, height);
                tracing::warn!("Screensaver GPU surface lost or outdated; reconfigured");
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                tracing::warn!("Screensaver GPU surface timeout");
            }
            other => {
                tracing::warn!(?other, "Screensaver GPU surface error");
            }
        }

        Some(gpu_state.canvas.next_frame_delay())
    }

    /// React to a configure on a screensaver layer: build its contents on the
    /// first configure, or resize the existing swapchain on later ones.
    fn reconfigure_screensaver_layer(&mut self, layer_idx: usize) {
        let has_gpu_state = self
            .screensaver
            .as_ref()
            .and_then(|s| s.layers.get(layer_idx))
            .is_some_and(|l| l.gpu_state.is_some());

        if !has_gpu_state {
            self.init_screensaver_layer(layer_idx);
            return;
        }

        // Resize path: reconfigure the swapchain to the new physical size and
        // point the viewport at the new logical size.
        let Some(gpu) = self.gpu_renderer.as_ref() else {
            return;
        };
        let Some(screensaver) = self.screensaver.as_mut() else {
            return;
        };
        let Some(layer) = screensaver.layers.get_mut(layer_idx) else {
            return;
        };
        let Some((logical_w, logical_h)) = layer.size else {
            return;
        };

        let output_mode_dims = layer
            .output_info
            .modes
            .iter()
            .find(|m| m.current)
            .map(|m| (m.dimensions.0 as u32, m.dimensions.1 as u32));
        let physical =
            Self::shader_physical_size(layer.size, layer.fractional_scale, output_mode_dims);

        let Some(gpu_state) = layer.gpu_state.as_mut() else {
            return;
        };
        let (physical_w, physical_h) = Self::scaled_buffer_size(physical, gpu_state.render_scale);

        gpu_state.surface_config =
            gpu.configure_surface(&gpu_state.surface, physical_w, physical_h);
        gpu_state
            .canvas
            .update_resolution(gpu.queue(), physical_w, physical_h);

        layer
            .viewport
            .set_destination(logical_w as i32, logical_h as i32);

        let wl_surface = layer.wl_surface().clone();
        wl_surface.frame(&self.qh, wl_surface.clone());
        layer.layer.commit();
    }

    /// Set up a screensaver layer's contents once the compositor has configured
    /// it: a GPU surface for shader layers, a black single-pixel buffer
    /// otherwise.
    fn init_screensaver_layer(&mut self, layer_idx: usize) {
        let Some(screensaver) = self.screensaver.as_ref() else {
            return;
        };
        let Some(layer) = screensaver.layers.get(layer_idx) else {
            return;
        };

        let Some((logical_w, logical_h)) = layer.size else {
            return;
        };
        let shader_source = layer.shader_source.clone();
        let output_info = layer.output_info.clone();
        let fractional_scale = layer.fractional_scale;
        let wl_surface = layer.wl_surface().clone();

        // Viewport destination is the logical size regardless of contents: it is
        // what upscales both a render-scaled shader buffer and a 1x1 black one.
        layer
            .viewport
            .set_destination(logical_w as i32, logical_h as i32);

        // Resolve the layer's contents. Every failure path converges on black
        // rather than leaving an empty surface on the overlay layer.
        let gpu_state = match shader_source.as_ref() {
            None => None,
            Some(shader_source) => {
                if self.gpu_renderer.is_none() {
                    tracing::info!("Lazily initializing GPU renderer for screensaver");
                    match gpu::GpuRenderer::new() {
                        Ok(renderer) => self.gpu_renderer = Some(renderer),
                        Err(err) => tracing::error!(
                            ?err,
                            "GPU initialization failed; screensaver falling back to black"
                        ),
                    }
                }

                self.gpu_renderer.as_ref().and_then(|gpu| {
                    Self::create_gpu_state(
                        gpu,
                        &self.connection,
                        &wl_surface,
                        &output_info,
                        Some((logical_w, logical_h)),
                        fractional_scale,
                        shader_source,
                    )
                })
            }
        };

        match gpu_state {
            Some(gpu_state) => {
                if let Some(screensaver) = self.screensaver.as_mut()
                    && let Some(layer) = screensaver.layers.get_mut(layer_idx)
                {
                    layer.gpu_state = Some(gpu_state);
                    tracing::info!(
                        output = ?layer.output_info.name,
                        "Initialized screensaver shader layer"
                    );
                }
            }
            None => {
                if shader_source.is_some() {
                    tracing::warn!(
                        output = ?output_info.name,
                        "Screensaver shader setup failed; falling back to black"
                    );
                }
                self.attach_black_buffer(layer_idx);
            }
        }

        // Kick off the frame loop for this surface. Black layers need this as
        // much as shader layers do: it is what drives their fade.
        wl_surface.frame(&self.qh, wl_surface.clone());
        if let Some(screensaver) = self.screensaver.as_ref()
            && let Some(layer) = screensaver.layers.get(layer_idx)
        {
            layer.layer.commit();
        }
    }

    /// Attach an opaque black single-pixel buffer, scaled to fill the output by
    /// the viewport. This is how a black screensaver (and a screensaver past its
    /// runtime cap) paints without allocating a full-resolution buffer.
    fn attach_black_buffer(&mut self, layer_idx: usize) {
        let Some(manager) = self.single_pixel_buffer.clone() else {
            tracing::warn!(
                "Compositor does not advertise wp_single_pixel_buffer_manager_v1; \
                 black screensaver surface will be empty"
            );
            return;
        };

        let qh = self.qh.clone();
        let Some(screensaver) = self.screensaver.as_mut() else {
            return;
        };
        let Some(layer) = screensaver.layers.get_mut(layer_idx) else {
            return;
        };

        if layer.black_buffer.is_some() {
            return;
        }

        // Opaque black; the fade is applied separately by the alpha modifier.
        let buffer = manager.create_u32_rgba_buffer(0, 0, 0, u32::MAX, &qh, ());
        let surface = layer.wl_surface().clone();
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        layer.black_buffer = Some(buffer);
        layer.gpu_state = None;
        layer.layer.commit();

        tracing::debug!(output = ?layer.output_info.name, "Attached black screensaver buffer");
    }

    /// Schedule the next frame callback for a shader surface after `delay`.
    ///
    /// A one-shot timer requests the actual `wl_surface.frame` callback when
    /// the shader's frame interval has elapsed, so the process wakes at the
    /// shader's frame rate instead of the output's refresh rate. Rendering
    /// happens when the compositor answers the callback, which also inherits
    /// its throttling (occluded/locked outputs answer at ~1 Hz or not at all).
    fn schedule_shader_frame(&mut self, surface: wl_surface::WlSurface, delay: Duration) {
        let cb_surface = surface.clone();
        let timer = calloop::timer::Timer::from_duration(delay);
        let inserted = self.loop_handle.insert_source(timer, move |_, _, state| {
            // The layer may have been torn down while the timer was pending
            // (config change, output removed); only re-arm if it still exists.
            // Screensaver surfaces count as alive even without gpu_state: black
            // layers still need callbacks to drive their fade.
            let alive = state.wallpapers.iter().any(|w| {
                w.layers
                    .iter()
                    .any(|l| l.gpu_state.is_some() && l.layer.wl_surface() == &cb_surface)
            }) || state.is_screensaver_surface(&cb_surface);
            if alive {
                cb_surface.frame(&state.qh, cb_surface.clone());
                cb_surface.commit();
            }
            calloop::timer::TimeoutAction::Drop
        });

        if inserted.is_err() {
            // Fall back to the previous per-vblank behavior rather than
            // letting the animation stall.
            tracing::error!("failed to schedule shader frame timer; re-arming immediately");
            surface.frame(&self.qh, surface.clone());
            surface.commit();
        }
    }

    /// Reapply frame rate settings based on current power state and config.
    /// Called when config changes or battery state changes.
    fn reapply_frame_rates(&mut self) {
        let on_battery = self
            .power_monitor
            .as_ref()
            .map(|pm| pm.current().on_battery)
            .unwrap_or(false);

        // Determine new frame rate override
        let new_override = if on_battery {
            self.power_saving_config.on_battery_action.frame_rate()
        } else {
            None // Restore to configured rate
        };

        // Check if override actually changed
        if new_override == self.current_frame_rate_override {
            return;
        }

        self.current_frame_rate_override = new_override;

        // Apply to all shader canvases
        for wallpaper in &mut self.wallpapers {
            for layer in &mut wallpaper.layers {
                if let Some(gpu_state) = &mut layer.gpu_state {
                    gpu_state.canvas.set_frame_rate_override(new_override);
                    tracing::info!(
                        output = ?layer.output_info.name,
                        override_fps = ?new_override,
                        configured_fps = gpu_state.canvas.configured_frame_rate(),
                        "Updated shader frame rate"
                    );
                }
            }
        }
    }

    /// Called when power state changes (from D-Bus notification).
    /// This handles resuming from paused state and updating frame rates.
    fn on_power_state_changed(&mut self) {
        // Use the tracked state from before the power change occurred.
        // Note: power_monitor already has the NEW state when this is called,
        // so we can't compute was_paused from current state.
        let was_paused = self.was_animation_paused;

        // Reapply frame rates based on new power state
        self.reapply_frame_rates();

        let is_paused = self.should_pause_animation();

        // If we were paused and now we're not, request frame callbacks to resume
        if was_paused && !is_paused {
            tracing::info!("Resuming shader animation after power state change");
            self.was_animation_paused = false;
            self.request_frame_callbacks();
        }
    }

    /// Request frame callbacks for all shader layers.
    /// Used to resume animation after being paused.
    fn request_frame_callbacks(&mut self) {
        let qh = self.qh.clone();
        for wallpaper in &mut self.wallpapers {
            for layer in &mut wallpaper.layers {
                if layer.gpu_state.is_some() {
                    let wl_surface = layer.layer.wl_surface();
                    wl_surface.frame(&qh, wl_surface.clone());
                    layer.layer.commit();
                }
            }
        }
    }

    /// Save the list of currently connected outputs to state.
    /// This allows the settings app to know which displays are currently available.
    fn save_connected_outputs(&self) {
        let connected: Vec<String> = self
            .active_outputs
            .iter()
            .filter_map(|o| self.output_state.info(o))
            .filter_map(|info| info.name.clone())
            .collect();

        if let Ok(state_helper) = State::state() {
            let mut state = State::get_entry(&state_helper).unwrap_or_default();
            if state.connected_outputs != connected {
                state.connected_outputs = connected;
                if let Err(err) = state.write_entry(&state_helper) {
                    tracing::error!("Failed to save connected outputs: {err}");
                } else {
                    tracing::debug!(outputs = ?state.connected_outputs, "Saved connected outputs to state");
                }
            }
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

    fn shader_layer_physical_size(layer: &GlowBerryLayer) -> (u32, u32) {
        let output_mode_dims = layer
            .output_info
            .modes
            .iter()
            .find(|m| m.current)
            .map(|m| (m.dimensions.0 as u32, m.dimensions.1 as u32));

        Self::shader_physical_size(layer.size, layer.fractional_scale, output_mode_dims)
    }

    /// Apply a render scale (0.25-1.0) to a physical buffer size. The
    /// compositor upscales the smaller buffer to the surface's logical size
    /// via wp_viewport, cutting fragment work by scale².
    fn scaled_buffer_size((w, h): (u32, u32), render_scale: f32) -> (u32, u32) {
        let scale = render_scale.clamp(0.25, 1.0);
        (
            ((w as f32 * scale).round() as u32).max(1),
            ((h as f32 * scale).round() as u32).max(1),
        )
    }

    fn update_shader_layer_surface(
        gpu: &gpu::GpuRenderer,
        qh: &QueueHandle<Self>,
        layer: &mut GlowBerryLayer,
    ) {
        let physical = Self::shader_layer_physical_size(layer);
        let Some(gpu_state) = layer.gpu_state.as_mut() else {
            return;
        };
        let (physical_w, physical_h) = Self::scaled_buffer_size(physical, gpu_state.render_scale);

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

        // Release the GPU renderer entirely when no shader wallpaper remains,
        // freeing the Vulkan device, its driver threads, and VRAM.
        // It is recreated lazily if a shader wallpaper is applied again.
        if self.gpu_renderer.is_some() && !self.wallpapers.iter().any(Wallpaper::is_shader) {
            tracing::info!("No shader wallpapers remain; releasing GPU renderer");
            self.gpu_renderer = None;
        }
    }

    #[must_use]
    pub fn new_layer(&self, output: WlOutput, output_info: OutputInfo) -> GlowBerryLayer {
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

        GlowBerryLayer {
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

    /// Build the wgpu surface, swapchain and canvas for a shader layer.
    ///
    /// Shared by wallpaper layers and screensaver layers so both get identical
    /// render-scale handling, initial-frame behaviour and failure semantics.
    /// Returns `None` if the shader could not be compiled, leaving the caller to
    /// decide on a fallback.
    pub(crate) fn create_gpu_state(
        gpu: &gpu::GpuRenderer,
        connection: &Connection,
        wl_surface: &wl_surface::WlSurface,
        output_info: &OutputInfo,
        layer_size: Option<(u32, u32)>,
        fractional_scale: Option<u32>,
        shader_source: &glowberry_config::ShaderSource,
    ) -> Option<GpuLayerState> {
        let output_name = output_info.name.clone();

        // Get native resolution from the current output mode
        let native = output_info
            .modes
            .iter()
            .find(|m| m.current)
            .map(|m| (m.dimensions.0 as u32, m.dimensions.1 as u32))
            .unwrap_or_else(|| {
                // Fallback to layer size with scale if no mode info
                let (w, h) = layer_size.unwrap_or((1920, 1080));
                let scale = fractional_scale.unwrap_or(120);
                (w * scale / 120, h * scale / 120)
            });

        // Render at a fraction of native resolution when configured; the
        // compositor upscales via wp_viewport (set_destination by the caller).
        let render_scale = shader_source.render_scale.clamp(0.25, 1.0);
        let (physical_width, physical_height) = Self::scaled_buffer_size(native, render_scale);

        tracing::debug!(
            output = ?output_name,
            physical_width,
            physical_height,
            render_scale,
            "GPU layer dimensions"
        );

        // Create GPU surface
        let surface = unsafe { gpu.create_surface(connection, wl_surface) };

        // Configure surface at the (possibly scaled) render resolution
        let surface_config = gpu.configure_surface(&surface, physical_width, physical_height);

        // Create fragment canvas
        match fragment_canvas::FragmentCanvas::new(gpu, shader_source, surface_config.format) {
            Ok(mut canvas) => {
                canvas.update_resolution(gpu.queue(), physical_width, physical_height);

                // Render the first frame immediately to avoid showing default wallpaper
                if let wgpu::CurrentSurfaceTexture::Success(surface_texture) =
                    surface.get_current_texture()
                {
                    let view = surface_texture
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    canvas.render(gpu, &view);
                    surface_texture.present();
                    canvas.mark_frame_rendered();
                    tracing::debug!(output = ?output_name, "Rendered initial shader frame");
                }

                Some(GpuLayerState {
                    surface,
                    surface_config,
                    canvas,
                    render_scale,
                })
            }
            Err(err) => {
                tracing::error!(?err, "Failed to create fragment canvas");
                None
            }
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
            match gpu::GpuRenderer::new() {
                Ok(renderer) => self.gpu_renderer = Some(renderer),
                Err(err) => {
                    tracing::error!(
                        ?err,
                        "GPU initialization failed — cannot render shader wallpaper"
                    );
                    return;
                }
            }
        }

        let gpu = self.gpu_renderer.as_ref().unwrap();

        // Get layer info needed for surface creation
        let layer = &self.wallpapers[wallpaper_idx].layers[layer_idx];
        let wl_surface = layer.layer.wl_surface().clone();
        let output_name = layer.output_info.name.clone();

        let gpu_state = Self::create_gpu_state(
            gpu,
            &self.connection,
            &wl_surface,
            &layer.output_info,
            layer.size,
            layer.fractional_scale,
            shader_source,
        );

        let Some(gpu_state) = gpu_state else {
            tracing::error!(
                output = ?output_name,
                "Failed to create fragment canvas for shader wallpaper"
            );
            return;
        };

        let layer = &mut self.wallpapers[wallpaper_idx].layers[layer_idx];
        layer.gpu_state = Some(gpu_state);

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

    /// Hot-reload a shader by rebuilding the FragmentCanvas for all layers of a wallpaper.
    /// Keeps the existing surface and surface_config; only replaces the canvas.
    /// On failure, keeps the previous (working) canvas.
    fn reload_shader(&mut self, wallpaper_idx: usize) {
        let Some(gpu) = self.gpu_renderer.as_ref() else {
            return;
        };

        let shader_source = match &self.wallpapers[wallpaper_idx].entry.source {
            Source::Shader(s) => s.clone(),
            _ => return,
        };

        for layer_idx in 0..self.wallpapers[wallpaper_idx].layers.len() {
            let layer = &mut self.wallpapers[wallpaper_idx].layers[layer_idx];
            let Some(gpu_state) = layer.gpu_state.as_mut() else {
                continue;
            };

            match fragment_canvas::FragmentCanvas::new(
                gpu,
                &shader_source,
                gpu_state.surface_config.format,
            ) {
                Ok(canvas) => {
                    canvas.update_resolution(
                        gpu.queue(),
                        gpu_state.surface_config.width,
                        gpu_state.surface_config.height,
                    );
                    gpu_state.canvas = canvas;
                    tracing::info!(
                        output = ?layer.output_info.name,
                        "Hot-reloaded shader"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        ?err,
                        output = ?layer.output_info.name,
                        "Shader hot-reload failed, keeping previous version"
                    );
                }
            }
        }
    }
}

impl CompositorHandler for GlowBerry {
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
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // Screensaver surfaces have their own render path: they advance a fade,
        // may be black rather than shader-backed, and can outlive their renderer
        // once the runtime cap hits.
        if self.is_screensaver_surface(surface) {
            if let Some(delay) = self.render_screensaver_frame(surface) {
                self.schedule_shader_frame(surface.clone(), delay);
            }
            return;
        }

        // Check if animation should be paused due to power state.
        // (Battery-state transitions themselves are handled event-driven via
        // the D-Bus notification channel -> on_power_state_changed.)
        let should_pause = self.should_pause_animation();

        // When set, schedule the next frame callback after this delay.
        let mut next_frame: Option<Duration> = None;

        // Find the wallpaper and layer for this surface
        for wallpaper in &mut self.wallpapers {
            if let Some(layer) = wallpaper
                .layers
                .iter_mut()
                .find(|l| l.layer.wl_surface() == surface)
            {
                // Check if this is a shader wallpaper with GPU state
                if let Some(gpu_state) = &mut layer.gpu_state {
                    if !should_pause {
                        // Check if we should render this frame (frame rate limiting)
                        if gpu_state.canvas.should_render()
                            && let Some(gpu) = &self.gpu_renderer
                        {
                            // Get current texture
                            match gpu_state.surface.get_current_texture() {
                                wgpu::CurrentSurfaceTexture::Success(surface_texture)
                                | wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => {
                                    let view = surface_texture
                                        .texture
                                        .create_view(&wgpu::TextureViewDescriptor::default());

                                    tracing::trace!(
                                        output = ?layer.output_info.name,
                                        width = gpu_state.surface_config.width,
                                        height = gpu_state.surface_config.height,
                                        "Rendering shader frame"
                                    );

                                    // Render the shader
                                    gpu_state.canvas.render(gpu, &view);

                                    // Present
                                    surface_texture.present();

                                    gpu_state.canvas.mark_frame_rendered();
                                }
                                wgpu::CurrentSurfaceTexture::Timeout => {
                                    tracing::warn!("GPU surface timeout");
                                }
                                wgpu::CurrentSurfaceTexture::Lost
                                | wgpu::CurrentSurfaceTexture::Outdated => {
                                    let width = gpu_state.surface_config.width;
                                    let height = gpu_state.surface_config.height;
                                    gpu_state.surface_config =
                                        gpu.configure_surface(&gpu_state.surface, width, height);
                                    gpu_state
                                        .canvas
                                        .update_resolution(gpu.queue(), width, height);
                                    tracing::warn!(
                                        "GPU surface lost or outdated; reconfigured surface"
                                    );
                                }
                                other => {
                                    tracing::warn!(?other, "GPU surface error");
                                }
                            }
                        }

                        // Pace the next frame with a timer instead of re-arming
                        // the callback immediately: the process then sleeps
                        // between frames rather than waking on every vblank.
                        next_frame = Some(gpu_state.canvas.next_frame_delay());
                    } else {
                        // Track that we're paused so on_power_state_changed can resume us
                        self.was_animation_paused = true;
                        tracing::debug!(output = ?layer.output_info.name, "Shader paused, not requesting frame callback");
                    }
                }
                break;
            }
        }

        if let Some(delay) = next_frame {
            self.schedule_shader_frame(surface.clone(), delay);
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
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

impl OutputHandler for GlowBerry {
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

        // Update connected outputs in state for settings app
        self.save_connected_outputs();
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
            // Still try to save connected outputs even if we can't get info
            self.save_connected_outputs();
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

        // Update connected outputs in state for settings app
        self.save_connected_outputs();

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

impl LayerShellHandler for GlowBerry {
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

        // If the compositor closed a screensaver surface, drop the whole
        // screensaver: a partially-covered screen is worse than none, and the
        // next idle notification will build a fresh set.
        if let Some(screensaver) = self.screensaver.as_ref()
            && screensaver.layers.iter().any(|l| &l.layer == dropped_layer)
        {
            tracing::debug!("Compositor closed a screensaver surface; tearing down");
            self.teardown_screensaver();
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
        let span = tracing::debug_span!("<GlowBerry as LayerShellHandler>::configure");
        let _handle = span.enter();

        let (w, h) = configure.new_size;

        // Screensaver layers are configured separately: their contents depend on
        // whether the layer is shader-backed or black, not on a Wallpaper entry.
        if let Some(screensaver) = self.screensaver.as_mut()
            && let Some(layer_idx) = screensaver.layers.iter().position(|l| &l.layer == layer)
        {
            let needs_init = {
                let s_layer = &mut screensaver.layers[layer_idx];
                let first_configure = s_layer.size.is_none();
                let resized = s_layer.size != Some((w, h));
                s_layer.size = Some((w, h));
                first_configure || resized
            };

            if needs_init {
                self.reconfigure_screensaver_layer(layer_idx);
            }
            return;
        }

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

impl ShmHandler for GlowBerry {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

impl SeatHandler for GlowBerry {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
        // The idle notification is bound to a seat, so it can only be created
        // once one exists. Startup usually races ahead of this.
        if self.idle_notification.is_none() {
            self.refresh_idle_notification();
        }
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        // Keyboard and pointer exist purely to dismiss the screensaver. They are
        // still created when the screensaver is disabled — cheap, and it avoids
        // a race where enabling it at runtime leaves us without input.
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => {
                match self.seat_state.get_keyboard(qh, &seat, None) {
                    Ok(keyboard) => self.keyboard = Some(keyboard),
                    Err(err) => tracing::warn!(?err, "Failed to obtain keyboard"),
                }
            }
            Capability::Pointer if self.pointer.is_none() => {
                match self.seat_state.get_pointer(qh, &seat) {
                    Ok(pointer) => self.pointer = Some(pointer),
                    Err(err) => tracing::warn!(?err, "Failed to obtain pointer"),
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard => {
                if let Some(keyboard) = self.keyboard.take() {
                    keyboard.release();
                }
            }
            Capability::Pointer => {
                if let Some(pointer) = self.pointer.take() {
                    pointer.release();
                }
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
        // The idle notification was tied to this seat; rebuild against whatever
        // seat remains, if any.
        self.refresh_idle_notification();
    }
}

impl KeyboardHandler for GlowBerry {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
        // The keystroke is consumed here rather than reaching the focused
        // application, which is the point of the Exclusive keyboard grab.
        self.dismiss_screensaver_on(DismissEvent::Key);
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _raw_modifiers: RawModifiers,
        _layout: u32,
    ) {
    }
}

impl PointerHandler for GlowBerry {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        if self.screensaver.is_none() {
            return;
        }

        for event in events {
            tracing::trace!(kind = ?event.kind, position = ?event.position, "Screensaver pointer event");
            match event.kind {
                PointerEventKind::Motion { .. } => {
                    self.dismiss_screensaver_on(DismissEvent::MouseMove);
                }
                PointerEventKind::Press { .. } => {
                    self.dismiss_screensaver_on(DismissEvent::MouseClick);
                }
                _ => continue,
            }

            // One dismissal is enough; the rest of the frame is redundant.
            if self
                .screensaver
                .as_ref()
                .is_some_and(super::screensaver::ScreensaverState::is_dismissing)
            {
                return;
            }
        }
    }
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for GlowBerry {
    fn event(
        state: &mut GlowBerry,
        notification: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<GlowBerry>,
    ) {
        // Ignore events from a notification we have already replaced: a stale
        // object can still deliver one queued event after destroy().
        if state.idle_notification.as_ref() != Some(notification) {
            return;
        }

        match event {
            ext_idle_notification_v1::Event::Idled => {
                tracing::debug!("Seat went idle");
                state.activate_screensaver();
            }
            ext_idle_notification_v1::Event::Resumed => {
                tracing::debug!("Seat resumed");
                // Dismiss even if no input reached our surfaces — the session may
                // have locked, in which case cosmic-greeter has the input and we
                // would otherwise render behind it forever.
                state.dismiss_screensaver();
            }
            _ => {}
        }
    }
}

delegate_compositor!(GlowBerry);
delegate_output!(GlowBerry);
delegate_shm!(GlowBerry);
delegate_layer!(GlowBerry);
delegate_registry!(GlowBerry);
delegate_seat!(GlowBerry);
delegate_keyboard!(GlowBerry);
delegate_pointer!(GlowBerry);
delegate_noop!(GlowBerry: wp_viewporter::WpViewporter);
delegate_noop!(GlowBerry: wp_viewport::WpViewport);
delegate_noop!(GlowBerry: wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(GlowBerry: ignore wl_buffer::WlBuffer);
delegate_noop!(GlowBerry: ext_idle_notifier_v1::ExtIdleNotifierV1);
delegate_noop!(GlowBerry: wp_alpha_modifier_v1::WpAlphaModifierV1);
delegate_noop!(GlowBerry: wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1);
delegate_noop!(GlowBerry: wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1);

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, Weak<wl_surface::WlSurface>>
    for GlowBerry
{
    fn event(
        state: &mut GlowBerry,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        surface: &Weak<wl_surface::WlSurface>,
        _: &Connection,
        _: &QueueHandle<GlowBerry>,
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
                                GlowBerry::update_shader_layer_surface(gpu, &qh, layer);
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

impl ProvidesRegistryState for GlowBerry {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

#[cfg(test)]
mod tests {
    use super::GlowBerry;

    #[test]
    fn shader_physical_size_prefers_layer_size_over_mode() {
        let size = Some((100, 50));
        let scale = Some(150);
        let mode = Some((1920, 1080));

        let result = GlowBerry::shader_physical_size(size, scale, mode);

        assert_eq!(result, (125, 62));
    }

    #[test]
    fn shader_physical_size_uses_mode_when_size_missing() {
        let result = GlowBerry::shader_physical_size(None, Some(150), Some((1280, 720)));

        assert_eq!(result, (1280, 720));
    }

    #[test]
    fn shader_physical_size_defaults_scale_to_120() {
        let result = GlowBerry::shader_physical_size(Some((1200, 800)), None, Some((640, 480)));

        assert_eq!(result, (1200, 800));
    }
}
