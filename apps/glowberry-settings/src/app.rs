// SPDX-License-Identifier: MPL-2.0

//! Main application state and logic for GlowBerry Settings

use crate::fl;
use crate::shader_analysis::{self, Complexity};
use crate::shader_params::{ParamType, ParamValue, ParsedShader};
use cosmetics::widgets::scrub_spin::scrub_spin;
use cosmic::app::context_drawer::{self, ContextDrawer};
use cosmic::app::{Core, Task};
use cosmic::iced::Subscription;
use cosmic::iced::widget::image::Handle as ImageHandle;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{
    self, button, container, dropdown, menu, segmented_button, settings, slider, text, toggler,
};
use cosmic::{ApplicationExt, Element};
use cosmic_config::{ConfigGet, ConfigSet, CosmicConfigEntry};
use glowberry_config::extend::ExtendConfig;
use glowberry_config::power_saving::{OnBatteryAction, PowerSavingConfig};
use glowberry_config::state::State;
use glowberry_config::{Color, Config, Context as ConfigContext, Entry, Gradient, Source};
use image::{ImageBuffer, Rgba};
use slotmap::{DefaultKey, SecondaryMap, SlotMap};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;

/// Wrapper for output name to store in segmented button data
#[derive(Clone, Debug)]
struct OutputName(String);

mod wallpaper_subscription;
use wallpaper_subscription::WallpaperEvent;

/// Application ID for GlowBerry Settings
pub const APP_ID: &str = "io.github.hojjatabdollahi.glowberry-settings";

const SIMULATED_WIDTH: u16 = 300;
const SIMULATED_HEIGHT: u16 = 169;

/// Context page for the settings drawer
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ContextPage {
    #[default]
    Settings,
    About,
}

/// Main application state
pub struct GlowBerrySettings {
    core: Core,
    config: Config,
    config_context: Option<ConfigContext>,

    /// Current context drawer page
    context_page: ContextPage,
    /// About information
    about: widget::about::About,

    /// Model for selecting between display outputs
    outputs: segmented_button::SingleSelectModel,
    /// The display that is currently being configured (None means "all")
    active_output: Option<String>,
    /// Whether to show the tab bar (more than one display)
    show_tab_bar: bool,

    /// Category dropdown model
    categories: dropdown::multi::Model<String, Category>,

    /// Wallpaper selection context
    selection: SelectionContext,

    /// Available system shaders
    available_shaders: Vec<ShaderInfo>,
    /// Shader preview thumbnails
    shader_thumbnails: Vec<ImageHandle>,
    /// Selected shader frame rate index
    selected_shader_frame_rate: usize,
    /// Frame rate options
    frame_rate_options: Vec<String>,

    /// Fit options (Zoom, Fit) — used by color/shader modes
    #[allow(dead_code)]
    fit_options: Vec<String>,
    #[allow(dead_code)]
    selected_fit: usize,

    /// Cached display preview image
    cached_display_handle: Option<ImageHandle>,

    /// Current wallpaper folder
    current_folder: PathBuf,
    /// User-added wallpaper sources (image files and/or directories), shown in
    /// the grid in addition to the default folder.
    wallpaper_sources: Vec<PathBuf>,

    /// Prefer low power GPU for shader rendering
    prefer_low_power: bool,

    /// Whether GlowBerry is currently set as the default background service
    glowberry_is_default: bool,

    /// Current shader parameter values (shader_index -> param_name -> value)
    shader_param_values: HashMap<usize, HashMap<String, ParamValue>>,

    /// Whether shader details section is expanded
    shader_details_expanded: bool,

    /// Power saving configuration
    power_saving: PowerSavingConfig,

    /// On battery action options for dropdown
    on_battery_action_options: Vec<String>,
    /// Selected on battery action index
    selected_on_battery_action: usize,

    /// Low battery threshold options for dropdown
    low_battery_threshold_options: Vec<String>,
    /// Selected low battery threshold index
    selected_low_battery_threshold: usize,

    /// Window background opacity (0.0 = transparent, 1.0 = opaque)
    window_opacity: f32,

    /// Extend-on-all-screens state
    extend_config: ExtendConfig,
    /// Monitor geometry (loaded when extend editor opens)
    monitor_geometry: Vec<crate::monitor_query::MonitorGeometry>,
    /// Which layer's context menu is showing in the canvas, and where
    layer_context_menu: Option<(DefaultKey, (f32, f32))>,
    /// Image layers on the virtual desktop canvas
    extend_layers: SlotMap<DefaultKey, ExtendLayerState>,
    /// For canvas items that represent a color (color mode): the color to fill.
    extend_layer_colors: SecondaryMap<DefaultKey, Color>,
    /// For canvas items that represent a color or live shader: the config source
    /// to write when applying to a display, instead of an image path.
    extend_layer_sources: SecondaryMap<DefaultKey, Source>,
    /// Currently selected layer
    extend_selected_layer: Option<DefaultKey>,
    /// Next z-index to assign
    extend_next_z: usize,
    /// Request the canvas to fit all content in view
    extend_fit_view_requested: bool,
}

#[derive(Clone, Debug)]
struct ExtendLayerState {
    source_path: PathBuf,
    image_handle: Option<ImageHandle>,
    image_size: (u32, u32),
    offset: (f64, f64),
    scale: f64,
    z_index: usize,
    locked: bool,
    target_output: Option<String>,
}

/// Information about an available shader
#[derive(Clone, Debug)]
pub struct ShaderInfo {
    pub path: PathBuf,
    pub name: String,
    /// Parsed shader with metadata and parameters
    pub parsed: Option<ParsedShader>,
}

/// What is currently selected
#[derive(Clone, Debug, PartialEq)]
enum Choice {
    Wallpaper(DefaultKey),
    Color(Color),
    Shader(usize),
}

impl Default for Choice {
    fn default() -> Self {
        Self::Wallpaper(DefaultKey::default())
    }
}

/// Selection context containing wallpapers, colors, and state
#[derive(Clone, Debug, Default)]
struct SelectionContext {
    active: Choice,
    paths: SlotMap<DefaultKey, PathBuf>,
    display_images: SecondaryMap<DefaultKey, ImageBuffer<Rgba<u8>, Vec<u8>>>,
    selection_handles: SecondaryMap<DefaultKey, ImageHandle>,
}

/// Category options for the dropdown
#[derive(Clone, Debug, PartialEq)]
pub enum Category {
    Wallpapers,
    Colors,
    Shaders,
}

/// Application messages
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Message {
    /// Category changed (from dropdown)
    ChangeCategory(Category),
    /// Category toggle changed (from header toggle, index: 0=wallpapers, 1=colors, 2=shaders)
    CategoryToggle(usize),
    /// Wallpaper selected
    Select(DefaultKey),
    /// Color selected
    ColorSelect(Color),
    /// Apply a grid color (index) to all displays
    ColorApplyAll(usize),
    /// Apply a grid color (index) to one display (monitor index)
    ColorShowOn(usize, usize),
    /// Shader selected
    ShaderSelect(usize),
    /// Apply a grid shader (index) to all displays
    ShaderApplyAll(usize),
    /// Apply a grid shader (index) to one display (monitor index)
    ShaderShowOn(usize, usize),
    /// Shader thumbnail loaded
    ShaderThumbnail(usize, Option<ImageHandle>),
    /// Frame rate changed
    ShaderFrameRate(usize),
    /// Fit mode changed
    Fit(usize),
    /// Wallpaper event from subscription
    WallpaperEvent(WallpaperEvent),
    /// Open a file picker to add image files to the grid
    AddWallpaperImages,
    /// Open a folder picker to add a directory to the grid
    AddWallpaperFolder,
    /// Paths chosen from a picker were added as wallpaper sources
    WallpaperSourcesPicked(Vec<PathBuf>),
    /// Remove a user-added wallpaper source by index
    RemoveWallpaperSource(usize),
    /// Toggle context drawer page
    ToggleContextPage(ContextPage),
    /// Open URL (for about page links)
    OpenUrl(String),
    /// Same wallpaper on all displays (used by colors/shaders)
    SameWallpaper(bool),
    /// Display output changed (for per-display mode)
    OutputChanged(segmented_button::Entity),
    /// Prefer low power GPU toggle
    PreferLowPower(bool),
    /// Config or state changed externally (from daemon or another instance)
    ConfigOrStateChanged(Option<Config>),
    /// Toggle GlowBerry as the default background service
    SetGlowBerryDefault(bool),
    /// Result of setting GlowBerry as default
    SetGlowBerryDefaultResult(Result<bool, String>),
    /// Shader parameter changed (shader_index, param_name, value) - updates UI only
    ShaderParamChanged(usize, String, ParamValue),
    /// Shader parameter slider released - applies to config
    ShaderParamReleased,
    /// Toggle shader details section
    ToggleShaderDetails,
    /// Reset shader parameters to defaults
    ResetShaderParams(usize),

    // Power saving messages
    /// Change on battery action
    SetOnBatteryAction(usize),
    /// Toggle pause on low battery
    SetPauseOnLowBattery(bool),
    /// Change low battery threshold
    SetLowBatteryThreshold(usize),
    /// Toggle pause when lid closed
    SetPauseOnLidClosed(bool),

    /// Window opacity slider changed (live preview)
    SetWindowOpacity(f32),
    /// Window opacity slider released (save to config)
    WindowOpacityReleased,

    /// Bezel changed for a monitor (monitor_index, top, bottom, left, right)
    SetBezel(usize, f64, f64, f64, f64),
    /// Bezel slider released — save to config
    BezelReleased,

    /// Monitor geometry loaded from cosmic-randr
    MonitorsLoaded(Vec<crate::monitor_query::MonitorGeometry>),
    /// Add a wallpaper as a new layer (wallpaper key from selection)
    ExtendAddLayer(DefaultKey),
    /// Remove a layer
    ExtendRemoveLayer(DefaultKey),
    /// Layer moved in the editor
    ExtendLayerMoved(DefaultKey, f64, f64),
    /// Layer scaled in the editor
    ExtendLayerScaled(DefaultKey, f64),
    /// Layer selected/deselected
    ExtendLayerSelected(Option<DefaultKey>),
    /// Move selected layer up in z-order
    ExtendLayerUp,
    /// Move selected layer down in z-order
    ExtendLayerDown,
    /// Center the selected layer on the virtual desktop
    ExtendCenter,
    /// Apply extend configuration (composite and save)
    ApplyExtend,
    /// Extend compositing completed
    ExtendApplied(Result<Vec<(String, PathBuf)>, String>),
    /// Clear all layers
    ExtendClearAll,
    /// Toggle lock on a layer
    ExtendToggleLock(DefaultKey),

    /// Wallpaper clicked — show placement popup
    WallpaperClicked(DefaultKey),
    /// Close wallpaper popup
    WallpaperPopupClose,
    /// Add wallpaper as layer in the canvas
    WallpaperCustomize(DefaultKey),
    /// Set wallpaper on all screens (duplicate)
    WallpaperDuplicateAll(DefaultKey),
    /// Span wallpaper across all screens (add auto-scaled layer)
    WallpaperSpanAll(DefaultKey),
    /// Set wallpaper on a specific screen
    WallpaperShowOn(DefaultKey, String),
    /// Set wallpaper on a screen by monitor index
    WallpaperShowOnIdx(DefaultKey, usize),
    /// Right-click on a layer in the canvas (key, x, y relative to widget)
    ExtendLayerRightClick(DefaultKey, f32, f32),
    /// Close the canvas layer context menu
    ExtendLayerMenuClose,
    /// Duplicate a layer's image on all screens
    LayerDuplicateAll(DefaultKey),
    /// Set a layer's image on a specific screen
    LayerShowOn(DefaultKey, String),
    /// Reset canvas camera to fit all content
    ExtendFitView,
    /// Export wallpaper config to cosmic-bg (for lock screen)
    ExportToCosmicBg,
    /// Export completed
    ExportToCosmicBgDone(Result<(), String>),
    /// Bring a specific layer forward (z+1)
    ExtendLayerBringForward(DefaultKey),
    /// Send a specific layer back (z-1)
    ExtendLayerSendBack(DefaultKey),
}

/// Context menu actions for wallpaper thumbnails
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WallpaperAction {
    Customize(DefaultKey),
    DuplicateAll(DefaultKey),
    SpanAll(DefaultKey),
    ShowOn(DefaultKey, usize),
    /// Remove a user-added wallpaper source (by index).
    RemoveSource(usize),
}

impl menu::Action for WallpaperAction {
    type Message = Message;
    fn message(&self) -> Message {
        match self {
            Self::Customize(k) => Message::WallpaperCustomize(*k),
            Self::DuplicateAll(k) => Message::WallpaperDuplicateAll(*k),
            Self::SpanAll(k) => Message::WallpaperSpanAll(*k),
            Self::ShowOn(k, idx) => Message::WallpaperShowOnIdx(*k, *idx),
            Self::RemoveSource(idx) => Message::RemoveWallpaperSource(*idx),
        }
    }
}

/// Right-click actions for a color swatch in the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorAction {
    /// Apply this color (by index into `DEFAULT_COLORS`) to all displays.
    All(usize),
    /// Apply this color to a specific display (monitor index).
    ShowOn(usize, usize),
}

impl menu::Action for ColorAction {
    type Message = Message;
    fn message(&self) -> Message {
        match self {
            Self::All(c) => Message::ColorApplyAll(*c),
            Self::ShowOn(c, m) => Message::ColorShowOn(*c, *m),
        }
    }
}

/// Right-click actions for a shader thumbnail in the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShaderAction {
    /// Apply this shader (by index) to all displays.
    All(usize),
    /// Apply this shader to a specific display (monitor index).
    ShowOn(usize, usize),
}

impl menu::Action for ShaderAction {
    type Message = Message;
    fn message(&self) -> Message {
        match self {
            Self::All(s) => Message::ShaderApplyAll(*s),
            Self::ShowOn(s, m) => Message::ShaderShowOn(*s, *m),
        }
    }
}

/// Default colors available in the color picker
pub const DEFAULT_COLORS: &[Color] = &[
    Color::Single([0.580, 0.922, 0.922]),
    Color::Single([0.000, 0.286, 0.427]),
    Color::Single([1.000, 0.678, 0.000]),
    Color::Single([0.282, 0.725, 0.78]),
    Color::Single([0.333, 0.278, 0.259]),
    Color::Single([0.969, 0.878, 0.384]),
    Color::Single([0.063, 0.165, 0.298]),
    Color::Single([1.000, 0.843, 0.631]),
    Color::Single([0.976, 0.227, 0.514]),
    Color::Single([1.000, 0.612, 0.867]),
    Color::Single([0.812, 0.490, 1.000]),
    Color::Single([0.835, 0.549, 1.000]),
    Color::Single([0.243, 0.533, 1.000]),
    Color::Single([0.584, 0.769, 0.988]),
    Color::Gradient(Gradient {
        colors: Cow::Borrowed(&[[1.000, 0.678, 0.000], [0.282, 0.725, 0.78]]),
        radius: 180.0,
    }),
    Color::Gradient(Gradient {
        colors: Cow::Borrowed(&[[1.000, 0.843, 0.631], [0.58, 0.922, 0.922]]),
        radius: 180.0,
    }),
    Color::Gradient(Gradient {
        colors: Cow::Borrowed(&[[1.000, 0.612, 0.867], [0.976, 0.29, 0.514]]),
        radius: 180.0,
    }),
    Color::Gradient(Gradient {
        colors: Cow::Borrowed(&[[0.584, 0.769, 0.988], [0.063, 0.165, 0.298]]),
        radius: 180.0,
    }),
    Color::Gradient(Gradient {
        colors: Cow::Borrowed(&[[0.969, 0.878, 0.384], [0.333, 0.278, 0.259]]),
        radius: 180.0,
    }),
];

impl cosmic::Application for GlowBerrySettings {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(mut core: Core, _flags: Self::Flags) -> (Self, Task<Self::Message>) {
        // Disable the default content container so we can apply our own background with opacity
        core.window.content_container = false;

        // Load configuration
        let config_context = glowberry_config::context().ok();
        let config = config_context
            .as_ref()
            .and_then(|ctx| Config::load(ctx).ok())
            .unwrap_or_default();

