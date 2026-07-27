// SPDX-License-Identifier: MPL-2.0

//! Screensaver configuration for GlowBerry.
//!
//! GlowBerry's screensaver is an idle-triggered shader overlay on the
//! `overlay` layer. It is deliberately *not* a screen locker: on COSMIC the
//! session lock belongs to `cosmic-greeter` via `ext-session-lock-v1`, and
//! `cosmic-idle` owns the fade-to-black, screen-off and lock sequence.
//!
//! That division of labour constrains the timeout. `cosmic-idle` starts its
//! 5-second fade at its own `screen_off_time` and then locks 500ms later, so a
//! screensaver timeout at or beyond that value never becomes visible. See
//! [`ScreensaverConfig::exceeds_screen_off`].

use cosmic_config::{Config as CosmicConfig, ConfigGet, ConfigSet};
use serde::{Deserialize, Serialize};

use crate::{Context, ShaderSource};

// Config keys
pub const ENABLED: &str = "screensaver-enabled";
pub const IDLE_TIMEOUT_SECS: &str = "screensaver-idle-timeout-secs";
pub const SOURCE: &str = "screensaver-source";
pub const FADE_IN_MS: &str = "screensaver-fade-in-ms";
pub const FADE_OUT_MS: &str = "screensaver-fade-out-ms";
pub const DISMISS_ON: &str = "screensaver-dismiss-on";
pub const MAX_RUNTIME_SECS: &str = "screensaver-max-runtime-secs";
pub const ON_BATTERY: &str = "screensaver-on-battery";

/// cosmic-idle's config namespace, read to sanity-check our timeout.
pub const COSMIC_IDLE_NAME: &str = "com.system76.CosmicIdle";
/// Key in cosmic-idle's config holding the screen-off idle time, in ms.
pub const COSMIC_IDLE_SCREEN_OFF_TIME: &str = "screen_off_time";

/// Default idle timeout before the screensaver appears, in seconds.
pub const DEFAULT_IDLE_TIMEOUT_SECS: u32 = 300;
/// Default fade-in duration, in milliseconds.
pub const DEFAULT_FADE_IN_MS: u64 = 1000;
/// Default fade-out duration, in milliseconds.
pub const DEFAULT_FADE_OUT_MS: u64 = 300;

/// What the screensaver draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum ScreensaverSource {
    /// Reuse each output's current wallpaper shader. Outputs whose wallpaper is
    /// a static image or colour fall back to [`Self::Black`].
    #[default]
    SameAsWallpaper,
    /// A dedicated shader, independent of the wallpaper.
    Shader(ShaderSource),
    /// Plain black — a blanker rather than a screensaver.
    Black,
}

/// An input event that dismisses the screensaver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DismissEvent {
    /// Any key press.
    Key,
    /// Pointer motion.
    MouseMove,
    /// Pointer button press.
    MouseClick,
}

/// The dismissal events enabled by default.
#[must_use]
pub fn default_dismiss_on() -> Vec<DismissEvent> {
    vec![
        DismissEvent::Key,
        DismissEvent::MouseMove,
        DismissEvent::MouseClick,
    ]
}

/// Screensaver configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreensaverConfig {
    /// Whether the screensaver is enabled at all.
    pub enabled: bool,
    /// Seconds of inactivity before the screensaver appears.
    pub idle_timeout_secs: u32,
    /// What the screensaver draws.
    pub source: ScreensaverSource,
    /// Fade-in duration in milliseconds (0 disables the fade).
    pub fade_in_ms: u64,
    /// Fade-out duration in milliseconds (0 dismisses immediately).
    pub fade_out_ms: u64,
    /// Input events that dismiss the screensaver.
    pub dismiss_on: Vec<DismissEvent>,
    /// Stop rendering after this many seconds of screensaver runtime, leaving a
    /// black surface. Guards against burning GPU for the (potentially long) gap
    /// between our timeout and cosmic-idle's screen-off.
    pub max_runtime_secs: Option<u32>,
    /// Whether to run the screensaver while on battery power.
    pub on_battery: bool,
}

impl Default for ScreensaverConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Opt-in
            idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
            source: ScreensaverSource::default(),
            fade_in_ms: DEFAULT_FADE_IN_MS,
            fade_out_ms: DEFAULT_FADE_OUT_MS,
            dismiss_on: default_dismiss_on(),
            max_runtime_secs: Some(600),
            on_battery: false,
        }
    }
}

impl ScreensaverConfig {
    /// Load screensaver config from cosmic-config, falling back to defaults
    /// for any key that is missing or unreadable.
    #[must_use]
    pub fn load(context: &Context) -> Self {
        let defaults = Self::default();

        Self {
            enabled: context.0.get::<bool>(ENABLED).unwrap_or(defaults.enabled),
            idle_timeout_secs: context
                .0
                .get::<u32>(IDLE_TIMEOUT_SECS)
                .unwrap_or(defaults.idle_timeout_secs),
            source: context
                .0
                .get::<ScreensaverSource>(SOURCE)
                .unwrap_or(defaults.source),
            fade_in_ms: context
                .0
                .get::<u64>(FADE_IN_MS)
                .unwrap_or(defaults.fade_in_ms),
            fade_out_ms: context
                .0
                .get::<u64>(FADE_OUT_MS)
                .unwrap_or(defaults.fade_out_ms),
            dismiss_on: context
                .0
                .get::<Vec<DismissEvent>>(DISMISS_ON)
                .unwrap_or(defaults.dismiss_on),
            max_runtime_secs: context
                .0
                .get::<Option<u32>>(MAX_RUNTIME_SECS)
                .unwrap_or(defaults.max_runtime_secs),
            on_battery: context
                .0
                .get::<bool>(ON_BATTERY)
                .unwrap_or(defaults.on_battery),
        }
    }

