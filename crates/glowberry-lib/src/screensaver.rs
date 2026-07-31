// SPDX-License-Identifier: MPL-2.0

//! Idle-triggered shader screensaver.
//!
//! # How this fits into COSMIC
//!
//! GlowBerry's screensaver is a set of `wlr-layer-shell` surfaces on the
//! `overlay` layer, raised when `ext-idle-notify-v1` reports the seat idle and
//! torn down on the first input event. It is deliberately *not* a screen
//! locker: `ext-session-lock-v1` admits one client and `cosmic-greeter` holds
//! it. Once the session locks, cosmic-comp stops rendering ordinary clients
//! entirely, so these surfaces simply stop being drawn.
//!
//! The handoff is therefore cooperative rather than negotiated:
//!
//! ```text
//! 0s          our timeout        cosmic-idle's screen_off_time
//! |               |                        |----5s fade---|--0.5s--|
//! last input   screensaver up          fade to black    DPMS off   lock
//! ```
//!
//! Two consequences drive the design:
//!
//! 1. **We must never inhibit idle.** `cosmic-idle` treats an
//!    `org.freedesktop.ScreenSaver` inhibit as a reason to disable *both* its
//!    timeouts, which would cost the user the fade, the screen-off and the
//!    lock. So this module holds no inhibitor of any kind.
//! 2. **Our timeout has to stay below `screen_off_time`.** cosmic-idle's fade
//!    surface is also on the `overlay` layer, and z-order within a layer is
//!    insertion-ordered in smithay rather than specified by the protocol. It
//!    lands above us today because it is created later, but relying on that is
//!    not something the protocol guarantees — so we also self-destruct at
//!    screen-off time rather than trusting the stacking.
//!
//! # Fades
//!
//! Fading is done with `wp_alpha_modifier_v1`, which asks the compositor to
//! multiply the surface's alpha. That keeps the fade out of the shader: the
//! WGSL preamble in [`crate::shader_defs`] is a contract shared with the
//! settings-app preview renderer, and existing user shaders would ignore a new
//! uniform anyway. When the protocol is unavailable, fades are skipped.

use std::time::{Duration, Instant};

use glowberry_config::{
    ShaderSource, Source,
    screensaver::{DismissEvent, ScreensaverSource},
};
use sctk::{
    output::OutputInfo,
    reexports::{
        calloop,
        client::protocol::{wl_buffer, wl_output::WlOutput, wl_surface},
        protocols::wp::{
            alpha_modifier::v1::client::wp_alpha_modifier_surface_v1,
            viewporter::client::wp_viewport,
        },
    },
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerSurface},
    },
};

use crate::{
    engine::{GlowBerry, GpuLayerState},
    screensaver_inhibit::Inhibitor,
};

/// Where the screensaver's fade is in its lifecycle.
#[derive(Debug, Clone, Copy)]
pub(crate) enum FadePhase {
    /// Ramping alpha up from zero over `fade_in_ms`.
    FadingIn { start: Instant },
    /// Fully opaque.
    Active,
    /// Ramping alpha down to zero. `start_alpha` is the alpha at the moment
    /// dismissal began, so interrupting a fade-in does not cause a visible pop.
    FadingOut { start: Instant, start_alpha: f32 },
}

impl FadePhase {
    /// Current alpha in `[0.0, 1.0]`, and whether the phase has finished.
    ///
    /// A zero-length fade resolves immediately rather than dividing by zero.
    pub(crate) fn alpha(self, fade_in: Duration, fade_out: Duration) -> (f32, bool) {
        match self {
            Self::FadingIn { start } => {
                if fade_in.is_zero() {
                    return (1.0, true);
                }
                let progress = progress_of(start, fade_in);
                (progress, progress >= 1.0)
            }
            Self::Active => (1.0, true),
            Self::FadingOut { start, start_alpha } => {
                if fade_out.is_zero() {
                    return (0.0, true);
                }
                let progress = progress_of(start, fade_out);
                (start_alpha * (1.0 - progress), progress >= 1.0)
            }
        }
    }
}

