// SPDX-License-Identifier: MPL-2.0

pub mod extend;
pub mod power_saving;
pub mod state;

use cosmic_config::{Config as CosmicConfig, ConfigGet, ConfigSet};
use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::HashSet,
    path::{Path, PathBuf},
};
use thiserror::Error;

/// Package version from Cargo.toml.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short git commit hash captured at build time.
pub const GIT_HASH: &str = env!("GIT_HASH");

/// Combined version string (e.g. "0.2.0 (abc1234)").
pub fn version_string() -> String {
    format!("{VERSION} ({GIT_HASH})")
}

/// GlowBerry config namespace
pub const NAME: &str = "io.github.hojjatabdollahi.glowberry";
pub const BACKGROUNDS: &str = "backgrounds";
pub const DEFAULT_BACKGROUND: &str = "all";
pub const SAME_ON_ALL: &str = "same-on-all";
pub const PREFER_LOW_POWER: &str = "prefer-low-power";
/// Per display set (see `extend::display_key`): the complete per-output
/// wallpaper state last applied while exactly that set was connected.
pub const OUTPUT_PROFILES: &str = "output-profiles";
pub const WINDOW_OPACITY: &str = "window-opacity";

/// Errors that can occur during config operations
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config error: {0}")]
    Config(#[from] cosmic_config::Error),
}

/// Create a context to the GlowBerry config.
///
/// # Errors
///
/// Fails if config paths are missing or cannot be created.
pub fn context() -> Result<Context, cosmic_config::Error> {
    CosmicConfig::new(NAME, 1).map(Context)
}

/// Stable identity for an output built from its EDID descriptors
/// (`make|model|serial`). The same physical monitor yields the same value no
/// matter which connector it is on, so per-output config survives re-docking
/// (COSMIC hands out a fresh `DP-N` on most re-plugs). `None` when nothing is
/// known; callers then fall back to the connector name.
#[must_use]
pub fn output_identity(make: &str, model: &str, serial: &str) -> Option<String> {
    if make.is_empty() && model.is_empty() && serial.is_empty() {
        return None;
    }
    // Identities double as cosmic-config keys, i.e. file names.
    Some(format!("{make}|{model}|{serial}").replace('/', "_"))
}

/// Everything the daemon needs to bring back the wallpapers a user applied
/// for one display set: the same-on-all switch, the `all` entry, and the
/// per-output entries of the outputs that were connected at the time.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct OutputProfile {
    pub same_on_all: bool,
    pub all: Entry,
    pub outputs: Vec<Entry>,
}

#[derive(Clone, Debug)]
pub struct Context(pub CosmicConfig);

impl Context {
    /// The wallpaper state last applied for the display set `set_key`.
    #[must_use]
    pub fn output_profile(&self, set_key: &str) -> Option<OutputProfile> {
        self.0
            .get::<std::collections::HashMap<String, OutputProfile>>(OUTPUT_PROFILES)
            .ok()?
            .remove(set_key)
    }

    /// Record the wallpaper state for the display set `set_key`.
    pub fn save_output_profile(
        &self,
        set_key: &str,
        profile: &OutputProfile,
    ) -> Result<(), cosmic_config::Error> {
        let mut profiles = self
            .0
            .get::<std::collections::HashMap<String, OutputProfile>>(OUTPUT_PROFILES)
            .unwrap_or_default();
        if profiles.get(set_key) == Some(profile) {
            return Ok(());
        }
        profiles.insert(set_key.to_owned(), profile.clone());
        self.0.set(OUTPUT_PROFILES, profiles)
    }

    /// Get all stored backgrounds from cosmic-config.
    ///
    /// Returns an empty vector if the key doesn't exist or fails to parse.
    pub fn backgrounds(&self) -> Vec<String> {
        match self.0.get::<Vec<String>>(BACKGROUNDS) {
            Ok(value) => value,
            Err(why) => {
                // This is expected when no per-output backgrounds are configured
                tracing::debug!(?why, "no per-output backgrounds configured");
                Vec::new()
            }
        }
    }

    pub fn default_background(&self) -> Entry {
        self.entry("all").unwrap_or_else(|_| Entry::fallback())
    }

    /// Get the entry for an output from cosmic-config.
    ///
    /// # Errors
    ///
    /// Fails if the config is missing or fails to parse.
    pub fn entry(&self, output: &str) -> Result<Entry, cosmic_config::Error> {
        self.0.get::<Entry>(output)
    }