        // Set up category dropdown
        let mut categories = dropdown::multi::model();
        categories.insert(dropdown::multi::list(
            None,
            vec![(fl!("category-wallpapers"), Category::Wallpapers)],
        ));
        categories.insert(dropdown::multi::list(
            None,
            vec![(fl!("category-colors"), Category::Colors)],
        ));
        categories.insert(dropdown::multi::list(
            None,
            vec![(fl!("category-shaders"), Category::Shaders)],
        ));
        categories.selected = Some(Category::Wallpapers);

        // Default wallpaper folder - search XDG data directories
        let current_folder = find_wallpaper_folder();

        // Pre-discover shaders so they're ready when user clicks "Live Wallpapers"
        let available_shaders = discover_shaders();
        let placeholder = create_shader_placeholder(158, 105);
        let shader_thumbnails = vec![placeholder; available_shaders.len()];

        // About information
        let about = widget::about::About::default()
            .name(fl!("app-title"))
            .version(glowberry_config::version_string())
            .icon(widget::icon::from_name(
                "io.github.hojjatabdollahi.glowberry",
            ))
            .author("Hojjat Abdollahi")
            .license("MPL-2.0")
            .links([(
                fl!("repository"),
                "https://github.com/hojjatabdollahi/glowberry",
            )]);

        let mut app = Self {
            core,
            config,
            config_context,
            context_page: ContextPage::default(),
            about,
            outputs: segmented_button::SingleSelectModel::default(),
            active_output: None,
            show_tab_bar: false,
            categories,
            selection: SelectionContext::default(),
            available_shaders,
            shader_thumbnails,
            selected_shader_frame_rate: 1, // 30 FPS default
            frame_rate_options: vec![fl!("fps-15"), fl!("fps-30"), fl!("fps-60")],
            fit_options: vec![fl!("fit-fill"), fl!("fit-fit")],
            selected_fit: 0,
            cached_display_handle: None,
            current_folder,
            wallpaper_sources: Vec::new(), // Will be set below from config
            prefer_low_power: true,        // Will be set below
            glowberry_is_default: is_glowberry_default(),
            shader_param_values: HashMap::new(),
            shader_details_expanded: false,
            power_saving: PowerSavingConfig::default(),
            on_battery_action_options: vec![
                fl!("action-nothing"),
                fl!("action-pause"),
                fl!("action-reduce-15"),
                fl!("action-reduce-10"),
                fl!("action-reduce-5"),
            ],
            selected_on_battery_action: 0, // Nothing default
            low_battery_threshold_options: vec![
                "10%".to_string(),
                "20%".to_string(),
                "30%".to_string(),
                "50%".to_string(),
            ],
            selected_low_battery_threshold: 1, // 20% default
            window_opacity: 1.0,               // Will be set below from config
            extend_config: ExtendConfig::default(),
            monitor_geometry: Vec::new(),

            layer_context_menu: None,
            extend_layers: SlotMap::new(),
            extend_layer_colors: SecondaryMap::new(),
            extend_layer_sources: SecondaryMap::new(),
            extend_selected_layer: None,
            extend_next_z: 0,
            extend_fit_view_requested: false,
        };

        // Load prefer_low_power, power saving, extend config, and window opacity from config
        if let Some(ctx) = &app.config_context {
            app.prefer_low_power = ctx.prefer_low_power();
            app.wallpaper_sources = ctx
                .0
                .get::<Vec<PathBuf>>("wallpaper-sources")
                .unwrap_or_default();
            app.power_saving = ctx.power_saving_config();
            app.window_opacity = ctx.window_opacity();
            app.extend_config = ctx.extend_config();

            // Set dropdown indices based on loaded config
            app.selected_on_battery_action = match app.power_saving.on_battery_action {
                OnBatteryAction::Nothing => 0,
                OnBatteryAction::Pause => 1,
                OnBatteryAction::ReduceTo15Fps => 2,
                OnBatteryAction::ReduceTo10Fps => 3,
                OnBatteryAction::ReduceTo5Fps => 4,
            };
            app.selected_low_battery_threshold = match app.power_saving.low_battery_threshold {
                10 => 0,
                20 => 1,
                30 => 2,
                50 => 3,
                _ => 1, // Default to 20%
            };
        }

        // Populate outputs from config first - these are the outputs that have been configured
        // The daemon adds outputs to config as it discovers them via Wayland
        app.populate_outputs_from_config();

        // Initialize selection from config (needs outputs to be populated first for per-display mode)
        app.init_from_config();

        // Set the window title and start loading shader thumbnails
        let title_task = if let Some(id) = app.core.main_window_id() {
            app.set_window_title(fl!("app-title"), id)
        } else {
            Task::none()
        };

        let shader_task = if !app.available_shaders.is_empty() {
            app.load_shader_thumbnails()
        } else {
            Task::none()
        };

        // Load monitor geometry for the multi-monitor canvas
        let monitor_task = Task::perform(crate::monitor_query::query_monitors(), |result| {
            cosmic::Action::App(Message::MonitorsLoaded(result.unwrap_or_default()))
        });

        (app, Task::batch([title_task, shader_task, monitor_task]))
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        // Default folder plus any user-added files/directories.
        let mut sources = vec![self.current_folder.clone()];
        sources.extend(self.wallpaper_sources.iter().cloned());
        let mut subscriptions = vec![
            // Wallpaper loading subscription
            wallpaper_subscription::wallpapers(sources).map(Message::WallpaperEvent),
        ];

        // Watch for state changes from daemon (connected outputs, wallpaper state)
        // State implements CosmicConfigEntry and triggers on both config and state changes
        if self.config_context.is_some() {
            subscriptions.push(
                cosmic_config::config_subscription::<_, State>(
                    std::any::TypeId::of::<Self>(),
                    glowberry_config::NAME.into(),
                    State::version(),
                )
                .map(|update| {
                    if !update.errors.is_empty() {
                        for why in &update.errors {
                            tracing::error!(?why, "state subscription error");
                        }
                    }
                    // Reload config and refresh state
                    let config = glowberry_config::context()
                        .ok()
                        .and_then(|ctx| Config::load(&ctx).ok());
                    Message::ConfigOrStateChanged(config)
                }),
            );
        }

        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        // Clear one-shot flags
        self.extend_fit_view_requested = false;

        match message {
            Message::CategoryToggle(index) => {
                let category = match index {
                    0 => Category::Wallpapers,
                    1 => Category::Colors,
                    _ => Category::Shaders,
                };
                return self.update(Message::ChangeCategory(category));
            }

            Message::ChangeCategory(category) => {
                self.layer_context_menu = None;
                let changed = self.categories.selected.as_ref() != Some(&category);
                self.categories.selected = Some(category.clone());

                if changed {
                    // Load this page's saved working state so switching tabs
                    // isn't a fresh start.
                    self.load_category_canvas(&category);
                }

                if category == Category::Shaders {
                    // Load shaders if needed
                    if self.available_shaders.is_empty() {
                        self.available_shaders = discover_shaders();
                        let placeholder = create_shader_placeholder(158, 105);
                        self.shader_thumbnails = vec![placeholder; self.available_shaders.len()];
                    }
                    // Always try to load real thumbnails when switching to shaders
                    if !self.available_shaders.is_empty() {
                        return self.load_shader_thumbnails();
                    }
                }
            }

            Message::Select(id) => {
                self.selection.active = Choice::Wallpaper(id);
                self.cache_display_image();
                self.apply_selection();
            }

            Message::ColorSelect(color) => {
                self.selection.active = Choice::Color(color.clone());
                self.cached_display_handle = None;
                // Remember this as the color page's saved selection.
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.0.set("saved-color", color.clone());
                }
                // Stage on the canvas; the user applies via the right-click menu.
                if let Some(source) = self.build_active_source() {
                    self.fill_monitors_locked(PathBuf::new(), None, (0, 0), Some(color), source);
                }
            }

            Message::ShaderSelect(idx) => {
                if idx < self.available_shaders.len() {
                    self.selection.active = Choice::Shader(idx);
                    self.cached_display_handle = None;
                    let handle = self.shader_thumbnails.get(idx).cloned();
                    if let Some(source) = self.build_active_source() {
                        // Remember this as the live page's saved selection.
                        if let Some(ctx) = &self.config_context {
                            let _ = ctx.0.set("saved-shader", source.clone());
                        }
                        self.fill_monitors_locked(
                            PathBuf::new(),
                            handle,
                            (1920, 1080),
                            None,
                            source,
                        );
                    }
                }
            }

            Message::ColorApplyAll(color_idx) => {
                let Some(color) = DEFAULT_COLORS.get(color_idx).cloned() else {
                    return Task::none();
                };
                self.selection.active = Choice::Color(color.clone());
                self.cached_display_handle = None;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.0.set("saved-color", color.clone());
                }
                self.apply_content_to_all(Some(color.clone()), Source::Color(color), None, (0, 0));
            }