/// Elapsed fraction of `duration` since `start`, clamped to `[0.0, 1.0]`.
fn progress_of(start: Instant, duration: Duration) -> f32 {
    let elapsed = start.elapsed().as_secs_f32();
    let total = duration.as_secs_f32();
    if total <= 0.0 {
        return 1.0;
    }
    (elapsed / total).clamp(0.0, 1.0)
}

/// One screensaver surface, covering a single output.
pub(crate) struct ScreensaverLayer {
    pub(crate) layer: LayerSurface,
    pub(crate) viewport: wp_viewport::WpViewport,
    /// Compositor-side alpha multiplier, when `wp_alpha_modifier_v1` is available.
    pub(crate) alpha_surface: Option<wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1>,
    pub(crate) output_info: OutputInfo,
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) fractional_scale: Option<u32>,
    /// GPU state, absent for black layers and dropped once the runtime cap hits.
    pub(crate) gpu_state: Option<GpuLayerState>,
    /// The shader this layer renders, or `None` for a black layer.
    pub(crate) shader_source: Option<ShaderSource>,
    /// Opaque black single-pixel buffer, kept alive while attached.
    pub(crate) black_buffer: Option<wl_buffer::WlBuffer>,
    /// Alpha last sent to the compositor, to avoid redundant commits.
    pub(crate) last_alpha: Option<f32>,
}

impl std::fmt::Debug for ScreensaverLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreensaverLayer")
            .field("output", &self.output_info.name)
            .field("size", &self.size)
            .field("has_gpu_state", &self.gpu_state.is_some())
            .finish_non_exhaustive()
    }
}

impl ScreensaverLayer {
    /// Ask the compositor to multiply this surface's alpha by `alpha`.
    ///
    /// Returns `true` if a value was actually sent. The multiplier is
    /// double-buffered surface state, so it takes effect at the next commit —
    /// callers must commit (or present) afterwards.
    pub(crate) fn set_alpha(&mut self, alpha: f32) -> bool {
        let Some(alpha_surface) = self.alpha_surface.as_ref() else {
            return false;
        };

        let clamped = alpha.clamp(0.0, 1.0);
        // Skip sub-1/255 changes: they are imperceptible and each one costs a
        // roundtrip plus a compositor repaint.
        if let Some(last) = self.last_alpha
            && (last - clamped).abs() < 1.0 / 255.0
            && (clamped - 1.0).abs() > f32::EPSILON
        {
            return false;
        }

        // Protocol: 0 is fully transparent, u32::MAX fully opaque.
        let factor = (f64::from(clamped) * f64::from(u32::MAX)) as u32;
        alpha_surface.set_multiplier(factor);
        self.last_alpha = Some(clamped);
        true
    }

    /// True when this layer has been configured and has a usable size.
    pub(crate) fn is_configured(&self) -> bool {
        self.size.is_some()
    }

    pub(crate) fn wl_surface(&self) -> &wl_surface::WlSurface {
        self.layer.wl_surface()
    }
}

impl Drop for ScreensaverLayer {
    fn drop(&mut self) {
        // Order matters: the alpha modifier and viewport are surface add-ons and
        // must go before the surface they decorate.
        if let Some(alpha_surface) = self.alpha_surface.take() {
            alpha_surface.destroy();
        }
        if let Some(buffer) = self.black_buffer.take() {
            buffer.destroy();
        }
    }
}

/// Live screensaver state. Absent entirely when the screensaver is not showing.
pub(crate) struct ScreensaverState {
    pub(crate) layers: Vec<ScreensaverLayer>,
    pub(crate) phase: FadePhase,
    /// When the screensaver became visible, for the runtime cap.
    pub(crate) started: Instant,
    /// Set once the runtime cap has elapsed and GPU rendering was released.
    pub(crate) render_released: bool,
    /// When cosmic-idle is expected to start its fade-to-black, at which point
    /// we get out of the way. `None` when cosmic-idle's screen-off is disabled
    /// or unreadable, in which case nothing else is going to claim the screen.
    pub(crate) screen_off_deadline: Option<Instant>,
    /// Timer that fires at `screen_off_deadline`. Needed because a fully
    /// faded-in screensaver stops requesting frame callbacks, so nothing else
    /// would be running to notice the deadline pass.
    pub(crate) screen_off_timer: Option<calloop::RegistrationToken>,
}

