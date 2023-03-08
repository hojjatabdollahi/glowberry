// SPDX-License-Identifier: MPL-2.0-only

use std::path::PathBuf;

use cosmic_config::{Config, ConfigGet};
use serde::{Deserialize, Serialize};

/// Configuration for the panel's ouput
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub enum CosmicBgOutput {
    /// show panel on a specific output
    Name(String),
    /// show panel on all outputs
    All,
}

impl ToString for CosmicBgOutput {
    fn to_string(&self) -> String {
        match self {
            CosmicBgOutput::All => "all".into(),
            CosmicBgOutput::Name(name) => format!("output.{}", name.clone()),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CosmicBgEntry {
    /// the configured output
    pub output: CosmicBgOutput,
    /// the configured image source
    pub source: PathBuf,
    /// whether the images should be filtered by the active theme
    pub filter_by_theme: bool,
    /// frequency at which the wallpaper is rotated in seconds
    pub rotation_frequency: u64,
    /// filter used to scale images
    #[serde(default)]
    pub filter_method: FilterMethod,
    /// mode used to scale images,
    #[serde(default)]
    pub scaling_mode: ScalingMode,
    #[serde(default)]
    pub sampling_method: SamplingMethod,
}

/// Image filtering method
#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq, Eq)]
pub enum FilterMethod {
    // nearest neighbor filtering
    Nearest,
    // linear filtering
    Linear,
    // lanczos filtering with window 3
    #[default]
    Lanczos,
}

impl Into<image::imageops::FilterType> for FilterMethod {
    fn into(self) -> image::imageops::FilterType {
        match self {
            FilterMethod::Nearest => image::imageops::FilterType::Nearest,
            FilterMethod::Linear => image::imageops::FilterType::Triangle,
            FilterMethod::Lanczos => image::imageops::FilterType::Lanczos3,
        }
    }
}

/// Image filtering method
#[derive(Debug, Deserialize, Serialize, Clone, Copy, Default, PartialEq, Eq)]
pub enum SamplingMethod {
    // Rotate through images in Aplhanumeeric order
    #[default]
    Alphanumeric,
    // Rotate through images in Random order
    Random,
    // TODO GnomeWallpapers
}

/// Image scaling mode
#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq)]
pub enum ScalingMode {
    // Fit the image and fill the rest of the area with the given RGB color
    Fit([f32; 3]),
    /// Stretch the image ignoring any aspect ratio to fit the area
    Stretch,
    /// Zoom the image so that it fill the whole area
    #[default]
    Zoom,
}

impl CosmicBgEntry {
    pub fn key(&self) -> String {
        self.output.to_string()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CosmicBgConfig {
    /// the configured wallpapers
    pub backgrounds: Vec<CosmicBgEntry>,
}

// Fallback in case config and default schema can't be loaded
impl Default for CosmicBgConfig {
    fn default() -> Self {
        ron::de::from_str(include_str!("../config.ron")).unwrap()
    }
}

pub const NAME: &str = "com.system76.CosmicBackground";
pub const BG_KEY: &str = "backgrounds";

impl CosmicBgConfig {
    /// load config with the provided name
    pub fn load(config: &Config) -> Result<Self, cosmic_config::Error> {
        let entry_keys = config.get::<Vec<CosmicBgOutput>>(BG_KEY)?;
        let mut backgrounds: Vec<_> = entry_keys
            .into_iter()
            .filter_map(|c| config.get::<CosmicBgEntry>(&c.to_string()).ok())
            .collect();

        let def = Self::default();
        if backgrounds.is_empty() {
            eprintln!("No wallpapers configured. Using defaults.");
            // TODO try to use the default schema before falling back
            Ok(def)
        } else {
            if backgrounds.iter().all(|e| e.output != CosmicBgOutput::All) {
                // add the default all wallpaper if all is not already present
                if let Some(def_all) = def
                    .backgrounds
                    .iter()
                    .find(|e| e.output == CosmicBgOutput::All)
                {
                    backgrounds.push(def_all.clone());
                }
            }
            Ok(Self { backgrounds })
        }
    }
    pub fn helper() -> Result<Config, cosmic_config::Error> {
        Config::new(NAME, 1)
    }
}
