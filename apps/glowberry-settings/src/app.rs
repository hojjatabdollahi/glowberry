// SPDX-License-Identifier: MPL-2.0

//! Main application state and logic for GlowBerry Settings.
//!
//! The window is display-first: the canvas at the top shows the connected
//! displays with whatever is staged on them, the library below holds images,
//! live wallpapers and colors, and the drawer on the right shows the settings
//! for the selected display(s). Picking from the library puts that content on
//! the selected displays (all of them when none is selected). Nothing reaches
//! the daemon until Apply.

use crate::fl;
use crate::monitor_query::MonitorGeometry;
use crate::shader_analysis::{self, Complexity};
use crate::shader_params::{ParamType, ParamValue, ParsedShader};
use cosmic::app::context_drawer::{self, ContextDrawer};
use cosmic::app::{Core, Task};
use cosmic::iced::Subscription;
use cosmic::iced::widget::image::Handle as ImageHandle;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{
    self, button, container, dropdown, menu, segmented_button, segmented_control, settings, slider,
    text, toggler,
};
use cosmic::{ApplicationExt, Element};
use cosmic_config::{ConfigGet, ConfigSet};
use glowberry_config::extend::ExtendConfig;
use glowberry_config::power_saving::{OnBatteryAction, PowerSavingConfig};
use glowberry_config::state::State;
use glowberry_config::{
    Color, Config, Context as ConfigContext, Entry, Gradient, OutputProfile, ScalingMode, Source,
};
use image::{ImageBuffer, Rgba};
use slotmap::{DefaultKey, SecondaryMap, SlotMap};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod wallpaper_subscription;
use wallpaper_subscription::WallpaperEvent;

/// Application ID for GlowBerry Settings
pub const APP_ID: &str = "io.github.hojjatabdollahi.glowberry-settings";

/// Library thumbnail size.
const THUMB_WIDTH: u32 = 158;
const THUMB_HEIGHT: u32 = 105;

/// Resolution of live-animated shader preview frames. Kept modest so software
/// GPUs (llvmpipe) can sustain a smooth frame rate; the image is scaled up to
/// fill each display rect in the canvas.
const LIVE_PREVIEW_WIDTH: u32 = 320;
const LIVE_PREVIEW_HEIGHT: u32 = 180;

/// Number of usage tips in the help dock (`tip-1` .. `tip-N` in the strings).
const TIP_COUNT: usize = 6;

/// Intrinsic size of a live item on the canvas. The preview frame is stretched
/// to cover the display, so only the aspect matters.
const LIVE_ITEM_SIZE: (u32, u32) = (1920, 1080);

/// Persistent offscreen renderer for the live shader preview. Reused across
/// frames so we create the wgpu device once (rebuilt only when the shader
/// source or its parameter values change).
#[derive(Default)]
struct LivePreview {
    renderer: Option<crate::widgets::shader_preview::ShaderPreviewRenderer>,
    /// The WGSL source the current renderer was built from, used to detect when
    /// a rebuild is needed (e.g. parameter edits or shader switch).
    source: String,
}

/// Context drawer page
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

    /// Library filter: all, images, live, colors
    library_filter: segmented_button::SingleSelectModel,
    /// Library search text
    library_query: String,

    /// Loaded wallpaper images and the last picked item (drives the live preview)
    selection: SelectionContext,

    /// Available system shaders
    available_shaders: Vec<ShaderInfo>,
    /// Shader preview thumbnails
    shader_thumbnails: Vec<ImageHandle>,
    /// Selected shader frame rate index
    selected_shader_frame_rate: usize,
    /// Frame rate options
    frame_rate_options: Vec<String>,
    /// Selected shader render quality index (0=Full, 1=Half, 2=Quarter)
    selected_shader_render_scale: usize,
    /// Render quality options
    render_scale_options: Vec<String>,
    /// Image fit options (Zoom to fill, Fit inside, Stretch)
    fit_options: Vec<String>,

    /// Current wallpaper folder
    current_folder: PathBuf,
    /// User-added wallpaper sources (image files and/or directories), shown in
    /// the grid in addition to the default folder.
    wallpaper_sources: Vec<PathBuf>,

    /// Prefer low power GPU for shader rendering
    prefer_low_power: bool,

    /// Whether GlowBerry is currently set as the default background service
    glowberry_is_default: bool,

    /// Current shader parameter values (shader_index -> param_name -> value).
    // ponytail: parameters are per shader, not per display; two displays running
    // the same shader share one set of values. Move them onto the layer's
    // Source if per-display tuning is ever wanted (the config already allows it).
    shader_param_values: HashMap<usize, HashMap<String, ParamValue>>,

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

    /// Saved free (spanning) image layers
    extend_config: ExtendConfig,
    /// Connected displays
    monitor_geometry: Vec<MonitorGeometry>,
    /// Displays the next pick applies to (connector names). Empty means all.
    selected_displays: Vec<String>,
    /// Placement for images when several displays are selected
    placement_model: segmented_button::SingleSelectModel,
    /// Logical window size, for collapsing the inspector on narrow windows
    /// and scrolling the whole content on short ones
    window_width: f32,
    window_height: f32,
    /// Whether the inspector is shown while the window is narrow
    inspector_open: bool,
    /// Which usage tip the help dock shows
    tip_index: usize,
    /// The help dock was closed; remembered in config
    tips_hidden: bool,

    /// The canvas context menu that is open, and where
    canvas_menu: Option<(CanvasMenu, (f32, f32))>,
    /// Layers on the virtual desktop canvas: locked ones fill one display,
    /// free ones are images spanning wherever the user puts them.
    extend_layers: SlotMap<DefaultKey, ExtendLayerState>,
    /// Color of a locked color item
    extend_layer_colors: SecondaryMap<DefaultKey, Color>,
    /// Config source written for a locked item on Apply
    extend_layer_sources: SecondaryMap<DefaultKey, Source>,
    /// Fit mode of a locked image item
    extend_layer_fit: SecondaryMap<DefaultKey, ScalingMode>,
    /// Currently selected free layer
    extend_selected_layer: Option<DefaultKey>,
    /// Next z-index to assign
    extend_next_z: usize,
    /// Request the canvas to fit all content in view
    extend_fit_view_requested: bool,

    /// Persistent renderer for the live shader preview in the canvas.
    live_preview: Arc<Mutex<LivePreview>>,
    /// True while a live preview frame is being rendered off-thread, so ticks
    /// don't pile up faster than the GPU can render them.
    live_preview_in_flight: bool,
    /// True while the shader thumbnail grid is rendering. The live preview is
    /// paused during this window so we never create wgpu devices concurrently
    /// (which can crash software renderers like llvmpipe).
    shader_thumbnails_loading: bool,
    /// The config changed under us (daemon restored a display-set profile);
    /// restage the canvas once the monitor list has been refreshed.
    restage_on_monitors: bool,
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
    /// Estimated GPU load at default parameters
    pub load: Option<Complexity>,
}

/// The last picked library item. Drives the live preview and the shader
/// parameter editor.
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

/// Loaded wallpapers and the active choice
#[derive(Clone, Debug, Default)]
struct SelectionContext {
    active: Choice,
    paths: SlotMap<DefaultKey, PathBuf>,
    display_images: SecondaryMap<DefaultKey, ImageBuffer<Rgba<u8>, Vec<u8>>>,
    selection_handles: SecondaryMap<DefaultKey, ImageHandle>,
}

/// Which kinds the library grid shows
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LibraryFilter {
    All,
    Images,
    Live,
    Colors,
}

/// How an image lands on several selected displays
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// The same image, filling each display
    Each,
    /// One image spanning across the displays
    Span,
}

/// What a display currently shows on the canvas.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    Nothing,
    /// A locked image filling the display, or part of the free layer `span`
    /// stretched across several displays.
    Image {
        path: PathBuf,
        span: Option<DefaultKey>,
    },
    Color(Color),
    Shader(usize),
}

/// Application messages
#[derive(Debug, Clone)]
pub enum Message {
    /// Library filter changed
    LibraryFilter(segmented_button::Entity),
    /// Library search text changed
    LibrarySearch(String),
    /// Put an image on the selected displays
    PickImage(DefaultKey),
    /// Put a color (index into `DEFAULT_COLORS`) on the selected displays
    PickColor(usize),
    /// Put a live wallpaper on the selected displays
    PickShader(usize),
    /// A library card's context menu
    Card(CardAction),

    /// A display was clicked in the canvas (connector, add to selection)
    DisplaySelected(String, bool),
    /// Right-click on a display in the canvas (connector, x, y)
    DisplayRightClick(String, f32, f32),
    /// Remove whatever is on a display
    DisplayClear(String),
    /// Put what a display shows on every display
    DisplayDuplicateAll(String),
    /// Span the image a display shows across every display
    DisplaySpanAll(String),
    /// Empty canvas was clicked: back to all displays
    CanvasBackgroundClicked,
    /// Select every display
    SelectAllDisplays,
    /// Show or hide the inspector on a narrow window
    ToggleInspector,
    /// Show the next usage tip in the help dock
    NextTip,
    /// Close the help dock for good
    HideTips,
    /// Placement changed (same on each / span across)
    SetPlacement(segmented_button::Entity),
    /// Image fit changed for the selected displays
    SetImageFit(usize),

    /// Shader thumbnails finished rendering
    ShaderThumbnailsLoaded(Vec<(usize, Option<ImageHandle>)>),
    /// Frame tick driving the live shader preview animation.
    PreviewTick,
    /// A live preview frame finished rendering (shader index, frame image).
    PreviewFrame(usize, Option<ImageHandle>),
    /// Frame rate changed
    ShaderFrameRate(usize),
    /// Render quality (resolution scale) changed
    ShaderRenderScale(usize),
    /// Shader parameter changed (shader_index, param_name, value)
    ShaderParamChanged(usize, String, ParamValue),
    /// Shader parameter slider released
    ShaderParamReleased,
    /// Reset shader parameters to defaults
    ResetShaderParams(usize),

    /// Wallpaper event from subscription
    WallpaperEvent(WallpaperEvent),
    /// Open a file picker to add image files to the grid
    AddWallpaperImages,
    /// Open a folder picker to add a directory to the grid
    AddWallpaperFolder,
    /// Paths chosen from a picker were added as wallpaper sources
    WallpaperSourcesPicked(Vec<PathBuf>),

    /// Toggle context drawer page
    ToggleContextPage(ContextPage),
    /// Open URL (for about page links)
    OpenUrl(String),
    /// Prefer low power GPU toggle
    PreferLowPower(bool),
    /// Config or state changed externally (from daemon or another instance)
    ConfigOrStateChanged(Option<Config>),
    /// Toggle GlowBerry as the default background service
    SetGlowBerryDefault(bool),
    /// Result of setting GlowBerry as default
    SetGlowBerryDefaultResult(Result<bool, String>),

    // Power saving messages
    SetOnBatteryAction(usize),
    SetPauseOnLowBattery(bool),
    SetLowBatteryThreshold(usize),
    SetPauseOnLidClosed(bool),

    /// Window opacity slider changed (live preview)
    SetWindowOpacity(f32),
    /// Window opacity slider released (save to config)
    WindowOpacityReleased,

    /// Monitor geometry loaded from cosmic-randr
    MonitorsLoaded(Vec<MonitorGeometry>),
    /// Remove a layer
    ExtendRemoveLayer(DefaultKey),
    /// Layer moved in the editor
    ExtendLayerMoved(DefaultKey, f64, f64),
    /// Layer scaled in the editor
    ExtendLayerScaled(DefaultKey, f64),
    /// Free layer selected/deselected
    ExtendLayerSelected(Option<DefaultKey>),
    /// Move selected layer up in z-order
    ExtendLayerUp,
    /// Move selected layer down in z-order
    ExtendLayerDown,
    /// Center the selected layer on the virtual desktop
    ExtendCenter,
    /// Fit a free layer over all displays
    ExtendLayerFit(DefaultKey),
    /// Reset canvas camera to fit all content
    ExtendFitView,
    /// Clear all layers
    ExtendClearAll,
    /// Right-click on a layer in the canvas (key, x, y relative to widget)
    ExtendLayerRightClick(DefaultKey, f32, f32),
    /// Close the canvas layer context menu
    ExtendLayerMenuClose,
    /// Bring a specific layer forward (z+1)
    ExtendLayerBringForward(DefaultKey),
    /// Send a specific layer back (z-1)
    ExtendLayerSendBack(DefaultKey),

    /// Write the staged canvas to the daemon's config
    Apply,
    /// Spanning images were composited per display
    Applied(Result<Vec<(String, PathBuf)>, String>),
    /// Throw away staged changes and show what is applied
    Revert,
}

/// A library item, as a copyable handle for menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Image(DefaultKey),
    Color(usize),
    Shader(usize),
}

/// Context menu actions on library cards
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardAction {
    /// Put the item on every display
    PutOnAll(Item),
    /// Put the item on one display (index into `monitor_geometry`)
    PutOn(Item, usize),
    /// Span an image across every display
    SpanAll(DefaultKey),
    /// Remove a user-added source (index into `wallpaper_sources`).
    RemoveSource(usize),
}