impl std::fmt::Debug for ScreensaverState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreensaverState")
            .field("layers", &self.layers.len())
            .field("phase", &self.phase)
            .field("render_released", &self.render_released)
            .finish_non_exhaustive()
    }
}

impl ScreensaverState {
    /// True once dismissal has begun.
    pub(crate) fn is_dismissing(&self) -> bool {
        matches!(self.phase, FadePhase::FadingOut { .. })
    }
}

/// How long after the screensaver appears cosmic-idle is expected to begin its
/// fade-to-black.
///
/// `screen_off_ms` is measured from the last input event and `idle_timeout_ms`
/// is when we appear, so the gap between the two is what we get. Returns `None`
/// when we would already be past it — the caller warns about that separately
/// rather than tearing down instantly.
pub(crate) fn screen_off_deadline_from(
    screen_off_ms: Option<u32>,
    idle_timeout_ms: u32,
    now: Instant,
) -> Option<Instant> {
    let screen_off_ms = screen_off_ms?;
    let remaining_ms = screen_off_ms.checked_sub(idle_timeout_ms)?;
    if remaining_ms == 0 {
        return None;
    }
    Some(now + Duration::from_millis(u64::from(remaining_ms)))
}

/// What is asking for the screensaver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// The compositor reported the seat idle.
    Idle,
    /// The user asked for it outright, with `--screensaver`.
    Forced,
}

/// Why the screensaver is not allowed to appear right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Suppression {
    /// Turned off in settings.
    Disabled,
    /// Running on battery, which the user did not opt into.
    OnBattery,
    /// An app asked to keep the screen awake.
    ScreenWakeRequest,
}

/// Decide whether the screensaver may appear, given the current conditions.
///
/// Note there is no setting for `wake_requested`, unlike the battery rule:
/// covering a video the user is watching is never what they meant by enabling a
/// screensaver. It does not apply to [`Trigger::Forced`] though — someone
/// previewing a shader from the command line has said what they want, and a
/// video playing behind the preview does not change that.
fn screensaver_suppression(
    config: &glowberry_config::screensaver::ScreensaverConfig,
    on_battery: bool,
    wake_requested: bool,
    trigger: Trigger,
) -> Option<Suppression> {
    if !config.enabled {
        return Some(Suppression::Disabled);
    }
    if on_battery && !config.on_battery {
        return Some(Suppression::OnBattery);
    }
    if wake_requested && trigger == Trigger::Idle {
        return Some(Suppression::ScreenWakeRequest);
    }
    None
}

impl GlowBerry {
    /// Whether the screensaver may activate right now.
    ///
    /// Separate from [`Self::should_pause_animation`], which governs the
    /// wallpaper: pausing the wallpaper on battery is a sensible default,
    /// whereas suppressing the screensaver entirely is a distinct choice.
    fn screensaver_allowed(&self, trigger: Trigger) -> bool {
        let on_battery = self
            .power_monitor
            .as_ref()
            .is_some_and(|monitor| monitor.current().on_battery);
        let wake_request = self.screen_wake_request();

        match screensaver_suppression(
            &self.screensaver_config,
            on_battery,
            wake_request.is_some(),
            trigger,
        ) {
            None => true,
            Some(Suppression::Disabled) => false,
            Some(Suppression::OnBattery) => {
                tracing::debug!("Screensaver suppressed: on battery");
                false
            }
            Some(Suppression::ScreenWakeRequest) => {
                // Only reachable for the D-Bus kind: a Wayland idle inhibitor
                // keeps the compositor from reporting idle in the first place.
                if let Some(inhibitor) = wake_request.as_ref() {
                    tracing::debug!(
                        app = %inhibitor.app,
                        reason = %inhibitor.reason,
                        "Screensaver suppressed: an app asked to keep the screen awake"
                    );
                }
                false
            }
        }
    }

