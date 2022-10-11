// SPDX-License-Identifier: MPL-2.0-only

use std::{path::{PathBuf}, fs::File, str::FromStr, env};

use serde::{Deserialize, Serialize};
use xdg::BaseDirectories;

/// Configuration for the panel's ouput
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub enum CosmicBgOutput {
    /// show panel on all outputs
    All,
    /// show panel on a specific output
    MakeModel {
        make: String,
        model: String,
    },
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
}

impl CosmicBgEntry {
    /// defaults to /usr/share/backgrounds/pop/ if it fails to find configured path
    pub fn source_path(&self) -> PathBuf {
        match &self.source {
            CosmicBgImgSource::Wallpapers => {
                env::var("XDG_PICTURES_DIR").ok().map(|s| PathBuf::from(s)).or_else(|| xdg_user::pictures().unwrap_or(None)).map(|mut pics_dir| {
                    pics_dir.push("Wallpapers");
                    pics_dir
                })      
            },
            CosmicBgImgSource::Path(p) => {
                PathBuf::from_str(&p).ok()
            },
        }.unwrap_or_else(|| "/usr/share/backgrounds/pop/".into())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CosmicBgConfig {
    /// the configured wallpapers8
    pub backgrounds: Vec<CosmicBgEntry>
}

impl Default for CosmicBgConfig {
    fn default() -> Self {
        CosmicBgConfig {
            backgrounds: vec![CosmicBgEntry {
                output: CosmicBgOutput::All,
                source: CosmicBgImgSource::Wallpapers,
                filter_by_theme: true,
                rotation_frequency: 10,
            }]
        }
    }
}

static NAME: &str = "com.system76.CosmicBg";
static CONFIG: &str = "config.ron";


impl CosmicBgConfig {
    /// load config with the provided name
    pub fn load() -> anyhow::Result<Self> {
        let config_path: PathBuf = vec![NAME, CONFIG].iter().collect();
        let config_path =
            match BaseDirectories::new().map(|dirs| dirs.find_config_file(&config_path)) {
                Ok(Some(path)) => path,
                _ => anyhow::bail!("Failed to get find config file"),
            };

        let file = match File::open(&config_path) {
            Ok(file) => file,
            Err(err) => {
                anyhow::bail!("Failed to open '{}': {}", config_path.display(), err);
            }
        };

        match ron::de::from_reader::<_, Self>(file) {
            Ok(config) => Ok(config),
            Err(err) => {
                anyhow::bail!("Failed to parse '{}': {}", config_path.display(), err);
            }
        }
    }

    /// write config to config file
    pub fn write(&self) -> anyhow::Result<()> {
        let config_path: PathBuf = vec![NAME, CONFIG].iter().collect();
        let xdg = BaseDirectories::new()?;
        let f = xdg.place_config_file(&config_path).unwrap();
        let f = File::create(f)?;
        ron::ser::to_writer_pretty(&f, self, ron::ser::PrettyConfig::default())?;
        Ok(())
    }
}