            Message::ColorShowOn(color_idx, monitor_idx) => {
                let Some(color) = DEFAULT_COLORS.get(color_idx).cloned() else {
                    return Task::none();
                };
                self.selection.active = Choice::Color(color.clone());
                self.cached_display_handle = None;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.0.set("saved-color", color.clone());
                }
                self.apply_content_to_output(
                    Some(color.clone()),
                    Source::Color(color),
                    None,
                    (0, 0),
                    monitor_idx,
                );
            }

            Message::ShaderApplyAll(idx) => {
                if idx < self.available_shaders.len() {
                    self.selection.active = Choice::Shader(idx);
                    self.cached_display_handle = None;
                    let handle = self.shader_thumbnails.get(idx).cloned();
                    if let Some(source) = self.build_active_source() {
                        if let Some(ctx) = &self.config_context {
                            let _ = ctx.0.set("saved-shader", source.clone());
                        }
                        self.apply_content_to_all(None, source, handle, (1920, 1080));
                    }
                }
            }

            Message::ShaderShowOn(idx, monitor_idx) => {
                if idx < self.available_shaders.len() {
                    self.selection.active = Choice::Shader(idx);
                    self.cached_display_handle = None;
                    let handle = self.shader_thumbnails.get(idx).cloned();
                    if let Some(source) = self.build_active_source() {
                        if let Some(ctx) = &self.config_context {
                            let _ = ctx.0.set("saved-shader", source.clone());
                        }
                        self.apply_content_to_output(
                            None,
                            source,
                            handle,
                            (1920, 1080),
                            monitor_idx,
                        );
                    }
                }
            }

            Message::ShaderThumbnail(idx, handle) => {
                if let Some(handle) = handle
                    && idx < self.shader_thumbnails.len()
                {
                    self.shader_thumbnails[idx] = handle.clone();
                    // Update any staged canvas items showing this shader, so the
                    // per-output live preview shows the real thumbnail.
                    let to_update: Vec<DefaultKey> = self
                        .extend_layers
                        .keys()
                        .filter(|&k| {
                            self.extend_layer_sources
                                .get(k)
                                .is_some_and(|s| self.shader_idx_for_source(s) == Some(idx))
                        })
                        .collect();
                    for key in to_update {
                        if let Some(layer) = self.extend_layers.get_mut(key) {
                            layer.image_handle = Some(handle.clone());
                        }
                    }
                }
            }

            Message::ShaderFrameRate(idx) => {
                self.selected_shader_frame_rate = idx;
                self.apply_selection();
            }

            Message::Fit(idx) => {
                self.selected_fit = idx;
                self.cache_display_image();
                self.apply_selection();
            }

            Message::WallpaperEvent(event) => match event {
                WallpaperEvent::Loading => {
                    // Only reset the wallpaper-related data, preserve the active selection
                    // (which may be a Color or Shader from config)
                    self.selection.paths.clear();
                    self.selection.display_images.clear();
                    self.selection.selection_handles.clear();
                }
                WallpaperEvent::Load {
                    path,
                    display,
                    selection,
                } => {
                    let key = self.selection.paths.insert(path);
                    self.selection.display_images.insert(key, display);
                    self.selection.selection_handles.insert(
                        key,
                        ImageHandle::from_rgba(
                            selection.width(),
                            selection.height(),
                            selection.into_vec(),
                        ),
                    );
                }
                WallpaperEvent::Loaded => {
                    // Get the correct entry based on same_on_all and active_output
                    let entry = if self.config.same_on_all {
                        Some(&self.config.default_background)
                    } else if let Some(ref output_name) = self.active_output {
                        self.config.entry(output_name)
                    } else {
                        Some(&self.config.default_background)
                    };

                    // Only select a wallpaper if config source is a Path
                    // Don't override if user has a Color or Shader selected
                    if let Some(entry) = entry
                        && let Source::Path(config_path) = &entry.source
                    {
                        // Find the wallpaper that matches the config path
                        if let Some((key, _)) =
                            self.selection.paths.iter().find(|(_, p)| *p == config_path)
                        {
                            self.selection.active = Choice::Wallpaper(key);
                            self.categories.selected = Some(Category::Wallpapers);
                        } else {
                            // Config path not found in loaded wallpapers, pick first one
                            if let Some((key, _)) = self.selection.paths.iter().next() {
                                self.selection.active = Choice::Wallpaper(key);
                            }
                        }
                    }

                    // Only cache display image if a wallpaper is selected
                    if matches!(self.selection.active, Choice::Wallpaper(_)) {
                        self.cache_display_image();
                    }
                }
            },

            Message::AddWallpaperImages => {
                return Task::perform(
                    async {
                        cosmic::dialog::file_chooser::open::Dialog::new()
                            .open_files()
                            .await
                            .ok()
                            .map(|resp| {
                                resp.urls()
                                    .iter()
                                    .filter_map(|u| u.to_file_path().ok())
                                    .collect::<Vec<PathBuf>>()
                            })
                            .unwrap_or_default()
                    },
                    |paths| cosmic::Action::App(Message::WallpaperSourcesPicked(paths)),
                );
            }

            Message::AddWallpaperFolder => {
                return Task::perform(
                    async {
                        cosmic::dialog::file_chooser::open::Dialog::new()
                            .open_folder()
                            .await
                            .ok()
                            .and_then(|resp| resp.url().to_file_path().ok())
                            .map(|p| vec![p])
                            .unwrap_or_default()
                    },
                    |paths| cosmic::Action::App(Message::WallpaperSourcesPicked(paths)),
                );
            }

            Message::WallpaperSourcesPicked(paths) => {
                let mut changed = false;
                for p in paths {
                    if p != self.current_folder && !self.wallpaper_sources.contains(&p) {
                        self.wallpaper_sources.push(p);
                        changed = true;
                    }
                }
                if changed {
                    if let Some(ctx) = &self.config_context {
                        let _ = ctx
                            .0
                            .set("wallpaper-sources", self.wallpaper_sources.clone());
                    }
                    // Surface the result on the wallpaper page.
                    self.categories.selected = Some(Category::Wallpapers);
                }
            }

            Message::RemoveWallpaperSource(idx) => {
                if idx < self.wallpaper_sources.len() {
                    self.wallpaper_sources.remove(idx);
                    if let Some(ctx) = &self.config_context {
                        let _ = ctx
                            .0
                            .set("wallpaper-sources", self.wallpaper_sources.clone());
                    }
                }
            }

            Message::ToggleContextPage(context_page) => {
                if self.context_page == context_page {
                    // Toggle visibility if same page
                    self.set_show_context(!self.core.window.show_context);
                } else {
                    // Switch to new page and show drawer
                    self.set_show_context(true);
                }
                self.context_page = context_page;
            }

            Message::OpenUrl(url) => {
                let _ = open::that_detached(&url);
            }

            Message::SameWallpaper(value) => {
                self.config.same_on_all = value;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_same_on_all(value);
                }
                // Clear per-output backgrounds when switching to same-on-all
                if value {
                    self.config.backgrounds.clear();
                    self.config.outputs.clear();
                }
                self.apply_selection();
            }

            Message::OutputChanged(entity) => {
                self.outputs.activate(entity);
                if let Some(name) = self.outputs.data::<OutputName>(entity) {
                    self.active_output = Some(name.0.clone());

                    // Load the wallpaper for this specific output if it exists
                    if let Some(entry) = self.config.entry(&name.0) {
                        self.select_entry_source(&entry.source.clone());
                    }
                }
                self.cache_display_image();
            }

            Message::PreferLowPower(value) => {
                self.prefer_low_power = value;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_prefer_low_power(value);
                }
            }

            Message::ConfigOrStateChanged(maybe_config) => {
                // Update config if provided and different
                if let Some(config) = maybe_config
                    && self.config != config
                {
                    tracing::debug!("Config changed externally, updating data");
                    // Refresh our copy of the config, but DON'T call
                    // init_from_config() here: it resets the selected category and
                    // selection from the applied wallpaper, and the daemon writes
                    // state frequently — so doing it on every change would wipe the
                    // page the user just navigated to (e.g. switching to Colors).
                    self.config = config;

                    // Update prefer_low_power from config
                    if let Some(ctx) = &self.config_context {
                        self.prefer_low_power = ctx.prefer_low_power();
                    }

                    // Re-cache display image if needed
                    if matches!(self.selection.active, Choice::Wallpaper(_)) {
                        self.cache_display_image();
                    }
                }

                // Always refresh connected outputs (state may have changed)
                self.populate_outputs_from_config();
            }

            Message::SetGlowBerryDefault(enable) => {
                // Run the enable/disable command asynchronously with pkexec
                return Task::perform(
                    async move { set_glowberry_default(enable).await },
                    |result| cosmic::Action::App(Message::SetGlowBerryDefaultResult(result)),
                );
            }

            Message::SetGlowBerryDefaultResult(result) => {
                match result {
                    Ok(is_default) => {
                        self.glowberry_is_default = is_default;
                        tracing::info!(
                            "GlowBerry is now {}",
                            if is_default { "enabled" } else { "disabled" }
                        );
                    }
                    Err(e) => {
                        tracing::error!("Failed to set GlowBerry default: {}", e);
                        // Refresh the actual state
                        self.glowberry_is_default = is_glowberry_default();
                    }
                }
            }

            Message::ShaderParamChanged(shader_idx, param_name, value) => {
                // Store the new value in memory only (don't write to config yet)
                self.shader_param_values
                    .entry(shader_idx)
                    .or_default()
                    .insert(param_name, value);
                // UI will update to show new value, but config is not written
            }

            Message::ShaderParamReleased => {
                // Apply the shader with current parameters when slider is released
                if matches!(self.selection.active, Choice::Shader(_)) {
                    self.apply_selection();
                }
            }

            Message::ToggleShaderDetails => {
                self.shader_details_expanded = !self.shader_details_expanded;
            }

            Message::ResetShaderParams(shader_idx) => {
                // Remove all custom parameter values for this shader
                self.shader_param_values.remove(&shader_idx);

                // Re-apply the shader with default parameters
                if let Choice::Shader(idx) = self.selection.active
                    && idx == shader_idx
                {
                    self.apply_selection();
                }
            }

            // Power saving messages
            Message::SetOnBatteryAction(idx) => {
                self.selected_on_battery_action = idx;
                let action = match idx {
                    0 => OnBatteryAction::Nothing,
                    1 => OnBatteryAction::Pause,
                    2 => OnBatteryAction::ReduceTo15Fps,
                    3 => OnBatteryAction::ReduceTo10Fps,
                    4 => OnBatteryAction::ReduceTo5Fps,
                    _ => OnBatteryAction::Nothing,
                };
                self.power_saving.on_battery_action = action;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_on_battery_action(action);
                }
            }

            Message::SetPauseOnLowBattery(value) => {
                self.power_saving.pause_on_low_battery = value;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_pause_on_low_battery(value);
                }
            }

            Message::SetLowBatteryThreshold(idx) => {
                self.selected_low_battery_threshold = idx;
                let threshold = match idx {
                    0 => 10,
                    1 => 20,
                    2 => 30,
                    3 => 50,
                    _ => 20,
                };
                self.power_saving.low_battery_threshold = threshold;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_low_battery_threshold(threshold);
                }
            }

            Message::SetPauseOnLidClosed(value) => {
                self.power_saving.pause_on_lid_closed = value;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_pause_on_lid_closed(value);
                }
            }

            Message::SetWindowOpacity(value) => {
                // Update the opacity value for live preview
                self.window_opacity = value.clamp(0.0, 1.0);
            }

            Message::SetBezel(idx, top, bottom, left, right) => {
                if let Some(mon) = self.monitor_geometry.get_mut(idx) {
                    mon.bezel = glowberry_config::extend::Bezel {
                        top,
                        bottom,
                        left,
                        right,
                    };
                }
            }

            Message::BezelReleased => {
                if let Some(ctx) = &self.config_context {
                    let mut bezels = glowberry_config::extend::ExtendConfig::load_bezels(ctx);
                    for mon in &self.monitor_geometry {
                        bezels.insert(mon.bezel_key(), mon.bezel.clone());
                    }
                    let _ = glowberry_config::extend::ExtendConfig::save_bezels(ctx, &bezels);
                }
            }

            Message::WindowOpacityReleased => {
                // Save the opacity value to config when slider is released
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_window_opacity(self.window_opacity);
                }
            }

            Message::MonitorsLoaded(mut monitors) => {
                // Apply saved bezel config per monitor
                if let Some(ctx) = &self.config_context {
                    let bezels = glowberry_config::extend::ExtendConfig::load_bezels(ctx);
                    for mon in &mut monitors {
                        if let Some(bezel) = bezels.get(&mon.bezel_key()) {
                            mon.bezel = bezel.clone();
                        }
                    }
                }
                self.monitor_geometry = monitors;
                // Stage the current page's saved content now that monitors exist
                // (only on first load, so monitor hotplugs don't wipe edits).
                if self.extend_layers.is_empty() {
                    let monitor_names: Vec<String> = self
                        .monitor_geometry
                        .iter()
                        .map(|m| m.name.clone())
                        .collect();
                    if let Some(ctx) = &self.config_context {
                        let layers = glowberry_config::extend::ExtendConfig::load_for_displays(
                            ctx,
                            &monitor_names,
                        );
                        if !layers.is_empty() {
                            self.extend_config.layers = layers;
                        }
                    }
                    let cat = self
                        .categories
                        .selected
                        .clone()
                        .unwrap_or(Category::Wallpapers);
                    self.load_category_canvas(&cat);
                    self.extend_fit_view_requested = true;
                    // Render shader thumbnails so the per-output live preview
                    // shows the actual shaders, not placeholders.
                    if cat == Category::Shaders && !self.available_shaders.is_empty() {
                        return self.load_shader_thumbnails();
                    }
                }
                self.extend_fit_view_requested = true;
            }

            Message::ExtendAddLayer(wp_key) => {
                let Some(path) = self.selection.paths.get(wp_key).cloned() else {
                    return Task::none();
                };
                let image_handle = self
                    .selection
                    .display_images
                    .get(wp_key)
                    .map(|img| ImageHandle::from_rgba(img.width(), img.height(), img.to_vec()));
                let image_size = image::image_dimensions(&path).unwrap_or((800, 600));

                let z = self.extend_next_z;
                self.extend_next_z += 1;

                let mut layer = ExtendLayerState {
                    source_path: path,
                    image_handle,
                    image_size,
                    offset: (0.0, 0.0),
                    scale: 1.0,
                    z_index: z,
                    locked: false,
                    target_output: None,
                };
                self.auto_center_layer(&mut layer);
                let key = self.extend_layers.insert(layer);
                self.extend_selected_layer = Some(key);
            }

            Message::ExtendRemoveLayer(key) => {
                self.layer_context_menu = None;
                self.extend_layers.remove(key);
                self.extend_layer_colors.remove(key);
                self.extend_layer_sources.remove(key);
                if self.extend_selected_layer == Some(key) {
                    self.extend_selected_layer = None;
                }
                self.renormalize_z_indices();
                self.persist_canvas_per_output();
            }

            Message::ExtendLayerMoved(key, x, y) => {
                self.layer_context_menu = None;
                if let Some(layer) = self.extend_layers.get_mut(key) {
                    layer.offset = (x, y);
                }
            }

            Message::ExtendLayerScaled(key, scale) => {
                self.layer_context_menu = None;
                if let Some(layer) = self.extend_layers.get_mut(key) {
                    layer.scale = scale;
                }
            }

            Message::ExtendLayerSelected(maybe_key) => {
                self.extend_selected_layer = maybe_key;
                self.layer_context_menu = None;
            }

            Message::ExtendLayerUp => {
                if let Some(sel_key) = self.extend_selected_layer {
                    let sel_z = self.extend_layers[sel_key].z_index;
                    if let Some((swap_key, _)) = self
                        .extend_layers
                        .iter()
                        .filter(|(k, l)| *k != sel_key && l.z_index > sel_z)
                        .min_by_key(|(_, l)| l.z_index)
                    {
                        let swap_z = self.extend_layers[swap_key].z_index;
                        self.extend_layers[sel_key].z_index = swap_z;
                        self.extend_layers[swap_key].z_index = sel_z;
                    }
                }
            }

            Message::ExtendLayerDown => {
                if let Some(sel_key) = self.extend_selected_layer {
                    let sel_z = self.extend_layers[sel_key].z_index;
                    if let Some((swap_key, _)) = self
                        .extend_layers
                        .iter()
                        .filter(|(k, l)| *k != sel_key && l.z_index < sel_z)
                        .max_by_key(|(_, l)| l.z_index)
                    {
                        let swap_z = self.extend_layers[swap_key].z_index;
                        self.extend_layers[sel_key].z_index = swap_z;
                        self.extend_layers[swap_key].z_index = sel_z;
                    }
                }
            }

            Message::ExtendCenter => {
                if let Some(sel_key) = self.extend_selected_layer {
                    let mut layer = self.extend_layers[sel_key].clone();
                    self.auto_center_layer(&mut layer);
                    self.extend_layers[sel_key] = layer;
                }
            }

            Message::ApplyExtend => {
                if self.extend_layers.is_empty() {
                    return Task::none();
                }

                // Ensure per-output mode is active so the daemon loads per-output wallpapers
                self.config.same_on_all = false;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_same_on_all(false);
                }

                // 1. Save locked layers directly as per-output wallpapers. Color
                // and live items carry a source override (color/shader); images
                // fall back to their path.
                let mut locked_monitors: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                let mut locked_entries: Vec<(String, Source)> = Vec::new();
                for (key, layer) in self.extend_layers.iter() {
                    if layer.locked
                        && let Some(ref output) = layer.target_output
                    {
                        let source = self
                            .extend_layer_sources
                            .get(key)
                            .cloned()
                            .unwrap_or_else(|| Source::Path(layer.source_path.clone()));
                        locked_entries.push((output.clone(), source));
                        locked_monitors.insert(output.clone());
                    }
                }
                for (output, source) in locked_entries {
                    let entry = Entry::new(output, source);
                    if let Some(ctx) = &self.config_context
                        && let Err(e) = self.config.set_entry(ctx, entry)
                    {
                        tracing::error!("Failed to set locked wallpaper: {}", e);
                    }
                }

                // 2. Composite unlocked layers for monitors not covered by locked layers
                let unlocked_layers: Vec<glowberry_lib::extend_crop::LayerInfo> = self
                    .extend_layers
                    .values()
                    .filter(|l| !l.locked)
                    .map(|l| glowberry_lib::extend_crop::LayerInfo {
                        source_path: l.source_path.clone(),
                        offset: l.offset,
                        img_scale: l.scale,
                        z_index: l.z_index,
                    })
                    .collect();

                let monitors_to_composite: Vec<glowberry_lib::extend_crop::MonitorInfo> = self
                    .monitor_geometry
                    .iter()
                    .filter(|m| !locked_monitors.contains(&m.name))
                    .map(|m| glowberry_lib::extend_crop::MonitorInfo {
                        name: m.name.clone(),
                        position: m.position,
                        logical_size: m.logical_size,
                        physical_size: m.physical_size,
                        scale: m.scale,
                    })
                    .collect();

                // Persist the multi-monitor image layout only when applying from
                // the wallpaper page. On the color/live pages the canvas holds
                // color/shader items (no image layers), so saving here would wipe
                // the wallpaper page's saved layout.
                if matches!(self.categories.selected, Some(Category::Wallpapers)) {
                    self.extend_config.layers = self
                        .extend_layers
                        .values()
                        .filter(|l| !l.source_path.as_os_str().is_empty())
                        .map(|l| glowberry_config::extend::ExtendLayer {
                            source_path: l.source_path.clone(),
                            img_offset_x: l.offset.0,
                            img_offset_y: l.offset.1,
                            img_scale: l.scale,
                            z_index: l.z_index,
                            locked: l.locked,
                            target_output: l.target_output.clone(),
                        })
                        .collect();
                    if let Some(ctx) = &self.config_context {
                        let _ = ctx.save_extend_config(&self.extend_config);
                        let monitor_names: Vec<String> = self
                            .monitor_geometry
                            .iter()
                            .map(|m| m.name.clone())
                            .collect();
                        let _ = glowberry_config::extend::ExtendConfig::save_for_displays(
                            ctx,
                            &monitor_names,
                            &self.extend_config.layers,
                        );
                    }
                }

                if unlocked_layers.is_empty() || monitors_to_composite.is_empty() {
                    // Nothing to composite — locked layers already saved
                    return Task::none();
                }

                let mut layer_infos = unlocked_layers;
                let cache_dir = glowberry_lib::extend_crop::cache_dir();

                return Task::perform(
                    async move {
                        glowberry_lib::extend_crop::composite_for_monitors(
                            &mut layer_infos,
                            &monitors_to_composite,
                            &cache_dir,
                        )
                        .map_err(|e| e.to_string())
                    },
                    |result| cosmic::Action::App(Message::ExtendApplied(result)),
                );
            }

            Message::ExtendApplied(result) => match result {
                Ok(crops) => {
                    if let Some(ctx) = &self.config_context {
                        for (output_name, cached_path) in crops {
                            let entry = Entry::new(output_name, Source::Path(cached_path));
                            if let Err(e) = self.config.set_entry(ctx, entry) {
                                tracing::error!("Failed to set wallpaper: {}", e);
                            }
                        }
                    }
                    tracing::info!("Multi-monitor wallpapers applied");
                }
                Err(e) => {
                    tracing::error!("Failed to composite wallpapers: {}", e);
                }
            },

            Message::ExtendClearAll => {
                self.extend_layers.clear();
                self.extend_layer_colors.clear();
                self.extend_layer_sources.clear();
                self.extend_selected_layer = None;
                self.extend_next_z = 0;
            }

            Message::ExtendToggleLock(key) => {
                if let Some(layer) = self.extend_layers.get_mut(key) {
                    layer.locked = !layer.locked;
                    if !layer.locked {
                        layer.target_output = None;
                    }
                }
            }

            Message::WallpaperClicked(_key) => {}

            Message::WallpaperPopupClose => {}

            Message::WallpaperCustomize(key) => {
                let Some(path) = self.selection.paths.get(key).cloned() else {
                    return Task::none();
                };
                let image_handle = self
                    .selection
                    .display_images
                    .get(key)
                    .map(|img| ImageHandle::from_rgba(img.width(), img.height(), img.to_vec()));
                let image_size = image::image_dimensions(&path).unwrap_or((800, 600));
                let z = self.extend_next_z;
                self.extend_next_z += 1;
                let mut layer = ExtendLayerState {
                    source_path: path,
                    image_handle,
                    image_size,
                    offset: (0.0, 0.0),
                    scale: 1.0,
                    z_index: z,
                    locked: false,
                    target_output: None,
                };
                self.auto_center_layer(&mut layer);
                let lkey = self.extend_layers.insert(layer);
                self.extend_selected_layer = Some(lkey);
            }

            Message::WallpaperDuplicateAll(key) => {
                let Some(path) = self.selection.paths.get(key).cloned() else {
                    return Task::none();
                };

                // Apply to config: set wallpaper on all screens
                self.config.same_on_all = true;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_same_on_all(true);
                }
                let entry = Entry::new("all".to_string(), Source::Path(path.clone()));
                if let Some(ctx) = &self.config_context
                    && let Err(e) = self.config.set_entry(ctx, entry)
                {
                    tracing::error!("Failed to set wallpaper: {}", e);
                }

                // Clear existing layers and add one per monitor to show in preview
                self.extend_layers.clear();
                self.extend_selected_layer = None;
                self.extend_next_z = 0;

                let image_handle = self
                    .selection
                    .display_images
                    .get(key)
                    .map(|img| ImageHandle::from_rgba(img.width(), img.height(), img.to_vec()));
                let image_size = image::image_dimensions(&path).unwrap_or((800, 600));

                for monitor in &self.monitor_geometry {
                    let mon_w = monitor.logical_size.0 as f64;
                    let mon_h = monitor.logical_size.1 as f64;
                    let img_w = image_size.0 as f64;
                    let img_h = image_size.1 as f64;
                    // Scale to cover each monitor (zoom mode)
                    let scale = (mon_w / img_w).max(mon_h / img_h);
                    let offset_x = monitor.position.0 as f64 + (mon_w - img_w * scale) / 2.0;
                    let offset_y = monitor.position.1 as f64 + (mon_h - img_h * scale) / 2.0;

                    let z = self.extend_next_z;
                    self.extend_next_z += 1;
                    self.extend_layers.insert(ExtendLayerState {
                        source_path: path.clone(),
                        image_handle: image_handle.clone(),
                        image_size,
                        offset: (offset_x, offset_y),
                        scale,
                        z_index: z,
                        locked: true,
                        target_output: Some(monitor.name.clone()),
                    });
                }
            }

            Message::WallpaperSpanAll(key) => {
                let Some(path) = self.selection.paths.get(key).cloned() else {
                    return Task::none();
                };
                let image_handle = self
                    .selection
                    .display_images
                    .get(key)
                    .map(|img| ImageHandle::from_rgba(img.width(), img.height(), img.to_vec()));
                let image_size = image::image_dimensions(&path).unwrap_or((800, 600));
                let z = self.extend_next_z;
                self.extend_next_z += 1;
                let mut layer = ExtendLayerState {
                    source_path: path,
                    image_handle,
                    image_size,
                    offset: (0.0, 0.0),
                    scale: 1.0,
                    z_index: z,
                    locked: false,
                    target_output: None,
                };
                self.auto_center_layer(&mut layer);
                let lkey = self.extend_layers.insert(layer);
                self.extend_selected_layer = Some(lkey);
            }

            Message::WallpaperShowOn(key, screen_name) => {
                let Some(path) = self.selection.paths.get(key).cloned() else {
                    return Task::none();
                };

                // Ensure per-output mode
                self.config.same_on_all = false;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_same_on_all(false);
                }

                // Write config directly
                let entry = Entry::new(screen_name.clone(), Source::Path(path.clone()));
                if let Some(ctx) = &self.config_context
                    && let Err(e) = self.config.set_entry(ctx, entry)
                {
                    tracing::error!("Failed to set wallpaper: {}", e);
                }

                // Add locked layer to preview on the target monitor
                if let Some(monitor) = self.monitor_geometry.iter().find(|m| m.name == screen_name)
                {
                    let image_handle =
                        self.selection.display_images.get(key).map(|img| {
                            ImageHandle::from_rgba(img.width(), img.height(), img.to_vec())
                        });
                    let image_size = image::image_dimensions(&path).unwrap_or((800, 600));
                    let mon_w = monitor.logical_size.0 as f64;
                    let mon_h = monitor.logical_size.1 as f64;
                    let img_w = image_size.0 as f64;
                    let img_h = image_size.1 as f64;
                    let scale = (mon_w / img_w).max(mon_h / img_h);
                    let offset_x = monitor.position.0 as f64 + (mon_w - img_w * scale) / 2.0;
                    let offset_y = monitor.position.1 as f64 + (mon_h - img_h * scale) / 2.0;

                    let z = self.extend_next_z;
                    self.extend_next_z += 1;
                    self.extend_layers.insert(ExtendLayerState {
                        source_path: path,
                        image_handle,
                        image_size,
                        offset: (offset_x, offset_y),
                        scale,
                        z_index: z,
                        locked: true,
                        target_output: Some(screen_name),
                    });
                }
            }

            Message::WallpaperShowOnIdx(key, idx) => {
                if let Some(monitor) = self.monitor_geometry.get(idx) {
                    let name = monitor.name.clone();
                    return self.update(Message::WallpaperShowOn(key, name));
                }
            }

            Message::LayerDuplicateAll(layer_key) => {
                self.layer_context_menu = None;
                let Some(layer) = self.extend_layers.get(layer_key) else {
                    return Task::none();
                };
                let path = layer.source_path.clone();
                let image_handle = layer.image_handle.clone();
                let image_size = layer.image_size;
                let color = self.extend_layer_colors.get(layer_key).cloned();
                let source = self
                    .extend_layer_sources
                    .get(layer_key)
                    .cloned()
                    .unwrap_or_else(|| Source::Path(path.clone()));

                // Apply the content to all displays.
                self.config.same_on_all = true;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_same_on_all(true);
                }
                let entry = Entry::new("all".to_string(), source.clone());
                if let Some(ctx) = &self.config_context
                    && let Err(e) = self.config.set_entry(ctx, entry)
                {
                    tracing::error!("Failed to set wallpaper: {}", e);
                }

                // Replace layers with locked per-monitor items.
                self.fill_monitors_locked(path, image_handle, image_size, color, source);
            }

            Message::LayerShowOn(layer_key, screen_name) => {
                self.layer_context_menu = None;
                let Some(layer) = self.extend_layers.get(layer_key) else {
                    return Task::none();
                };
                let path = layer.source_path.clone();
                let image_handle = layer.image_handle.clone();
                let image_size = layer.image_size;
                let color = self.extend_layer_colors.get(layer_key).cloned();
                let source = self
                    .extend_layer_sources
                    .get(layer_key)
                    .cloned()
                    .unwrap_or_else(|| Source::Path(path.clone()));

                // Ensure per-output mode
                self.config.same_on_all = false;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_same_on_all(false);
                }
                let entry = Entry::new(screen_name.clone(), source.clone());
                if let Some(ctx) = &self.config_context
                    && let Err(e) = self.config.set_entry(ctx, entry)
                {
                    tracing::error!("Failed to set wallpaper: {}", e);
                }

                // Add a locked item on the target monitor, removing any existing
                // one for that output first so it isn't duplicated.
                let existing: Vec<DefaultKey> = self
                    .extend_layers
                    .iter()
                    .filter(|(_, l)| l.target_output.as_deref() == Some(screen_name.as_str()))
                    .map(|(k, _)| k)
                    .collect();
                for k in existing {
                    self.extend_layers.remove(k);
                    self.extend_layer_colors.remove(k);
                    self.extend_layer_sources.remove(k);
                }
                if let Some(monitor) = self.monitor_geometry.iter().find(|m| m.name == screen_name)
                {
                    let mon_w = monitor.logical_size.0 as f64;
                    let mon_h = monitor.logical_size.1 as f64;
                    let (size, scale, offset) = if color.is_some() {
                        (
                            monitor.logical_size,
                            1.0,
                            (monitor.position.0 as f64, monitor.position.1 as f64),
                        )
                    } else {
                        let img_w = image_size.0.max(1) as f64;
                        let img_h = image_size.1.max(1) as f64;
                        let scale = (mon_w / img_w).max(mon_h / img_h);
                        (
                            image_size,
                            scale,
                            (
                                monitor.position.0 as f64 + (mon_w - img_w * scale) / 2.0,
                                monitor.position.1 as f64 + (mon_h - img_h * scale) / 2.0,
                            ),
                        )
                    };
                    let z = self.extend_next_z;
                    self.extend_next_z += 1;
                    let key = self.extend_layers.insert(ExtendLayerState {
                        source_path: path,
                        image_handle,
                        image_size: size,
                        offset,
                        scale,
                        z_index: z,
                        locked: true,
                        target_output: Some(screen_name),
                    });
                    if let Some(c) = &color {
                        self.extend_layer_colors.insert(key, c.clone());
                    }
                    self.extend_layer_sources.insert(key, source);
                }
                self.persist_canvas_per_output();
            }

            Message::ExtendFitView => {
                self.extend_fit_view_requested = true;
            }

            Message::ExportToCosmicBg => {
                if self.extend_layers.is_empty() {
                    return Task::none();
                }

                // Collect locked layers (direct per-output entries). Color/live
                // items (empty path) can't be shown on the lock screen, so skip
                // them — the greeter only supports image paths.
                let mut locked_entries: Vec<(String, PathBuf)> = Vec::new();
                // Live shaders can't be shown on the lock screen directly, so we
                // render a static snapshot per output and export that image.
                let mut shader_jobs: Vec<(String, PathBuf, (u32, u32))> = Vec::new();
                for (key, layer) in self.extend_layers.iter() {
                    if !layer.locked {
                        continue;
                    }
                    let Some(output) = layer.target_output.clone() else {
                        continue;
                    };
                    if let Some(glowberry_config::Source::Shader(shader)) =
                        self.extend_layer_sources.get(key)
                    {
                        let render_path = match &shader.shader {
                            glowberry_config::ShaderContent::Path(p) => Some(p.clone()),
                            glowberry_config::ShaderContent::Code(_) => shader.source_path.clone(),
                        };
                        if let Some(path) = render_path {
                            let size = self
                                .monitor_geometry
                                .iter()
                                .find(|m| m.name == output)
                                .map(|m| m.physical_size)
                                .unwrap_or((1920, 1080));
                            shader_jobs.push((output, path, size));
                        }
                    } else if !layer.source_path.as_os_str().is_empty() {
                        locked_entries.push((output, layer.source_path.clone()));
                    }
                }

                // Collect unlocked layers for compositing
                let mut unlocked_layer_infos: Vec<glowberry_lib::extend_crop::LayerInfo> = self
                    .extend_layers
                    .values()
                    .filter(|l| !l.locked)
                    .map(|l| glowberry_lib::extend_crop::LayerInfo {
                        source_path: l.source_path.clone(),
                        offset: l.offset,
                        img_scale: l.scale,
                        z_index: l.z_index,
                    })
                    .collect();

                let locked_monitor_names: std::collections::HashSet<String> = locked_entries
                    .iter()
                    .map(|(n, _)| n.clone())
                    .chain(shader_jobs.iter().map(|(n, _, _)| n.clone()))
                    .collect();

                let monitors_to_composite: Vec<glowberry_lib::extend_crop::MonitorInfo> = self
                    .monitor_geometry
                    .iter()
                    .filter(|m| !locked_monitor_names.contains(&m.name))
                    .map(|m| glowberry_lib::extend_crop::MonitorInfo {
                        name: m.name.clone(),
                        position: m.position,
                        logical_size: m.logical_size,
                        physical_size: m.physical_size,
                        scale: m.scale,
                    })
                    .collect();

                let cache_dir = glowberry_lib::extend_crop::cache_dir();

                return Task::perform(
                    async move {
                        // Composite unlocked layers
                        let mut composited: Vec<(String, PathBuf)> = Vec::new();
                        if !unlocked_layer_infos.is_empty() && !monitors_to_composite.is_empty() {
                            composited = glowberry_lib::extend_crop::composite_for_monitors(
                                &mut unlocked_layer_infos,
                                &monitors_to_composite,
                                &cache_dir,
                            )
                            .map_err(|e| e.to_string())?;
                        }

                        // Render a static snapshot of each live shader to an image
                        // so the lock screen (which can't run shaders) can show it.
                        if !shader_jobs.is_empty() {
                            let cache = cache_dir.clone();
                            let rendered = tokio::task::spawn_blocking(move || {
                                let mut out: Vec<(String, PathBuf)> = Vec::new();
                                for (output, path, (w, h)) in shader_jobs {
                                    match crate::widgets::shader_preview::render_shader_preview(
                                        &path, w, h,
                                    ) {
                                        Ok((rw, rh, rgba)) => {
                                            // Hash the pixels into the filename so the
                                            // path changes when the snapshot does —
                                            // downstream consumers cache by path.
                                            let digest = {
                                                use std::hash::{Hash, Hasher};
                                                let mut h =
                                                    std::collections::hash_map::DefaultHasher::new(
                                                    );
                                                rgba.hash(&mut h);
                                                h.finish()
                                            };
                                            if let Some(img) =
                                                image::RgbaImage::from_raw(rw, rh, rgba)
                                            {
                                                let out_path = cache.join(format!(
                                                    "{output}-shader-{digest:016x}.png"
                                                ));
                                                if let Err(e) = img.save(&out_path) {
                                                    tracing::warn!(
                                                        ?e,
                                                        "failed to save shader snapshot"
                                                    );
                                                } else {
                                                    out.push((output, out_path));
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(?e, "failed to render shader snapshot");
                                        }
                                    }
                                }
                                out
                            })
                            .await
                            .unwrap_or_default();
                            composited.extend(rendered);
                        }

                        // Write to cosmic-bg config
                        let bg_ctx =
                            glowberry_config::cosmic_bg_context().map_err(|e| e.to_string())?;
                        let mut bg_config = glowberry_config::Config {
                            same_on_all: false,
                            ..Default::default()
                        };
                        bg_ctx
                            .0
                            .set(glowberry_config::SAME_ON_ALL, false)
                            .map_err(|e| e.to_string())?;

                        // Write locked layer entries
                        for (output, path) in &locked_entries {
                            let entry = glowberry_config::Entry::new(
                                output.clone(),
                                glowberry_config::Source::Path(path.clone()),
                            );
                            bg_config
                                .set_entry(&bg_ctx, entry)
                                .map_err(|e| e.to_string())?;
                        }

                        // Write composited entries
                        for (output, path) in &composited {
                            let entry = glowberry_config::Entry::new(
                                output.clone(),
                                glowberry_config::Source::Path(path.clone()),
                            );
                            bg_config
                                .set_entry(&bg_ctx, entry)
                                .map_err(|e| e.to_string())?;
                        }

                        // The cosmic-bg *config* above is only consumed by the
                        // cosmic-bg daemon. cosmic-greeter paints the lock screen
                        // from the cosmic-bg *state* instead, so mirror the applied
                        // wallpapers there too — otherwise the lock screen keeps
                        // showing the stale/default wallpaper.
                        let mut state_wallpapers = locked_entries.clone();
                        state_wallpapers.extend(composited.iter().cloned());
                        glowberry_config::export_lock_screen_wallpapers(&state_wallpapers)
                            .map_err(|e| e.to_string())?;

                        tracing::info!(
                            "Exported {} wallpaper(s) to cosmic-bg config and state",
                            state_wallpapers.len()
                        );
                        Ok(())
                    },
                    |result| cosmic::Action::App(Message::ExportToCosmicBgDone(result)),
                );
            }

            Message::ExportToCosmicBgDone(result) => {
                if let Err(e) = result {
                    tracing::error!("Failed to export to cosmic-bg: {}", e);
                }
            }

            Message::ExtendLayerRightClick(key, x, y) => {
                self.extend_selected_layer = Some(key);
                self.layer_context_menu = Some((key, (x, y)));
            }

            Message::ExtendLayerMenuClose => {
                self.layer_context_menu = None;
            }

            Message::ExtendLayerBringForward(key) => {
                self.layer_context_menu = None;
                let sel_z = self.extend_layers[key].z_index;
                if let Some((swap_key, _)) = self
                    .extend_layers
                    .iter()
                    .filter(|(k, l)| *k != key && l.z_index > sel_z)
                    .min_by_key(|(_, l)| l.z_index)
                {
                    let swap_z = self.extend_layers[swap_key].z_index;
                    self.extend_layers[key].z_index = swap_z;
                    self.extend_layers[swap_key].z_index = sel_z;
                }
            }

            Message::ExtendLayerSendBack(key) => {
                self.layer_context_menu = None;
                let sel_z = self.extend_layers[key].z_index;
                if let Some((swap_key, _)) = self
                    .extend_layers
                    .iter()
                    .filter(|(k, l)| *k != key && l.z_index < sel_z)
                    .max_by_key(|(_, l)| l.z_index)
                {
                    let swap_z = self.extend_layers[swap_key].z_index;
                    self.extend_layers[key].z_index = swap_z;
                    self.extend_layers[swap_key].z_index = sel_z;
                }
            }
        }

        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let mut children: Vec<Element<'_, Message>> = Vec::with_capacity(6);

        let is_wallpaper_mode = matches!(self.categories.selected, Some(Category::Wallpapers));

        // 1. Preview area (always slot 1) — the multi-monitor canvas in every
        // mode (wallpaper, color, live).
        children.push(self.view_multi_monitor_canvas());

        // 2. Settings list (always slot 2 — empty for wallpapers)
        if is_wallpaper_mode {
            children.push(widget::Space::new().into());
        } else {
            children.push(
                container(self.view_settings_list())
                    .width(Length::Fill)
                    .align_x(Alignment::Center)
                    .into(),
            );
        }

        // Slot 3 (category selector moved to header toggle)
        children.push(widget::Space::new().into());

        // Selection grid
        let grid = match self.categories.selected {
            Some(Category::Wallpapers) => self.view_wallpaper_grid(),
            Some(Category::Colors) => self.view_color_grid(),
            Some(Category::Shaders) => self.view_shader_grid(),
            None => widget::Space::new().into(),
        };
        children.push(
            container(grid)
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .into(),
        );

        // Wrap everything in a scrollable container
        let scrollable_content = widget::scrollable(
            widget::column::with_children(children)
                .spacing(22)
                .padding(20)
                .width(Length::Fill)
                .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill);

        // Apply custom background with opacity
        let opacity = self.window_opacity;
        container(scrollable_content)
            .width(Length::Fill)
            .height(Length::Fill)
            .class(cosmic::theme::Container::custom(move |theme| {
                let cosmic = theme.cosmic();
                let mut bg_color: cosmic::iced::Color = cosmic.background.base.into();
                bg_color.a = opacity;
                cosmic::widget::container::Style {
                    background: Some(cosmic::iced::Background::Color(bg_color)),
                    icon_color: Some(cosmic.background.on.into()),
                    text_color: Some(cosmic.background.on.into()),
                    border: cosmic::iced::Border::default(),
                    shadow: cosmic::iced::Shadow::default(),
                    snap: false,
                }
            }))
            .into()
    }

    fn header_center(&self) -> Vec<Element<'_, Self::Message>> {
        let selected = match self.categories.selected {
            Some(Category::Wallpapers) => 0,
            Some(Category::Colors) => 1,
            Some(Category::Shaders) => 2,
            None => 0,
        };
        vec![
            cosmetics::widgets::toggle::toggle3(
                "preferences-desktop-wallpaper-symbolic",
                "applications-graphics-symbolic",
                "applications-multimedia-symbolic",
                selected,
            )
            .on_select(Message::CategoryToggle)
            .into(),
        ]
    }

    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        vec![
            widget::button::icon(widget::icon::from_name("preferences-system-symbolic"))
                .on_press(Message::ToggleContextPage(ContextPage::Settings))
                .into(),
            widget::button::icon(widget::icon::from_name("help-about-symbolic"))
                .on_press(Message::ToggleContextPage(ContextPage::About))
                .into(),
        ]
    }

    fn context_drawer(&self) -> Option<ContextDrawer<'_, Self::Message>> {
        if !self.core.window.show_context {
            return None;
        }

        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::OpenUrl(url.to_string()),
                Message::ToggleContextPage(ContextPage::About),
            ),
            ContextPage::Settings => context_drawer::context_drawer(
                self.settings_drawer_view(),
                Message::ToggleContextPage(ContextPage::Settings),
            )
            .title(fl!("settings")),
        })
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        // Return transparent background for the window surface
        // The actual background with opacity is applied via our custom container in view()
        let theme = cosmic::theme::active();
        let cosmic_theme = theme.cosmic();

        Some(cosmic::iced::theme::Style {
            background_color: cosmic::iced::Color::TRANSPARENT,
            text_color: cosmic_theme.on_bg_color().into(),
            icon_color: cosmic_theme.on_bg_color().into(),
        })
    }
}