    /// One of the apps currently asking to keep the screen awake, if any.
    pub(crate) fn screen_wake_request(&self) -> Option<Inhibitor> {
        self.screensaver_inhibitors
            .as_ref()?
            .current()
            .into_iter()
            .next()
    }

    /// React to an app taking or releasing a D-Bus screen-wake request.
    ///
    /// Taking one while the screensaver is up fades it away, which is the point:
    /// video playback starting is exactly when the user does not want it. And
    /// releasing the last one while the seat is still idle raises the screensaver
    /// that was suppressed, so a finished video does not cost the user the
    /// screensaver for the rest of the night.
    pub(crate) fn on_screensaver_inhibitors_changed(&mut self) {
        // The settings app shows these, and cannot observe them for itself.
        self.save_screen_wake_requests();

        if let Some(inhibitor) = self.screen_wake_request() {
            if self.screensaver.is_some() {
                tracing::info!(
                    app = %inhibitor.app,
                    reason = %inhibitor.reason,
                    "An app asked to keep the screen awake; dismissing the screensaver"
                );
                self.dismiss_screensaver();
            }
            return;
        }

        if self.seat_idle && self.screensaver.is_none() {
            tracing::info!(
                "Screen-wake requests released while the seat is still idle; \
                 raising the screensaver"
            );
            self.activate_screensaver();
        }
    }

    /// Raise the screensaver because the seat went idle.
    ///
    /// Idempotent: a second call while the screensaver is already up is ignored,
    /// except that it cancels an in-progress fade-out.
    pub(crate) fn activate_screensaver(&mut self) {
        self.activate_screensaver_for(Trigger::Idle);
    }

    /// Raise the screensaver because the user asked for it directly.
    pub(crate) fn force_activate_screensaver(&mut self) {
        self.activate_screensaver_for(Trigger::Forced);
    }

    fn activate_screensaver_for(&mut self, trigger: Trigger) {
        if !self.screensaver_allowed(trigger) {
            return;
        }

        if let Some(screensaver) = self.screensaver.as_mut() {
            if screensaver.is_dismissing() {
                // Idle re-fired mid-dismissal; fade back in from where we are.
                let (alpha, _) = screensaver.phase.alpha(
                    Duration::from_millis(self.screensaver_config.fade_in_ms),
                    Duration::from_millis(self.screensaver_config.fade_out_ms),
                );
                let fade_in = Duration::from_millis(self.screensaver_config.fade_in_ms);
                // Rewind the fade-in start so the ramp resumes at `alpha`
                // instead of restarting from black.
                let offset = fade_in.mul_f32(alpha.clamp(0.0, 1.0));
                screensaver.phase = FadePhase::FadingIn {
                    start: Instant::now() - offset,
                };
                self.request_screensaver_frames();
            }
            return;
        }

        if self.active_outputs.is_empty() {
            tracing::debug!("Screensaver activation skipped: no active outputs");
            return;
        }

        // Warn once per activation rather than at config-load time: the user may
        // have changed cosmic-idle's setting since.
        if self.screensaver_config.exceeds_screen_off() == Some(true) {
            tracing::warn!(
                timeout_secs = self.screensaver_config.idle_timeout_secs,
                "Screensaver timeout is at or beyond cosmic-idle's screen_off_time; \
                 the screen will blank at about the same moment the screensaver appears"
            );
        }

        let fade_in_ms = self.screensaver_config.fade_in_ms;
        let mut layers = Vec::with_capacity(self.active_outputs.len());

        for output in self.active_outputs.clone() {
            let Some(output_info) = self.output_state.info(&output) else {
                continue;
            };
            let shader_source = self.screensaver_shader_for(&output_info);
            if let Some(layer) = self.new_screensaver_layer(&output, output_info, shader_source) {
                layers.push(layer);
            }
        }

        if layers.is_empty() {
            tracing::warn!("Screensaver activation produced no surfaces");
            return;
        }

        let now = Instant::now();
        let screen_off_deadline = screen_off_deadline_from(
            glowberry_config::screensaver::cosmic_idle_screen_off_ms(),
            self.screensaver_config.idle_timeout_ms(),
            now,
        );

        tracing::info!(
            outputs = layers.len(),
            fade_in_ms,
            screen_off_in = ?screen_off_deadline.map(|d| d.saturating_duration_since(now)),
            "Screensaver activated"
        );

        let screen_off_timer = screen_off_deadline.and_then(|deadline| {
            let timer = calloop::timer::Timer::from_deadline(deadline);
            self.loop_handle
                .insert_source(timer, |_, _, state: &mut GlowBerry| {
                    if state.screensaver.is_some() {
                        tracing::info!(
                            "Reached cosmic-idle's screen-off time; removing screensaver so \
                             the fade-to-black and lock proceed on a clean surface"
                        );
                        // The token is about to be invalidated by Drop, so clear
                        // it first to keep teardown from removing a dead source.
                        if let Some(screensaver) = state.screensaver.as_mut() {
                            screensaver.screen_off_timer = None;
                        }
                        state.teardown_screensaver();
                    }
                    calloop::timer::TimeoutAction::Drop
                })
                .inspect_err(|err| {
                    tracing::warn!(
                        ?err,
                        "Could not arm screen-off timer; screensaver will rely on \
                         frame callbacks to notice the deadline"
                    );
                })
                .ok()
        });

        self.screensaver = Some(ScreensaverState {
            layers,
            phase: if fade_in_ms == 0 {
                FadePhase::Active
            } else {
                FadePhase::FadingIn { start: now }
            },
            started: now,
            render_released: false,
            screen_off_deadline,
            screen_off_timer,
        });
    }