impl menu::Action for CardAction {
    type Message = Message;
    fn message(&self) -> Message {
        Message::Card(*self)
    }
}

/// What the canvas context menu is about
#[derive(Clone, Debug)]
enum CanvasMenu {
    /// A free (spanning) image layer
    Layer(DefaultKey),
    /// A display, including the locked item on it
    Display(String),
}

/// Default colors shown in the library
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

    fn on_window_resize(&mut self, _id: cosmic::iced::window::Id, width: f32, height: f32) {
        self.window_width = width;
        self.window_height = height;
    }

    fn init(mut core: Core, _flags: Self::Flags) -> (Self, Task<Self::Message>) {
        // We paint our own background and panel, so drop libcosmic's content
        // container and the padding it leaves around the content and footer.
        core.window.content_container = false;
        core.window.border_padding = Some(0);

        // Load configuration
        let config_context = glowberry_config::context().ok();
        let config = config_context
            .as_ref()
            .and_then(|ctx| Config::load(ctx).ok())
            .unwrap_or_default();

        let library_filter = segmented_button::Model::builder()
            .insert(|b| {
                b.text(fl!("filter-all"))
                    .data(LibraryFilter::All)
                    .activate()
            })
            .insert(|b| b.text(fl!("filter-images")).data(LibraryFilter::Images))
            .insert(|b| b.text(fl!("filter-live")).data(LibraryFilter::Live))
            .insert(|b| b.text(fl!("filter-colors")).data(LibraryFilter::Colors))
            .build();

        let placement_model = segmented_button::Model::builder()
            .insert(|b| {
                b.text(fl!("placement-each"))
                    .data(Placement::Each)
                    .activate()
            })
            .insert(|b| b.text(fl!("placement-span")).data(Placement::Span))
            .build();

        // Default wallpaper folder - search XDG data directories
        let current_folder = find_wallpaper_folder();

        let available_shaders = discover_shaders();
        let placeholder = create_shader_placeholder(THUMB_WIDTH, THUMB_HEIGHT);
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
            library_filter,
            library_query: String::new(),
            selection: SelectionContext::default(),
            available_shaders,
            shader_thumbnails,
            selected_shader_frame_rate: 1, // 30 FPS default
            frame_rate_options: vec![fl!("fps-15"), fl!("fps-30"), fl!("fps-60")],
            selected_shader_render_scale: 0, // Full resolution default
            render_scale_options: vec![
                fl!("quality-full"),
                fl!("quality-half"),
                fl!("quality-quarter"),
            ],
            fit_options: vec![fl!("fit-zoom"), fl!("fit-inside"), fl!("fit-stretch")],
            current_folder,
            wallpaper_sources: Vec::new(), // Will be set below from config
            prefer_low_power: true,        // Will be set below
            glowberry_is_default: is_glowberry_default(),
            shader_param_values: HashMap::new(),
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
            selected_displays: Vec::new(),
            placement_model,
            window_width: 0.0,
            window_height: 0.0,
            inspector_open: false,
            tip_index: 0,
            tips_hidden: false,
            canvas_menu: None,
            extend_layers: SlotMap::new(),
            extend_layer_colors: SecondaryMap::new(),
            extend_layer_sources: SecondaryMap::new(),
            extend_layer_fit: SecondaryMap::new(),
            extend_selected_layer: None,
            extend_next_z: 0,
            extend_fit_view_requested: false,
            live_preview: Arc::new(Mutex::new(LivePreview::default())),
            live_preview_in_flight: false,
            shader_thumbnails_loading: false,
            restage_on_monitors: false,
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
            app.tips_hidden = ctx.0.get::<bool>("tips-hidden").unwrap_or(false);

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

        // Load monitor geometry for the canvas
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
            // The daemon records connected outputs in the *state* namespace,
            // which the config subscription above does not watch.
            subscriptions.push(
                cosmic_config::config_state_subscription::<_, State>(
                    std::any::TypeId::of::<State>(),
                    glowberry_config::NAME.into(),
                    State::version(),
                )
                .map(|_| Message::ConfigOrStateChanged(None)),
            );
        }

        // Drive the live shader preview animation only while a shader is the
        // active selection — no wasted repaints for images/colors.
        if matches!(self.selection.active, Choice::Shader(_)) {
            // Cap the preview at 30 FPS; the 60 FPS option is for the wallpaper
            // itself, not this software-rendered preview.
            let fps = if self.selected_shader_frame_rate == 0 {
                15
            } else {
                30
            };
            subscriptions.push(
                cosmic::iced::time::every(Duration::from_millis(1000 / fps))
                    .map(|_| Message::PreviewTick),
            );
        }

        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        // Clear one-shot flags
        self.extend_fit_view_requested = false;

        match message {
            Message::LibraryFilter(entity) => self.library_filter.activate(entity),
            Message::LibrarySearch(query) => self.library_query = query,

            Message::PickImage(key) => {
                let targets = self.target_monitors();
                let placement = self.placement();
                self.pick_image(key, &targets, placement);
            }
            Message::PickColor(idx) => {
                let targets = self.target_monitors();
                self.pick(Item::Color(idx), &targets, Placement::Each);
            }
            Message::PickShader(idx) => {
                let targets = self.target_monitors();
                self.pick_shader(idx, &targets);
            }
            Message::Card(action) => match action {
                CardAction::PutOnAll(item) => {
                    let all = self.monitor_geometry.clone();
                    self.pick(item, &all, Placement::Each);
                }
                CardAction::PutOn(item, i) => {
                    if let Some(m) = self.monitor_geometry.get(i).cloned() {
                        self.pick(item, &[m], Placement::Each);
                    }
                }
                CardAction::SpanAll(key) => {
                    let all = self.monitor_geometry.clone();
                    self.pick_image(key, &all, Placement::Span);
                }
                CardAction::RemoveSource(idx) => {
                    if idx < self.wallpaper_sources.len() {
                        self.wallpaper_sources.remove(idx);
                        if let Some(ctx) = &self.config_context {
                            let _ = ctx
                                .0
                                .set("wallpaper-sources", self.wallpaper_sources.clone());
                        }
                    }
                }
            },

            Message::DisplayRightClick(name, x, y) => {
                self.canvas_menu = Some((CanvasMenu::Display(name), (x, y)));
            }

            Message::DisplayClear(name) => {
                self.canvas_menu = None;
                self.remove_locked_on(&name);
                self.renormalize_z_indices();
            }

            Message::DisplayDuplicateAll(name) => {
                self.canvas_menu = None;
                self.spread_display(&name, Placement::Each);
            }

            Message::DisplaySpanAll(name) => {
                self.canvas_menu = None;
                self.spread_display(&name, Placement::Span);
            }

            Message::ExtendLayerFit(key) => {
                self.canvas_menu = None;
                if let Some(mut layer) = self.extend_layers.get(key).cloned() {
                    fit_layer_to(&mut layer, &self.monitor_geometry);
                    self.extend_layers[key] = layer;
                }
            }

            Message::DisplaySelected(name, additive) => {
                self.canvas_menu = None;
                if additive {
                    if let Some(pos) = self.selected_displays.iter().position(|n| *n == name) {
                        self.selected_displays.remove(pos);
                    } else {
                        self.selected_displays.push(name);
                    }
                } else {
                    self.selected_displays = vec![name];
                }
                self.sync_choice_to_targets();
            }

            Message::CanvasBackgroundClicked => {
                self.extend_selected_layer = None;
                self.canvas_menu = None;
                self.selected_displays.clear();
                self.sync_choice_to_targets();
            }

            Message::ToggleInspector => self.inspector_open = !self.inspector_open,
            Message::NextTip => self.tip_index = (self.tip_index + 1) % TIP_COUNT,
            Message::HideTips => {
                self.tips_hidden = true;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.0.set("tips-hidden", true);
                }
            }

            Message::SelectAllDisplays => {
                self.selected_displays.clear();
                self.sync_choice_to_targets();
            }

            Message::SetPlacement(entity) => {
                self.placement_model.activate(entity);
                // Re-place the image the selected displays already share.
                let targets = self.target_monitors();
                if let Some(Shown::Image { path, .. }) = self.shared_shown(&targets)
                    && let Some((key, _)) = self.selection.paths.iter().find(|(_, p)| **p == path)
                {
                    let placement = self.placement();
                    self.pick_image(key, &targets, placement);
                }
            }

            Message::SetImageFit(idx) => {
                let mode = fit_mode(idx);
                for m in self.target_monitors() {
                    if let Some(k) = self.locked_layer_on(&m.name)
                        && matches!(self.extend_layer_sources.get(k), Some(Source::Path(_)))
                    {
                        self.extend_layer_fit.insert(k, mode.clone());
                    }
                }
            }

            Message::ShaderThumbnailsLoaded(thumbnails) => {
                self.shader_thumbnails_loading = false;
                for (idx, handle) in thumbnails {
                    if let Some(handle) = handle {
                        self.set_shader_thumbnail(idx, handle);
                    }
                }
            }

            Message::PreviewTick => {
                // Render the next frame of the currently-selected shader, unless
                // a frame is already in flight or the thumbnail grid is still
                // rendering (avoid concurrent wgpu device creation).
                if self.live_preview_in_flight || self.shader_thumbnails_loading {
                    return Task::none();
                }
                let Choice::Shader(idx) = self.selection.active else {
                    return Task::none();
                };
                let Some(code) = self.preview_shader_code(idx) else {
                    return Task::none();
                };

                self.live_preview_in_flight = true;
                let live_preview = self.live_preview.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let mut lp = live_preview.lock().ok()?;

                            // (Re)build the renderer when the source changes
                            // (shader switch or parameter edit).
                            if lp.renderer.is_none() || lp.source != code {
                                lp.source = code.clone();
                                lp.renderer = match crate::widgets::shader_preview::
                                    ShaderPreviewRenderer::from_code(
                                        &code,
                                        LIVE_PREVIEW_WIDTH,
                                        LIVE_PREVIEW_HEIGHT,
                                    ) {
                                    Ok(r) => Some(r),
                                    Err(e) => {
                                        tracing::debug!(?e, "live preview build failed");
                                        None
                                    }
                                };
                            }

                            let (w, h, rgba) = lp.renderer.as_ref()?.render_frame().ok()?;
                            Some(ImageHandle::from_rgba(w, h, rgba))
                        })
                        .await
                        .ok()
                        .flatten()
                    },
                    move |handle| cosmic::Action::App(Message::PreviewFrame(idx, handle)),
                );
            }

            Message::PreviewFrame(idx, handle) => {
                self.live_preview_in_flight = false;
                // Ignore stale frames if the selection changed while rendering.
                // Only update the canvas layers, not the fixed-size grid thumbnail.
                if let Some(handle) = handle
                    && self.selection.active == Choice::Shader(idx)
                {
                    self.update_shader_canvas_layers(idx, handle);
                }
            }

            // Frame rate, quality and parameters are preview-only until Apply:
            // writing the config here would make the daemon rebuild the
            // wallpaper on every change.
            Message::ShaderFrameRate(idx) => self.selected_shader_frame_rate = idx,
            Message::ShaderRenderScale(idx) => self.selected_shader_render_scale = idx,
            Message::ShaderParamChanged(shader_idx, param_name, value) => {
                self.shader_param_values
                    .entry(shader_idx)
                    .or_default()
                    .insert(param_name, value);
            }
            Message::ShaderParamReleased => {}
            Message::ResetShaderParams(shader_idx) => {
                self.shader_param_values.remove(&shader_idx);
            }

            Message::WallpaperEvent(event) => match event {
                WallpaperEvent::Loading => {
                    self.selection.paths.clear();
                    self.selection.display_images.clear();
                    self.selection.selection_handles.clear();
                }
                WallpaperEvent::Load {
                    path,
                    display,
                    selection,
                } => {
                    // Staged layers restored from config get their picture once
                    // the library has loaded it.
                    let handle =
                        ImageHandle::from_rgba(display.width(), display.height(), display.to_vec());
                    for layer in self.extend_layers.values_mut() {
                        if layer.image_handle.is_none() && layer.source_path == path {
                            layer.image_handle = Some(handle.clone());
                        }
                    }
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
                    // Point the active choice at the applied image, if any.
                    if let Some(Source::Path(config_path)) =
                        self.applied_source(&self.first_connector())
                        && let Some((key, _)) = self
                            .selection
                            .paths
                            .iter()
                            .find(|(_, p)| **p == config_path)
                    {
                        self.selection.active = Choice::Wallpaper(key);
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
                if changed && let Some(ctx) = &self.config_context {
                    let _ = ctx
                        .0
                        .set("wallpaper-sources", self.wallpaper_sources.clone());
                }
            }

            Message::ToggleContextPage(page) => {
                if self.context_page == page {
                    self.set_show_context(!self.core.window.show_context);
                } else {
                    self.context_page = page;
                    self.set_show_context(true);
                }
            }

            Message::OpenUrl(url) => {
                let _ = open::that_detached(&url);
            }

            Message::PreferLowPower(value) => {
                self.prefer_low_power = value;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_prefer_low_power(value);
                }
            }

            Message::ConfigOrStateChanged(maybe_config) => {
                if let Some(config) = maybe_config
                    && self.config != config
                {
                    tracing::debug!("Config changed externally, updating data");
                    // Our own writes update `self.config` first and compare equal
                    // here, so a difference means another process changed the
                    // applied wallpapers (the daemon restoring a display-set
                    // profile). Show that once the monitors are refreshed below.
                    self.config = config;
                    self.restage_on_monitors = true;

                    if let Some(ctx) = &self.config_context {
                        self.prefer_low_power = ctx.prefer_low_power();
                    }
                }

                // Outputs or their arrangement may have changed; refresh the canvas.
                return Task::perform(crate::monitor_query::query_monitors(), |result| {
                    cosmic::Action::App(Message::MonitorsLoaded(result.unwrap_or_default()))
                });
            }

            Message::SetGlowBerryDefault(enable) => {
                return Task::perform(
                    async move { set_glowberry_default(enable).await },
                    |result| cosmic::Action::App(Message::SetGlowBerryDefaultResult(result)),
                );
            }

            Message::SetGlowBerryDefaultResult(result) => match result {
                Ok(is_default) => {
                    self.glowberry_is_default = is_default;
                    tracing::info!(
                        "GlowBerry is now {}",
                        if is_default { "enabled" } else { "disabled" }
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to set GlowBerry default: {}", e);
                    self.glowberry_is_default = is_glowberry_default();
                }
            },

            // Power saving messages
            Message::SetOnBatteryAction(idx) => {
                self.selected_on_battery_action = idx;
                let action = match idx {
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
                self.window_opacity = value.clamp(0.0, 1.0);
            }

            Message::WindowOpacityReleased => {
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_window_opacity(self.window_opacity);
                }
            }

            Message::MonitorsLoaded(monitors) => {
                if monitors.is_empty() && !self.monitor_geometry.is_empty() {
                    // cosmic-randr failed; keep what we have rather than wiping the canvas.
                    return Task::none();
                }
                let names = |m: &[MonitorGeometry]| {
                    let mut v: Vec<&str> = m.iter().map(|m| m.name.as_str()).collect();
                    v.sort_unstable();
                    v.iter().map(ToString::to_string).collect::<Vec<_>>()
                };
                let set_changed = names(&self.monitor_geometry) != names(&monitors);
                self.monitor_geometry = monitors;
                let connected = self.monitor_geometry.clone();
                self.selected_displays
                    .retain(|n| connected.iter().any(|m| m.name == *n));
                // Stage the applied content for this display set on first load and
                // whenever the set of connected outputs changes. A pure
                // rearrangement only moves the displays under the existing layers.
                if self.extend_layers.is_empty() || set_changed || self.restage_on_monitors {
                    self.restage_on_monitors = false;
                    // Identity-keyed entries can only be resolved once the
                    // monitors are known.
                    self.init_from_config();
                    let keys = self.display_keys();
                    if let Some(ctx) = &self.config_context {
                        let layers = ExtendConfig::load_for_displays(ctx, &keys);
                        if !layers.is_empty() {
                            self.extend_config.layers = layers;
                        }
                    }
                    self.restage_from_config();
                    self.extend_fit_view_requested = true;
                }
            }

            Message::ExtendRemoveLayer(key) => {
                self.canvas_menu = None;
                self.remove_layer(key);
                self.renormalize_z_indices();
            }

            Message::ExtendLayerMoved(key, x, y) => {
                self.canvas_menu = None;
                if let Some(layer) = self.extend_layers.get_mut(key) {
                    layer.offset = (x, y);
                }
            }

            Message::ExtendLayerScaled(key, scale) => {
                self.canvas_menu = None;
                if let Some(layer) = self.extend_layers.get_mut(key) {
                    layer.scale = scale;
                }
            }

            Message::ExtendLayerSelected(maybe_key) => {
                self.extend_selected_layer = maybe_key;
                self.canvas_menu = None;
            }

            Message::ExtendLayerUp => {
                if let Some(key) = self.extend_selected_layer {
                    self.swap_z(key, true);
                }
            }

            Message::ExtendLayerDown => {
                if let Some(key) = self.extend_selected_layer {
                    self.swap_z(key, false);
                }
            }

            Message::ExtendCenter => {
                if let Some(key) = self.extend_selected_layer {
                    let mut layer = self.extend_layers[key].clone();
                    fit_layer_to(&mut layer, &self.monitor_geometry);
                    self.extend_layers[key] = layer;
                }
            }

            Message::ExtendFitView => {
                self.extend_fit_view_requested = true;
            }

            Message::ExtendClearAll => {
                self.extend_layers.clear();
                self.extend_layer_colors.clear();
                self.extend_layer_sources.clear();
                self.extend_layer_fit.clear();
                self.extend_selected_layer = None;
                self.extend_next_z = 0;
            }

            Message::ExtendLayerRightClick(key, x, y) => {
                if self.extend_layers.get(key).is_some_and(|l| !l.locked) {
                    self.extend_selected_layer = Some(key);
                }
                self.canvas_menu = Some((CanvasMenu::Layer(key), (x, y)));
            }

            Message::ExtendLayerMenuClose => {
                self.canvas_menu = None;
            }

            Message::ExtendLayerBringForward(key) => {
                self.canvas_menu = None;
                self.swap_z(key, true);
            }

            Message::ExtendLayerSendBack(key) => {
                self.canvas_menu = None;
                self.swap_z(key, false);
            }

            Message::Apply => return self.apply(),

            Message::Applied(result) => match result {
                Ok(crops) => {
                    for (output_name, cached_path) in crops {
                        let entry =
                            Entry::new(self.output_key(&output_name), Source::Path(cached_path));
                        self.set_entry(entry);
                    }
                    tracing::info!("Multi-monitor wallpapers applied");
                }
                Err(e) => {
                    tracing::error!("Failed to composite wallpapers: {}", e);
                }
            },

            Message::Revert => {
                self.init_from_config();
                self.restage_from_config();
                self.extend_fit_view_requested = true;
            }
        }

        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let condensed = self.condensed();

        // Height left for the canvas and the library once the header bar,
        // footer and padding are taken. Unknown until the first resize.
        let content_h = if self.window_height > 0.0 {
            (self.window_height - 46.0 - 57.0 - 40.0).max(0.0)
        } else {
            600.0
        };
        // The canvas takes 40% but never less than a legible minimum.
        let canvas_h = (content_h * 0.4).max(160.0);
        // Toolbar plus one row of cards is the least the library needs; below
        // that the whole column scrolls instead of squeezing.
        let toolbar_h = if condensed { 88.0 } else { 44.0 };
        let fits = content_h >= canvas_h + 16.0 + toolbar_h + 12.0 + 170.0;

        let column = widget::column::with_children(vec![
            container(self.view_canvas())
                .width(Length::Fill)
                .height(Length::Fixed(canvas_h))
                .into(),
            container(self.view_library(fits))
                .width(Length::Fill)
                .height(if fits { Length::Fill } else { Length::Shrink })
                .into(),
        ])
        .spacing(16)
        .padding(20)
        .width(Length::Fill);
        let main: Element<'_, Message> = if fits {
            column.height(Length::Fill).into()
        } else {
            widget::scrollable(column)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        let panel = container(
            widget::column::with_children(vec![
                text::title4(self.inspector_title()).into(),
                widget::scrollable(self.view_inspector())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            ])
            .spacing(12)
            .padding(20),
        )
        .width(if condensed {
            Length::Fill
        } else {
            Length::Fixed(360.0)
        })
        .height(Length::Fill);

        // On a narrow window the inspector is toggled from the header and
        // takes the whole window, like the nav bar does.
        let content: Element<'_, Message> = if condensed && self.inspector_open {
            panel.into()
        } else if condensed {
            main
        } else {
            widget::row::with_children(vec![
                main,
                widget::divider::vertical::default().into(),
                panel.into(),
            ])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        };

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .class(self.window_bg())
            .into()
    }

    fn footer(&self) -> Option<Element<'_, Self::Message>> {
        let dirty = self.dirty_displays();
        let status = if self.extend_layers.is_empty() {
            fl!("hint-empty")
        } else if dirty.is_empty() {
            fl!("status-applied")
        } else {
            fl!("status-changed", n = (dirty.len() as u64))
        };

        let mut revert = button::text(fl!("revert"));
        let mut apply = button::text(fl!("apply")).class(cosmic::theme::Button::Suggested);
        if !dirty.is_empty() {
            revert = revert.on_press(Message::Revert);
            apply = apply.on_press(Message::Apply);
        }

        let bar = widget::row::with_children(vec![
            text::body(status).into(),
            widget::Space::new().width(Length::Fill).into(),
            revert.into(),
            apply.into(),
        ])
        .spacing(8)
        .align_y(Alignment::Center);

        Some(
            container(widget::column::with_children(vec![
                widget::divider::horizontal::default().into(),
                container(bar).padding([12, 20]).into(),
            ]))
            .width(Length::Fill)
            .class(self.window_bg())
            .into(),
        )
    }

    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        let mut items: Vec<Element<'_, Self::Message>> = Vec::with_capacity(3);
        if self.condensed() {
            items.push(
                widget::nav_bar_toggle()
                    .active(self.inspector_open)
                    .on_toggle(Message::ToggleInspector)
                    .into(),
            );
        }
        items.extend([
            widget::button::icon(widget::icon::from_name("preferences-system-symbolic"))
                .on_press(Message::ToggleContextPage(ContextPage::Settings))
                .into(),
            widget::button::icon(widget::icon::from_name("help-about-symbolic"))
                .on_press(Message::ToggleContextPage(ContextPage::About))
                .into(),
        ]);
        items
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
        // Transparent window surface; the content paints its own background
        // with the configured opacity.
        let theme = cosmic::theme::active();
        let cosmic_theme = theme.cosmic();

        Some(cosmic::iced::theme::Style {
            background_color: cosmic::iced::Color::TRANSPARENT,
            text_color: cosmic_theme.on_bg_color().into(),
            icon_color: cosmic_theme.on_bg_color().into(),
        })
    }
}