    #[must_use]
    pub fn same_on_all(&self) -> bool {
        if let Ok(value) = self.0.get::<bool>(SAME_ON_ALL) {
            return value;
        }

        let _res = self.0.set(SAME_ON_ALL, true);

        true
    }

    pub fn set_same_on_all(&self, value: bool) -> Result<(), cosmic_config::Error> {
        if self.same_on_all() != value {
            return self.0.set(SAME_ON_ALL, value);
        }

        Ok(())
    }

    /// Get the prefer low power GPU setting.
    /// When enabled, uses integrated GPU for shader rendering to save power.
    #[must_use]
    pub fn prefer_low_power(&self) -> bool {
        self.0.get::<bool>(PREFER_LOW_POWER).unwrap_or(true)
    }

    /// Set the prefer low power GPU setting.
    pub fn set_prefer_low_power(&self, value: bool) -> Result<(), cosmic_config::Error> {
        if self.prefer_low_power() != value {
            return self.0.set(PREFER_LOW_POWER, value);
        }
        Ok(())
    }

    /// Get the window opacity setting for the settings app.
    /// Returns a value between 0.0 (fully transparent) and 1.0 (fully opaque).
    /// Default is 1.0 (fully opaque).
    #[must_use]
    pub fn window_opacity(&self) -> f32 {
        self.0
            .get::<f32>(WINDOW_OPACITY)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0)
    }

    /// Set the window opacity setting.
    pub fn set_window_opacity(&self, value: f32) -> Result<(), cosmic_config::Error> {
        let value = value.clamp(0.0, 1.0);
        if (self.window_opacity() - value).abs() > f32::EPSILON {
            return self.0.set(WINDOW_OPACITY, value);
        }
        Ok(())
    }
}

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
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, PartialOrd)]
pub enum Color {
    Single([f32; 3]),
    Gradient(Gradient),
}

/// A background image which is colored by a gradient.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, PartialOrd)]
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
    /// A GPU-rendered shader for live wallpapers.
    Shader(ShaderSource),
}

/// Configuration for a shader-based live wallpaper.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct ShaderSource {
    /// The shader code source (path or inline).
    pub shader: ShaderContent,
    /// Original shader path (preserved when using customized Code content).
    /// This allows the UI to identify which shader is selected even when
    /// parameters have been modified and the code is inlined.
    #[serde(default)]
    pub source_path: Option<PathBuf>,
    /// Custom parameter values for the shader (param_name -> value).
    /// Values are stored as f64 to accommodate both f32 and i32 parameters.
    #[serde(default)]
    pub params: std::collections::HashMap<String, f64>,
    /// Optional background image the shader can sample.
    #[serde(default)]
    pub background_image: Option<PathBuf>,
    /// Shader language (auto-detected from file extension if path).
    #[serde(default)]
    pub language: ShaderLanguage,
    /// Target frame rate (1-60, default 30).
    #[serde(default = "default_frame_rate")]
    pub frame_rate: u8,
    /// Resolution scale for shader rendering (0.25-1.0, default 1.0).
    /// The shader renders at this fraction of the output's native resolution
    /// and the compositor upscales the buffer via wp_viewport. Half scale
    /// costs a quarter of the fragment work.
    #[serde(default = "default_render_scale")]
    pub render_scale: f32,
}

fn default_frame_rate() -> u8 {
    30
}

fn default_render_scale() -> f32 {
    1.0
}

/// Where the shader code comes from.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub enum ShaderContent {
    /// Path to a shader file (.wgsl, GLSL is not supported yet).
    Path(PathBuf),
    /// Inline shader code.
    Code(String),
}

/// Resolve a stored shader path, falling back to a same-named shader in the XDG
/// data dirs when the stored file is gone (e.g. config written against
/// `~/.local/share/glowberry/shaders/` after the shaders moved to `/usr/share`).
pub fn resolve_shader_path(path: &Path) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }
    path.file_name()
        .and_then(|name| {
            xdg::BaseDirectories::with_prefix("glowberry")
                .find_data_file(Path::new("shaders").join(name))
        })
        .unwrap_or_else(|| path.to_path_buf())
}