impl GlowBerrySettings {
    /// Build the settings drawer content
    fn settings_drawer_view(&self) -> Element<'_, Message> {
        // Build power saving section
        let mut power_saving_section = widget::settings::section().title(fl!("power-saving"));

        // On battery power action
        power_saving_section = power_saving_section.add(settings::item(
            fl!("on-battery"),
            dropdown(
                &self.on_battery_action_options,
                Some(self.selected_on_battery_action),
                Message::SetOnBatteryAction,
            ),
        ));

        // Pause on low battery (with conditional threshold dropdown)
        {
            let toggle_row = settings::item(
                fl!("pause-low-battery"),
                toggler(self.power_saving.pause_on_low_battery)
                    .on_toggle(Message::SetPauseOnLowBattery),
            );

            if self.power_saving.pause_on_low_battery {
                let dropdown_row = settings::item(
                    fl!("low-battery-threshold"),
                    dropdown(
                        &self.low_battery_threshold_options,
                        Some(self.selected_low_battery_threshold),
                        Message::SetLowBatteryThreshold,
                    ),
                );

                power_saving_section = power_saving_section.add(
                    widget::column::with_children(vec![toggle_row.into(), dropdown_row.into()])
                        .spacing(8),
                );
            } else {
                power_saving_section = power_saving_section.add(toggle_row);
            }
        }

