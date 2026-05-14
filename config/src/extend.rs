// SPDX-License-Identifier: MPL-2.0

use cosmic_config::{ConfigGet, ConfigSet};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::Context;

pub const EXTEND_ON_ALL: &str = "extend-on-all";
pub const EXTEND_LAYERS: &str = "extend-layers";

// Old keys for migration
const OLD_EXTEND_SOURCE_PATH: &str = "extend-source-path";
const OLD_EXTEND_IMG_OFFSET_X: &str = "extend-img-offset-x";
const OLD_EXTEND_IMG_OFFSET_Y: &str = "extend-img-offset-y";
const OLD_EXTEND_IMG_SCALE: &str = "extend-img-scale";

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

impl ExtendConfig {
    pub fn load(context: &Context) -> Self {
        let enabled = context.0.get::<bool>(EXTEND_ON_ALL).unwrap_or(false);

        // Try new format first
        if let Ok(layers) = context.0.get::<Vec<ExtendLayer>>(EXTEND_LAYERS) {
            return Self { enabled, layers };
        }

        // Fall back to old single-image format and migrate
        let source_path = context
            .0
            .get::<Option<PathBuf>>(OLD_EXTEND_SOURCE_PATH)
            .unwrap_or(None);
        let offset_x = context.0.get::<f64>(OLD_EXTEND_IMG_OFFSET_X).unwrap_or(0.0);
        let offset_y = context.0.get::<f64>(OLD_EXTEND_IMG_OFFSET_Y).unwrap_or(0.0);
        let scale = context.0.get::<f64>(OLD_EXTEND_IMG_SCALE).unwrap_or(1.0);

        let layers = if let Some(path) = source_path {
            vec![ExtendLayer {
                source_path: path,
                img_offset_x: offset_x,
                img_offset_y: offset_y,
                img_scale: scale,
                z_index: 0,
                locked: false,
                target_output: None,
            }]
        } else {
            Vec::new()
        };

        Self { enabled, layers }
    }

    pub fn save(&self, context: &Context) -> Result<(), cosmic_config::Error> {
        context.0.set(EXTEND_ON_ALL, self.enabled)?;
        context.0.set(EXTEND_LAYERS, &self.layers)?;
        Ok(())
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