// ---------------------------------------------------------------------------
// Display targeting and staging
// ---------------------------------------------------------------------------

impl GlowBerrySettings {
    /// Too narrow for the canvas, the library and the inspector side by side.
    fn condensed(&self) -> bool {
        self.window_width > 0.0 && self.window_width < 960.0
    }

    fn placement(&self) -> Placement {
        self.placement_model
            .active_data::<Placement>()
            .copied()
            .unwrap_or(Placement::Each)
    }

    /// The displays the next pick lands on: the selected ones, or all.
    fn target_monitors(&self) -> Vec<MonitorGeometry> {
        let selected: Vec<MonitorGeometry> = self
            .monitor_geometry
            .iter()
            .filter(|m| self.selected_displays.contains(&m.name))
            .cloned()
            .collect();
        if selected.is_empty() {
            self.monitor_geometry.clone()
        } else {
            selected
        }
    }

    /// Connector of the first display, for choosing what the preview follows.
    fn first_connector(&self) -> String {
        self.monitor_geometry
            .first()
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "all".to_string())
    }

    fn next_z(&mut self) -> usize {
        let z = self.extend_next_z;
        self.extend_next_z += 1;
        z
    }

    /// Put a library item on `targets`.
    fn pick(&mut self, item: Item, targets: &[MonitorGeometry], placement: Placement) {
        match item {
            Item::Image(key) => self.pick_image(key, targets, placement),
            Item::Color(idx) => {
                if let Some(color) = DEFAULT_COLORS.get(idx).cloned() {
                    self.pick_color(color, targets);
                }
            }
            Item::Shader(idx) => self.pick_shader(idx, targets),
        }
    }

    fn pick_image(&mut self, key: DefaultKey, targets: &[MonitorGeometry], placement: Placement) {
        let Some(path) = self.selection.paths.get(key).cloned() else {
            return;
        };
        if targets.is_empty() {
            return;
        }
        let handle = self
            .selection
            .display_images
            .get(key)
            .map(|img| ImageHandle::from_rgba(img.width(), img.height(), img.to_vec()));
        let size = image::image_dimensions(&path).unwrap_or((800, 600));
        self.selection.active = Choice::Wallpaper(key);

        for m in targets {
            self.remove_locked_on(&m.name);
        }
        if placement == Placement::Span {
            let z = self.next_z();
            let mut layer = ExtendLayerState {
                source_path: path,
                image_handle: handle,
                image_size: size,
                offset: (0.0, 0.0),
                scale: 1.0,
                z_index: z,
                locked: false,
                target_output: None,
            };
            fit_layer_to(&mut layer, targets);
            let k = self.extend_layers.insert(layer);
            self.extend_selected_layer = Some(k);
        } else {
            for m in targets {
                self.insert_locked_item(
                    m,
                    None,
                    Source::Path(path.clone()),
                    handle.clone(),
                    size,
                    path.clone(),
                );
            }
            self.extend_selected_layer = None;
        }
        self.after_pick();
    }

    fn pick_color(&mut self, color: Color, targets: &[MonitorGeometry]) {
        self.selection.active = Choice::Color(color.clone());
        for m in targets {
            self.remove_locked_on(&m.name);
            self.insert_locked_item(
                m,
                Some(color.clone()),
                Source::Color(color.clone()),
                None,
                (0, 0),
                PathBuf::new(),
            );
        }
        self.extend_selected_layer = None;
        self.after_pick();
    }

    fn pick_shader(&mut self, idx: usize, targets: &[MonitorGeometry]) {
        if idx >= self.available_shaders.len() {
            return;
        }
        self.selection.active = Choice::Shader(idx);
        let Some(source) = self.build_active_source() else {
            return;
        };
        let handle = self.shader_thumbnails.get(idx).cloned();
        for m in targets {
            self.remove_locked_on(&m.name);
            self.insert_locked_item(
                m,
                None,
                source.clone(),
                handle.clone(),
                LIVE_ITEM_SIZE,
                PathBuf::new(),
            );
        }
        self.extend_selected_layer = None;
        self.after_pick();
    }

    /// Put what one display shows on every display, side by side or spanned.
    fn spread_display(&mut self, connector: &str, placement: Placement) {
        let Some(m) = self
            .monitor_geometry
            .iter()
            .find(|m| m.name == connector)
            .cloned()
        else {
            return;
        };
        let all = self.monitor_geometry.clone();
        match self.shown_on(&m) {
            Shown::Image { path, .. } => {
                let key = self
                    .selection
                    .paths
                    .iter()
                    .find(|(_, p)| **p == path)
                    .map(|(k, _)| k);
                if let Some(key) = key {
                    self.pick_image(key, &all, placement);
                }
            }
            Shown::Color(c) => self.pick_color(c, &all),
            Shown::Shader(idx) => self.pick_shader(idx, &all),
            Shown::Nothing => {}
        }
    }

    fn after_pick(&mut self) {
        self.prune_hidden_free_layers();
        self.renormalize_z_indices();
        self.canvas_menu = None;
    }

    /// Insert one locked item filling `monitor` (colors fill exactly;
    /// images/shaders cover-fit). Records its color/source.
    fn insert_locked_item(
        &mut self,
        monitor: &MonitorGeometry,
        color: Option<Color>,
        source: Source,
        image_handle: Option<ImageHandle>,
        image_size: (u32, u32),
        source_path: PathBuf,
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
        let z = self.next_z();
        let key = self.extend_layers.insert(ExtendLayerState {
            source_path,
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

    fn remove_layer(&mut self, key: DefaultKey) {
        self.extend_layers.remove(key);
        self.extend_layer_colors.remove(key);
        self.extend_layer_sources.remove(key);
        self.extend_layer_fit.remove(key);
        if self.extend_selected_layer == Some(key) {
            self.extend_selected_layer = None;
        }
    }

    /// Remove the locked item on a display, if any.
    fn remove_locked_on(&mut self, connector: &str) {
        let keys: Vec<DefaultKey> = self
            .extend_layers
            .iter()
            .filter(|(_, l)| l.locked && l.target_output.as_deref() == Some(connector))
            .map(|(k, _)| k)
            .collect();
        for k in keys {
            self.remove_layer(k);
        }
    }

    /// Drop free layers that no display can see any more: every display they
    /// cover has a locked item or a higher free layer on top.
    fn prune_hidden_free_layers(&mut self) {
        let free: Vec<(DefaultKey, usize)> = self
            .extend_layers
            .iter()
            .filter(|(_, l)| !l.locked)
            .map(|(k, l)| (k, l.z_index))
            .collect();
        for (key, z) in free {
            let covered: Vec<MonitorGeometry> = self
                .monitor_geometry
                .iter()
                .filter(|m| layer_covers(&self.extend_layers[key], m))
                .cloned()
                .collect();
            if covered.is_empty() {
                continue;
            }
            let hidden = covered.iter().all(|m| {
                self.locked_layer_on(&m.name).is_some()
                    || self.extend_layers.iter().any(|(k2, l2)| {
                        k2 != key && !l2.locked && l2.z_index > z && layer_covers(l2, m)
                    })
            });
            if hidden {
                self.remove_layer(key);
            }
        }
    }

    /// The locked item filling a display, if any.
    fn locked_layer_on(&self, connector: &str) -> Option<DefaultKey> {
        self.extend_layers
            .iter()
            .find(|(_, l)| l.locked && l.target_output.as_deref() == Some(connector))
            .map(|(k, _)| k)
    }

    /// The topmost free layer covering a display's center, if any.
    fn free_layer_on(&self, monitor: &MonitorGeometry) -> Option<DefaultKey> {
        self.extend_layers
            .iter()
            .filter(|(_, l)| !l.locked && layer_covers(l, monitor))
            .max_by_key(|(_, l)| l.z_index)
            .map(|(k, _)| k)
    }

    fn shown_on(&self, monitor: &MonitorGeometry) -> Shown {
        if let Some(k) = self.locked_layer_on(&monitor.name) {
            if let Some(c) = self.extend_layer_colors.get(k) {
                return Shown::Color(c.clone());
            }
            return match self.extend_layer_sources.get(k) {
                Some(src @ Source::Shader(_)) => self
                    .shader_idx_for_source(src)
                    .map_or(Shown::Nothing, Shown::Shader),
                Some(Source::Path(p)) => Shown::Image {
                    path: p.clone(),
                    span: None,
                },
                _ => Shown::Nothing,
            };
        }
        if let Some(k) = self.free_layer_on(monitor) {
            return Shown::Image {
                path: self.extend_layers[k].source_path.clone(),
                span: Some(k),
            };
        }
        Shown::Nothing
    }

    /// What all `targets` show, or `None` when they differ.
    fn shared_shown(&self, targets: &[MonitorGeometry]) -> Option<Shown> {
        let mut iter = targets.iter().map(|m| self.shown_on(m));
        let first = iter.next()?;
        iter.all(|s| s == first).then_some(first)
    }

    /// Point the live preview and the parameter editor at what the selected
    /// displays show.
    fn sync_choice_to_targets(&mut self) {
        let targets = self.target_monitors();
        match self.shared_shown(&targets) {
            Some(Shown::Shader(idx)) => {
                self.selection.active = Choice::Shader(idx);
                let staged = targets
                    .first()
                    .and_then(|m| self.locked_layer_on(&m.name))
                    .and_then(|k| self.extend_layer_sources.get(k).cloned());
                if let Some(Source::Shader(ss)) = staged {
                    self.sync_shader_dropdowns(&ss);
                }
            }
            Some(Shown::Color(c)) => self.selection.active = Choice::Color(c),
            Some(Shown::Image { path, .. }) => {
                let key = self
                    .selection
                    .paths
                    .iter()
                    .find(|(_, p)| **p == path)
                    .map(|(k, _)| k);
                if let Some(key) = key {
                    self.selection.active = Choice::Wallpaper(key);
                }
            }
            _ => {}
        }
    }

    /// Displays where a library item is staged, as a short badge label.
    fn staged_badge(&self, is_item: impl Fn(&Shown) -> bool) -> Option<String> {
        let on: Vec<&str> = self
            .monitor_geometry
            .iter()
            .filter(|m| is_item(&self.shown_on(m)))
            .map(|m| m.name.as_str())
            .collect();
        let total = self.monitor_geometry.len();
        match on.len() {
            0 => None,
            1 => Some(on[0].to_string()),
            n if n == total => Some(fl!("badge-all")),
            n => Some(fl!("badge-some", n = (n as u64), total = (total as u64))),
        }
    }

    /// Move a free layer one step up (`forward`) or down in z-order.
    fn swap_z(&mut self, key: DefaultKey, forward: bool) {
        let Some(sel_z) = self.extend_layers.get(key).map(|l| l.z_index) else {
            return;
        };
        let candidates = self
            .extend_layers
            .iter()
            .filter(|(k, l)| *k != key && !l.locked)
            .filter(|(_, l)| {
                if forward {
                    l.z_index > sel_z
                } else {
                    l.z_index < sel_z
                }
            })
            .map(|(k, l)| (k, l.z_index));
        let swap = if forward {
            candidates.min_by_key(|(_, z)| *z)
        } else {
            candidates.max_by_key(|(_, z)| *z)
        };
        if let Some((swap_key, swap_z)) = swap {
            self.extend_layers[key].z_index = swap_z;
            self.extend_layers[swap_key].z_index = sel_z;
        }
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

    // ------------------------------------------------------------------
    // Applied state, dirty tracking, apply and revert
    // ------------------------------------------------------------------

    /// The entry currently applied to a connector, honouring same-on-all.
    fn applied_entry(&self, connector: &str) -> Option<&Entry> {
        if self.config.same_on_all {
            Some(&self.config.default_background)
        } else {
            self.entry_for_output(connector)
        }
    }

    /// The source currently applied to a connector, honouring same-on-all.
    fn applied_source(&self, connector: &str) -> Option<Source> {
        self.applied_entry(connector).map(|e| e.source.clone())
    }

    /// Free layers as comparable rows: (path, x, y, scale in 1/1000ths, z).
    fn free_layers_snapshot(&self) -> Vec<(PathBuf, i64, i64, i64, usize)> {
        let mut rows: Vec<_> = self
            .extend_layers
            .values()
            .filter(|l| !l.locked)
            .map(|l| {
                (
                    l.source_path.clone(),
                    l.offset.0.round() as i64,
                    l.offset.1.round() as i64,
                    (l.scale * 1000.0).round() as i64,
                    l.z_index,
                )
            })
            .collect();
        rows.sort();
        rows
    }

    fn saved_free_layers_snapshot(&self) -> Vec<(PathBuf, i64, i64, i64, usize)> {
        let mut rows: Vec<_> = self
            .extend_config
            .layers
            .iter()
            .filter(|l| !l.locked)
            .map(|l| {
                (
                    l.source_path.clone(),
                    l.img_offset_x.round() as i64,
                    l.img_offset_y.round() as i64,
                    (l.img_scale * 1000.0).round() as i64,
                    l.z_index,
                )
            })
            .collect();
        rows.sort();
        rows
    }

    /// The config source a locked item would be written as right now.
    fn staged_source(&self, key: DefaultKey) -> Source {
        let source = self
            .extend_layer_sources
            .get(key)
            .cloned()
            .unwrap_or_else(|| Source::Path(self.extend_layers[key].source_path.clone()));
        self.refresh_shader_source(source)
    }

    /// Displays whose staged content differs from what is applied.
    fn dirty_displays(&self) -> Vec<String> {
        let free_dirty = self.free_layers_snapshot() != self.saved_free_layers_snapshot();
        let cache = glowberry_lib::extend_crop::cache_dir();
        self.monitor_geometry
            .iter()
            .filter(|m| {
                if let Some(k) = self.locked_layer_on(&m.name) {
                    let staged = self.staged_source(k);
                    let fit = self.extend_layer_fit.get(k).cloned().unwrap_or_default();
                    match self.applied_entry(&m.name) {
                        Some(e) => e.source != staged || e.scaling_mode != fit,
                        None => true,
                    }
                } else if self.free_layer_on(m).is_some() {
                    let applied_is_crop = matches!(
                        self.applied_source(&m.name),
                        Some(Source::Path(p)) if p.starts_with(&cache)
                    );
                    free_dirty || !applied_is_crop
                } else {
                    false
                }
            })
            .map(|m| m.name.clone())
            .collect()
    }

    /// Write the staged canvas to config: locked items become per-display
    /// entries (or one `all` entry when every display shows the same thing),
    /// free layers are saved and composited per display.
    fn apply(&mut self) -> Task<Message> {
        let monitors = self.monitor_geometry.clone();
        if monitors.is_empty() {
            return Task::none();
        }

        let mut locked: Vec<(String, Source, ScalingMode)> = Vec::new();
        for m in &monitors {
            if let Some(k) = self.locked_layer_on(&m.name) {
                let source = self.staged_source(k);
                let fit = self.extend_layer_fit.get(k).cloned().unwrap_or_default();
                locked.push((m.name.clone(), source, fit));
            }
        }

        // Same on all displays is derived, not a switch.
        let all_same = locked.len() == monitors.len()
            && locked
                .iter()
                .all(|(_, s, f)| (s, f) == (&locked[0].1, &locked[0].2));
        if all_same {
            let (_, source, fit) = locked[0].clone();
            self.set_same_on_all(true);
            self.set_entry(Entry::new("all".to_string(), source).scaling_mode(fit));
        } else {
            self.set_same_on_all(false);
            for (connector, source, fit) in locked.iter().cloned() {
                let key = self.output_key(&connector);
                self.set_entry(Entry::new(key, source).scaling_mode(fit));
            }
        }

        // Free layers: remember them, and composite a crop for every display
        // they cover that has no locked item.
        self.extend_config.layers = self
            .extend_layers
            .values()
            .filter(|l| !l.locked)
            .map(|l| glowberry_config::extend::ExtendLayer {
                source_path: l.source_path.clone(),
                img_offset_x: l.offset.0,
                img_offset_y: l.offset.1,
                img_scale: l.scale,
                z_index: l.z_index,
                locked: false,
                target_output: None,
            })
            .collect();
        let keys = self.display_keys();
        if let Some(ctx) = &self.config_context {
            let _ = ctx.save_extend_config(&self.extend_config);
            let _ = ExtendConfig::save_for_displays(ctx, &keys, &self.extend_config.layers);
        }

        let mut layer_infos: Vec<glowberry_lib::extend_crop::LayerInfo> = self
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
        let to_composite: Vec<glowberry_lib::extend_crop::MonitorInfo> = monitors
            .iter()
            .filter(|m| self.locked_layer_on(&m.name).is_none() && self.free_layer_on(m).is_some())
            .map(|m| glowberry_lib::extend_crop::MonitorInfo {
                name: m.name.clone(),
                position: m.position,
                logical_size: m.logical_size,
                physical_size: m.physical_size,
                scale: m.scale,
            })
            .collect();
        if layer_infos.is_empty() || to_composite.is_empty() {
            return Task::none();
        }
        let cache_dir = glowberry_lib::extend_crop::cache_dir();
        Task::perform(
            async move {
                glowberry_lib::extend_crop::composite_for_monitors(
                    &mut layer_infos,
                    &to_composite,
                    &cache_dir,
                )
                .map_err(|e| e.to_string())
            },
            |result| cosmic::Action::App(Message::Applied(result)),
        )
    }

    /// Rebuild the canvas from what is applied: saved free layers plus one
    /// locked item per display for its applied entry. Composited crops are
    /// skipped since the free layers already cover those displays.
    fn restage_from_config(&mut self) {
        self.extend_layers.clear();
        self.extend_layer_colors.clear();
        self.extend_layer_sources.clear();
        self.extend_layer_fit.clear();
        self.extend_selected_layer = None;
        self.canvas_menu = None;
        self.extend_next_z = 0;

        for saved in self.extend_config.layers.clone() {
            if saved.locked {
                continue;
            }
            let image_size = image::image_dimensions(&saved.source_path).unwrap_or((800, 600));
            let image_handle = self.library_handle(&saved.source_path);
            let z = self.next_z().max(saved.z_index);
            self.extend_next_z = self.extend_next_z.max(z + 1);
            self.extend_layers.insert(ExtendLayerState {
                source_path: saved.source_path,
                image_handle,
                image_size,
                offset: (saved.img_offset_x, saved.img_offset_y),
                scale: saved.img_scale,
                z_index: z,
                locked: false,
                target_output: None,
            });
        }

        let cache = glowberry_lib::extend_crop::cache_dir();
        for m in self.monitor_geometry.clone() {
            let Some(entry) = self.applied_entry(&m.name).cloned() else {
                continue;
            };
            let fit = entry.scaling_mode.clone();
            match entry.source {
                Source::Color(c) => {
                    self.insert_locked_item(
                        &m,
                        Some(c.clone()),
                        Source::Color(c),
                        None,
                        (0, 0),
                        PathBuf::new(),
                    );
                }
                src @ Source::Shader(_) => {
                    let handle = self
                        .shader_idx_for_source(&src)
                        .and_then(|i| self.shader_thumbnails.get(i).cloned());
                    self.insert_locked_item(&m, None, src, handle, LIVE_ITEM_SIZE, PathBuf::new());
                }
                Source::Path(p) if !p.starts_with(&cache) && p.is_file() => {
                    let size = image::image_dimensions(&p).unwrap_or((800, 600));
                    let handle = self.library_handle(&p);
                    let k =
                        self.insert_locked_item(&m, None, Source::Path(p.clone()), handle, size, p);
                    self.extend_layer_fit.insert(k, fit);
                }
                _ => {}
            }
        }
        self.renormalize_z_indices();
    }

    /// The library's display image for a path, if loaded. Layers without one
    /// get it when the wallpaper subscription delivers it.
    fn library_handle(&self, path: &Path) -> Option<ImageHandle> {
        let (key, _) = self
            .selection
            .paths
            .iter()
            .find(|(_, p)| p.as_path() == path)?;
        self.selection
            .display_images
            .get(key)
            .map(|img| ImageHandle::from_rgba(img.width(), img.height(), img.to_vec()))
    }

    /// Point the active choice at the applied wallpaper so the live preview and
    /// the parameter editor start from what the daemon is showing.
    fn init_from_config(&mut self) {
        let entry = if self.config.same_on_all {
            self.config.default_background.clone()
        } else if let Some(first) = self.config.backgrounds.first() {
            first.clone()
        } else {
            self.config.default_background.clone()
        };
        self.select_entry_source(&entry.source);
    }

    /// Build the config `Source` for the active choice (image path, color,
    /// or live shader), or `None` if it can't be resolved.
    fn build_active_source(&self) -> Option<Source> {
        let source = match &self.selection.active {
            Choice::Wallpaper(key) => Source::Path(self.selection.paths.get(*key)?.clone()),
            Choice::Color(color) => Source::Color(color.clone()),
            Choice::Shader(idx) => {
                let shader = self.available_shaders.get(*idx)?;
                let frame_rate = self.current_frame_rate();
                let render_scale = self.current_render_scale();

                let (shader_content, source_path, params) = if let Some(parsed) = &shader.parsed {
                    let values = self
                        .shader_param_values
                        .get(idx)
                        .cloned()
                        .unwrap_or_default();
                    let params: HashMap<String, f64> = values
                        .iter()
                        .map(|(k, v)| (k.clone(), v.as_f32() as f64))
                        .collect();
                    if values.is_empty() {
                        (
                            glowberry_config::ShaderContent::Path(shader.path.clone()),
                            None,
                            params,
                        )
                    } else {
                        (
                            glowberry_config::ShaderContent::Code(parsed.generate_source(&values)),
                            Some(shader.path.clone()),
                            params,
                        )
                    }
                } else {
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
                    render_scale,
                })
            }
        };
        Some(source)
    }

    /// Sync the frame-rate and render-quality dropdowns to a shader source's
    /// stored settings.
    fn sync_shader_dropdowns(&mut self, ss: &glowberry_config::ShaderSource) {
        self.selected_shader_frame_rate = match ss.frame_rate {
            0..=22 => 0,
            23..=45 => 1,
            _ => 2,
        };
        self.selected_shader_render_scale = if ss.render_scale <= 0.375 {
            2 // Quarter
        } else if ss.render_scale <= 0.75 {
            1 // Half
        } else {
            0 // Full
        };
    }

    /// Frame rate from the current dropdown selection.
    fn current_frame_rate(&self) -> u8 {
        match self.selected_shader_frame_rate {
            0 => 15,
            2 => 60,
            _ => 30,
        }
    }

    /// Render scale from the current dropdown selection.
    fn current_render_scale(&self) -> f32 {
        match self.selected_shader_render_scale {
            1 => 0.5,
            2 => 0.25,
            _ => 1.0,
        }
    }

    /// Rebuild a shader `Source` from the current in-memory settings.
    ///
    /// Staged sources are captured at pick time, before the user edits
    /// anything. Shader settings are preview-only until Apply, so this
    /// refreshes frame rate, render quality, and parameter values
    /// (regenerating the inline shader code, or falling back to a plain path
    /// when no custom values exist). Non-shader sources are returned unchanged.
    fn refresh_shader_source(&self, source: Source) -> Source {
        let Source::Shader(ss) = &source else {
            return source;
        };

        let mut ss = ss.clone();
        ss.frame_rate = self.current_frame_rate();
        ss.render_scale = self.current_render_scale();

        let path = ss.source_path.clone().or_else(|| {
            if let glowberry_config::ShaderContent::Path(p) = &ss.shader {
                Some(p.clone())
            } else {
                None
            }
        });

        if let Some(path) = path
            && let Some(idx) = self.available_shaders.iter().position(|s| s.path == path)
            && let Some(parsed) = &self.available_shaders[idx].parsed
        {
            let shader_path = self.available_shaders[idx].path.clone();
            let values = self
                .shader_param_values
                .get(&idx)
                .cloned()
                .unwrap_or_default();

            ss.params = values
                .iter()
                .map(|(k, v)| (k.clone(), v.as_f32() as f64))
                .collect();

            if values.is_empty() {
                ss.shader = glowberry_config::ShaderContent::Path(shader_path);
                ss.source_path = None;
            } else {
                ss.shader = glowberry_config::ShaderContent::Code(parsed.generate_source(&values));
                ss.source_path = Some(shader_path);
            }
        }

        Source::Shader(ss)
    }

    /// Make the active choice match a config source (on init and revert).
    fn select_entry_source(&mut self, source: &Source) {
        match source {
            Source::Path(path) => {
                let key = self
                    .selection
                    .paths
                    .iter()
                    .find(|(_, p)| *p == path)
                    .map(|(k, _)| k);
                if let Some(key) = key {
                    self.selection.active = Choice::Wallpaper(key);
                }
            }
            Source::Color(color) => {
                self.selection.active = Choice::Color(color.clone());
            }
            Source::Shader(shader_source) => {
                let Some(idx) = self
                    .shader_idx_for_source(source)
                    .or_else(|| (!self.available_shaders.is_empty()).then_some(0))
                else {
                    return;
                };
                self.selection.active = Choice::Shader(idx);

                // Load parameter values from config
                if !shader_source.params.is_empty()
                    && let Some(parsed) = self.available_shaders[idx].parsed.as_ref()
                {
                    let mut param_values: HashMap<String, ParamValue> = HashMap::new();
                    for param in &parsed.params {
                        if let Some(&value) = shader_source.params.get(&param.name) {
                            let param_value = match param.param_type {
                                ParamType::F32 => ParamValue::F32(value as f32),
                                ParamType::I32 => ParamValue::I32(value as i32),
                            };
                            param_values.insert(param.name.clone(), param_value);
                        }
                    }
                    if !param_values.is_empty() {
                        self.shader_param_values.insert(idx, param_values);
                    }
                }

                self.sync_shader_dropdowns(shader_source);
            }
        }
    }

    /// Config key for a connected output: its EDID identity when known and
    /// unique, else the connector name. Two identical monitors without serial
    /// numbers share an identity, so they keep connector keys and can still
    /// hold different content (at the cost of not surviving a re-dock).
    fn output_key(&self, connector: &str) -> String {
        let Some(id) = self
            .monitor_geometry
            .iter()
            .find(|m| m.name == connector)
            .and_then(|m| m.edid.as_deref())
        else {
            return connector.to_string();
        };
        let duplicates = self
            .monitor_geometry
            .iter()
            .filter(|m| m.edid.as_deref() == Some(id))
            .count();
        if duplicates > 1 {
            connector.to_string()
        } else {
            id.to_string()
        }
    }

    /// Output keys for the connected monitors, for extend profile lookup.
    fn display_keys(&self) -> Vec<String> {
        self.monitor_geometry
            .iter()
            .map(|m| self.output_key(&m.name))
            .collect()
    }

    /// Switch between one wallpaper everywhere and per-output wallpapers,
    /// keeping the in-memory config shaped like a fresh `Config::load` so an
    /// external change can be told apart from our own writes.
    fn set_same_on_all(&mut self, value: bool) {
        self.config.same_on_all = value;
        let Some(ctx) = &self.config_context else {
            return;
        };
        if let Err(e) = ctx.set_same_on_all(value) {
            tracing::error!("Failed to set same-on-all: {}", e);
        }
        if value {
            self.config.backgrounds.clear();
            self.config.outputs.clear();
        } else {
            self.config.load_backgrounds(ctx);
        }
    }

    /// Write a wallpaper entry and snapshot the resulting state for the current
    /// display set, so the daemon can bring it back when this set reconnects.
    fn set_entry(&mut self, entry: Entry) {
        let Some(ctx) = &self.config_context else {
            return;
        };
        if let Err(e) = self.config.set_entry(ctx, entry) {
            tracing::error!("Failed to set wallpaper: {}", e);
        }
        self.save_output_profile();
    }

    fn save_output_profile(&self) {
        let Some(ctx) = &self.config_context else {
            return;
        };
        if self.monitor_geometry.is_empty() {
            return;
        }
        let profile = OutputProfile {
            same_on_all: self.config.same_on_all,
            all: self.config.default_background.clone(),
            outputs: self
                .monitor_geometry
                .iter()
                .filter_map(|m| self.entry_for_output(&m.name).cloned())
                .collect(),
        };
        let set_key = glowberry_config::extend::display_key(&self.display_keys());
        if let Err(e) = ctx.save_output_profile(&set_key, &profile) {
            tracing::warn!(?e, "failed to save display-set wallpaper profile");
        }
    }

    /// Per-output entry for a connector: identity key first, then the legacy
    /// connector-named key.
    fn entry_for_output(&self, connector: &str) -> Option<&Entry> {
        self.config
            .entry(&self.output_key(connector))
            .or_else(|| self.config.entry(connector))
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

    /// Store a rendered shader thumbnail and propagate it to any staged canvas
    /// items showing that shader, so the per-output live preview updates too.
    fn set_shader_thumbnail(&mut self, idx: usize, handle: ImageHandle) {
        if idx >= self.shader_thumbnails.len() {
            return;
        }
        self.shader_thumbnails[idx] = handle.clone();
        self.update_shader_canvas_layers(idx, handle);
    }

    /// Push a freshly-rendered frame into every staged canvas layer showing the
    /// given shader, without touching the fixed-size grid thumbnail.
    fn update_shader_canvas_layers(&mut self, idx: usize, handle: ImageHandle) {
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

    /// Build the WGSL source for a shader's live preview, with the user's
    /// current parameter values substituted in (falling back to the raw file
    /// when no custom values exist or the shader has no parsed params).
    fn preview_shader_code(&self, idx: usize) -> Option<String> {
        let shader = self.available_shaders.get(idx)?;
        match &shader.parsed {
            Some(parsed) => {
                let values = self
                    .shader_param_values
                    .get(&idx)
                    .cloned()
                    .unwrap_or_default();
                if values.is_empty() {
                    std::fs::read_to_string(&shader.path).ok()
                } else {
                    Some(parsed.generate_source(&values))
                }
            }
            None => std::fs::read_to_string(&shader.path).ok(),
        }
    }

    fn load_shader_thumbnails(&mut self) -> Task<Message> {
        self.shader_thumbnails_loading = true;
        let shader_paths: Vec<_> = self
            .available_shaders
            .iter()
            .map(|s| s.path.clone())
            .collect();

        // Render previews sequentially in a single blocking task. Each render
        // spins up its own wgpu instance/adapter/device; creating many of them
        // concurrently can crash on software renderers (e.g. llvmpipe).
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let mut thumbnails = Vec::with_capacity(shader_paths.len());
                    for (idx, path) in shader_paths.into_iter().enumerate() {
                        let handle = match crate::widgets::shader_preview::render_shader_preview(
                            &path,
                            THUMB_WIDTH,
                            THUMB_HEIGHT,
                        ) {
                            Ok((width, height, rgba)) => {
                                Some(ImageHandle::from_rgba(width, height, rgba))
                            }
                            Err(e) => {
                                tracing::warn!(?path, ?e, "shader thumbnail render failed");
                                None
                            }
                        };
                        thumbnails.push((idx, handle));
                    }
                    thumbnails
                })
                .await
                .unwrap_or_default()
            },
            |thumbnails| cosmic::Action::App(Message::ShaderThumbnailsLoaded(thumbnails)),
        )
    }
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

impl GlowBerrySettings {
    /// Window background at the configured opacity.
    fn window_bg(&self) -> cosmic::theme::Container<'static> {
        let opacity = self.window_opacity;
        cosmic::theme::Container::custom(move |theme| {
            let cosmic = theme.cosmic();
            let mut bg_color: cosmic::iced::Color =
                cosmic.background(theme.transparent).base.into();
            bg_color.a = opacity;
            cosmic::widget::container::Style {
                background: Some(cosmic::iced::Background::Color(bg_color)),
                icon_color: Some(cosmic.background(theme.transparent).on.into()),
                text_color: Some(cosmic.background(theme.transparent).on.into()),
                border: cosmic::iced::Border::default(),
                shadow: cosmic::iced::Shadow::default(),
                snap: false,
            }
        })
    }

    /// The canvas: connected displays with their staged content.
    fn view_canvas(&self) -> Element<'_, Message> {
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
                selected: !layer.locked && self.extend_selected_layer == Some(key),
                locked: layer.locked,
                target_output: layer.target_output.as_deref(),
                color: self.extend_layer_colors.get(key),
            })
            .collect();
        layer_views.sort_by_key(|l| l.z_index);

        let editor = ExtendEditor::new(
            &self.monitor_geometry,
            layer_views,
            &self.selected_displays,
            Message::ExtendLayerMoved,
            Message::ExtendLayerScaled,
            Message::ExtendLayerSelected,
        )
        .on_display_click(Message::DisplaySelected)
        .on_display_right_click(Message::DisplayRightClick)
        .on_background_click(Message::CanvasBackgroundClicked)
        .on_right_click(Message::ExtendLayerRightClick)
        .fit_requested(self.extend_fit_view_requested);

        let selected_free = self
            .extend_selected_layer
            .filter(|k| self.extend_layers.get(*k).is_some_and(|l| !l.locked));

        // Top-right: selection and z-order tools.
        let mut top_right: Vec<Element<'_, Message>> = Vec::new();
        if !self.selected_displays.is_empty() {
            top_right.push(
                button::text(fl!("select-all-displays"))
                    .on_press(Message::SelectAllDisplays)
                    .into(),
            );
        }
        if selected_free.is_some() {
            top_right.push(with_tip(
                widget::button::icon(widget::icon::from_name("go-up-symbolic"))
                    .on_press(Message::ExtendLayerUp),
                fl!("tip-layer-up"),
            ));
            top_right.push(with_tip(
                widget::button::icon(widget::icon::from_name("go-down-symbolic"))
                    .on_press(Message::ExtendLayerDown),
                fl!("tip-layer-down"),
            ));
            top_right.push(with_tip(
                widget::button::icon(widget::icon::from_name("format-justify-center-symbolic"))
                    .on_press(Message::ExtendCenter),
                fl!("tip-center"),
            ));
        }

        // Bottom-left: delete / clear / fit.
        let mut bottom_left: Vec<Element<'_, Message>> = Vec::new();
        if let Some(key) = selected_free {
            bottom_left.push(with_tip(
                widget::button::icon(widget::icon::from_name("user-trash-symbolic"))
                    .on_press(Message::ExtendRemoveLayer(key))
                    .class(cosmic::theme::Button::Destructive),
                fl!("tip-delete"),
            ));
        } else if !self.extend_layers.is_empty() {
            bottom_left.push(with_tip(
                widget::button::icon(widget::icon::from_name("edit-clear-symbolic"))
                    .on_press(Message::ExtendClearAll)
                    .class(cosmic::theme::Button::Destructive),
                fl!("tip-clear-all"),
            ));
        }
        bottom_left.push(with_tip(
            widget::button::icon(widget::icon::from_name("zoom-fit-best-symbolic"))
                .on_press(Message::ExtendFitView),
            fl!("tip-fit"),
        ));

        let mut stack = cosmic::iced::widget::Stack::new()
            .push(container(editor).width(Length::Fill).height(Length::Fill))
            .push(
                container(widget::column::with_children(bottom_left).spacing(4))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::Start)
                    .align_y(Alignment::End)
                    .padding(6),
            )
            .push(
                container(
                    widget::row::with_children(top_right)
                        .spacing(4)
                        .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::End)
                .align_y(Alignment::Start)
                .padding(6),
            );
        if self.tips_visible() {
            stack = stack.push(
                container(self.view_tips())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::End)
                    .align_y(Alignment::End)
                    .padding(10),
            );
        }
        let canvas: Element<'_, Message> = stack.width(Length::Fill).height(Length::Fill).into();

        // Context menu on a display or on a free layer.
        let mut popover = widget::popover(canvas);
        if let Some((menu, (cx, cy))) = &self.canvas_menu {
            // Flat rows like a real menu; the destructive one only tints its label.
            let entry = |label: String, msg: Message| -> Element<'_, Message> {
                button::custom(text::body(label))
                    .on_press(msg)
                    .width(Length::Fill)
                    .padding([8, 12])
                    .class(cosmic::theme::Button::MenuItem)
                    .into()
            };
            let danger = |label: String, msg: Message| -> Element<'_, Message> {
                let red: cosmic::iced::Color =
                    cosmic::theme::active().cosmic().destructive.base.into();
                button::custom(text::body(label).class(cosmic::theme::Text::Color(red)))
                    .on_press(msg)
                    .width(Length::Fill)
                    .padding([8, 12])
                    .class(cosmic::theme::Button::MenuItem)
                    .into()
            };
            let many = self.monitor_geometry.len() > 1;
            let mut items: Vec<Element<'_, Message>> = Vec::new();
            match menu {
                CanvasMenu::Layer(key) => {
                    items.push(entry(
                        fl!("ctx-bring-forward"),
                        Message::ExtendLayerBringForward(*key),
                    ));
                    items.push(entry(
                        fl!("ctx-send-back"),
                        Message::ExtendLayerSendBack(*key),
                    ));
                    items.push(entry(fl!("ctx-fit"), Message::ExtendLayerFit(*key)));
                    items.push(widget::divider::horizontal::light().into());
                    items.push(danger(fl!("ctx-remove"), Message::ExtendRemoveLayer(*key)));
                }
                CanvasMenu::Display(name) => {
                    let shown = self
                        .monitor_geometry
                        .iter()
                        .find(|m| m.name == *name)
                        .map_or(Shown::Nothing, |m| self.shown_on(m));
                    items.push(entry(
                        fl!("ctx-select"),
                        Message::DisplaySelected(name.clone(), false),
                    ));
                    if many {
                        items.push(entry(
                            fl!("ctx-add-selection"),
                            Message::DisplaySelected(name.clone(), true),
                        ));
                    }
                    if many && shown != Shown::Nothing {
                        items.push(widget::divider::horizontal::light().into());
                        items.push(entry(
                            fl!("ctx-duplicate-all"),
                            Message::DisplayDuplicateAll(name.clone()),
                        ));
                        if matches!(shown, Shown::Image { .. }) {
                            items.push(entry(
                                fl!("ctx-span-all"),
                                Message::DisplaySpanAll(name.clone()),
                            ));
                        }
                    }
                    if shown != Shown::Nothing {
                        items.push(widget::divider::horizontal::light().into());
                        items.push(danger(
                            fl!("ctx-clear-display"),
                            Message::DisplayClear(name.clone()),
                        ));
                    }
                }
            }

            let popup = container(
                widget::column::with_children(items)
                    .spacing(2)
                    .padding(8)
                    .width(Length::Fixed(220.0)),
            )
            .class(cosmic::theme::Container::custom(|theme| {
                let cosmic = theme.cosmic();
                cosmic::widget::container::Style {
                    background: Some(cosmic::iced::Background::Color(
                        cosmic.background(theme.transparent).component.base.into(),
                    )),
                    icon_color: Some(cosmic.background(theme.transparent).component.on.into()),
                    text_color: Some(cosmic.background(theme.transparent).component.on.into()),
                    border: cosmic::iced::Border {
                        radius: cosmic.corner_radii.radius_m.into(),
                        width: 1.0,
                        color: cosmic
                            .background(theme.transparent)
                            .component
                            .divider
                            .into(),
                    },
                    shadow: cosmic::iced::Shadow {
                        color: cosmic::iced::Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                        offset: cosmic::iced::Vector::new(0.0, 2.0),
                        blur_radius: 8.0,
                    },
                    snap: false,
                }
            }));

            popover = popover
                .popup(popup)
                .position(widget::popover::Position::Point(cosmic::iced::Point {
                    x: *cx,
                    y: *cy,
                }))
                .on_close(Message::ExtendLayerMenuClose);
        }

        popover.into()
    }

    /// The help dock has room only on a reasonably wide canvas.
    fn tips_visible(&self) -> bool {
        !self.tips_hidden && (self.window_width == 0.0 || self.window_width >= 720.0)
    }

    /// A translucent card in the canvas corner with one usage tip, a button
    /// for the next one, and a close button.
    fn view_tips(&self) -> Element<'_, Message> {
        let tip = match self.tip_index % TIP_COUNT {
            0 => fl!("tip-1"),
            1 => fl!("tip-2"),
            2 => fl!("tip-3"),
            3 => fl!("tip-4"),
            4 => fl!("tip-5"),
            _ => fl!("tip-6"),
        };
        let row = widget::row::with_children(vec![
            widget::icon::from_name("dialog-information-symbolic")
                .size(16)
                .into(),
            text::body(tip).width(Length::Fixed(320.0)).into(),
            with_tip(
                widget::button::icon(widget::icon::from_name("go-next-symbolic"))
                    .on_press(Message::NextTip),
                fl!("tip-next"),
            ),
            with_tip(
                widget::button::icon(widget::icon::from_name("window-close-symbolic"))
                    .on_press(Message::HideTips),
                fl!("tip-hide"),
            ),
        ])
        .spacing(10)
        .align_y(Alignment::Center);

        container(row)
            .padding([8, 12])
            .class(cosmic::theme::Container::custom(|theme| {
                let cosmic = theme.cosmic();
                let mut bg: cosmic::iced::Color =
                    cosmic.background(theme.transparent).component.base.into();
                bg.a = 0.9;
                container::Style {
                    background: Some(cosmic::iced::Background::Color(bg)),
                    icon_color: Some(cosmic.background(theme.transparent).component.on.into()),
                    text_color: Some(cosmic.background(theme.transparent).component.on.into()),
                    border: cosmic::iced::Border {
                        radius: cosmic.corner_radii.radius_m.into(),
                        width: 1.0,
                        color: cosmic
                            .background(theme.transparent)
                            .component
                            .divider
                            .into(),
                    },
                    shadow: cosmic::iced::Shadow {
                        color: cosmic::iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35),
                        offset: cosmic::iced::Vector::new(0.0, 4.0),
                        blur_radius: 16.0,
                    },
                    snap: false,
                }
            }))
            .into()
    }

    /// Title of the inspector drawer: the selected display, or how many.
    fn inspector_title(&self) -> String {
        let targets = self.target_monitors();
        match targets.as_slice() {
            [one] => one.name.clone(),
            _ if targets.len() == self.monitor_geometry.len() => fl!("all-displays"),
            _ => fl!("n-displays", n = (targets.len() as u64)),
        }
    }

    /// The inspector: the selected display(s), what they show, and its settings.
    /// Free text and the content header sit flat on the panel; the settings
    /// rows share one list container so the controls read as a group.
    fn view_inspector(&self) -> Element<'_, Message> {
        let targets = self.target_monitors();
        let mut head: Vec<Element<'_, Message>> = Vec::new();
        let mut rows: Vec<Element<'_, Message>> = Vec::new();

        if targets.is_empty() {
            head.push(text::body(fl!("no-displays")).into());
            return widget::column::with_children(head).into();
        }

        let subtitle = match targets.as_slice() {
            [one] => one.display_label(),
            _ => targets
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        };
        head.push(text::body(subtitle).into());

        let shared = self.shared_shown(&targets);
        let show_placement = matches!(
            shared,
            None | Some(Shown::Nothing) | Some(Shown::Image { .. })
        );
        let placement = || -> Element<'_, Message> {
            settings::flex_item(
                fl!("placement"),
                segmented_control::horizontal(&self.placement_model)
                    .on_activate(Message::SetPlacement),
            )
            .into()
        };

        match shared {
            None => {
                head.push(text::body(fl!("mixed-content")).into());
                for m in &targets {
                    let what = self.describe_shown(&self.shown_on(m));
                    rows.push(
                        button::text(format!("{}: {what}", m.name))
                            .on_press(Message::DisplaySelected(m.name.clone(), false))
                            .width(Length::Fill)
                            .into(),
                    );
                }
                if show_placement {
                    rows.push(placement());
                }
            }
            Some(Shown::Nothing) => {
                head.push(text::body(fl!("nothing-staged")).into());
                if show_placement {
                    rows.push(placement());
                }
            }
            Some(Shown::Image { path, span }) => {
                let name = image_name(&path);
                let detail = match &span {
                    Some(k) => {
                        let covered: Vec<&str> = self
                            .monitor_geometry
                            .iter()
                            .filter(|m| layer_covers(&self.extend_layers[*k], m))
                            .map(|m| m.name.as_str())
                            .collect();
                        fl!("spanning", displays = covered.join(", "))
                    }
                    None => image::image_dimensions(&path)
                        .map(|(w, h)| format!("{w} × {h}"))
                        .unwrap_or_default(),
                };
                let thumb: Element<'_, Message> = match self
                    .selection
                    .paths
                    .iter()
                    .find(|(_, p)| **p == path)
                    .and_then(|(k, _)| self.selection.selection_handles.get(k))
                {
                    Some(handle) => widget::image(handle.clone())
                        .content_fit(cosmic::iced::ContentFit::Cover)
                        .width(Length::Fixed(88.0))
                        .height(Length::Fixed(50.0))
                        .into(),
                    None => widget::Space::new().width(88).height(50).into(),
                };
                head.push(content_header(thumb, name, detail));
                if show_placement {
                    rows.push(placement());
                }
                if span.is_none() {
                    let fit_idx = targets
                        .first()
                        .and_then(|m| self.locked_layer_on(&m.name))
                        .and_then(|k| self.extend_layer_fit.get(k))
                        .map_or(0, fit_index);
                    rows.push(
                        settings::item(
                            fl!("fit"),
                            framed(dropdown(
                                &self.fit_options,
                                Some(fit_idx),
                                Message::SetImageFit,
                            )),
                        )
                        .into(),
                    );
                }
            }
            Some(Shown::Color(color)) => {
                let name = color_name(&color);
                let kind = match color {
                    Color::Single(_) => fl!("solid-color"),
                    Color::Gradient(_) => fl!("color-gradient"),
                };
                head.push(content_header(color_image(color, 88, 50), kind, name));
            }
            Some(Shown::Shader(idx)) => {
                head.push(self.shader_header(idx));
                self.shader_settings(&mut rows, idx);
            }
        }

        if !rows.is_empty() {
            let list = rows
                .into_iter()
                .fold(widget::list_column(), |list, row| list.add(row));
            head.push(list.into());
        }
        widget::column::with_children(head)
            .spacing(12)
            .width(Length::Fill)
            .into()
    }

    /// Thumbnail, name and author of a live wallpaper.
    fn shader_header(&self, idx: usize) -> Element<'_, Message> {
        let Some(info) = self.available_shaders.get(idx) else {
            return widget::Space::new().into();
        };
        let author = info
            .parsed
            .as_ref()
            .map(|p| &p.metadata)
            .filter(|m| !m.author.is_empty())
            .map(|m| fl!("adapted-by", author = m.author.clone()))
            .unwrap_or_default();
        let thumb: Element<'_, Message> = match self.shader_thumbnails.get(idx) {
            Some(handle) => widget::image(handle.clone())
                .content_fit(cosmic::iced::ContentFit::Cover)
                .width(Length::Fixed(88.0))
                .height(Length::Fixed(50.0))
                .into(),
            None => widget::Space::new().width(88).height(50).into(),
        };
        content_header(thumb, info.name.clone(), author)
    }

    /// Settings rows for a live wallpaper: how heavy it is and its knobs.
    fn shader_settings<'a>(&'a self, rows: &mut Vec<Element<'a, Message>>, idx: usize) {
        let Some(info) = self.available_shaders.get(idx) else {
            return;
        };
        let meta = info.parsed.as_ref().map(|p| &p.metadata);

        if let Some(load) = info.load {
            rows.push(settings::item(fl!("gpu-load"), text::body(load_label(load))).into());
        }
        rows.push(
            settings::item(
                fl!("frame-rate"),
                framed(dropdown(
                    &self.frame_rate_options,
                    Some(self.selected_shader_frame_rate),
                    Message::ShaderFrameRate,
                )),
            )
            .into(),
        );
        rows.push(
            settings::item(
                fl!("render-quality"),
                framed(dropdown(
                    &self.render_scale_options,
                    Some(self.selected_shader_render_scale),
                    Message::ShaderRenderScale,
                )),
            )
            .into(),
        );

        if let Some(parsed) = &info.parsed {
            let values = self.shader_param_values.get(&idx);
            for param in &parsed.params {
                let current = values
                    .and_then(|v| v.get(&param.name))
                    .copied()
                    .unwrap_or(param.default);
                let name = param.name.clone();
                let (min, max, step, value, shown) = match param.param_type {
                    ParamType::F32 => (
                        param.min.as_f32(),
                        param.max.as_f32(),
                        param.step.as_f32(),
                        current.as_f32(),
                        format!("{:.2}", current.as_f32()),
                    ),
                    ParamType::I32 => (
                        param.min.as_i32() as f32,
                        param.max.as_i32() as f32,
                        param.step.as_i32() as f32,
                        current.as_i32() as f32,
                        current.as_i32().to_string(),
                    ),
                };
                let kind = param.param_type;
                rows.push(
                    // Label and value on one line, slider full width below:
                    // a fixed-width slider beside the label does not fit the
                    // panel and pushes every row to a different edge.
                    widget::column::with_children(vec![
                        widget::row::with_children(vec![
                            text::body(param.label.clone()).width(Length::Fill).into(),
                            text::body(shown).into(),
                        ])
                        .align_y(Alignment::Center)
                        .into(),
                        slider(min..=max, value, move |v| {
                            let value = match kind {
                                ParamType::F32 => ParamValue::F32(v),
                                ParamType::I32 => ParamValue::I32(v as i32),
                            };
                            Message::ShaderParamChanged(idx, name.clone(), value)
                        })
                        .on_release(Message::ShaderParamReleased)
                        .step(step)
                        .width(Length::Fill)
                        .into(),
                    ])
                    .spacing(6)
                    .width(Length::Fill)
                    .into(),
                );
            }
            if !parsed.params.is_empty() {
                rows.push(
                    container(
                        button::text(fl!("reset-to-defaults"))
                            .on_press(Message::ResetShaderParams(idx)),
                    )
                    .width(Length::Fill)
                    .align_x(Alignment::End)
                    .into(),
                );
            }
        }

        if let Some(m) = meta {
            if !m.license.is_empty() {
                rows.push(settings::item(fl!("shader-license"), widget::text(&m.license)).into());
            }
            if !m.source.is_empty() {
                let source: Element<'_, Message> = if m.source.starts_with("http") {
                    widget::button::link(m.source.clone())
                        .on_press(Message::OpenUrl(m.source.clone()))
                        .into()
                } else {
                    widget::text(&m.source).into()
                };
                rows.push(settings::item(fl!("shader-source"), source).into());
            }
        }
    }

    fn describe_shown(&self, shown: &Shown) -> String {
        match shown {
            Shown::Nothing => fl!("nothing-yet"),
            Shown::Image { path, span } => {
                let name = image_name(path);
                if span.is_some() {
                    fl!("spanned-image", name = name)
                } else {
                    name
                }
            }
            Shown::Color(c) => color_name(c),
            Shown::Shader(idx) => self
                .available_shaders
                .get(*idx)
                .map(|s| s.name.clone())
                .unwrap_or_default(),
        }
    }

    /// The library: one grid of images, live wallpapers, and colors, with a
    /// kind filter and a name search above it.
    fn view_library(&self, scroll_grid: bool) -> Element<'_, Message> {
        let filter = self
            .library_filter
            .active_data::<LibraryFilter>()
            .copied()
            .unwrap_or(LibraryFilter::All);
        let query = self.library_query.trim().to_lowercase();
        let matches = |name: &str| query.is_empty() || name.to_lowercase().contains(&query);
        let mut cards: Vec<Element<'_, Message>> = Vec::new();
        if matches!(filter, LibraryFilter::All | LibraryFilter::Images) {
            cards.extend(self.wallpaper_cards(&matches));
        }
        if matches!(filter, LibraryFilter::All | LibraryFilter::Live) {
            cards.extend(self.shader_cards(&matches));
        }
        if matches!(filter, LibraryFilter::All | LibraryFilter::Colors) {
            cards.extend(self.color_cards(&matches));
        }

        let filter_control = segmented_control::horizontal(&self.library_filter)
            .on_activate(Message::LibraryFilter)
            .width(Length::Shrink);
        let search = widget::search_input(fl!("search-library"), &self.library_query)
            .on_input(Message::LibrarySearch)
            .on_clear(Message::LibrarySearch(String::new()));
        let add_images = with_tip(
            widget::button::icon(widget::icon::from_name("list-add-symbolic"))
                .on_press(Message::AddWallpaperImages),
            fl!("add-images"),
        );
        let add_folder = with_tip(
            widget::button::icon(widget::icon::from_name("folder-new-symbolic"))
                .on_press(Message::AddWallpaperFolder),
            fl!("add-folder"),
        );
        let toolbar: Element<'_, Message> = if self.condensed() {
            // Not enough room for everything on one line: search goes below.
            widget::column::with_children(vec![
                filter_control.into(),
                widget::row::with_children(vec![
                    search.width(Length::Fill).into(),
                    add_images,
                    add_folder,
                ])
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            ])
            .spacing(8)
            .into()
        } else {
            widget::row::with_children(vec![
                filter_control.into(),
                search.width(Length::Fixed(260.0)).into(),
                widget::Space::new().width(Length::Fill).into(),
                add_images,
                add_folder,
            ])
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        };

        let grid: Element<'_, Message> = if cards.is_empty() {
            let msg = if filter == LibraryFilter::Live && self.available_shaders.is_empty() {
                fl!("no-shaders")
            } else {
                fl!("library-empty")
            };
            container(text::body(msg)).padding(24).into()
        } else {
            widget::flex_row(cards)
                .column_spacing(12)
                .row_spacing(16)
                .into()
        };
        let grid: Element<'_, Message> = if scroll_grid {
            widget::scrollable(container(grid).width(Length::Fill).padding([0, 12, 12, 0]))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            container(grid).width(Length::Fill).into()
        };

        widget::column::with_children(vec![toolbar, grid])
            .spacing(12)
            .width(Length::Fill)
            .height(if scroll_grid {
                Length::Fill
            } else {
                Length::Shrink
            })
            .into()
    }

    /// Context menu shared by every library card: put the item on all
    /// displays, on one of them, or (images) span it across all of them.
    fn card_menu(&self, item: Item) -> Vec<menu::Item<CardAction, String>> {
        let mut items = vec![menu::Item::Button(
            fl!("put-on-all"),
            None,
            CardAction::PutOnAll(item),
        )];
        if self.monitor_geometry.len() > 1 {
            for (i, m) in self.monitor_geometry.iter().enumerate() {
                items.push(menu::Item::Button(
                    fl!("put-on", display = m.name.clone()),
                    None,
                    CardAction::PutOn(item, i),
                ));
            }
            if let Item::Image(key) = item {
                items.push(menu::Item::Button(
                    fl!("span-all"),
                    None,
                    CardAction::SpanAll(key),
                ));
            }
        }
        items
    }

    fn wallpaper_cards(&self, matches: &dyn Fn(&str) -> bool) -> Vec<Element<'_, Message>> {
        self.selection
            .selection_handles
            .iter()
            .filter_map(|(id, handle)| {
                let path = self.selection.paths.get(id)?;
                let name = image_name(path);
                if !matches(&name) {
                    return None;
                }
                let thumb = widget::button::image(handle.clone()).on_press(Message::PickImage(id));
                let on =
                    self.staged_badge(|s| matches!(s, Shown::Image { path: p, .. } if p == path));
                let card = library_card(card_thumb(thumb, None, on.map(accent_pill), None), name);

                let mut items = self.card_menu(Item::Image(id));
                // User-added sources can be removed again; bundled ones can't.
                if let Some(src_idx) = self.wallpaper_source_index_for(path) {
                    items.push(menu::Item::Divider);
                    items.push(menu::Item::Button(
                        fl!("wp-remove-source"),
                        None,
                        CardAction::RemoveSource(src_idx),
                    ));
                }
                Some(widget::context_menu(card, Some(menu::items(&HashMap::new(), items))).into())
            })
            .collect()
    }

    fn color_cards(&self, matches: &dyn Fn(&str) -> bool) -> Vec<Element<'_, Message>> {
        DEFAULT_COLORS
            .iter()
            .enumerate()
            .filter_map(|(idx, color)| {
                let name = color_name(color);
                if !matches(&name) {
                    return None;
                }
                let swatch = button::custom_image_button(
                    color_image(color.clone(), THUMB_WIDTH as u16, THUMB_HEIGHT as u16),
                    None::<Message>,
                )
                .padding(0)
                .class(button::ButtonClass::Image)
                .on_press(Message::PickColor(idx));
                let on = self.staged_badge(|s| matches!(s, Shown::Color(c) if c == color));
                let card = library_card(card_thumb(swatch, None, on.map(accent_pill), None), name);
                let items = self.card_menu(Item::Color(idx));
                Some(widget::context_menu(card, Some(menu::items(&HashMap::new(), items))).into())
            })
            .collect()
    }

    fn shader_cards(&self, matches: &dyn Fn(&str) -> bool) -> Vec<Element<'_, Message>> {
        self.shader_thumbnails
            .iter()
            .enumerate()
            .filter_map(|(idx, handle)| {
                let info = self.available_shaders.get(idx)?;
                if !matches(&info.name) {
                    return None;
                }
                let thumb =
                    widget::button::image(handle.clone()).on_press(Message::PickShader(idx));
                let on = self.staged_badge(|s| matches!(s, Shown::Shader(i) if *i == idx));
                let load = info
                    .load
                    .map(|l| dark_pill(fl!("gpu-pill", load = load_label(l))));
                let card = library_card(
                    card_thumb(thumb, Some(live_badge()), on.map(accent_pill), load),
                    info.name.clone(),
                );
                let items = self.card_menu(Item::Shader(idx));
                Some(widget::context_menu(card, Some(menu::items(&HashMap::new(), items))).into())
            })
            .collect()
    }

    /// Index of the user-added source a wallpaper path belongs to (the file
    /// itself, or a directory that contains it). `None` for bundled wallpapers.
    fn wallpaper_source_index_for(&self, path: &Path) -> Option<usize> {
        self.wallpaper_sources
            .iter()
            .position(|src| src == path || (src.is_dir() && path.starts_with(src)))
    }

    /// Build the settings drawer content
    fn settings_drawer_view(&self) -> Element<'_, Message> {
        // Build power saving section
        let mut power_saving_section = widget::settings::section().title(fl!("power-saving"));

        power_saving_section = power_saving_section.add(settings::item(
            fl!("on-battery"),
            framed(dropdown(
                &self.on_battery_action_options,
                Some(self.selected_on_battery_action),
                Message::SetOnBatteryAction,
            )),
        ));

        {
            let toggle_row = settings::item(
                fl!("pause-low-battery"),
                toggler(self.power_saving.pause_on_low_battery)
                    .on_toggle(Message::SetPauseOnLowBattery),
            );

            if self.power_saving.pause_on_low_battery {
                let dropdown_row = settings::item(
                    fl!("low-battery-threshold"),
                    framed(dropdown(
                        &self.low_battery_threshold_options,
                        Some(self.selected_low_battery_threshold),
                        Message::SetLowBatteryThreshold,
                    )),
                );

                power_saving_section = power_saving_section.add(
                    widget::column::with_children(vec![toggle_row.into(), dropdown_row.into()])
                        .spacing(8),
                );
            } else {
                power_saving_section = power_saving_section.add(toggle_row);
            }
        }

        power_saving_section = power_saving_section.add(settings::item(
            fl!("pause-lid-closed"),
            toggler(self.power_saving.pause_on_lid_closed).on_toggle(Message::SetPauseOnLidClosed),
        ));

        power_saving_section = power_saving_section.add(settings::item(
            fl!("prefer-low-power"),
            toggler(self.prefer_low_power).on_toggle(Message::PreferLowPower),
        ));

        // Build background service section with optional PATH warning
        let mut bg_service_section = widget::settings::section()
            .title(fl!("background-service"))
            .add(settings::item(
                fl!("use-glowberry"),
                toggler(self.glowberry_is_default).on_toggle(Message::SetGlowBerryDefault),
            ));

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

        widget::settings::view_column(vec![
            bg_service_section.into(),
            appearance_section.into(),
            power_saving_section.into(),
        ])
        .into()
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// Wrap a widget (typically an icon button) with a hover tooltip.
fn with_tip<'a>(content: impl Into<Element<'a, Message>>, tip: String) -> Element<'a, Message> {
    widget::tooltip(
        content,
        widget::text::body(tip),
        widget::tooltip::Position::Top,
    )
    .into()
}

