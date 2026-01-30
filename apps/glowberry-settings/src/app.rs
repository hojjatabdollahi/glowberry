// SPDX-License-Identifier: MPL-2.0

//! Main application state and logic for GlowBerry Settings

use crate::fl;
use cosmic::app::context_drawer::{self, ContextDrawer};
use cosmic::app::{Core, Task};
use cosmic::iced::Subscription;
use cosmic::iced_runtime::core::image::Handle as ImageHandle;
use cosmic::widget::{self, button, container, dropdown, menu, settings, toggler};
use cosmic::{ApplicationExt, Element};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use glowberry_config::{Color, Config, Context as ConfigContext, Entry, Gradient, Source};
use image::{ImageBuffer, Rgba};
use slotmap::{DefaultKey, SecondaryMap, SlotMap};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;

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
    /// Key bindings for the application's menu bar
    key_binds: HashMap<menu::KeyBind, MenuAction>,

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
}

/// Information about an available shader
#[derive(Clone, Debug)]
pub struct ShaderInfo {
    pub path: PathBuf,
    pub name: String,
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
    /// Prefer low power GPU toggle
    PreferLowPower(bool),
    /// Config changed externally (from daemon or another instance)
    ConfigChanged(Config),
    /// Toggle GlowBerry as the default background service
    SetGlowBerryDefault(bool),
    /// Result of setting GlowBerry as default
    SetGlowBerryDefaultResult(Result<bool, String>),
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

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Self::Message>) {
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

        // Default wallpaper folder
        let current_folder = PathBuf::from("/usr/share/backgrounds/cosmic");

        // Pre-discover shaders so they're ready when user clicks "Live Wallpapers"
        let available_shaders = discover_shaders();
        let placeholder = create_shader_placeholder(158, 105);
        let shader_thumbnails = vec![placeholder; available_shaders.len()];

        // About information
        let about = widget::about::About::default()
            .name(fl!("app-title"))
            .version(env!("CARGO_PKG_VERSION"))
            .icon(widget::icon::from_name("io.github.hojjatabdollahi.glowberry"))
            .author("Hojjat Abdollahi")
            .license("MPL-2.0")
            .links([
                (fl!("repository"), "https://github.com/hojjatabdollahi/glowberry"),
            ]);

        let mut app = Self {
            core,
            config,
            config_context,
            context_page: ContextPage::default(),
            about,
            key_binds: HashMap::new(),
            categories,
            selection: SelectionContext::default(),
            available_shaders,
            shader_thumbnails,
            selected_shader_frame_rate: 1, // 30 FPS default
            frame_rate_options: vec![
                fl!("fps-15"),
                fl!("fps-30"),
                fl!("fps-60"),
            ],
            fit_options: vec![fl!("fit-fill"), fl!("fit-fit")],
            selected_fit: 0,
            cached_display_handle: None,
            current_folder,
            prefer_low_power: true, // Will be set below
            glowberry_is_default: is_glowberry_default(),
        };
        
        // Load prefer_low_power from config
        if let Some(ctx) = &app.config_context {
            app.prefer_low_power = ctx.prefer_low_power();
        }

        // Initialize selection from config
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

        // Watch for config changes from other sources (daemon, other instances)
        // We use State which implements CosmicConfigEntry
        if self.config_context.is_some() {
            subscriptions.push(
                cosmic_config::config_subscription::<_, glowberry_config::state::State>(
                    std::any::TypeId::of::<Self>(),
                    glowberry_config::NAME.into(),
                    glowberry_config::state::State::version(),
                )
                .map(|update| {
                    if !update.errors.is_empty() {
                        for why in &update.errors {
                            tracing::error!(?why, "config subscription error");
                        }
                    }
                    // Reload the full config when changes are detected
                    if let Ok(ctx) = glowberry_config::context() {
                        if let Ok(config) = Config::load(&ctx) {
                            return Message::ConfigChanged(config);
                        }
                    }
                    // Return current config if reload fails
                    Message::ConfigChanged(Config::default())
                }),
            );
        }

        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::ChangeCategory(category) => {
                self.categories.selected = Some(category.clone());

                match category {
                    Category::Shaders => {
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
                    _ => {}
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
                if let Some(handle) = handle {
                    if idx < self.shader_thumbnails.len() {
                        self.shader_thumbnails[idx] = handle;
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
                    // Only select a wallpaper if config source is a Path
                    // Don't override if user has a Color or Shader selected
                    if let Source::Path(config_path) = &self.config.default_background.source {
                        // Find the wallpaper that matches the config path
                        if let Some((key, _)) = self
                            .selection
                            .paths
                            .iter()
                            .find(|(_, p)| *p == config_path)
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
                WallpaperEvent::Error(e) => {
                    tracing::error!("Wallpaper loading error: {}", e);
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
                self.apply_selection();
            }

            Message::PreferLowPower(value) => {
                self.prefer_low_power = value;
                if let Some(ctx) = &self.config_context {
                    let _ = ctx.set_prefer_low_power(value);
                }
            }

            Message::ConfigChanged(config) => {
                // Only update if config actually changed
                if self.config != config {
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
            }

            Message::SetGlowBerryDefault(enable) => {
                // Run the enable/disable command asynchronously with pkexec
                return Task::perform(
                    async move {
                        set_glowberry_default(enable).await
                    },
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
        }

        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let mut children: Vec<Element<'_, Message>> = Vec::with_capacity(5);

        // 1. Display preview (centered)
        children.push(
            container(self.view_display_preview())
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .into(),
        );

        // 2. Settings list (same on all displays, fit) - centered
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
            None => widget::Space::new(0, 0).into(),
        };
        children.push(
            container(grid)
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .into(),
        );

        // Wrap everything in a scrollable container
        widget::scrollable(
            widget::column::with_children(children)
                .spacing(22)
                .padding(20)
                .width(Length::Fill)
                .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        let menu_bar = menu::bar(vec![menu::Tree::with_children(
            menu::root(fl!("view")).apply(Element::from),
            menu::items(
                &self.key_binds,
                vec![
                    menu::Item::Button(fl!("about"), None, MenuAction::About),
                    menu::Item::Button(fl!("settings"), None, MenuAction::Settings),
                ],
            ),
        )]);

        vec![menu_bar.into()]
    }

    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        vec![]
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
}

impl GlowBerrySettings {
    /// Build the settings drawer content
    fn settings_drawer_view(&self) -> Element<'_, Message> {
        widget::settings::view_column(vec![
            // Default background service section
            widget::settings::section()
                .title(fl!("background-service"))
                .add(settings::item(
                    fl!("use-glowberry"),
                    toggler(self.glowberry_is_default).on_toggle(Message::SetGlowBerryDefault),
                ))
                .into(),
            // GPU settings section
            widget::settings::section()
                .title(fl!("performance"))
                .add(settings::item(
                    fl!("prefer-low-power"),
                    toggler(self.prefer_low_power).on_toggle(Message::PreferLowPower),
                ))
                .into(),
        ])
        .into()
    }

    fn init_from_config(&mut self) {
        match &self.config.default_background.source {
            Source::Path(_path) => {
                // Will be set when wallpapers load
            }
            Source::Color(color) => {
                self.selection.active = Choice::Color(color.clone());
                self.categories.selected = Some(Category::Colors);
            }
            Source::Shader(shader) => {
                // Shaders are already pre-discovered in init, just find the selected one
                if let glowberry_config::ShaderContent::Path(config_path) = &shader.shader {
                    // Try exact path match first
                    let found = if let Some(idx) = self.available_shaders.iter().position(|s| &s.path == config_path) {
                        self.selection.active = Choice::Shader(idx);
                        true
                    } else {
                        // Fall back to filename match (in case paths differ due to XDG_DATA_DIRS)
                        if let Some(config_filename) = config_path.file_name() {
                            if let Some(idx) = self.available_shaders.iter().position(|s| {
                                s.path.file_name() == Some(config_filename)
                            }) {
                                self.selection.active = Choice::Shader(idx);
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    };
                    
                    // If no shader found, select the first one if available
                    if !found && !self.available_shaders.is_empty() {
                        self.selection.active = Choice::Shader(0);
                    }
                } else if !self.available_shaders.is_empty() {
                    // Inline shader content - just select first shader
                    self.selection.active = Choice::Shader(0);
                }
                
                self.selected_shader_frame_rate = match shader.frame_rate {
                    0..=22 => 0,
                    23..=45 => 1,
                    _ => 2,
                };
                self.categories.selected = Some(Category::Shaders);
            }
        }
    }

    fn cache_display_image(&mut self) {
        self.cached_display_handle = None;

        if let Choice::Wallpaper(id) = self.selection.active {
            if let Some(image) = self.selection.display_images.get(id) {
                self.cached_display_handle = Some(ImageHandle::from_rgba(
                    image.width(),
                    image.height(),
                    image.to_vec(),
                ));
            }
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
                    Source::Shader(glowberry_config::ShaderSource {
                        shader: glowberry_config::ShaderContent::Path(shader.path.clone()),
                        background_image: None,
                        language: glowberry_config::ShaderLanguage::Wgsl,
                        frame_rate,
                    })
                } else {
                    return;
                }
            }
        };

        let entry = Entry::new("all".to_string(), source);
        if let Err(e) = self.config.set_entry(ctx, entry) {
            tracing::error!("Failed to set wallpaper: {}", e);
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
                                        tracing::debug!(?path, ?e, "Failed to render shader preview");
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

        container(content)
            .padding(8)
            .class(cosmic::theme::Container::Card)
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

        // Frame rate dropdown (only for shaders)
        if matches!(self.selection.active, Choice::Shader(_)) {
            list = list.add(settings::item(
                fl!("frame-rate"),
                dropdown(
                    &self.frame_rate_options,
                    Some(self.selected_shader_frame_rate),
                    Message::ShaderFrameRate,
                ),
            ));
        }

        list.into()
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
    use cosmic::iced_core::{gradient::Linear, Background, Degrees};

    container(widget::Space::new(width, height))
        .class(cosmic::theme::Container::custom(move |theme| {
            container::Style {
                background: Some(match &color {
                    Color::Single([r, g, b]) => {
                        Background::Color(cosmic::iced::Color::from_rgb(*r, *g, *b))
                    }
                    Color::Gradient(Gradient { colors, radius }) => {
                        let stop_increment = 1.0 / (colors.len() - 1) as f32;
                        let mut stop = 0.0;
                        let mut linear = Linear::new(Degrees(*radius));
                        for &[r, g, b] in &**colors {
                            linear = linear.add_stop(stop, cosmic::iced::Color::from_rgb(r, g, b));
                            stop += stop_increment;
                        }
                        Background::Gradient(cosmic::iced_core::Gradient::Linear(linear))
                    }
                }),
                border: cosmic::iced_core::Border {
                    radius: theme.cosmic().corner_radii.radius_s.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        }))
        .into()
}

fn shader_placeholder<'a, M: 'a>(width: u16, height: u16) -> Element<'a, M> {
    use cosmic::iced_core::{gradient::Linear, Background, Degrees};

    container(widget::Space::new(width, height))
        .class(cosmic::theme::Container::custom(|_| container::Style {
            background: Some(Background::Gradient(cosmic::iced_core::Gradient::Linear(
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

fn discover_shaders() -> Vec<ShaderInfo> {
    let mut shaders = Vec::new();

    // Check XDG_DATA_DIRS
    if let Some(data_dirs) = std::env::var_os("XDG_DATA_DIRS") {
        if let Some(data_dirs) = data_dirs.to_str() {
            for data_dir in data_dirs.split(':') {
                let shader_dir = PathBuf::from(data_dir).join("glowberry/shaders");
                collect_shaders_from_dir(&shader_dir, &mut shaders);
                let cosmic_dir = PathBuf::from(data_dir).join("cosmic-bg/shaders");
                collect_shaders_from_dir(&cosmic_dir, &mut shaders);
            }
        }
    }

    // Standard paths
    for dir in &["/usr/share/glowberry/shaders", "/usr/share/cosmic-bg/shaders"] {
        collect_shaders_from_dir(std::path::Path::new(dir), &mut shaders);
    }

    shaders.sort_by(|a, b| a.name.cmp(&b.name));
    shaders.dedup_by(|a, b| a.name == b.name);
    shaders
}

fn collect_shaders_from_dir(dir: &std::path::Path, shaders: &mut Vec<ShaderInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "wgsl") {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| titlecase(&s.replace('_', " ")))
                .unwrap_or_else(|| "Unknown".to_string());
            shaders.push(ShaderInfo { path, name });
        }
    }
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

/// Menu actions for the application
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    About,
    Settings,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        match self {
            MenuAction::About => Message::ToggleContextPage(ContextPage::About),
            MenuAction::Settings => Message::ToggleContextPage(ContextPage::Settings),
        }
    }
}

/// Check if GlowBerry is currently set as the default cosmic-bg
fn is_glowberry_default() -> bool {
    // Check if /usr/local/bin/cosmic-bg exists and points to glowberry
    match std::fs::read_link("/usr/local/bin/cosmic-bg") {
        Ok(target) => {
            target.to_string_lossy().contains("glowberry")
        }
        Err(_) => false,
    }
}

/// Set or unset GlowBerry as the default cosmic-bg
/// This requires elevated privileges, so we use pkexec
async fn set_glowberry_default(enable: bool) -> Result<bool, String> {
    use tokio::process::Command;

    let script = if enable {
        // Create symlink to make glowberry the default
        r#"
            set -e
            ln -sf /usr/bin/glowberry /usr/local/bin/cosmic-bg
            # Restart cosmic-bg to apply the change
            pkill -f cosmic-bg || true
        "#
    } else {
        // Remove symlink to restore original cosmic-bg
        r#"
            set -e
            rm -f /usr/local/bin/cosmic-bg
            # Restart cosmic-bg to apply the change
            pkill -f cosmic-bg || true
        "#
    };

    let output = Command::new("pkexec")
        .args(["sh", "-c", script])
        .output()
        .await
        .map_err(|e| format!("Failed to run pkexec: {}", e))?;

    if output.status.success() {
        Ok(enable)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Command failed: {}", stderr))
    }
}