        // Pause when lid closed
        power_saving_section = power_saving_section.add(settings::item(
            fl!("pause-lid-closed"),
            toggler(self.power_saving.pause_on_lid_closed).on_toggle(Message::SetPauseOnLidClosed),
        ));

        // Build background service section with optional PATH warning
        let mut bg_service_section = widget::settings::section()
            .title(fl!("background-service"))
            .add(settings::item(
                fl!("use-glowberry"),
                toggler(self.glowberry_is_default).on_toggle(Message::SetGlowBerryDefault),
            ));

        // Add PATH order warning if incorrect
        if !is_path_order_correct() {
            bg_service_section =
                bg_service_section.add(widget::text(fl!("path-order-warning")).size(12).class(
                    cosmic::theme::Text::Color(cosmic::iced::Color::from_rgb(0.9, 0.6, 0.2)),
                ));
        }

        // Build appearance section with window opacity slider
        let appearance_section =
            widget::settings::section()
                .title(fl!("appearance"))
                .add(settings::item(
                    fl!("window-opacity"),
                    widget::row::with_children(vec![
                        slider(0.0..=1.0, self.window_opacity, Message::SetWindowOpacity)
                            .on_release(Message::WindowOpacityReleased)
                            .step(0.01)
                            .width(Length::Fixed(150.0))
                            .into(),
                        widget::text(format!("{:.0}%", self.window_opacity * 100.0))
                            .width(Length::Fixed(50.0))
                            .into(),
                    ])
                    .spacing(8)
                    .align_y(Alignment::Center),
                ));

        // Build bezel section (one group of sliders per monitor)
        let mut bezel_section = widget::settings::section().title(fl!("bezels"));

        for (idx, monitor) in self.monitor_geometry.iter().enumerate() {
            let bz = &monitor.bezel;

            bezel_section = bezel_section.add(widget::text::heading(monitor.display_label()));

            let i = idx;
            let top = bz.top;
            let bottom = bz.bottom;
            let left = bz.left;
            let right = bz.right;

            bezel_section = bezel_section.add(settings::item(
                fl!("bezel-top"),
                scrub_spin(0.0..=200.0, top)
                    .step(1.0)
                    .decimals(0)
                    .width(Length::Fixed(150.0))
                    .on_change(move |v| Message::SetBezel(i, v, bottom, left, right))
                    .on_release(move |_| Message::BezelReleased),
            ));

            bezel_section = bezel_section.add(settings::item(
                fl!("bezel-bottom"),
                scrub_spin(0.0..=200.0, bottom)
                    .step(1.0)
                    .decimals(0)
                    .width(Length::Fixed(150.0))
                    .on_change(move |v| Message::SetBezel(i, top, v, left, right))
                    .on_release(move |_| Message::BezelReleased),
            ));

            bezel_section = bezel_section.add(settings::item(
                fl!("bezel-left"),
                scrub_spin(0.0..=200.0, left)
                    .step(1.0)
                    .decimals(0)
                    .width(Length::Fixed(150.0))
                    .on_change(move |v| Message::SetBezel(i, top, bottom, v, right))
                    .on_release(move |_| Message::BezelReleased),
            ));

            bezel_section = bezel_section.add(settings::item(
                fl!("bezel-right"),
                scrub_spin(0.0..=200.0, right)
                    .step(1.0)
                    .decimals(0)
                    .width(Length::Fixed(150.0))
                    .on_change(move |v| Message::SetBezel(i, top, bottom, left, v))
                    .on_release(move |_| Message::BezelReleased),
            ));
        }