/// Whether a free layer covers a display's center.
fn layer_covers(layer: &ExtendLayerState, monitor: &MonitorGeometry) -> bool {
    let cx = monitor.position.0 as f64 + monitor.logical_size.0 as f64 / 2.0;
    let cy = monitor.position.1 as f64 + monitor.logical_size.1 as f64 / 2.0;
    let w = layer.image_size.0 as f64 * layer.scale;
    let h = layer.image_size.1 as f64 * layer.scale;
    cx >= layer.offset.0
        && cx <= layer.offset.0 + w
        && cy >= layer.offset.1
        && cy <= layer.offset.1 + h
}

/// Scale and place a free layer so it covers the bounding box of `monitors`.
fn fit_layer_to(layer: &mut ExtendLayerState, monitors: &[MonitorGeometry]) {
    if monitors.is_empty() || layer.image_size == (0, 0) {
        return;
    }
    let min_x = monitors.iter().map(|m| m.position.0).min().unwrap_or(0) as f64;
    let min_y = monitors.iter().map(|m| m.position.1).min().unwrap_or(0) as f64;
    let max_x = monitors
        .iter()
        .map(|m| m.position.0 + m.logical_size.0 as i32)
        .max()
        .unwrap_or(0) as f64;
    let max_y = monitors
        .iter()
        .map(|m| m.position.1 + m.logical_size.1 as i32)
        .max()
        .unwrap_or(0) as f64;

    let vd_w = max_x - min_x;
    let vd_h = max_y - min_y;
    let img_w = layer.image_size.0 as f64;
    let img_h = layer.image_size.1 as f64;
    let scale = (vd_w / img_w).max(vd_h / img_h);
    layer.scale = scale;
    layer.offset = (
        min_x + (vd_w - img_w * scale) / 2.0,
        min_y + (vd_h - img_h * scale) / 2.0,
    );
}