    /// Save screensaver config to cosmic-config.
    ///
    /// # Errors
    ///
    /// Fails if any key cannot be written.
    pub fn save(&self, context: &Context) -> Result<(), cosmic_config::Error> {
        context.0.set(ENABLED, self.enabled)?;
        context.0.set(IDLE_TIMEOUT_SECS, self.idle_timeout_secs)?;
        context.0.set(SOURCE, self.source.clone())?;
        context.0.set(FADE_IN_MS, self.fade_in_ms)?;
        context.0.set(FADE_OUT_MS, self.fade_out_ms)?;
        context.0.set(DISMISS_ON, self.dismiss_on.clone())?;
        context.0.set(MAX_RUNTIME_SECS, self.max_runtime_secs)?;
        context.0.set(ON_BATTERY, self.on_battery)?;
        Ok(())
    }

    /// The idle timeout in milliseconds, as `ext-idle-notify-v1` wants it.
    ///
    /// Clamped to at least one second: a zero timeout would have the compositor
    /// report idle immediately and continuously.
    #[must_use]
    pub fn idle_timeout_ms(&self) -> u32 {
        self.idle_timeout_secs.max(1).saturating_mul(1000)
    }

    /// True when the configured timeout is at or beyond cosmic-idle's
    /// screen-off time, meaning the screensaver would never become visible
    /// before cosmic-idle fades the screen to black and locks.
    ///
    /// Returns `None` when cosmic-idle's config cannot be read, or when its
    /// screen-off time is disabled (in which case there is no conflict — and
    /// no idle lock either, since cosmic-idle only locks after screen-off).
    #[must_use]
    pub fn exceeds_screen_off(&self) -> Option<bool> {
        let screen_off_ms = cosmic_idle_screen_off_ms()?;
        Some(self.idle_timeout_ms() >= screen_off_ms)
    }
}

/// Read cosmic-idle's `screen_off_time`, in milliseconds.
///
/// Returns `None` if cosmic-idle is not configured, its config is unreadable,
/// or screen-off is explicitly disabled.
#[must_use]
pub fn cosmic_idle_screen_off_ms() -> Option<u32> {
    let config = CosmicConfig::new(COSMIC_IDLE_NAME, 1).ok()?;
    config
        .get::<Option<u32>>(COSMIC_IDLE_SCREEN_OFF_TIME)
        .ok()?
}

impl Context {
    /// Load the full screensaver config.
    #[must_use]
    pub fn screensaver_config(&self) -> ScreensaverConfig {
        ScreensaverConfig::load(self)
    }

    /// Get whether the screensaver is enabled.
    #[must_use]
    pub fn screensaver_enabled(&self) -> bool {
        self.0.get::<bool>(ENABLED).unwrap_or(false)
    }

    /// Set whether the screensaver is enabled.
    ///
    /// # Errors
    ///
    /// Fails if the key cannot be written.
    pub fn set_screensaver_enabled(&self, value: bool) -> Result<(), cosmic_config::Error> {
        self.0.set(ENABLED, value)
    }

    /// Get the screensaver idle timeout in seconds.
    #[must_use]
    pub fn screensaver_idle_timeout_secs(&self) -> u32 {
        self.0
            .get::<u32>(IDLE_TIMEOUT_SECS)
            .unwrap_or(DEFAULT_IDLE_TIMEOUT_SECS)
    }

    /// Set the screensaver idle timeout in seconds.
    ///
    /// # Errors
    ///
    /// Fails if the key cannot be written.
    pub fn set_screensaver_idle_timeout_secs(
        &self,
        value: u32,
    ) -> Result<(), cosmic_config::Error> {
        self.0.set(IDLE_TIMEOUT_SECS, value)
    }

    /// Get the screensaver source.
    #[must_use]
    pub fn screensaver_source(&self) -> ScreensaverSource {
        self.0.get::<ScreensaverSource>(SOURCE).unwrap_or_default()
    }

    /// Set the screensaver source.
    ///
    /// # Errors
    ///
    /// Fails if the key cannot be written.
    pub fn set_screensaver_source(
        &self,
        value: ScreensaverSource,
    ) -> Result<(), cosmic_config::Error> {
        self.0.set(SOURCE, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_opt_in() {
        let config = ScreensaverConfig::default();
        assert!(!config.enabled, "screensaver must be opt-in");
    }

    #[test]
    fn idle_timeout_converts_to_milliseconds() {
        let config = ScreensaverConfig {
            idle_timeout_secs: 300,
            ..Default::default()
        };
        assert_eq!(config.idle_timeout_ms(), 300_000);
    }

    #[test]
    fn idle_timeout_clamps_zero_to_one_second() {
        let config = ScreensaverConfig {
            idle_timeout_secs: 0,
            ..Default::default()
        };
        assert_eq!(config.idle_timeout_ms(), 1000);
    }

    #[test]
    fn idle_timeout_saturates_instead_of_overflowing() {
        let config = ScreensaverConfig {
            idle_timeout_secs: u32::MAX,
            ..Default::default()
        };
        assert_eq!(config.idle_timeout_ms(), u32::MAX);
    }

    #[test]
    fn default_dismiss_events_cover_key_and_pointer() {
        let events = default_dismiss_on();
        assert!(events.contains(&DismissEvent::Key));
        assert!(events.contains(&DismissEvent::MouseMove));
        assert!(events.contains(&DismissEvent::MouseClick));
    }

    #[test]
    fn default_source_reuses_wallpaper() {
        assert_eq!(
            ScreensaverConfig::default().source,
            ScreensaverSource::SameAsWallpaper
        );
    }
}
