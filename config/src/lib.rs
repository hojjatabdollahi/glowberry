// SPDX-License-Identifier: MPL-2.0-only

pub mod state;

use cosmic_config::{Config as CosmicConfig, ConfigGet, ConfigSet};
use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashSet, path::PathBuf};

pub const NAME: &str = "com.system76.CosmicBackground";
pub const BACKGROUNDS: &str = "backgrounds";
pub const DEFAULT_BACKGROUND: &str = "all";
pub const SAME_ON_ALL: &str = "same-on-all";

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Setters)]
#[serde(deny_unknown_fields)]
#[must_use]
pub struct Entry {
    /// the configured output
    #[setters(skip)]
    pub output: String,
    /// the configured image source
    #[setters(skip)]
    pub source: Source,
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

/// A background image which is colored.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum Color {
    Single([f32; 3]),
    Gradient(Gradient),
}

/// A background image which is colored by a gradient.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Gradient {
    pub colors: Cow<'static, [[f32; 3]]>,
    pub radius: f32,
}

/// The source of a background image.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub enum Source {
    /// Background image(s) from a path.
    Path(PathBuf),
    /// A background color or gradient.
    Color(Color),
}

impl Entry {
    /// Define a preferred background for a given output device.
    pub fn new(output: String, source: Source) -> Self {
        Self {
            output,
            source,
            filter_by_theme: false,
            rotation_frequency: 900,
            filter_method: FilterMethod::default(),
            scaling_mode: ScalingMode::default(),
            sampling_method: SamplingMethod::default(),
        }
    }

    /// Fallback in case config and default schema can't be loaded
    pub fn fallback() -> Self {
        Self {
            output: String::from("all"),
            source: Source::Path(PathBuf::from("/usr/share/backgrounds/pop/")),
            filter_by_theme: true,
            rotation_frequency: 3600,
            filter_method: FilterMethod::default(),
            scaling_mode: ScalingMode::default(),
            sampling_method: SamplingMethod::default(),
        }
    }
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

impl From<FilterMethod> for image::imageops::FilterType {
    fn from(method: FilterMethod) -> Self {
        match method {
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

impl Entry {
    #[must_use]
    pub fn key(&self) -> String {
        self.output.to_string()
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub same_on_all: bool,
    pub outputs: HashSet<String>,
    pub backgrounds: Vec<Entry>,
    pub default_background: Entry,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            same_on_all: true,
            outputs: HashSet::new(),
            backgrounds: Vec::new(),
            default_background: Entry::fallback(),
        }
    }
}

impl Config {
    /// Convenience function for cosmic-config
    ///
    /// # Errors
    ///
    /// Fails if cosmic-config paths are missing or cannot be created.
    pub fn helper() -> Result<CosmicConfig, cosmic_config::Error> {
        CosmicConfig::new(NAME, 1)
    }

    /// Load config with the provided name from cosmic-config.
    ///
    /// # Errors
    ///
    /// Fails if invalid iter are stored within cosmic-config at time of parsing them.
    pub fn load(context: &CosmicConfig) -> Result<Self, cosmic_config::Error> {
        let mut config = Self {
            same_on_all: Self::load_same_on_all(context),
            ..Default::default()
        };

        config.default_background =
            Self::load_entry(context, "all").unwrap_or_else(|_| Entry::fallback());

        if !config.same_on_all {
            config.load_backgrounds(context);
        }

        tracing::debug!(
            same_on_all = config.same_on_all,
            outputs = ?config.outputs,
            backgrounds = ?config.backgrounds,
            default_background = ?config.default_background,
            "loaded config"
        );

        Ok(config)
    }

    pub fn load_backgrounds(&mut self, context: &CosmicConfig) {
        self.backgrounds.clear();
        self.outputs.clear();

        let entries = Self::load_outputs(context)
            .into_iter()
            .filter_map(|output| Self::load_entry(context, &["output.", &output].concat()).ok());

        for entry in entries {
            self.outputs.insert(entry.output.clone());
            self.backgrounds.push(entry);
        }

        self.default_background = Self::load_default_background(context);
    }

    pub fn load_default_background(context: &CosmicConfig) -> Entry {
        Self::load_entry(context, "all").unwrap_or_else(|_| Entry::fallback())
    }

    /// Get the entry for a given output.
    #[must_use]
    pub fn entry(&self, output: &str) -> Option<&Entry> {
        self.backgrounds.iter().find(|entry| entry.output == output)
    }

    /// get a mutable entry for a given output.
    #[must_use]
    pub fn entry_mut(&mut self, output: &str) -> Option<&mut Entry> {
        self.backgrounds
            .iter_mut()
            .find(|entry| entry.output == output)
    }

    /// Get the entry for an output from cosmic-config.
    ///
    /// # Errors
    ///
    /// Fails if the config is missing or fails to parse.
    pub fn load_entry(config: &CosmicConfig, output: &str) -> Result<Entry, cosmic_config::Error> {
        config.get::<Entry>(output)
    }

    /// Applies the entry for the given output to cosmic-config.
    ///
    /// # Errors
    ///
    /// Fails if the config could not be set in cosmic-config.
    pub fn set_entry(
        &mut self,
        config: &CosmicConfig,
        entry: Entry,
    ) -> Result<(), cosmic_config::Error> {
        let output_key = if entry.output == "all" {
            entry.output.clone()
        } else {
            self.outputs.insert(entry.output.clone());
            ["output.", &entry.output].concat()
        };

        if config.get(&output_key).ok().as_ref() != Some(&entry) {
            config.set(&output_key, entry.clone())?;
        }

        if let Some(old) = self.entry_mut(&output_key) {
            *old = entry;
        } else if entry.output != "all" {
            self.backgrounds.push(entry);
        }

        let new_value = self.outputs.iter().cloned().collect::<Vec<_>>();

        if config.get::<Vec<String>>(BACKGROUNDS).ok().as_deref() != Some(&new_value) {
            if let Err(why) = config.set::<Vec<String>>(BACKGROUNDS, new_value) {
                tracing::error!(?why, "failed to update outputs");
            }
        }

        Ok(())
    }

    /// Get all stored outputs from cosmic-config.
    ///
    /// # Errors
    ///
    /// Fails if the config is missing or fails to parse.
    pub fn load_outputs(config: &CosmicConfig) -> Vec<String> {
        match config.get::<Vec<String>>(BACKGROUNDS) {
            Ok(value) => value,
            Err(why) => {
                tracing::error!(?why, "error reading background config");
                Vec::new()
            }
        }
    }

    #[must_use]
    pub fn load_same_on_all(config: &CosmicConfig) -> bool {
        if let Ok(value) = config.get::<bool>(SAME_ON_ALL) {
            return value;
        }

        let _res = config.set(SAME_ON_ALL, true);

        true
    }

    pub fn set_same_on_all(config: &CosmicConfig, value: bool) -> Result<(), cosmic_config::Error> {
        if Self::load_same_on_all(config) != value {
            return config.set(SAME_ON_ALL, value);
        }

        Ok(())
    }
}
