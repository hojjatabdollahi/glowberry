pub mod colored;
pub mod draw;
pub mod engine;
pub mod external_surface;
pub mod fragment_canvas;
pub mod gpu;
pub mod img_source;
pub mod scaler;
pub mod upower;
pub mod user_context;
pub mod wallpaper;

pub use engine::{BackgroundEngine, BackgroundHandle, EngineConfig, GlowBerry, GlowBerryLayer};
pub use external_surface::{
    has_shader_background, load_background_image, load_background_source, load_shader_source,
    BackgroundSource, ExternalSurfaceError,
};
pub use user_context::{EnvGuard, UserContext};
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