        widget::settings::view_column(vec![
            // Default background service section
            bg_service_section.into(),
            // Appearance section
            appearance_section.into(),
            // GPU settings section
            widget::settings::section()
                .title(fl!("performance"))
                .add(settings::item(
                    fl!("prefer-low-power"),
                    toggler(self.prefer_low_power).on_toggle(Message::PreferLowPower),
                ))
                .into(),
            // Power saving section
            power_saving_section.into(),
            // Bezel section
            bezel_section.into(),
        ])
        .into()
    }

    fn init_from_config(&mut self) {
        // Determine which entry reflects the applied wallpaper, so the window
        // opens on the matching page (wallpaper / color / live).
        let entry = if self.config.same_on_all {
            &self.config.default_background
        } else if let Some(ref output_name) = self.active_output {
            // Try to find a per-output entry
            self.config
                .entry(output_name)
                .unwrap_or(&self.config.default_background)
        } else if let Some(first) = self.config.backgrounds.first() {
            // Per-output mode with no specific output selected: reflect what's
            // actually applied (a per-output entry), not the stale "all" default.
            first
        } else {
            &self.config.default_background
        };

        self.select_entry_source(&entry.source.clone());
    }

    fn cache_display_image(&mut self) {
        self.cached_display_handle = None;

        if let Choice::Wallpaper(id) = self.selection.active
            && let Some(image) = self.selection.display_images.get(id)
        {
            self.cached_display_handle = Some(ImageHandle::from_rgba(
                image.width(),
                image.height(),
                image.to_vec(),
            ));
        }
    }

    /// Build the config `Source` for the current selection (image path, color,
    /// or live shader), or `None` if it can't be resolved.
    fn build_active_source(&self) -> Option<Source> {
        let source = match &self.selection.active {
            Choice::Wallpaper(key) => {
                if let Some(path) = self.selection.paths.get(*key) {
                    Source::Path(path.clone())
                } else {
                    return None;
                }
            }
            Choice::Color(color) => Source::Color(color.clone()),
            Choice::Shader(idx) => {
                if let Some(shader) = self.available_shaders.get(*idx) {
                    let frame_rate = match self.selected_shader_frame_rate {
                        0 => 15,
                        2 => 60,
                        _ => 30,
                    };

                    // Check if we have custom parameter values for this shader
                    let (shader_content, source_path, params) = if let Some(parsed) = &shader.parsed
                    {
                        // Get current parameter values, falling back to defaults
                        let values = self
                            .shader_param_values
                            .get(idx)
                            .cloned()
                            .unwrap_or_default();

                        // Convert ParamValue HashMap to f64 HashMap for config storage
                        let params: HashMap<String, f64> = values
                            .iter()
                            .map(|(k, v)| (k.clone(), v.as_f32() as f64))
                            .collect();

                        // Only generate custom source if we have any custom values
                        if values.is_empty() {
                            // No custom params, use path for efficiency
                            (
                                glowberry_config::ShaderContent::Path(shader.path.clone()),
                                None,
                                params,
                            )
                        } else {
                            // Generate shader source with parameter values
                            // Keep source_path so we can identify the shader later
                            let generated_source = parsed.generate_source(&values);
                            (
                                glowberry_config::ShaderContent::Code(generated_source),
                                Some(shader.path.clone()),
                                params,
                            )
                        }
                    } else {
                        // No parsed shader, use path
                        (
                            glowberry_config::ShaderContent::Path(shader.path.clone()),
                            None,
                            HashMap::new(),
                        )
                    };

                    Source::Shader(glowberry_config::ShaderSource {
                        shader: shader_content,
                        source_path,
                        params,
                        background_image: None,
                        language: glowberry_config::ShaderLanguage::Wgsl,
                        frame_rate,
                    })
                } else {
                    return None;
                }
            }
        };
        Some(source)
    }

    fn apply_selection(&mut self) {
        let Some(ctx) = &self.config_context else {
            return;
        };
        let Some(source) = self.build_active_source() else {
            return;
        };

        // Determine the output name to use
        let output = if self.config.same_on_all {
            "all".to_string()
        } else if let Some(ref name) = self.active_output {
            name.clone()
        } else {
            "all".to_string()
        };

        let entry = Entry::new(output, source);
        if let Err(e) = self.config.set_entry(ctx, entry) {
            tracing::error!("Failed to set wallpaper: {}", e);
        }
    }

    /// Select a source from entry (used when switching displays or on init)
    fn select_entry_source(&mut self, source: &Source) {
        match source {
            Source::Path(path) => {
                // Find the wallpaper in our loaded wallpapers
                if let Some((key, _)) = self.selection.paths.iter().find(|(_, p)| *p == path) {
                    self.selection.active = Choice::Wallpaper(key);
                }
                // Always set category to wallpapers for path sources
                // (the actual wallpaper will be selected when WallpaperEvent::Loaded fires if not found yet)
                self.categories.selected = Some(Category::Wallpapers);
            }
            Source::Color(color) => {
                self.selection.active = Choice::Color(color.clone());
                self.categories.selected = Some(Category::Colors);
            }
            Source::Shader(shader_source) => {
                // Determine which path to use for matching:
                // - If source_path is set (customized shader), use that
                // - Otherwise use the path from ShaderContent::Path
                let match_path = shader_source.source_path.as_ref().or({
                    if let glowberry_config::ShaderContent::Path(p) = &shader_source.shader {
                        Some(p)
                    } else {
                        None
                    }
                });

                let mut matched_idx = None;
                if let Some(config_path) = match_path {
                    // Try exact path match first
                    if let Some(idx) = self
                        .available_shaders
                        .iter()
                        .position(|s| &s.path == config_path)
                    {
                        self.selection.active = Choice::Shader(idx);
                        matched_idx = Some(idx);
                    } else {
                        // Fall back to filename match (in case paths differ due to XDG_DATA_DIRS)
                        if let Some(config_filename) = config_path.file_name()
                            && let Some(idx) = self
                                .available_shaders
                                .iter()
                                .position(|s| s.path.file_name() == Some(config_filename))
                        {
                            self.selection.active = Choice::Shader(idx);
                            matched_idx = Some(idx);
                        }
                    }

                    // If no shader found, select the first one if available
                    if matched_idx.is_none() && !self.available_shaders.is_empty() {
                        self.selection.active = Choice::Shader(0);
                        matched_idx = Some(0);
                    }
                } else if !self.available_shaders.is_empty() {
                    // Inline shader content with no source_path - just select first shader
                    self.selection.active = Choice::Shader(0);
                    matched_idx = Some(0);
                }

                // Load parameter values from config
                if let Some(idx) = matched_idx
                    && !shader_source.params.is_empty()
                {
                    // Convert f64 values back to ParamValue based on shader's param definitions
                    let mut param_values: HashMap<String, ParamValue> = HashMap::new();

                    if let Some(shader_info) = self.available_shaders.get(idx)
                        && let Some(parsed) = &shader_info.parsed
                    {
                        for param in &parsed.params {
                            if let Some(&value) = shader_source.params.get(&param.name) {
                                let param_value = match param.param_type {
                                    ParamType::F32 => ParamValue::F32(value as f32),
                                    ParamType::I32 => ParamValue::I32(value as i32),
                                };
                                param_values.insert(param.name.clone(), param_value);
                            }
                        }
                    }

                    if !param_values.is_empty() {
                        self.shader_param_values.insert(idx, param_values);
                    }
                }

                self.selected_shader_frame_rate = match shader_source.frame_rate {
                    0..=22 => 0,
                    23..=45 => 1,
                    _ => 2,
                };
                self.categories.selected = Some(Category::Shaders);
            }
        }
        self.cache_display_image();
    }

    /// Populate the outputs tab bar from state (connected outputs)
    /// The daemon updates the state with currently connected outputs
    fn populate_outputs_from_config(&mut self) {
        self.outputs.clear();

        // Get connected outputs from state - these are the currently connected displays
        let connected_outputs: Vec<String> = State::state()
            .ok()
            .and_then(|state_helper| State::get_entry(&state_helper).ok())
            .map(|state| state.connected_outputs)
            .unwrap_or_default();

        // If no connected outputs in state, fall back to config outputs
        // (This handles the case where daemon hasn't written state yet)
        let output_names: Vec<String> = if connected_outputs.is_empty() {
            self.config.outputs.iter().cloned().collect()
        } else {
            connected_outputs
        };

        self.show_tab_bar = output_names.len() > 1;

        let mut first = None;
        for name in output_names {
            let is_internal = name == "eDP-1";

            // Use the output name directly (e.g., "DP-1", "HDMI-A-1", "eDP-1")
            let entity = self
                .outputs
                .insert()
                .text(name.clone())
                .data(OutputName(name));

            if is_internal || first.is_none() {
                first = Some(entity.id());
            }
        }

        if let Some(id) = first {
            self.outputs.activate(id);
            if let Some(name) = self.outputs.data::<OutputName>(id) {
                self.active_output = Some(name.0.clone());
            }
        }
    }

    fn auto_center_layer(&self, layer: &mut ExtendLayerState) {
        if self.monitor_geometry.is_empty() || layer.image_size == (0, 0) {
            return;
        }

        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for m in &self.monitor_geometry {
            min_x = min_x.min(m.position.0);
            min_y = min_y.min(m.position.1);
            max_x = max_x.max(m.position.0 + m.logical_size.0 as i32);
            max_y = max_y.max(m.position.1 + m.logical_size.1 as i32);
        }

        let vd_w = (max_x - min_x) as f64;
        let vd_h = (max_y - min_y) as f64;
        let vd_cx = min_x as f64 + vd_w / 2.0;
        let vd_cy = min_y as f64 + vd_h / 2.0;

        let img_w = layer.image_size.0 as f64;
        let img_h = layer.image_size.1 as f64;
        let scale = (vd_w / img_w).max(vd_h / img_h);
        layer.scale = scale;
        layer.offset = (vd_cx - img_w * scale / 2.0, vd_cy - img_h * scale / 2.0);
    }

    fn renormalize_z_indices(&mut self) {
        let mut sorted: Vec<(DefaultKey, usize)> = self
            .extend_layers
            .iter()
            .map(|(k, l)| (k, l.z_index))
            .collect();
        sorted.sort_by_key(|(_, z)| *z);
        for (i, (key, _)) in sorted.into_iter().enumerate() {
            self.extend_layers[key].z_index = i;
        }
        self.extend_next_z = self.extend_layers.len();
    }

    /// Insert one locked, non-expandable canvas item filling `monitor` (colors
    /// fill exactly; images/shaders cover-fit). Records its color/source.
    fn insert_locked_item(
        &mut self,
        monitor: &crate::monitor_query::MonitorGeometry,
        color: Option<Color>,
        source: Source,
        image_handle: Option<ImageHandle>,
        image_size: (u32, u32),
    ) -> DefaultKey {
        let mon_w = monitor.logical_size.0 as f64;
        let mon_h = monitor.logical_size.1 as f64;
        let (size, scale, offset) = if color.is_some() {
            (
                monitor.logical_size,
                1.0,
                (monitor.position.0 as f64, monitor.position.1 as f64),
            )
        } else {
            let img_w = image_size.0.max(1) as f64;
            let img_h = image_size.1.max(1) as f64;
            let scale = (mon_w / img_w).max(mon_h / img_h);
            (
                image_size,
                scale,
                (
                    monitor.position.0 as f64 + (mon_w - img_w * scale) / 2.0,
                    monitor.position.1 as f64 + (mon_h - img_h * scale) / 2.0,
                ),
            )
        };
        let z = self.extend_next_z;
        self.extend_next_z += 1;
        let key = self.extend_layers.insert(ExtendLayerState {
            source_path: PathBuf::new(),
            image_handle,
            image_size: size,
            offset,
            scale,
            z_index: z,
            locked: true,
            target_output: Some(monitor.name.clone()),
        });
        if let Some(c) = &color {
            self.extend_layer_colors.insert(key, c.clone());
        }
        self.extend_layer_sources.insert(key, source);
        key
    }

    /// Apply a color/live content to every display (same wallpaper everywhere)
    /// and stage it on the canvas. Used by the grid right-click "apply to all".
    fn apply_content_to_all(
        &mut self,
        color: Option<Color>,
        source: Source,
        image_handle: Option<ImageHandle>,
        image_size: (u32, u32),
    ) {
        self.config.same_on_all = true;
        if let Some(ctx) = &self.config_context {
            let _ = ctx.set_same_on_all(true);
            let entry = Entry::new("all".to_string(), source.clone());
            let _ = self.config.set_entry(ctx, entry);
        }
        self.fill_monitors_locked(PathBuf::new(), image_handle, image_size, color, source);
    }

    /// Apply a color/live content to a single display, leaving the others as they
    /// are. Used by the grid right-click "show on <display>".
    fn apply_content_to_output(
        &mut self,
        color: Option<Color>,
        source: Source,
        image_handle: Option<ImageHandle>,
        image_size: (u32, u32),
        monitor_idx: usize,
    ) {
        let Some(monitor) = self.monitor_geometry.get(monitor_idx).cloned() else {
            return;
        };
        self.config.same_on_all = false;
        if let Some(ctx) = &self.config_context {
            let _ = ctx.set_same_on_all(false);
            let entry = Entry::new(monitor.name.clone(), source.clone());
            let _ = self.config.set_entry(ctx, entry);
        }
        // Replace any existing item for this output so it isn't duplicated.
        let existing: Vec<DefaultKey> = self
            .extend_layers
            .iter()
            .filter(|(_, l)| l.target_output.as_deref() == Some(monitor.name.as_str()))
            .map(|(k, _)| k)
            .collect();
        for k in existing {
            self.extend_layers.remove(k);
            self.extend_layer_colors.remove(k);
            self.extend_layer_sources.remove(k);
        }
        let key = self.insert_locked_item(&monitor, color, source, image_handle, image_size);
        self.extend_selected_layer = Some(key);
        self.extend_fit_view_requested = true;
        self.persist_canvas_per_output();
    }

    /// Fill every connected monitor with a locked, non-expandable item showing
    /// the given content. Used for color/live staging (empty `source_path`,
    /// rendered via `color`/shader thumbnail) and for "apply to all". Each item
    /// carries `source` so it can be written to config when applied.
    fn fill_monitors_locked(
        &mut self,
        source_path: PathBuf,
        image_handle: Option<ImageHandle>,
        image_size: (u32, u32),
        color: Option<Color>,
        source: Source,
    ) {
        self.extend_layers.clear();
        self.extend_layer_colors.clear();
        self.extend_layer_sources.clear();
        self.extend_selected_layer = None;
        self.layer_context_menu = None;
        self.extend_next_z = 0;
        self.extend_fit_view_requested = true;

        let monitors: Vec<crate::monitor_query::MonitorGeometry> = self.monitor_geometry.to_vec();
        for monitor in &monitors {
            let mon_w = monitor.logical_size.0 as f64;
            let mon_h = monitor.logical_size.1 as f64;
            // Colors fill the monitor exactly; images/shaders cover-fit it.
            let (size, scale, offset) = if color.is_some() {
                (
                    monitor.logical_size,
                    1.0,
                    (monitor.position.0 as f64, monitor.position.1 as f64),
                )
            } else {
                let img_w = image_size.0.max(1) as f64;
                let img_h = image_size.1.max(1) as f64;
                let scale = (mon_w / img_w).max(mon_h / img_h);
                (
                    image_size,
                    scale,
                    (
                        monitor.position.0 as f64 + (mon_w - img_w * scale) / 2.0,
                        monitor.position.1 as f64 + (mon_h - img_h * scale) / 2.0,
                    ),
                )
            };
            let z = self.extend_next_z;
            self.extend_next_z += 1;
            let key = self.extend_layers.insert(ExtendLayerState {
                source_path: source_path.clone(),
                image_handle: image_handle.clone(),
                image_size: size,
                offset,
                scale,
                z_index: z,
                locked: true,
                target_output: Some(monitor.name.clone()),
            });
            if let Some(c) = &color {
                self.extend_layer_colors.insert(key, c.clone());
            }
            self.extend_layer_sources.insert(key, source.clone());
        }
        self.persist_canvas_per_output();
    }

    /// Load and stage the saved working content for a page so switching tabs (or
    /// reopening the window) isn't a fresh start. A page whose type matches the
    /// currently-applied wallpaper shows that; otherwise it shows the page's last
    /// saved selection (`saved-color` / `saved-shader`).
    fn load_category_canvas(&mut self, category: &Category) {
        self.extend_layers.clear();
        self.extend_layer_colors.clear();
        self.extend_layer_sources.clear();
        self.extend_selected_layer = None;
        self.extend_next_z = 0;
        self.extend_fit_view_requested = true;

        match category {
            Category::Wallpapers => self.restore_extend_layers_from_config(),
            Category::Colors => self.restore_per_output_locked(true),
            Category::Shaders => {
                if self.available_shaders.is_empty() {
                    self.available_shaders = discover_shaders();
                    let placeholder = create_shader_placeholder(158, 105);
                    self.shader_thumbnails = vec![placeholder; self.available_shaders.len()];
                }
                self.restore_per_output_locked(false);
            }
        }
    }

    /// Find the index of the shader matching a `Source::Shader`, so we can show
    /// its thumbnail. Matches by full path, then by file name.
    fn shader_idx_for_source(&self, source: &Source) -> Option<usize> {
        let Source::Shader(shader) = source else {
            return None;
        };
        let match_path = shader.source_path.as_ref().or({
            if let glowberry_config::ShaderContent::Path(p) = &shader.shader {
                Some(p)
            } else {
                None
            }
        })?;
        self.available_shaders
            .iter()
            .position(|s| &s.path == match_path)
            .or_else(|| {
                let fname = match_path.file_name()?;
                self.available_shaders
                    .iter()
                    .position(|s| s.path.file_name() == Some(fname))
            })
    }

    /// Persist the current color/live canvas as this page's per-output state, so
    /// each page remembers its own per-display assignments independently of which
    /// one is currently applied. Called whenever the canvas changes.
    fn persist_canvas_per_output(&mut self) {
        let Some(ctx) = &self.config_context else {
            return;
        };
        match &self.categories.selected {
            Some(Category::Colors) => {
                let map: Vec<(String, Color)> = self
                    .extend_layers
                    .iter()
                    .filter_map(|(k, l)| {
                        Some((
                            l.target_output.clone()?,
                            self.extend_layer_colors.get(k)?.clone(),
                        ))
                    })
                    .collect();
                let _ = ctx.0.set("saved-color-outputs", map);
            }
            Some(Category::Shaders) => {
                let map: Vec<(String, Source)> = self
                    .extend_layers
                    .iter()
                    .filter_map(|(k, l)| {
                        let out = l.target_output.clone()?;
                        let src = self.extend_layer_sources.get(k)?.clone();
                        matches!(src, Source::Shader(_)).then_some((out, src))
                    })
                    .collect();
                let _ = ctx.0.set("saved-shader-outputs", map);
            }
            _ => {}
        }
    }

    /// Restore the color/live canvas so each display shows its own saved
    /// per-output content. Lookup order per display: this page's saved
    /// per-output map, then the page's single saved selection.
    fn restore_per_output_locked(&mut self, want_color: bool) {
        let monitors = self.monitor_geometry.to_vec();
        let (color_map, shader_map, saved_color, saved_shader) = {
            let ctx = self.config_context.as_ref();
            (
                ctx.and_then(|c| c.0.get::<Vec<(String, Color)>>("saved-color-outputs").ok())
                    .unwrap_or_default(),
                ctx.and_then(|c| {
                    c.0.get::<Vec<(String, Source)>>("saved-shader-outputs")
                        .ok()
                })
                .unwrap_or_default(),
                ctx.and_then(|c| c.0.get::<Color>("saved-color").ok()),
                ctx.and_then(|c| c.0.get::<Source>("saved-shader").ok()),
            )
        };

        let mut first_active: Option<Choice> = None;
        for monitor in &monitors {
            if want_color {
                let color = color_map
                    .iter()
                    .find(|(o, _)| o == &monitor.name)
                    .map(|(_, c)| c.clone())
                    .or_else(|| saved_color.clone());
                if let Some(color) = color {
                    if first_active.is_none() {
                        first_active = Some(Choice::Color(color.clone()));
                    }
                    self.insert_locked_item(
                        monitor,
                        Some(color.clone()),
                        Source::Color(color),
                        None,
                        (0, 0),
                    );
                }
            } else {
                let src = shader_map
                    .iter()
                    .find(|(o, _)| o == &monitor.name)
                    .map(|(_, s)| s.clone())
                    .or_else(|| saved_shader.clone());
                if let Some(src) = src {
                    let idx = self.shader_idx_for_source(&src);
                    let handle = idx.and_then(|i| self.shader_thumbnails.get(i).cloned());
                    if first_active.is_none()
                        && let Some(i) = idx
                    {
                        first_active = Some(Choice::Shader(i));
                    }
                    self.insert_locked_item(monitor, None, src, handle, (1920, 1080));
                }
            }
        }

        if let Some(active) = first_active {
            self.selection.active = active;
        }
    }

    fn restore_extend_layers_from_config(&mut self) {
        self.extend_layers.clear();
        self.extend_next_z = 0;
        for config_layer in &self.extend_config.layers {
            let image_handle = self
                .selection
                .display_images
                .iter()
                .find(|(_, _)| {
                    // Try to match by path
                    false
                })
                .map(|(_, img)| ImageHandle::from_rgba(img.width(), img.height(), img.to_vec()));
            // Try to load the image handle from the path directly
            let image_handle = image_handle.or_else(|| {
                image::open(&config_layer.source_path).ok().map(|img| {
                    let rgba = img.to_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    // Resize to a reasonable thumbnail size
                    let thumb = image::imageops::resize(
                        &rgba,
                        w.min(400),
                        h.min(300),
                        image::imageops::FilterType::Triangle,
                    );
                    ImageHandle::from_rgba(thumb.width(), thumb.height(), thumb.into_vec())
                })
            });
            let image_size =
                image::image_dimensions(&config_layer.source_path).unwrap_or((800, 600));

            let z = config_layer.z_index;
            self.extend_next_z = self.extend_next_z.max(z + 1);
            self.extend_layers.insert(ExtendLayerState {
                source_path: config_layer.source_path.clone(),
                image_handle,
                image_size,
                offset: (config_layer.img_offset_x, config_layer.img_offset_y),
                scale: config_layer.img_scale,
                z_index: z,
                locked: config_layer.locked,
                target_output: config_layer.target_output.clone(),
            });
        }
    }

    /// Load shader thumbnails
    fn load_shader_thumbnails(&self) -> Task<Message> {
        let shader_paths: Vec<_> = self
            .available_shaders
            .iter()
            .map(|s| s.path.clone())
            .collect();

        Task::batch(
            shader_paths
                .into_iter()
                .enumerate()
                .map(|(idx, path)| {
                    Task::perform(
                        async move {
                            let handle = tokio::task::spawn_blocking(move || {
                                match crate::widgets::shader_preview::render_shader_preview(
                                    &path, 158, 105,
                                ) {
                                    Ok((width, height, rgba)) => {
                                        Some(ImageHandle::from_rgba(width, height, rgba))
                                    }
                                    Err(e) => {
                                        tracing::debug!(
                                            ?path,
                                            ?e,
                                            "Failed to render shader preview"
                                        );
                                        None
                                    }
                                }
                            })
                            .await
                            .ok()
                            .flatten();
                            (idx, handle)
                        },
                        |(idx, handle)| cosmic::Action::App(Message::ShaderThumbnail(idx, handle)),
                    )
                })
                .collect::<Vec<_>>(),
        )
    }

    #[allow(dead_code)]
    fn view_display_preview(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.selection.active {
            Choice::Wallpaper(key) => {
                // First try the cached display handle, then fall back to thumbnail
                if let Some(handle) = &self.cached_display_handle {
                    widget::image(handle.clone())
                        .width(Length::Fixed(SIMULATED_WIDTH as f32))
                        .height(Length::Fixed(SIMULATED_HEIGHT as f32))
                        .into()
                } else if let Some(handle) = self.selection.selection_handles.get(*key) {
                    // Use the selection thumbnail scaled up if display image not ready
                    widget::image(handle.clone())
                        .content_fit(cosmic::iced::ContentFit::Cover)
                        .width(Length::Fixed(SIMULATED_WIDTH as f32))
                        .height(Length::Fixed(SIMULATED_HEIGHT as f32))
                        .into()
                } else {
                    // Show loading placeholder - wallpapers are still loading
                    container(widget::text(fl!("loading-wallpapers")))
                        .width(Length::Fixed(SIMULATED_WIDTH as f32))
                        .height(Length::Fixed(SIMULATED_HEIGHT as f32))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center)
                        .into()
                }
            }
            Choice::Color(color) => color_image(color.clone(), SIMULATED_WIDTH, SIMULATED_HEIGHT),
            Choice::Shader(idx) => {
                // For shaders, always show the thumbnail (placeholder or real)
                if let Some(handle) = self.shader_thumbnails.get(*idx) {
                    widget::image(handle.clone())
                        .content_fit(cosmic::iced::ContentFit::Cover)
                        .width(Length::Fixed(SIMULATED_WIDTH as f32))
                        .height(Length::Fixed(SIMULATED_HEIGHT as f32))
                        .into()
                } else {
                    // Shader index out of bounds - show placeholder
                    shader_placeholder(SIMULATED_WIDTH, SIMULATED_HEIGHT)
                }
            }
        };

        let opacity = self.window_opacity;
        container(content)
            .padding(8)
            .class(cosmic::theme::Container::custom(move |theme| {
                let cosmic = theme.cosmic();
                let mut bg_color: cosmic::iced::Color = cosmic.background.component.base.into();
                bg_color.a = opacity;
                cosmic::widget::container::Style {
                    icon_color: Some(cosmic.background.component.on.into()),
                    text_color: Some(cosmic.background.component.on.into()),
                    background: Some(cosmic::iced::Background::Color(bg_color)),
                    border: cosmic::iced::Border {
                        radius: cosmic.corner_radii.radius_s.into(),
                        ..Default::default()
                    },
                    shadow: cosmic::iced::Shadow::default(),
                    snap: false,
                }
            }))
            .width(Length::Shrink)
            .into()
    }

    fn view_settings_list(&self) -> Element<'_, Message> {
        let mut list = widget::list_column();

        // Frame rate dropdown and shader parameters (only for shaders)
        if let Choice::Shader(shader_idx) = self.selection.active {
            // Frame rate is always visible
            list = list.add(settings::item(
                fl!("frame-rate"),
                dropdown(
                    &self.frame_rate_options,
                    Some(self.selected_shader_frame_rate),
                    Message::ShaderFrameRate,
                ),
            ));

            // Show Details button (centered, pull-down style with chevron icon)
            let (details_label, chevron_icon) = if self.shader_details_expanded {
                (fl!("hide-details"), "go-up-symbolic")
            } else {
                (fl!("show-details"), "go-down-symbolic")
            };

            let details_button = widget::button::text(details_label)
                .trailing_icon(widget::icon::from_name(chevron_icon).size(16))
                .on_press(Message::ToggleShaderDetails);

            list = list.add(
                container(details_button)
                    .width(Length::Fill)
                    .align_x(Alignment::Center),
            );

            // Collapsible details section
            if self.shader_details_expanded
                && let Some(shader_info) = self.available_shaders.get(shader_idx)
                && let Some(parsed) = &shader_info.parsed
            {
                let metadata = &parsed.metadata;

                // Author
                if !metadata.author.is_empty() {
                    list = list.add(settings::item(
                        fl!("shader-author"),
                        widget::text(&metadata.author),
                    ));
                }

                // Source (as a clickable link if it looks like a URL)
                if !metadata.source.is_empty() {
                    let source_url = metadata.source.clone();
                    let source_widget: Element<'_, Message> = if metadata.source.starts_with("http")
                    {
                        widget::button::link(source_url.clone())
                            .on_press(Message::OpenUrl(source_url))
                            .into()
                    } else {
                        widget::text(&metadata.source).into()
                    };
                    list = list.add(settings::item(fl!("shader-source"), source_widget));
                }

                // License
                if !metadata.license.is_empty() {
                    list = list.add(settings::item(
                        fl!("shader-license"),
                        widget::text(&metadata.license),
                    ));
                }

                // Resource usage estimate using naga-based analysis
                let param_values = self.shader_param_values.get(&shader_idx);
                let iteration_multiplier =
                    calculate_iteration_multiplier(&parsed.params, param_values);
                let has_texture = parsed.source_body.contains("iTexture")
                    || parsed.source_body.contains("textureSample");

                let complexity = shader_analysis::analyze_glowberry_shader(
                    &parsed.source_body,
                    has_texture,
                    Some(iteration_multiplier),
                )
                .map(|m| m.complexity())
                .unwrap_or(Complexity::Medium); // Default to medium if parsing fails

                let usage_label = match complexity {
                    Complexity::Low => fl!("resource-low"),
                    Complexity::Medium => fl!("resource-medium"),
                    Complexity::High => fl!("resource-high"),
                };
                list = list.add(settings::item(
                    fl!("shader-resource-usage"),
                    widget::text(usage_label),
                ));

                // Shader parameters
                for param in &parsed.params {
                    let current_values = self.shader_param_values.get(&shader_idx);
                    let current = current_values
                        .and_then(|v| v.get(&param.name))
                        .copied()
                        .unwrap_or(param.default);

                    let param_name = param.name.clone();
                    let idx = shader_idx;

                    match param.param_type {
                        ParamType::F32 => {
                            let min = param.min.as_f32();
                            let max = param.max.as_f32();
                            let step = param.step.as_f32();
                            let value = current.as_f32();

                            list = list.add(settings::item(
                                &param.label,
                                widget::row::with_children(vec![
                                    slider(min..=max, value, move |v| {
                                        Message::ShaderParamChanged(
                                            idx,
                                            param_name.clone(),
                                            ParamValue::F32(v),
                                        )
                                    })
                                    .on_release(Message::ShaderParamReleased)
                                    .step(step)
                                    .width(Length::Fixed(150.0))
                                    .into(),
                                    widget::text(format!("{:.2}", value))
                                        .width(Length::Fixed(50.0))
                                        .into(),
                                ])
                                .spacing(8)
                                .align_y(Alignment::Center),
                            ));
                        }
                        ParamType::I32 => {
                            let min = param.min.as_i32() as f32;
                            let max = param.max.as_i32() as f32;
                            let step = param.step.as_i32() as f32;
                            let value = current.as_i32() as f32;

                            let param_name_clone = param_name.clone();
                            list = list.add(settings::item(
                                &param.label,
                                widget::row::with_children(vec![
                                    slider(min..=max, value, move |v| {
                                        Message::ShaderParamChanged(
                                            idx,
                                            param_name_clone.clone(),
                                            ParamValue::I32(v as i32),
                                        )
                                    })
                                    .on_release(Message::ShaderParamReleased)
                                    .step(step)
                                    .width(Length::Fixed(150.0))
                                    .into(),
                                    widget::text(format!("{}", current.as_i32()))
                                        .width(Length::Fixed(50.0))
                                        .into(),
                                ])
                                .spacing(8)
                                .align_y(Alignment::Center),
                            ));
                        }
                    }
                }

                // Reset to defaults button
                list = list.add(
                    widget::button::destructive(fl!("reset-to-defaults"))
                        .on_press(Message::ResetShaderParams(shader_idx)),
                );
            }
        }

        // Apply custom style with opacity to the list
        let opacity = self.window_opacity;
        list.style(cosmic::theme::Container::custom(move |theme| {
            let cosmic = theme.cosmic();
            let component = &cosmic.background.component;
            let mut bg_color: cosmic::iced::Color = component.base.into();
            bg_color.a = opacity;
            cosmic::widget::container::Style {
                icon_color: Some(component.on.into()),
                text_color: Some(component.on.into()),
                background: Some(cosmic::iced::Background::Color(bg_color)),
                border: cosmic::iced::Border {
                    radius: cosmic.corner_radii.radius_s.into(),
                    ..Default::default()
                },
                shadow: cosmic::iced::Shadow::default(),
                snap: false,
            }
        }))
        .into()
    }

    fn view_multi_monitor_canvas(&self) -> Element<'_, Message> {
        use crate::widgets::extend_editor::{ExtendEditor, LayerView};

        let mut layer_views: Vec<LayerView<'_>> = self
            .extend_layers
            .iter()
            .map(|(key, layer)| LayerView {
                id: key,
                image_handle: layer.image_handle.as_ref(),
                image_size: layer.image_size,
                offset_x: layer.offset.0,
                offset_y: layer.offset.1,
                img_scale: layer.scale,
                z_index: layer.z_index,
                selected: self.extend_selected_layer == Some(key),
                locked: layer.locked,
                color: self.extend_layer_colors.get(key),
            })
            .collect();
        layer_views.sort_by_key(|l| l.z_index);

        let editor = ExtendEditor::new(
            &self.monitor_geometry,
            layer_views,
            Message::ExtendLayerMoved,
            Message::ExtendLayerScaled,
            Message::ExtendLayerSelected,
        )
        .on_right_click(Message::ExtendLayerRightClick)
        .fit_requested(self.extend_fit_view_requested);

        // Side buttons (right of canvas) — z-order and center only
        let mut side_buttons: Vec<Element<'_, Message>> = Vec::new();

        if let Some(sel_key) = self.extend_selected_layer {
            let is_locked = self.extend_layers.get(sel_key).is_some_and(|l| l.locked);

            if !is_locked {
                side_buttons.push(with_tip(
                    widget::button::icon(widget::icon::from_name("go-up-symbolic"))
                        .on_press(Message::ExtendLayerUp),
                    fl!("tip-layer-up"),
                ));
                side_buttons.push(with_tip(
                    widget::button::icon(widget::icon::from_name("go-down-symbolic"))
                        .on_press(Message::ExtendLayerDown),
                    fl!("tip-layer-down"),
                ));
                side_buttons.push(with_tip(
                    widget::button::icon(widget::icon::from_name("format-justify-center-symbolic"))
                        .on_press(Message::ExtendCenter),
                    fl!("tip-center"),
                ));
            }
        }

        let side_col = widget::column::with_children(side_buttons)
            .spacing(4)
            .align_x(Alignment::Center);

        // In color/live modes items are always locked and can't be expanded; the
        // lock/unlock and z-order controls don't apply.
        let locked_content_mode = matches!(
            self.categories.selected,
            Some(Category::Colors | Category::Shaders)
        );

        // Tool buttons overlaid on bottom-left of canvas
        let mut overlay_buttons: Vec<Element<'_, Message>> = Vec::new();

        if let Some(sel_key) = self.extend_selected_layer {
            let is_locked = self.extend_layers.get(sel_key).is_some_and(|l| l.locked);

            if !locked_content_mode {
                if is_locked {
                    overlay_buttons.push(with_tip(
                        widget::button::icon(widget::icon::from_name("changes-allow-symbolic"))
                            .on_press(Message::ExtendToggleLock(sel_key)),
                        fl!("tip-unlock"),
                    ));
                } else {
                    overlay_buttons.push(with_tip(
                        widget::button::icon(widget::icon::from_name("changes-prevent-symbolic"))
                            .on_press(Message::ExtendToggleLock(sel_key)),
                        fl!("tip-lock"),
                    ));
                }
            }

            overlay_buttons.push(with_tip(
                widget::button::icon(widget::icon::from_name("user-trash-symbolic"))
                    .on_press(Message::ExtendRemoveLayer(sel_key))
                    .class(cosmic::theme::Button::Destructive),
                fl!("tip-delete"),
            ));
        }

        // Only offer clear-all when nothing is selected, so it isn't mistaken
        // for (and adjacent to) the per-item delete button.
        if !self.extend_layers.is_empty() && self.extend_selected_layer.is_none() {
            overlay_buttons.push(with_tip(
                widget::button::icon(widget::icon::from_name("edit-clear-symbolic"))
                    .on_press(Message::ExtendClearAll)
                    .class(cosmic::theme::Button::Destructive),
                fl!("tip-clear-all"),
            ));
        }

        overlay_buttons.push(with_tip(
            widget::button::icon(widget::icon::from_name("zoom-fit-best-symbolic"))
                .on_press(Message::ExtendFitView),
            fl!("tip-fit"),
        ));

        let tool_col = widget::column::with_children(overlay_buttons).spacing(4);

        let tool_overlay = container(tool_col)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Start)
            .align_y(Alignment::End)
            .padding(6);

        // Z-order / center buttons overlaid on the top-right of the canvas.
        let side_overlay = container(side_col)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::End)
            .align_y(Alignment::Start)
            .padding(6);

        let canvas_container: Element<'_, Message> = cosmic::iced::widget::stack![
            container(editor)
                .width(Length::Fill)
                .height(Length::Fixed(300.0)),
            tool_overlay,
            side_overlay
        ]
        .width(Length::Fill)
        .height(Length::Fixed(300.0))
        .into();

        // Popover for layer right-click menu (always structurally present)
        let mut canvas_popover = widget::popover(canvas_container);
        if let Some((key, (cx, cy))) = self.layer_context_menu {
            let mut menu_items: Vec<Element<'_, Message>> = Vec::new();

            // Duplicate / show-on options (use Layer* messages that look up path from layer)
            menu_items.push(
                button::text(fl!("wp-duplicate-all"))
                    .on_press(Message::LayerDuplicateAll(key))
                    .width(Length::Fill)
                    .into(),
            );
            for monitor in &self.monitor_geometry {
                let name = monitor.name.clone();
                menu_items.push(
                    button::text(format!("{} {}", fl!("wp-show-on"), &name))
                        .on_press(Message::LayerShowOn(key, name))
                        .width(Length::Fill)
                        .into(),
                );
            }

            // Layer ordering and unlock don't apply to color/live items.
            if !locked_content_mode {
                menu_items.push(widget::divider::horizontal::light().into());
                menu_items.push(
                    button::text(fl!("ctx-bring-forward"))
                        .on_press(Message::ExtendLayerBringForward(key))
                        .width(Length::Fill)
                        .into(),
                );
                menu_items.push(
                    button::text(fl!("ctx-send-back"))
                        .on_press(Message::ExtendLayerSendBack(key))
                        .width(Length::Fill)
                        .into(),
                );
            }

            // Unlock / Remove
            menu_items.push(widget::divider::horizontal::light().into());
            if !locked_content_mode && self.extend_layers.get(key).is_some_and(|l| l.locked) {
                menu_items.push(
                    button::text(fl!("unlock-layer"))
                        .on_press(Message::ExtendToggleLock(key))
                        .width(Length::Fill)
                        .into(),
                );
            }
            menu_items.push(
                button::text(fl!("ctx-remove"))
                    .on_press(Message::ExtendRemoveLayer(key))
                    .width(Length::Fill)
                    .class(cosmic::theme::Button::Destructive)
                    .into(),
            );

            let popup = container(
                widget::column::with_children(menu_items)
                    .spacing(2)
                    .padding(8)
                    .width(Length::Fixed(220.0)),
            )
            .class(cosmic::theme::Container::custom(|theme| {
                let cosmic = theme.cosmic();
                cosmic::widget::container::Style {
                    background: Some(cosmic::iced::Background::Color(
                        cosmic.background.component.base.into(),
                    )),
                    icon_color: Some(cosmic.background.component.on.into()),
                    text_color: Some(cosmic.background.component.on.into()),
                    border: cosmic::iced::Border {
                        radius: cosmic.corner_radii.radius_m.into(),
                        width: 1.0,
                        color: cosmic.background.component.divider.into(),
                    },
                    shadow: cosmic::iced::Shadow {
                        color: cosmic::iced::Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                        offset: cosmic::iced::Vector::new(0.0, 2.0),
                        blur_radius: 8.0,
                    },
                    snap: false,
                }
            }));

            canvas_popover = canvas_popover
                .popup(popup)
                .position(widget::popover::Position::Point(cosmic::iced::Point {
                    x: cx,
                    y: cy,
                }))
                .on_close(Message::ExtendLayerMenuClose);
        }

        // Canvas row: editor (buttons are overlaid inside the canvas).
        let canvas_row: Element<'_, Message> = canvas_popover.into();

        // Bottom controls: clear all + apply
        let mut bottom: Vec<Element<'_, Message>> = Vec::new();

        bottom.push(
            button::text(fl!("extend-apply"))
                .on_press(Message::ApplyExtend)
                .class(cosmic::theme::Button::Suggested)
                .into(),
        );
        // Lock-screen export: images and live shaders (rendered to a snapshot).
        // Colors aren't supported by the greeter, so hide it in color mode.
        let is_color_mode = matches!(self.categories.selected, Some(Category::Colors));
        if !self.extend_layers.is_empty() && !is_color_mode {
            bottom.push(
                button::text(fl!("export-cosmic-bg"))
                    .on_press(Message::ExportToCosmicBg)
                    .into(),
            );
        }

        let bottom_row = widget::row::with_children(bottom)
            .spacing(8)
            .align_y(Alignment::Center);

        // Hint text
        let hint_text = match (self.extend_layers.is_empty(), locked_content_mode) {
            (true, true) => fl!("live-no-items"),
            (true, false) => fl!("extend-no-layers"),
            (false, true) => fl!("live-hint"),
            (false, false) => fl!("extend-hint"),
        };
        let hint: Element<'_, Message> = text::body(hint_text)
            .align_x(Alignment::Center)
            .width(Length::Fill)
            .into();

        widget::column::with_children(vec![
            canvas_row,
            hint,
            container(bottom_row)
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .into(),
        ])
        .spacing(8)
        .width(Length::Fill)
        .into()
    }

    /// Index of the user-added source a wallpaper path belongs to (the file
    /// itself, or a directory that contains it). `None` for bundled wallpapers.
    fn wallpaper_source_index_for(&self, path: &std::path::Path) -> Option<usize> {
        self.wallpaper_sources
            .iter()
            .position(|src| src.as_path() == path || (src.is_dir() && path.starts_with(src)))
    }

    fn view_wallpaper_grid(&self) -> Element<'_, Message> {
        let buttons: Vec<Element<'_, Message>> = self
            .selection
            .selection_handles
            .iter()
            .map(|(id, handle)| {
                // Left-click = add to canvas
                let img_button: Element<'_, Message> = widget::button::image(handle.clone())
                    .on_press(Message::WallpaperCustomize(id))
                    .into();

                // Right-click context menu
                let mut ctx_items = vec![
                    menu::Item::Button(fl!("wp-customize"), None, WallpaperAction::Customize(id)),
                    menu::Item::Button(
                        fl!("wp-duplicate-all"),
                        None,
                        WallpaperAction::DuplicateAll(id),
                    ),
                    menu::Item::Button(fl!("wp-span-all"), None, WallpaperAction::SpanAll(id)),
                ];
                for (idx, monitor) in self.monitor_geometry.iter().enumerate() {
                    ctx_items.push(menu::Item::Button(
                        format!("{} {}", fl!("wp-show-on"), &monitor.name),
                        None,
                        WallpaperAction::ShowOn(id, idx),
                    ));
                }

                // Removing an added wallpaper: only offered for user-added
                // sources, not the bundled ones.
                if let Some(path) = self.selection.paths.get(id)
                    && let Some(src_idx) = self.wallpaper_source_index_for(path)
                {
                    ctx_items.push(menu::Item::Divider);
                    ctx_items.push(menu::Item::Button(
                        fl!("wp-remove-source"),
                        None,
                        WallpaperAction::RemoveSource(src_idx),
                    ));
                }

                widget::context_menu(img_button, Some(menu::items(&HashMap::new(), ctx_items)))
                    .into()
            })
            .collect();

        let grid = widget::flex_row(buttons).column_spacing(12).row_spacing(16);

        // Toolbar: add images / add folder.
        let toolbar = widget::row::with_children(vec![
            button::text(fl!("add-images"))
                .leading_icon(widget::icon::from_name("list-add-symbolic"))
                .on_press(Message::AddWallpaperImages)
                .into(),
            button::text(fl!("add-folder"))
                .leading_icon(widget::icon::from_name("folder-new-symbolic"))
                .on_press(Message::AddWallpaperFolder)
                .into(),
        ])
        .spacing(8)
        .align_y(Alignment::Center);

        widget::column::with_children(vec![toolbar.into(), grid.into()])
            .spacing(12)
            .into()
    }

    fn view_color_grid(&self) -> Element<'_, Message> {
        let selected = if let Choice::Color(ref c) = self.selection.active {
            Some(c)
        } else {
            None
        };

        let buttons: Vec<Element<'_, Message>> = DEFAULT_COLORS
            .iter()
            .enumerate()
            .map(|(idx, color)| {
                let content = color_image(color.clone(), 70, 70);
                let swatch: Element<'_, Message> =
                    button::custom_image_button(content, None::<Message>)
                        .padding(0)
                        .selected(selected == Some(color))
                        .class(button::ButtonClass::Image)
                        .on_press(Message::ColorSelect(color.clone()))
                        .into();

                let mut ctx_items = vec![menu::Item::Button(
                    fl!("apply-all"),
                    None,
                    ColorAction::All(idx),
                )];
                for (m, monitor) in self.monitor_geometry.iter().enumerate() {
                    ctx_items.push(menu::Item::Button(
                        format!("{} {}", fl!("wp-show-on"), &monitor.name),
                        None,
                        ColorAction::ShowOn(idx, m),
                    ));
                }
                widget::context_menu(swatch, Some(menu::items(&HashMap::new(), ctx_items))).into()
            })
            .collect();

        widget::flex_row(buttons)
            .column_spacing(12)
            .row_spacing(16)
            .into()
    }

    fn view_shader_grid(&self) -> Element<'_, Message> {
        let selected = if let Choice::Shader(idx) = self.selection.active {
            Some(idx)
        } else {
            None
        };

        if self.available_shaders.is_empty() {
            return widget::text(fl!("no-shaders")).into();
        }

        let buttons: Vec<Element<'_, Message>> = self
            .shader_thumbnails
            .iter()
            .enumerate()
            .map(|(idx, handle)| {
                let name = self
                    .available_shaders
                    .get(idx)
                    .map(|s| s.name.as_str())
                    .unwrap_or("Unknown");

                let item: Element<'_, Message> = widget::column::with_children(vec![
                    widget::button::image(handle.clone())
                        .selected(selected == Some(idx))
                        .on_press(Message::ShaderSelect(idx))
                        .into(),
                    widget::text::caption(name)
                        .width(Length::Fixed(158.0))
                        .align_x(Alignment::Center)
                        .into(),
                ])
                .spacing(4)
                .align_x(Alignment::Center)
                .into();

                let mut ctx_items = vec![menu::Item::Button(
                    fl!("apply-all"),
                    None,
                    ShaderAction::All(idx),
                )];
                for (m, monitor) in self.monitor_geometry.iter().enumerate() {
                    ctx_items.push(menu::Item::Button(
                        format!("{} {}", fl!("wp-show-on"), &monitor.name),
                        None,
                        ShaderAction::ShowOn(idx, m),
                    ));
                }
                widget::context_menu(item, Some(menu::items(&HashMap::new(), ctx_items))).into()
            })
            .collect();

        widget::flex_row(buttons)
            .column_spacing(12)
            .row_spacing(16)
            .into()
    }
}