    /// True when cosmic-idle should be starting its fade about now, meaning we
    /// should remove ourselves.
    pub(crate) fn screensaver_past_screen_off_deadline(&self) -> bool {
        self.screensaver
            .as_ref()
            .and_then(|s| s.screen_off_deadline)
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// Pick the shader a given output's screensaver should render.
    ///
    /// `SameAsWallpaper` resolves per output, so a mixed setup (shader on one
    /// monitor, photo on another) yields a shader on the first and black on the
    /// second rather than failing outright.
    fn screensaver_shader_for(&self, output_info: &OutputInfo) -> Option<ShaderSource> {
        match &self.screensaver_config.source {
            ScreensaverSource::Black => None,
            ScreensaverSource::Shader(shader) => Some(shader.clone()),
            ScreensaverSource::SameAsWallpaper => {
                let name = output_info.name.clone().unwrap_or_default();

                // Prefer an output-specific wallpaper, then the catch-all one,
                // matching how apply_backgrounds assigns layers.
                let specific = self
                    .wallpapers
                    .iter()
                    .find(|w| w.entry.output == name)
                    .and_then(|w| match &w.entry.source {
                        Source::Shader(shader) => Some(shader.clone()),
                        _ => None,
                    });

                specific.or_else(|| {
                    self.wallpapers
                        .iter()
                        .find(|w| w.entry.output == "all")
                        .and_then(|w| match &w.entry.source {
                            Source::Shader(shader) => Some(shader.clone()),
                            _ => None,
                        })
                })
            }
        }
    }

    /// Create one overlay surface for `output`.
    fn new_screensaver_layer(
        &self,
        output: &WlOutput,
        output_info: OutputInfo,
        shader_source: Option<ShaderSource>,
    ) -> Option<ScreensaverLayer> {
        let surface = self.compositor_state.create_surface(&self.qh);

        let layer = self.layer_state.create_layer_surface(
            &self.qh,
            surface.clone(),
            Layer::Overlay,
            Some("glowberry-screensaver"),
            Some(output),
        );

        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        // Exclusive so the keystroke that dismisses the screensaver is consumed
        // here instead of reaching whatever application had focus. Without this
        // the user types blind into their editor while the shader covers it.
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);

        let alpha_surface = self
            .alpha_modifier
            .as_ref()
            .map(|manager| manager.get_surface(&surface, &self.qh, ()));

        // Start fully transparent when fading in, so the first commit does not
        // flash the shader at full opacity before the ramp begins.
        let mut layer = ScreensaverLayer {
            layer,
            viewport: self.viewporter.get_viewport(&surface, &self.qh, ()),
            alpha_surface,
            output_info,
            size: None,
            fractional_scale: None,
            gpu_state: None,
            shader_source,
            black_buffer: None,
            last_alpha: None,
        };

        if self.screensaver_config.fade_in_ms > 0 {
            layer.set_alpha(0.0);
        }

        surface.commit();

        Some(layer)
    }

