// SPDX-License-Identifier: MPL-2.0

use cosmic_config::{ConfigGet, ConfigSet};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::Context;

pub const EXTEND_ON_ALL: &str = "extend-on-all";
pub const EXTEND_SOURCE_PATH: &str = "extend-source-path";
pub const EXTEND_IMG_OFFSET_X: &str = "extend-img-offset-x";
pub const EXTEND_IMG_OFFSET_Y: &str = "extend-img-offset-y";
pub const EXTEND_IMG_SCALE: &str = "extend-img-scale";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtendConfig {
    pub enabled: bool,
    pub source_path: Option<PathBuf>,
    pub img_offset_x: f64,
    pub img_offset_y: f64,
    pub img_scale: f64,
}

impl Default for ExtendConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            source_path: None,
            img_offset_x: 0.0,
            img_offset_y: 0.0,
            img_scale: 1.0,
        }
    }
}

impl ExtendConfig {
    pub fn load(context: &Context) -> Self {
        Self {
            enabled: context.0.get::<bool>(EXTEND_ON_ALL).unwrap_or(false),
            source_path: context
                .0
                .get::<Option<PathBuf>>(EXTEND_SOURCE_PATH)
                .unwrap_or(None),
            img_offset_x: context.0.get::<f64>(EXTEND_IMG_OFFSET_X).unwrap_or(0.0),
            img_offset_y: context.0.get::<f64>(EXTEND_IMG_OFFSET_Y).unwrap_or(0.0),
            img_scale: context.0.get::<f64>(EXTEND_IMG_SCALE).unwrap_or(1.0),
        }
    }

    pub fn save(&self, context: &Context) -> Result<(), cosmic_config::Error> {
        context.0.set(EXTEND_ON_ALL, self.enabled)?;
        context.0.set(EXTEND_SOURCE_PATH, &self.source_path)?;
        context.0.set(EXTEND_IMG_OFFSET_X, self.img_offset_x)?;
        context.0.set(EXTEND_IMG_OFFSET_Y, self.img_offset_y)?;
        context.0.set(EXTEND_IMG_SCALE, self.img_scale)?;
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