// Helper functions

/// Wrap a widget (typically an icon button) with a hover tooltip.
fn with_tip<'a>(content: impl Into<Element<'a, Message>>, tip: String) -> Element<'a, Message> {
    widget::tooltip(
        content,
        widget::text::body(tip),
        widget::tooltip::Position::Top,
    )
    .into()
}

fn color_image<'a, M: 'a>(color: Color, width: u16, height: u16) -> Element<'a, M> {
    use cosmic::iced::{Background, Border, Degrees, Gradient, gradient::Linear};

    container(widget::Space::new().width(width).height(height))
        .class(cosmic::theme::Container::custom(move |theme| {
            container::Style {
                background: Some(match &color {
                    Color::Single([r, g, b]) => {
                        Background::Color(cosmic::iced::Color::from_rgb(*r, *g, *b))
                    }
                    Color::Gradient(crate::app::Gradient { colors, radius }) => {
                        let stop_increment = 1.0 / (colors.len() - 1) as f32;
                        let mut stop = 0.0;
                        let mut linear = Linear::new(Degrees(*radius));
                        for &[r, g, b] in &**colors {
                            linear = linear.add_stop(stop, cosmic::iced::Color::from_rgb(r, g, b));
                            stop += stop_increment;
                        }
                        Background::Gradient(Gradient::Linear(linear))
                    }
                }),
                border: Border {
                    radius: theme.cosmic().corner_radii.radius_s.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        }))
        .into()
}

fn shader_placeholder<'a, M: 'a>(width: u16, height: u16) -> Element<'a, M> {
    use cosmic::iced::{Background, Degrees, Gradient, gradient::Linear};

    container(widget::Space::new().width(width).height(height))
        .class(cosmic::theme::Container::custom(|_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(
                Linear::new(Degrees(135.0))
                    .add_stop(0.0, cosmic::iced::Color::from_rgb(0.08, 0.02, 0.15))
                    .add_stop(0.5, cosmic::iced::Color::from_rgb(0.02, 0.08, 0.12))
                    .add_stop(1.0, cosmic::iced::Color::from_rgb(0.05, 0.02, 0.1)),
            ))),
            ..Default::default()
        }))
        .into()
}