/// Supported shader languages.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShaderLanguage {
    #[default]
    Wgsl,
    Glsl,
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

    /// Fallback in case config and default schema can't be loaded.
    /// Searches XDG data directories for the default cosmic wallpaper.
    pub fn fallback() -> Self {
        let wallpaper = "backgrounds/cosmic/orion_nebula_nasa_heic0601a.jpg";

        // Use xdg crate to search all XDG data directories
        // (searches ~/.local/share, then XDG_DATA_DIRS / defaults)
        let xdg = xdg::BaseDirectories::new();
        let source_path = xdg
            .find_data_file(wallpaper)
            .unwrap_or_else(|| PathBuf::from("/usr/share").join(wallpaper));

        Self {
            output: String::from("all"),
            source: Source::Path(source_path),
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
    /// Load config with the provided name from cosmic-config.
    ///
    /// # Errors
    ///
    /// Fails if invalid iter are stored within cosmic-config at time of parsing them.
    pub fn load(context: &Context) -> Result<Self, cosmic_config::Error> {
        let same_on_all = context.same_on_all();
        let mut config = Self {
            same_on_all,
            ..Default::default()
        };

        config.default_background = context.default_background();

        if !config.same_on_all {
            config.load_backgrounds(context);
        }

        Ok(config)
    }

    pub fn load_backgrounds(&mut self, context: &Context) {
        self.backgrounds.clear();
        self.outputs.clear();

        let entries = context
            .backgrounds()
            .into_iter()
            .filter_map(|output| context.entry(&["output.", &output].concat()).ok());

        for entry in entries {
            self.outputs.insert(entry.output.clone());
            self.backgrounds.push(entry);
        }
        // Keep a stable order so two loads (or a load and in-memory edits)
        // compare equal when they hold the same entries.
        self.backgrounds.sort_by(|a, b| a.output.cmp(&b.output));

        self.default_background = context.default_background();
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

    /// Applies the entry for the given output to cosmic-config.
    ///
    /// # Errors
    ///
    /// Fails if the config could not be set in cosmic-config.
    pub fn set_entry(
        &mut self,
        context: &Context,
        entry: Entry,
    ) -> Result<(), cosmic_config::Error> {
        let output_key = if entry.output == "all" {
            entry.output.clone()
        } else {
            self.outputs.insert(entry.output.clone());
            ["output.", &entry.output].concat()
        };

        if context.0.get(&output_key).ok().as_ref() != Some(&entry) {
            context.0.set(&output_key, entry.clone())?;
        }

        // Match in-memory entries by the bare output name (e.g. "DP-5"), which is
        // what `Entry::output` holds — not the on-disk key ("output.DP-5").
        // Using the key here never matched, so every set_entry pushed a duplicate
        // and `entry()` returned a stale copy until the next reload.
        if entry.output == "all" {
            self.default_background = entry;
        } else if let Some(old) = self.entry_mut(&entry.output) {
            *old = entry;
        } else {
            self.backgrounds.push(entry);
            self.backgrounds.sort_by(|a, b| a.output.cmp(&b.output));
        }

        let new_value = self.outputs.iter().cloned().collect::<Vec<_>>();

        if context.backgrounds() != new_value
            && let Err(why) = context.0.set::<Vec<String>>(BACKGROUNDS, new_value)
        {
            tracing::error!(?why, "failed to update outputs");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::output_identity;

    #[test]
    fn identity_is_none_without_descriptors_and_is_a_safe_key() {
        assert_eq!(output_identity("", "", ""), None);
        assert_eq!(
            output_identity("LG Electronics", "LG HDR 4K", "0x1/2").as_deref(),
            Some("LG Electronics|LG HDR 4K|0x1_2")
        );
        assert_eq!(output_identity("Dell", "", "").as_deref(), Some("Dell||"));
    }
}

#[cfg(test)]
mod resolve_shader_path_tests {
    use super::resolve_shader_path;
    use std::path::Path;

    #[test]
    fn relocated_shader_is_found_by_file_name() {
        let tmp = std::env::temp_dir().join(format!("glowberry-resolve-{}", std::process::id()));
        let shaders = tmp.join("glowberry/shaders");
        std::fs::create_dir_all(&shaders).unwrap();
        let relocated = shaders.join("relocated.wgsl");
        std::fs::write(&relocated, "").unwrap();
        // SAFETY: single-threaded test; nothing else in this crate reads XDG_DATA_DIRS.
        unsafe { std::env::set_var("XDG_DATA_DIRS", &tmp) };

        let stale = Path::new("/nonexistent/old/relocated.wgsl");
        assert_eq!(resolve_shader_path(stale), relocated);
        assert_eq!(resolve_shader_path(&relocated), relocated);
        let missing = Path::new("/nonexistent/old/missing.wgsl");
        assert_eq!(resolve_shader_path(missing), missing);

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
