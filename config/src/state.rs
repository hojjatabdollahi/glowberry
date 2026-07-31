use cosmic_config::{Config, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use derive_setters::Setters;
use serde::{Deserialize, Serialize};

use crate::{NAME, Source};

#[derive(Default, Debug, Deserialize, Serialize, Clone, PartialEq, Setters, CosmicConfigEntry)]
#[serde(deny_unknown_fields)]
#[must_use]
pub struct State {
    /// The active wallpaper for each output
    /// (output_name, source of wallpaper)
    pub wallpapers: Vec<(String, Source)>,
    /// Currently connected outputs (updated by daemon)
    pub connected_outputs: Vec<String>,
    /// Apps currently asking to keep the screen awake over D-Bus, one entry per
    /// app (empty string for one that did not identify itself). Written by the
    /// daemon, which suppresses the screensaver while the list is non-empty.
    ///
    /// The settings app reads it from here rather than watching the bus itself:
    /// those requests can only be observed as they are made, and a monitor
    /// started when the settings window opens would miss a video that was
    /// already playing. The daemon has been watching since login.
    pub screen_wake_requests: Vec<String>,
}

impl State {
    pub fn version() -> u64 {
        1
    }

    pub fn state() -> Result<Config, cosmic_config::Error> {
        Config::new_state(NAME, Self::version())
    }
}