/// Fit dropdown index -> config scaling mode.
fn fit_mode(idx: usize) -> ScalingMode {
    match idx {
        1 => ScalingMode::Fit([0.0, 0.0, 0.0]),
        2 => ScalingMode::Stretch,
        _ => ScalingMode::Zoom,
    }
}

/// Config scaling mode -> fit dropdown index.
fn fit_index(mode: &ScalingMode) -> usize {
    match mode {
        ScalingMode::Zoom => 0,
        ScalingMode::Fit(_) => 1,
        ScalingMode::Stretch => 2,
    }
}

fn load_label(load: Complexity) -> String {
    match load {
        Complexity::Low => fl!("resource-low"),
        Complexity::Medium => fl!("resource-medium"),
        Complexity::High => fl!("resource-high"),
    }
}

/// Human name for an image: its file stem with separators as spaces.
fn image_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().replace(['_', '-'], " "))
        .unwrap_or_default()
}

/// Display name for a color: its hex code, or "Gradient".
fn color_name(color: &Color) -> String {
    match color {
        Color::Single([r, g, b]) => {
            let c = |v: &f32| (v * 255.0).round() as u8;
            format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
        }
        Color::Gradient(_) => fl!("color-gradient"),
    }
}

/// A dropdown with a visible button background, so it reads as a control
/// before it is hovered.
fn framed<'a>(control: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(control)
        .class(cosmic::theme::Container::custom(|theme| {
            let cosmic = theme.cosmic();
            container::Style {
                background: Some(cosmic::iced::Background::Color(cosmic.button.base.into())),
                icon_color: Some(cosmic.button.on.into()),
                text_color: Some(cosmic.button.on.into()),
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

/// Thumbnail + name + one line of detail, for the top of the inspector.
fn content_header<'a>(
    thumb: Element<'a, Message>,
    name: String,
    detail: String,
) -> Element<'a, Message> {
    widget::row::with_children(vec![
        thumb,
        widget::column::with_children(vec![
            text::heading(name).into(),
            text::caption(detail).into(),
        ])
        .spacing(2)
        .into(),
    ])
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

/// A library grid cell: the thumbnail with its name underneath.
fn library_card<'a>(
    content: impl Into<Element<'a, Message>>,
    name: String,
) -> Element<'a, Message> {
    widget::column::with_children(vec![
        content.into(),
        widget::text::caption(name)
            .width(Length::Fixed(THUMB_WIDTH as f32))
            .align_x(Alignment::Center)
            .into(),
    ])
    .spacing(4)
    .align_x(Alignment::Center)
    .into()
}

/// A thumbnail with optional badges in its corners. Badges are inert, so
/// clicks fall through to the thumbnail.
fn card_thumb<'a>(
    thumb: impl Into<Element<'a, Message>>,
    top_left: Option<Element<'a, Message>>,
    top_right: Option<Element<'a, Message>>,
    bottom_right: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let corner = |badge: Element<'a, Message>, x: Alignment, y: Alignment| {
        container(badge)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(x)
            .align_y(y)
            .padding(8)
    };
    let mut stack = cosmic::iced::widget::Stack::new().push(thumb.into());
    if let Some(b) = top_left {
        stack = stack.push(corner(b, Alignment::Start, Alignment::Start));
    }
    if let Some(b) = top_right {
        stack = stack.push(corner(b, Alignment::End, Alignment::Start));
    }
    if let Some(b) = bottom_right {
        stack = stack.push(corner(b, Alignment::End, Alignment::End));
    }
    stack.into()
}

