// SPDX-License-Identifier: MPL-2.0

//! Power saving configuration for GlowBerry shader animations.

use cosmic_config::{ConfigGet, ConfigSet};
use serde::{Deserialize, Serialize};

use crate::Context;

// Config keys
pub const PAUSE_ON_FULLSCREEN: &str = "pause-on-fullscreen";
pub const PAUSE_ON_COVERED: &str = "pause-on-covered";
pub const COVERAGE_THRESHOLD: &str = "coverage-threshold";
pub const ADJUST_ON_BATTERY: &str = "adjust-on-battery";
pub const ON_BATTERY_ACTION: &str = "on-battery-action";
pub const PAUSE_ON_LOW_BATTERY: &str = "pause-on-low-battery";
pub const LOW_BATTERY_THRESHOLD: &str = "low-battery-threshold";
pub const PAUSE_ON_LID_CLOSED: &str = "pause-on-lid-closed";

/// Action to take when on battery power.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OnBatteryAction {
    /// Do nothing (no change to animation)
    #[default]
    Nothing,
    /// Pause animation entirely
    Pause,
    /// Reduce to 15 FPS
    ReduceTo15Fps,
    /// Reduce to 10 FPS
    ReduceTo10Fps,
    /// Reduce to 5 FPS
    ReduceTo5Fps,
}

impl OnBatteryAction {
    /// Get the frame rate for this action, or None if pausing or doing nothing.
    #[must_use]
    pub fn frame_rate(&self) -> Option<u8> {
        match self {
            Self::Nothing => None,
            Self::Pause => None,
            Self::ReduceTo15Fps => Some(15),
            Self::ReduceTo10Fps => Some(10),
            Self::ReduceTo5Fps => Some(5),
        }
    }

    /// Returns true if this action should pause the animation.
    #[must_use]
    pub fn should_pause(&self) -> bool {
        matches!(self, Self::Pause)
    }

    /// Returns true if this action does nothing.
    #[must_use]
    pub fn is_nothing(&self) -> bool {
        matches!(self, Self::Nothing)
    }
}

/// Power saving configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerSavingConfig {
    /// Pause when any app enters fullscreen mode
    pub pause_on_fullscreen: bool,
    /// Pause when wallpaper is covered by windows
    pub pause_on_covered: bool,
    /// Coverage percentage threshold to trigger pause (50, 90, 99, 100)
    pub coverage_threshold: u8,
    /// Adjust animation when on battery power
    pub adjust_on_battery: bool,
    /// What to do when on battery
    pub on_battery_action: OnBatteryAction,
    /// Pause when battery below threshold
    pub pause_on_low_battery: bool,
    /// Battery percentage threshold (10, 20, 30, 50)
    pub low_battery_threshold: u8,
    /// Pause internal display when lid is closed
    pub pause_on_lid_closed: bool,
}

impl Default for PowerSavingConfig {
    fn default() -> Self {
        Self {
            pause_on_fullscreen: false, // Opt-in (some apps may be transparent)
            pause_on_covered: false,    // Opt-in (some apps may be transparent)
            coverage_threshold: 90,
            adjust_on_battery: false, // Opt-in
            on_battery_action: OnBatteryAction::Pause,
            pause_on_low_battery: true, // On by default
            low_battery_threshold: 20,
            pause_on_lid_closed: true, // On by default
        }
    }
}

impl PowerSavingConfig {
    /// Load power saving config from cosmic-config.
    pub fn load(context: &Context) -> Self {
        Self {
            pause_on_fullscreen: context.0.get::<bool>(PAUSE_ON_FULLSCREEN).unwrap_or(false),
            pause_on_covered: context.0.get::<bool>(PAUSE_ON_COVERED).unwrap_or(false),
            coverage_threshold: context.0.get::<u8>(COVERAGE_THRESHOLD).unwrap_or(90),
            adjust_on_battery: context.0.get::<bool>(ADJUST_ON_BATTERY).unwrap_or(false),
            on_battery_action: context
                .0
                .get::<OnBatteryAction>(ON_BATTERY_ACTION)
                .unwrap_or_default(),
            pause_on_low_battery: context.0.get::<bool>(PAUSE_ON_LOW_BATTERY).unwrap_or(true),
            low_battery_threshold: context.0.get::<u8>(LOW_BATTERY_THRESHOLD).unwrap_or(20),
            pause_on_lid_closed: context.0.get::<bool>(PAUSE_ON_LID_CLOSED).unwrap_or(true),
        }
    }

