pub(crate) mod colored;
pub(crate) mod draw;
pub mod engine;
pub(crate) mod fragment_canvas;
pub(crate) mod gpu;
pub(crate) mod img_source;
pub(crate) mod scaler;
pub mod shader_defs;
pub(crate) mod upower;
pub mod wallpaper;

pub use engine::{BackgroundEngine, EngineConfig, GlowBerry, GlowBerryLayer};
pub use wallpaper::Wallpaper;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_config_defaults() {
        let config = EngineConfig::default();
        assert!(config.enable_wayland);
    }
}
