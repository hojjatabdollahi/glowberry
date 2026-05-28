// SPDX-License-Identifier: MPL-2.0

use cosmic_config::{ConfigGet, ConfigSet};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::Context;

pub const EXTEND_ON_ALL: &str = "extend-on-all";
pub const EXTEND_LAYERS: &str = "extend-layers";
pub const EXTEND_PROFILES: &str = "extend-profiles";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtendLayer {
    pub source_path: PathBuf,
    pub img_offset_x: f64,
    pub img_offset_y: f64,
    pub img_scale: f64,
    pub z_index: usize,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub target_output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ExtendConfig {
    pub enabled: bool,
    pub layers: Vec<ExtendLayer>,
}

/// Map from display-set key (sorted monitor names joined by "+") to layer config
pub type DisplayProfiles = HashMap<String, Vec<ExtendLayer>>;

/// Build a display-set key from a list of monitor names
pub fn display_key(monitors: &[String]) -> String {
    let mut sorted: Vec<&str> = monitors.iter().map(|s| s.as_str()).collect();
    sorted.sort();
    sorted.join("+")
}

impl ExtendConfig {
    pub fn load(context: &Context) -> Self {
        let enabled = context.0.get::<bool>(EXTEND_ON_ALL).unwrap_or(false);

        // Try new format first
        if let Ok(layers) = context.0.get::<Vec<ExtendLayer>>(EXTEND_LAYERS) {
            return Self { enabled, layers };
        }

        Self {
            enabled,
            layers: Vec::new(),
        }
    }

    pub fn save(&self, context: &Context) -> Result<(), cosmic_config::Error> {
        context.0.set(EXTEND_ON_ALL, self.enabled)?;
        context.0.set(EXTEND_LAYERS, &self.layers)?;
        Ok(())
    }

    /// Load the layer config for a specific display configuration.
    /// Falls back to: exact match → best partial match → current layers → empty.
    pub fn load_for_displays(context: &Context, monitor_names: &[String]) -> Vec<ExtendLayer> {
        let key = display_key(monitor_names);
        let profiles = context
            .0
            .get::<DisplayProfiles>(EXTEND_PROFILES)
            .unwrap_or_default();

        // Exact match
        if let Some(layers) = profiles.get(&key) {
            return layers.clone();
        }

        // Find best partial match: profile that shares the most monitors with current set
        let current_set: std::collections::HashSet<&str> =
            monitor_names.iter().map(|s| s.as_str()).collect();

        let mut best: Option<(&str, &Vec<ExtendLayer>, usize)> = None;
        for (profile_key, layers) in &profiles {
            let profile_monitors: std::collections::HashSet<&str> =
                profile_key.split('+').collect();
            let overlap = current_set.intersection(&profile_monitors).count();
            if overlap > 0 {
                if best.is_none() || overlap > best.unwrap().2 {
                    best = Some((profile_key, layers, overlap));
                }
            }
        }

        if let Some((_, layers, _)) = best {
            return layers.clone();
        }

        // Fall back to current extend-layers key
        context
            .0
            .get::<Vec<ExtendLayer>>(EXTEND_LAYERS)
            .unwrap_or_default()
    }

    /// Save the layer config for a specific display configuration.
    pub fn save_for_displays(
        context: &Context,
        monitor_names: &[String],
        layers: &[ExtendLayer],
    ) -> Result<(), cosmic_config::Error> {
        let key = display_key(monitor_names);
        let mut profiles = context
            .0
            .get::<DisplayProfiles>(EXTEND_PROFILES)
            .unwrap_or_default();
        profiles.insert(key, layers.to_vec());
        context.0.set(EXTEND_PROFILES, &profiles)
    }
}

impl Context {
    #[must_use]
    pub fn extend_on_all(&self) -> bool {
        self.0.get::<bool>(EXTEND_ON_ALL).unwrap_or(false)
    }

    pub fn set_extend_on_all(&self, value: bool) -> Result<(), cosmic_config::Error> {
        self.0.set(EXTEND_ON_ALL, value)
    }

    #[must_use]
    pub fn extend_config(&self) -> ExtendConfig {
        ExtendConfig::load(self)
    }

    pub fn save_extend_config(&self, config: &ExtendConfig) -> Result<(), cosmic_config::Error> {
        config.save(self)
    }
}
