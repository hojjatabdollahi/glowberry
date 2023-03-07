// SPDX-License-Identifier: MPL-2.0-only

use std::{env, path::PathBuf, str::FromStr};

use cosmic_config::{Config, ConfigGet};
use serde::{Deserialize, Serialize};

/// Configuration for the panel's ouput
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub enum CosmicBgOutput {
    /// show panel on all outputs
    All,
    /// show panel on a specific output
    Name(String),
}

impl ToString for CosmicBgOutput {
    fn to_string(&self) -> String {
        match self {
            CosmicBgOutput::All => "all".into(),
            CosmicBgOutput::Name(name) => format!("output.{}", name.clone()),
        }
    }
}

/// Configuration for the panel's ouput
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub enum CosmicBgImgSource {
    /// pull images from the $HOME/Pictures/Wallpapers directory
    Wallpapers,
    /// pull images from a specific directory or file
    Path(String),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CosmicBgEntry {
    /// the configured output
    pub output: CosmicBgOutput,
    /// the configured image source
    pub source: CosmicBgImgSource,
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
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub enum FilterMethod {
    // nearest neighbor filtering
    Nearest,
    // linear filtering
    Linear,
    // lanczos filtering with window 3
    #[default]
    Lanczos,
}

/// Image filtering method
#[derive(Debug, Deserialize, Serialize, Clone, Copy, Default)]
pub enum SamplingMethod {
    // Rotate through images in Aplhanumeeric order
    #[default]
    Alphanumeric,
    // Rotate through images in Random order
    Random,
    // TODO GnomeWallpapers
}

/// Image scaling mode
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
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
    /// defaults to /usr/share/backgrounds/pop/ if it fails to find configured path
    pub fn source_path(&self) -> PathBuf {
        match &self.source {
            CosmicBgImgSource::Wallpapers => env::var("XDG_PICTURES_DIR")
                .ok()
                .map(|s| PathBuf::from(s))
                .or_else(|| xdg_user::pictures().unwrap_or(None))
                .map(|mut pics_dir| {
                    pics_dir.push("Wallpapers");
                    pics_dir
                }),
            CosmicBgImgSource::Path(p) => PathBuf::from_str(&p).ok(),
        }
        .unwrap_or_else(|| "/usr/share/backgrounds/pop/".into())
    }

    pub fn key(&self) -> String {
        self.output.to_string()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
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
        let backgrounds: Vec<_> = entry_keys.into_iter().filter_map(|c| {
            config.get::<CosmicBgEntry>(&c.to_string()).ok()
        }).collect();

        if backgrounds.is_empty() {
            eprintln!("No wallpapers configured. Using defaults.");
            // TODO try to use the default schema before falling back
            Ok(Self::default())
        } else {
            Ok(Self { backgrounds })
        }
    }
    pub fn helper() -> Result<Config, cosmic_config::Error> {
        Config::new(NAME, 1)
    }
}