    /// Save power saving config to cosmic-config.
    pub fn save(&self, context: &Context) -> Result<(), cosmic_config::Error> {
        context
            .0
            .set(PAUSE_ON_FULLSCREEN, self.pause_on_fullscreen)?;
        context.0.set(PAUSE_ON_COVERED, self.pause_on_covered)?;
        context.0.set(COVERAGE_THRESHOLD, self.coverage_threshold)?;
        context.0.set(ADJUST_ON_BATTERY, self.adjust_on_battery)?;
        context.0.set(ON_BATTERY_ACTION, self.on_battery_action)?;
        context
            .0
            .set(PAUSE_ON_LOW_BATTERY, self.pause_on_low_battery)?;
        context
            .0
            .set(LOW_BATTERY_THRESHOLD, self.low_battery_threshold)?;
        context
            .0
            .set(PAUSE_ON_LID_CLOSED, self.pause_on_lid_closed)?;
        Ok(())
    }
}

impl Context {
    /// Get the pause on fullscreen setting.
    #[must_use]
    pub fn pause_on_fullscreen(&self) -> bool {
        self.0.get::<bool>(PAUSE_ON_FULLSCREEN).unwrap_or(false)
    }

    /// Set the pause on fullscreen setting.
    pub fn set_pause_on_fullscreen(&self, value: bool) -> Result<(), cosmic_config::Error> {
        self.0.set(PAUSE_ON_FULLSCREEN, value)
    }

    /// Get the pause on covered setting.
    #[must_use]
    pub fn pause_on_covered(&self) -> bool {
        self.0.get::<bool>(PAUSE_ON_COVERED).unwrap_or(false)
    }

    /// Set the pause on covered setting.
    pub fn set_pause_on_covered(&self, value: bool) -> Result<(), cosmic_config::Error> {
        self.0.set(PAUSE_ON_COVERED, value)
    }

    /// Get the coverage threshold setting.
    #[must_use]
    pub fn coverage_threshold(&self) -> u8 {
        self.0.get::<u8>(COVERAGE_THRESHOLD).unwrap_or(90)
    }

    /// Set the coverage threshold setting.
    pub fn set_coverage_threshold(&self, value: u8) -> Result<(), cosmic_config::Error> {
        self.0.set(COVERAGE_THRESHOLD, value)
    }

    /// Get the adjust on battery setting.
    #[must_use]
    pub fn adjust_on_battery(&self) -> bool {
        self.0.get::<bool>(ADJUST_ON_BATTERY).unwrap_or(false)
    }

    /// Set the adjust on battery setting.
    pub fn set_adjust_on_battery(&self, value: bool) -> Result<(), cosmic_config::Error> {
        self.0.set(ADJUST_ON_BATTERY, value)
    }

    /// Get the on battery action setting.
    #[must_use]
    pub fn on_battery_action(&self) -> OnBatteryAction {
        self.0
            .get::<OnBatteryAction>(ON_BATTERY_ACTION)
            .unwrap_or_default()
    }

    /// Set the on battery action setting.
    pub fn set_on_battery_action(
        &self,
        value: OnBatteryAction,
    ) -> Result<(), cosmic_config::Error> {
        self.0.set(ON_BATTERY_ACTION, value)
    }

    /// Get the pause on low battery setting.
    #[must_use]
    pub fn pause_on_low_battery(&self) -> bool {
        self.0.get::<bool>(PAUSE_ON_LOW_BATTERY).unwrap_or(true)
    }

    /// Set the pause on low battery setting.
    pub fn set_pause_on_low_battery(&self, value: bool) -> Result<(), cosmic_config::Error> {
        self.0.set(PAUSE_ON_LOW_BATTERY, value)
    }

    /// Get the low battery threshold setting.
    #[must_use]
    pub fn low_battery_threshold(&self) -> u8 {
        self.0.get::<u8>(LOW_BATTERY_THRESHOLD).unwrap_or(20)
    }

    /// Set the low battery threshold setting.
    pub fn set_low_battery_threshold(&self, value: u8) -> Result<(), cosmic_config::Error> {
        self.0.set(LOW_BATTERY_THRESHOLD, value)
    }

    /// Get the pause on lid closed setting.
    #[must_use]
    pub fn pause_on_lid_closed(&self) -> bool {
        self.0.get::<bool>(PAUSE_ON_LID_CLOSED).unwrap_or(true)
    }

    /// Set the pause on lid closed setting.
    pub fn set_pause_on_lid_closed(&self, value: bool) -> Result<(), cosmic_config::Error> {
        self.0.set(PAUSE_ON_LID_CLOSED, value)
    }

    /// Load the full power saving config.
    #[must_use]
    pub fn power_saving_config(&self) -> PowerSavingConfig {
        PowerSavingConfig::load(self)
    }
}