    /// Begin dismissing the screensaver, fading out first if configured.
    pub(crate) fn dismiss_screensaver(&mut self) {
        let Some(screensaver) = self.screensaver.as_mut() else {
            return;
        };

        if screensaver.is_dismissing() {
            return;
        }

        let fade_out_ms = self.screensaver_config.fade_out_ms;
        if fade_out_ms == 0 {
            tracing::info!("Screensaver dismissed");
            self.teardown_screensaver();
            return;
        }

        let fade_in = Duration::from_millis(self.screensaver_config.fade_in_ms);
        let fade_out = Duration::from_millis(fade_out_ms);
        let (start_alpha, _) = screensaver.phase.alpha(fade_in, fade_out);

        screensaver.phase = FadePhase::FadingOut {
            start: Instant::now(),
            start_alpha,
        };

        tracing::debug!(fade_out_ms, start_alpha, "Screensaver fading out");

        // The screensaver may have been sitting idle with no pending frame
        // callbacks (static black, or a released renderer), so the fade needs
        // its own kick to start progressing.
        self.request_screensaver_frames();
    }

    /// Dismiss a dismissal that is triggered by an input event, honouring the
    /// user's `dismiss_on` selection.
    pub(crate) fn dismiss_screensaver_on(&mut self, event: DismissEvent) {
        // Pointer motion arrives as a stream of frames, so without this the
        // whole fade-out would be logged once per mouse event.
        let showing = self
            .screensaver
            .as_ref()
            .is_some_and(|s| !s.is_dismissing());
        if !showing {
            return;
        }

        if !self.screensaver_config.dismiss_on.contains(&event) {
            return;
        }

        tracing::debug!(?event, "Screensaver dismissal triggered by input");
        self.dismiss_screensaver();
    }

    /// Destroy the screensaver surfaces and release their GPU resources.
    pub(crate) fn teardown_screensaver(&mut self) {
        let Some(mut screensaver) = self.screensaver.take() else {
            return;
        };

        if let Some(token) = screensaver.screen_off_timer.take() {
            self.loop_handle.remove(token);
        }

        let count = screensaver.layers.len();
        // ScreensaverLayer::drop tears down the per-surface add-ons; dropping
        // gpu_state releases the wgpu surface and swapchain.
        drop(screensaver);

        // The renderer may have been spun up purely for the screensaver (a user
        // with static wallpapers and a shader screensaver). Release it again so
        // the idle daemon does not hold a Vulkan device for nothing.
        if self.gpu_renderer.is_some() && !self.wallpapers.iter().any(crate::Wallpaper::is_shader) {
            tracing::info!("No shader wallpapers remain; releasing GPU renderer");
            self.gpu_renderer = None;
        }

        tracing::debug!(surfaces = count, "Screensaver torn down");
    }

    /// Request a frame callback on every screensaver surface.
    pub(crate) fn request_screensaver_frames(&mut self) {
        let Some(screensaver) = self.screensaver.as_ref() else {
            return;
        };

        for layer in &screensaver.layers {
            let surface = layer.wl_surface().clone();
            surface.frame(&self.qh, surface.clone());
            layer.layer.commit();
        }
    }