fn create_shader_placeholder(width: u32, height: u32) -> ImageHandle {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let r = (25.0 + 15.0 * (x as f32 / width as f32)) as u8;
            let g = (12.0 + 20.0 * (y as f32 / height as f32)) as u8;
            let b = (50.0 + 25.0 * ((x + y) as f32 / (width + height) as f32)) as u8;
            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    ImageHandle::from_rgba(width, height, data)
}

/// Calculate iteration multiplier from shader parameters that control loops
fn calculate_iteration_multiplier(
    params: &[crate::shader_params::ShaderParam],
    param_values: Option<&HashMap<String, ParamValue>>,
) -> f32 {
    let mut multiplier = 1.0f32;

    for param in params {
        let name_lower = param.name.to_lowercase();
        let is_iteration_param = name_lower.contains("iteration")
            || name_lower.contains("layers")
            || name_lower.contains("steps")
            || name_lower.contains("samples")
            || (name_lower == "zoom" && param.param_type == ParamType::I32)
            || name_lower.contains("num_")
            || name_lower.contains("count");

        if is_iteration_param {
            let value = param_values
                .and_then(|v| v.get(&param.name))
                .unwrap_or(&param.default);

            let iter_count = value.as_i32().max(1) as f32;
            // Normalize: assume default of ~10 iterations is "normal"
            multiplier *= (iter_count / 10.0).max(0.5);
        }
    }

    multiplier
}

fn discover_shaders() -> Vec<ShaderInfo> {
    let mut shaders = Vec::new();

    // Use xdg crate to search all data directories for shader files.
    // With prefix "glowberry", this searches:
    //   ~/.local/share/glowberry/shaders/
    //   $XDG_DATA_DIRS/glowberry/shaders/ (defaults: /usr/local/share, /usr/share)
    // list_data_files_once deduplicates by filename (first occurrence wins).
    let xdg = xdg::BaseDirectories::with_prefix("glowberry");
    for path in xdg.list_data_files_once("shaders") {
        if path.extension().is_some_and(|e| e == "wgsl") {
            collect_shader_file(&path, &mut shaders);
        }
    }

    shaders.sort_by(|a, b| a.name.cmp(&b.name));
    shaders
}

fn collect_shader_file(path: &std::path::Path, shaders: &mut Vec<ShaderInfo>) {
    // Try to parse the shader to get metadata
    let parsed = ParsedShader::parse(path);

    // Use parsed name if available, otherwise derive from filename
    let name = parsed
        .as_ref()
        .filter(|p| !p.metadata.name.is_empty())
        .map(|p| p.metadata.name.clone())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| titlecase(&s.replace('_', " ")))
                .unwrap_or_else(|| "Unknown".to_string())
        });

    shaders.push(ShaderInfo {
        path: path.to_path_buf(),
        name,
        parsed,
    });
}

/// Find the wallpaper folder by searching XDG data directories.
fn find_wallpaper_folder() -> PathBuf {
    let subdir = "backgrounds/cosmic";

    // Use xdg crate to search all data directories
    // (checks ~/.local/share, then XDG_DATA_DIRS / defaults)
    let xdg = xdg::BaseDirectories::new();
    xdg.find_data_file(subdir)
        .unwrap_or_else(|| PathBuf::from("/usr/share").join(subdir))
}

fn titlecase(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Check if ~/.local/bin comes before /usr/bin in PATH.
///
/// Returns:
/// - `Ok(true)` if PATH order is correct (or /usr/bin not in PATH)
/// - `Ok(false)` if /usr/bin comes before ~/.local/bin
/// - `Err(msg)` if ~/.local/bin is not in PATH at all
fn check_path_order() -> Result<bool, &'static str> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    let local_bin = dirs::home_dir()
        .map(|h| h.join(".local/bin"))
        .unwrap_or_default();
    let local_bin_str = local_bin.to_string_lossy();

    let mut local_bin_pos: Option<usize> = None;
    let mut usr_bin_pos: Option<usize> = None;

    for (i, p) in path_var.split(':').enumerate() {
        if p == local_bin_str.as_ref() && local_bin_pos.is_none() {
            local_bin_pos = Some(i);
        } else if p == "/usr/bin" && usr_bin_pos.is_none() {
            usr_bin_pos = Some(i);
        }
    }

    match (local_bin_pos, usr_bin_pos) {
        (None, _) => Err("~/.local/bin is not in PATH"),
        (Some(_), None) => Ok(true), // /usr/bin not in PATH, so ~/.local/bin wins
        (Some(local), Some(usr)) => Ok(local < usr),
    }
}

/// Check if GlowBerry is currently enabled as the default background service.
///
/// This works by checking if ~/.local/bin/cosmic-bg exists and is a symlink
/// pointing to glowberry. Since ~/.local/bin is searched before /usr/bin in PATH,
/// cosmic-session will run glowberry instead of the original cosmic-bg when enabled.
fn is_glowberry_default() -> bool {
    let symlink_path = dirs::home_dir()
        .map(|h| h.join(".local/bin/cosmic-bg"))
        .unwrap_or_default();
    match std::fs::read_link(symlink_path) {
        Ok(target) => target.to_string_lossy().contains("glowberry"),
        Err(_) => false,
    }
}

/// Check if the PATH is configured correctly for GlowBerry override to work.
fn is_path_order_correct() -> bool {
    check_path_order().unwrap_or(false)
}

/// Enable or disable GlowBerry as the default background service.
///
/// When enabled, creates a symlink at ~/.local/bin/cosmic-bg -> ~/.local/bin/glowberry.
/// When disabled, removes the symlink so the original /usr/bin/cosmic-bg is used.
///
/// No elevated privileges needed since we operate in ~/.local/bin/.
async fn set_glowberry_default(enable: bool) -> Result<bool, String> {
    use tokio::process::Command;

    let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
    let local_bin = home.join(".local/bin");
    let symlink_path = local_bin.join("cosmic-bg");
    let glowberry_bin = local_bin.join("glowberry");

    // Check PATH order when enabling
    if enable {
        match check_path_order() {
            Err(msg) => {
                return Err(format!(
                    "Cannot enable GlowBerry: {}. \
                    Add ~/.local/bin to your PATH before /usr/bin.",
                    msg
                ));
            }
            Ok(false) => {
                return Err(
                    "Cannot enable GlowBerry: /usr/bin comes before ~/.local/bin in PATH. \
                    The symlink override won't work. Please fix your PATH configuration \
                    so that ~/.local/bin appears before /usr/bin."
                        .to_string(),
                );
            }
            Ok(true) => {} // PATH is correct, proceed
        }
    }

    if enable {
        // Ensure ~/.local/bin exists
        std::fs::create_dir_all(&local_bin)
            .map_err(|e| format!("Failed to create ~/.local/bin: {}", e))?;

        // Remove existing symlink/file if present
        let _ = std::fs::remove_file(&symlink_path);

        // Create symlink to make glowberry intercept cosmic-bg calls
        std::os::unix::fs::symlink(&glowberry_bin, &symlink_path)
            .map_err(|e| format!("Failed to create symlink: {}", e))?;
    } else {
        // Remove symlink to restore original cosmic-bg
        if symlink_path.is_symlink() {
            std::fs::remove_file(&symlink_path)
                .map_err(|e| format!("Failed to remove symlink: {}", e))?;
        }
    }

    // Kill the daemon processes so the correct one restarts
    // Use -x for exact match to avoid killing glowberry-settings
    let user = std::env::var("USER").unwrap_or_default();
    if !user.is_empty() {
        // Kill glowberry daemon (exact match, not glowberry-settings)
        let _ = Command::new("pkill")
            .args(["-x", "-u", &user, "glowberry"])
            .output()
            .await;

        // Kill cosmic-bg (exact match)
        let _ = Command::new("pkill")
            .args(["-x", "-u", &user, "cosmic-bg"])
            .output()
            .await;
    }

    Ok(enable)
}