fn pill_style(
    bg: cosmic::iced::Color,
    fg: cosmic::iced::Color,
) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |_| container::Style {
        background: Some(cosmic::iced::Background::Color(bg)),
        icon_color: Some(fg),
        text_color: Some(fg),
        border: cosmic::iced::Border {
            radius: 10.0.into(),
            ..Default::default()
        },
        shadow: cosmic::iced::Shadow::default(),
        snap: false,
    })
}

/// Small play glyph marking a card as a live wallpaper.
fn live_badge<'a>() -> Element<'a, Message> {
    container(widget::icon::from_name("media-playback-start-symbolic").size(12))
        .padding([3, 5])
        .class(pill_style(
            cosmic::iced::Color::from_rgba(0.0, 0.0, 0.0, 0.55),
            cosmic::iced::Color::WHITE,
        ))
        .into()
}

/// Translucent dark pill with a short caption (GPU load).
fn dark_pill<'a>(label: String) -> Element<'a, Message> {
    container(text::caption(label))
        .padding([1, 7])
        .class(pill_style(
            cosmic::iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6),
            cosmic::iced::Color::WHITE,
        ))
        .into()
}

/// Accent pill marking where an item is staged.
fn accent_pill<'a>(label: String) -> Element<'a, Message> {
    let theme = cosmic::theme::active();
    let cosmic = theme.cosmic();
    container(text::caption(label))
        .padding([1, 7])
        .class(pill_style(
            cosmic.accent_color().into(),
            cosmic.on_accent_color().into(),
        ))
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

fn collect_shader_file(path: &Path, shaders: &mut Vec<ShaderInfo>) {
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

    // GPU load estimate at default parameters, from the shader's AST.
    let load = parsed.as_ref().and_then(|p| {
        let has_texture =
            p.source_body.contains("iTexture") || p.source_body.contains("textureSample");
        shader_analysis::analyze_glowberry_shader(&p.source_body, has_texture, None)
            .ok()
            .map(|m| m.complexity())
    });

    shaders.push(ShaderInfo {
        path: path.to_path_buf(),
        name,
        parsed,
        load,
    });
}

/// Find the wallpaper folder by searching XDG data directories.
fn find_wallpaper_folder() -> PathBuf {
    let subdir = "backgrounds/cosmic";
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
/// When enabled, creates a symlink at ~/.local/bin/cosmic-bg -> the glowberry daemon,
/// which lives next to this binary (~/.local/bin for `just install`, /usr/bin for the .deb).
/// When disabled, removes the symlink so the original /usr/bin/cosmic-bg is used.
///
/// No elevated privileges needed since we operate in ~/.local/bin/.
async fn set_glowberry_default(enable: bool) -> Result<bool, String> {
    use tokio::process::Command;

    let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
    let local_bin = home.join(".local/bin");
    let symlink_path = local_bin.join("cosmic-bg");

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
        let glowberry_bin = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join("glowberry")))
            .filter(|p| p.is_file())
            .ok_or("Cannot find the glowberry daemon next to glowberry-settings")?;

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
