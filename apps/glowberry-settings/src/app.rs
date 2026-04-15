// SPDX-License-Identifier: MPL-2.0

//! Main application state and logic for GlowBerry Settings

use crate::fl;
use crate::shader_params::{ParamType, ParamValue, ParsedShader};
use cosmic::app::context_drawer::{self, ContextDrawer};
use cosmic::app::{Core, Task};
use cosmic::iced::Subscription;
use cosmic::iced::{Alignment, Length};
use cosmic::iced::widget::image::Handle as ImageHandle;
use cosmic::widget::{
    self, button, container, dropdown, segmented_button, settings, slider, tab_bar, text, toggler,
};
use cosmic::{ApplicationExt, Apply, Element};
use cosmic_config::CosmicConfigEntry;
use glowberry_config::power_saving::{OnBatteryAction, PowerSavingConfig};
use glowberry_config::state::State;
use glowberry_config::{Color, Config, Context as ConfigContext, Entry, Gradient, Source};
use crate::shader_analysis::{self, Complexity};
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

    /// Fit options (Zoom, Fit)
    fit_options: Vec<String>,
    selected_fit: usize,

    /// Cached display preview image
    cached_display_handle: Option<ImageHandle>,

    /// Current wallpaper folder
    current_folder: PathBuf,

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
pub enum Message {
    /// Category changed
    ChangeCategory(Category),
    /// Wallpaper selected
    Select(DefaultKey),
    /// Color selected
    ColorSelect(Color),
    /// Shader selected
    ShaderSelect(usize),
    /// Shader thumbnail loaded
    ShaderThumbnail(usize, Option<ImageHandle>),
    /// Frame rate changed
    ShaderFrameRate(usize),
    /// Fit mode changed
    Fit(usize),
    /// Wallpaper event from subscription
    WallpaperEvent(WallpaperEvent),
    /// Toggle context drawer page
    ToggleContextPage(ContextPage),
    /// Open URL (for about page links)
    OpenUrl(String),
    /// Same wallpaper on all displays toggle
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
            .version(env!("CARGO_PKG_VERSION"))
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
            prefer_low_power: true, // Will be set below
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
        };

        // Load prefer_low_power, power saving, and window opacity from config
        if let Some(ctx) = &app.config_context {
            app.prefer_low_power = ctx.prefer_low_power();
            app.power_saving = ctx.power_saving_config();
            app.window_opacity = ctx.window_opacity();

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
        let title_task = app.set_window_title(fl!("app-title"));

        let shader_task = if !app.available_shaders.is_empty() {
            app.load_shader_thumbnails()
        } else {
            Task::none()
        };

        (app, Task::batch([title_task, shader_task]))
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subscriptions = vec![
            // Wallpaper loading subscription
            wallpaper_subscription::wallpapers(self.current_folder.clone())
                .map(Message::WallpaperEvent),
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
        match message {
            Message::ChangeCategory(category) => {
                self.categories.selected = Some(category.clone());

                if category == Category::Shaders {
                    // Load shaders if needed
                    if self.available_shaders.is_empty() {
                        self.available_shaders = discover_shaders();
                        let placeholder = create_shader_placeholder(158, 105);
                        self.shader_thumbnails =
                            vec![placeholder; self.available_shaders.len()];
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
                self.selection.active = Choice::Color(color);
                self.cached_display_handle = None;
                self.apply_selection();
            }

            Message::ShaderSelect(idx) => {
                if idx < self.available_shaders.len() {
                    self.selection.active = Choice::Shader(idx);
                    self.cached_display_handle = None;
                    self.apply_selection();
                }
            }

            Message::ShaderThumbnail(idx, handle) => {
                if let Some(handle) = handle
                    && idx < self.shader_thumbnails.len() {
                        self.shader_thumbnails[idx] = handle;
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
                        && let Source::Path(config_path) = &entry.source {
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
                    && self.config != config {
                        tracing::debug!("Config changed externally, reloading");
                        self.config = config;
                        self.init_from_config();

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
                    && idx == shader_idx {
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

            Message::WindowOpacityReleased => {
                // Save the opacity value to config when slider is released
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_window_opacity(self.window_opacity);
                }
            }
        }

        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let mut children: Vec<Element<'_, Message>> = Vec::with_capacity(6);

        // 1. Display preview (centered)
        children.push(
            container(self.view_display_preview())
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .into(),
        );

        // 2. Display selector (tab bar or "All Displays" label)
        if self.config.same_on_all {
            // Show "All Displays" heading
            let element = text::heading(fl!("all-displays"))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .height(Length::Fill)
                .apply(container)
                .width(Length::Fill)
                .height(Length::Fixed(32.0));
            children.push(element.into());
        } else if self.show_tab_bar {
            // Show tab bar to select which display to configure
            let element = tab_bar::horizontal(&self.outputs)
                .button_alignment(Alignment::Center)
                .on_activate(Message::OutputChanged);
            children.push(element.into());
        }

        // 3. Settings list (same on all displays, fit) - centered
        children.push(
            container(self.view_settings_list())
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .into(),
        );

        // 3. Category dropdown - centered
        let category_dropdown =
            dropdown::multi::dropdown(&self.categories, Message::ChangeCategory);
        children.push(
            container(category_dropdown)
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .into(),
        );

        // 4. Selection grid based on category - centered
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

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        vec![]
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
        ])
        .into()
    }

    fn init_from_config(&mut self) {
        // Determine which entry to use based on same_on_all and active_output
        let entry = if self.config.same_on_all {
            &self.config.default_background
        } else if let Some(ref output_name) = self.active_output {
            // Try to find a per-output entry
            self.config
                .entry(output_name)
                .unwrap_or(&self.config.default_background)
        } else {
            &self.config.default_background
        };

        self.select_entry_source(&entry.source.clone());
    }

    fn cache_display_image(&mut self) {
        self.cached_display_handle = None;

        if let Choice::Wallpaper(id) = self.selection.active
            && let Some(image) = self.selection.display_images.get(id) {
                self.cached_display_handle = Some(ImageHandle::from_rgba(
                    image.width(),
                    image.height(),
                    image.to_vec(),
                ));
            }
    }

    fn apply_selection(&mut self) {
        let Some(ctx) = &self.config_context else {
            return;
        };

        let source = match &self.selection.active {
            Choice::Wallpaper(key) => {
                if let Some(path) = self.selection.paths.get(*key) {
                    Source::Path(path.clone())
                } else {
                    return;
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
                    return;
                }
            }
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
                    && !shader_source.params.is_empty() {
                        // Convert f64 values back to ParamValue based on shader's param definitions
                        let mut param_values: HashMap<String, ParamValue> = HashMap::new();

                        if let Some(shader_info) = self.available_shaders.get(idx)
                            && let Some(parsed) = &shader_info.parsed {
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

        // Same on all displays toggle
        list = list.add(settings::item(
            fl!("same-on-all"),
            toggler(self.config.same_on_all).on_toggle(Message::SameWallpaper),
        ));

        // Fit dropdown (only for wallpapers)
        if matches!(self.selection.active, Choice::Wallpaper(_)) {
            list = list.add(settings::item(
                fl!("fit"),
                dropdown(&self.fit_options, Some(self.selected_fit), Message::Fit),
            ));
        }

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
                    && let Some(parsed) = &shader_info.parsed {
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
                            let source_widget: Element<'_, Message> =
                                if metadata.source.starts_with("http") {
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

    fn view_wallpaper_grid(&self) -> Element<'_, Message> {
        let selected = if let Choice::Wallpaper(key) = self.selection.active {
            Some(key)
        } else {
            None
        };

        let buttons: Vec<Element<'_, Message>> = self
            .selection
            .selection_handles
            .iter()
            .map(|(id, handle)| {
                widget::button::image(handle.clone())
                    .selected(selected == Some(id))
                    .on_press(Message::Select(id))
                    .into()
            })
            .collect();

        if buttons.is_empty() {
            widget::text(fl!("loading-wallpapers")).into()
        } else {
            widget::flex_row(buttons)
                .column_spacing(12)
                .row_spacing(16)
                .into()
        }
    }

    fn view_color_grid(&self) -> Element<'_, Message> {
        let selected = if let Choice::Color(ref c) = self.selection.active {
            Some(c)
        } else {
            None
        };

        let buttons: Vec<Element<'_, Message>> = DEFAULT_COLORS
            .iter()
            .map(|color| {
                let content = color_image(color.clone(), 70, 70);
                button::custom_image_button(content, None::<Message>)
                    .padding(0)
                    .selected(selected == Some(color))
                    .class(button::ButtonClass::Image)
                    .on_press(Message::ColorSelect(color.clone()))
                    .into()
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

                widget::column::with_children(vec![
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
                .into()
            })
            .collect();

        widget::flex_row(buttons)
            .column_spacing(12)
            .row_spacing(16)
            .into()
    }
}

// Helper functions

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