    /// True when `surface` belongs to the screensaver.
    pub(crate) fn is_screensaver_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        self.screensaver
            .as_ref()
            .is_some_and(|s| s.layers.iter().any(|l| l.wl_surface() == surface))
    }

    /// Whether the runtime cap has elapsed, meaning we should stop rendering.
    pub(crate) fn screensaver_runtime_expired(&self) -> bool {
        let Some(screensaver) = self.screensaver.as_ref() else {
            return false;
        };
        let Some(max_secs) = self.screensaver_config.max_runtime_secs else {
            return false;
        };

        screensaver.started.elapsed() >= Duration::from_secs(u64::from(max_secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glowberry_config::screensaver::ScreensaverConfig;

    const FADE_IN: Duration = Duration::from_millis(1000);
    const FADE_OUT: Duration = Duration::from_millis(300);

    #[test]
    fn fade_in_starts_transparent() {
        let phase = FadePhase::FadingIn {
            start: Instant::now(),
        };
        let (alpha, done) = phase.alpha(FADE_IN, FADE_OUT);
        assert!(alpha < 0.1, "expected near-zero alpha, got {alpha}");
        assert!(!done);
    }

    #[test]
    fn fade_in_completes_after_duration() {
        let phase = FadePhase::FadingIn {
            start: Instant::now() - FADE_IN,
        };
        let (alpha, done) = phase.alpha(FADE_IN, FADE_OUT);
        assert!((alpha - 1.0).abs() < f32::EPSILON);
        assert!(done);
    }

    #[test]
    fn zero_length_fade_in_resolves_immediately() {
        let phase = FadePhase::FadingIn {
            start: Instant::now(),
        };
        let (alpha, done) = phase.alpha(Duration::ZERO, FADE_OUT);
        assert!((alpha - 1.0).abs() < f32::EPSILON);
        assert!(done);
    }

    #[test]
    fn active_is_opaque() {
        let (alpha, done) = FadePhase::Active.alpha(FADE_IN, FADE_OUT);
        assert!((alpha - 1.0).abs() < f32::EPSILON);
        assert!(done);
    }

    #[test]
    fn fade_out_from_partial_alpha_does_not_pop() {
        // Interrupting a fade-in at 40% must fade out from 0.4, not from 1.0.
        let phase = FadePhase::FadingOut {
            start: Instant::now(),
            start_alpha: 0.4,
        };
        let (alpha, done) = phase.alpha(FADE_IN, FADE_OUT);
        assert!(alpha <= 0.4, "alpha {alpha} rose above the starting value");
        assert!(!done);
    }

    #[test]
    fn fade_out_completes_at_zero() {
        let phase = FadePhase::FadingOut {
            start: Instant::now() - FADE_OUT,
            start_alpha: 1.0,
        };
        let (alpha, done) = phase.alpha(FADE_IN, FADE_OUT);
        assert!(alpha.abs() < f32::EPSILON);
        assert!(done);
    }

    #[test]
    fn zero_length_fade_out_resolves_immediately() {
        let phase = FadePhase::FadingOut {
            start: Instant::now(),
            start_alpha: 1.0,
        };
        let (alpha, done) = phase.alpha(FADE_IN, Duration::ZERO);
        assert!(alpha.abs() < f32::EPSILON);
        assert!(done);
    }

    #[test]
    fn progress_of_clamps_to_one() {
        let progress = progress_of(Instant::now() - Duration::from_secs(10), FADE_IN);
        assert!((progress - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn progress_of_handles_zero_duration() {
        assert!((progress_of(Instant::now(), Duration::ZERO) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn dismissing_only_true_while_fading_out() {
        assert!(!matches!(FadePhase::Active, FadePhase::FadingOut { .. }));
        assert!(matches!(
            FadePhase::FadingOut {
                start: Instant::now(),
                start_alpha: 1.0
            },
            FadePhase::FadingOut { .. }
        ));
    }

    #[test]
    fn screen_off_deadline_is_the_gap_after_our_timeout() {
        let now = Instant::now();
        // cosmic-idle blanks at 30 min, we appear at 5 min => 25 min of runway.
        let deadline = screen_off_deadline_from(Some(1_800_000), 300_000, now)
            .expect("a deadline should exist");
        let gap = deadline.saturating_duration_since(now);
        assert_eq!(gap, Duration::from_millis(1_500_000));
    }

    #[test]
    fn no_deadline_when_screen_off_disabled() {
        assert!(screen_off_deadline_from(None, 300_000, Instant::now()).is_none());
    }

    #[test]
    fn no_deadline_when_our_timeout_is_later_than_screen_off() {
        // Misconfigured: we would appear after the screen already blanked.
        assert!(screen_off_deadline_from(Some(300_000), 1_800_000, Instant::now()).is_none());
    }

    #[test]
    fn no_deadline_when_timeouts_coincide() {
        assert!(screen_off_deadline_from(Some(300_000), 300_000, Instant::now()).is_none());
    }

    /// An enabled screensaver that does not mind battery power, so each test can
    /// vary only the condition it is about.
    fn enabled_config() -> ScreensaverConfig {
        ScreensaverConfig {
            enabled: true,
            on_battery: true,
            ..Default::default()
        }
    }

    #[test]
    fn nothing_in_the_way_allows_the_screensaver() {
        assert_eq!(
            screensaver_suppression(&enabled_config(), false, false, Trigger::Idle),
            None
        );
    }

    #[test]
    fn a_screen_wake_request_suppresses_the_screensaver() {
        // Chrome's video wake lock: the compositor still reports idle, so this
        // is the only thing keeping the screensaver off the video.
        assert_eq!(
            screensaver_suppression(&enabled_config(), false, true, Trigger::Idle),
            Some(Suppression::ScreenWakeRequest)
        );
    }

    #[test]
    fn a_screen_wake_request_wins_over_the_battery_opt_in() {
        // Opting into running on battery says nothing about wanting to cover a
        // video, so the wake request still suppresses.
        assert_eq!(
            screensaver_suppression(&enabled_config(), true, true, Trigger::Idle),
            Some(Suppression::ScreenWakeRequest)
        );
    }

    #[test]
    fn a_forced_preview_ignores_screen_wake_requests() {
        // `--screensaver` is someone asking to see the shader now; a video
        // playing behind it does not override that.
        assert_eq!(
            screensaver_suppression(&enabled_config(), false, true, Trigger::Forced),
            None
        );
    }

    #[test]
    fn a_forced_preview_still_respects_battery_and_the_switch() {
        let config = ScreensaverConfig {
            enabled: true,
            on_battery: false,
            ..Default::default()
        };
        assert_eq!(
            screensaver_suppression(&config, true, false, Trigger::Forced),
            Some(Suppression::OnBattery)
        );
        assert_eq!(
            screensaver_suppression(&ScreensaverConfig::default(), false, false, Trigger::Forced),
            Some(Suppression::Disabled)
        );
    }

    #[test]
    fn battery_suppresses_unless_opted_in() {
        let config = ScreensaverConfig {
            enabled: true,
            on_battery: false,
            ..Default::default()
        };
        assert_eq!(
            screensaver_suppression(&config, true, false, Trigger::Idle),
            Some(Suppression::OnBattery)
        );
        assert_eq!(
            screensaver_suppression(&config, false, false, Trigger::Idle),
            None
        );
    }

    #[test]
    fn disabled_reports_itself_rather_than_another_reason() {
        // The reason drives logging, so a switched-off screensaver should not
        // claim an app is keeping the screen awake.
        let config = ScreensaverConfig {
            enabled: false,
            ..enabled_config()
        };
        assert_eq!(
            screensaver_suppression(&config, true, true, Trigger::Idle),
            Some(Suppression::Disabled)
        );
    }

    #[test]
    fn config_defaults_keep_screensaver_off() {
        let config = ScreensaverConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.fade_in_ms, 1000);
        assert_eq!(config.fade_out_ms, 300);
    }
}
