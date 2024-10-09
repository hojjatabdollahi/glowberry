// SPDX-License-Identifier: MPL-2.0-only

use crate::{CosmicBg, CosmicBgLayer};

use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use cosmic_bg_config::{state::State, Color, Entry, SamplingMethod, ScalingMode, Source};
use cosmic_config::CosmicConfigEntry;
use eyre::{eyre, OptionExt};
use image::{DynamicImage, GrayAlphaImage, GrayImage, ImageReader, RgbImage, RgbaImage};
use jxl_oxide::{EnumColourEncoding, JxlImage, PixelFormat};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rand::{seq::SliceRandom, thread_rng};
use sctk::reexports::{
    calloop::{
        self,
        timer::{TimeoutAction, Timer},
        RegistrationToken,
    },
    client::QueueHandle,
};
use tracing::error;
use walkdir::WalkDir;

// TODO filter images by whether they seem to match dark / light mode
// Alternatively only load from light / dark subdirectories given a directory source when this is active

#[derive(Debug)]
pub struct Wallpaper {
    pub entry: Entry,
    pub layers: Vec<CosmicBgLayer>,
    pub image_queue: VecDeque<PathBuf>,
    loop_handle: calloop::LoopHandle<'static, CosmicBg>,
    queue_handle: QueueHandle<CosmicBg>,
    current_source: Option<Source>,
    // Cache of source image, if `current_source` is a `Source::Path`
    current_image: Option<image::DynamicImage>,
    timer_token: Option<RegistrationToken>,
}

impl Drop for Wallpaper {
    fn drop(&mut self) {
        if let Some(token) = self.timer_token.take() {
            self.loop_handle.remove(token);
        }
    }
}

impl Wallpaper {
    pub fn new(
        entry: Entry,
        queue_handle: QueueHandle<CosmicBg>,
        loop_handle: calloop::LoopHandle<'static, CosmicBg>,
        source_tx: calloop::channel::SyncSender<(String, notify::Event)>,
    ) -> Self {
        let mut wallpaper = Wallpaper {
            entry,
            layers: Vec::new(),
            current_source: None,
            current_image: None,
            image_queue: VecDeque::default(),
            timer_token: None,
            loop_handle,
            queue_handle,
        };

        wallpaper.load_images();
        wallpaper.register_timer();
        wallpaper.watch_source(source_tx);
        wallpaper
    }

    pub fn save_state(&self) -> Result<(), cosmic_config::Error> {
        let Some(cur_source) = self.current_source.clone() else {
            return Ok(());
        };
        let state_helper = State::state()?;
        let mut state = State::get_entry(&state_helper).unwrap_or_default();
        for l in &self.layers {
            let name = l.output_info.name.clone().unwrap_or_default();
            if let Some((_, source)) = state
                .wallpapers
                .iter_mut()
                .find(|(output, _)| *output == name)
            {
                *source = cur_source.clone();
            } else {
                state.wallpapers.push((name, cur_source.clone()))
            }
        }
        state.write_entry(&state_helper)
    }

    #[allow(clippy::too_many_lines)]
    pub fn draw(&mut self) {
        let start = Instant::now();
        let mut cur_resized_img: Option<DynamicImage> = None;

        for layer in self.layers.iter_mut().filter(|layer| layer.needs_redraw) {
            let Some(pool) = layer.pool.as_mut() else {
                continue;
            };

            let Some(fractional_scale) = layer.fractional_scale else {
                continue;
            };

            let Some((width, height)) = layer.size else {
                continue;
            };

            let width = width * fractional_scale / 120;
            let height = height * fractional_scale / 120;

            if cur_resized_img
                .as_ref()
                .map_or(true, |img| img.width() != width || img.height() != height)
            {
                let Some(source) = self.current_source.as_ref() else {
                    tracing::info!("No source for wallpaper");
                    continue;
                };

                cur_resized_img = match source {
                    Source::Path(ref path) => {
                        if self.current_image.is_none() {
                            self.current_image = Some(match path.extension() {
                                Some(ext) if ext == "jxl" => match decode_jpegxl(&path) {
                                    Ok(image) => image,
                                    Err(why) => {
                                        tracing::warn!(
                                            ?why,
                                            "jpegl-xl image decode failed: {}",
                                            path.display()
                                        );
                                        continue;
                                    }
                                },

                                _ => match ImageReader::open(&path) {
                                    Ok(img) => {
                                        match img
                                            .with_guessed_format()
                                            .ok()
                                            .and_then(|f| f.decode().ok())
                                        {
                                            Some(img) => img,
                                            None => {
                                                tracing::warn!(
                                                    "could not decode image: {}",
                                                    path.display()
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                    Err(_) => continue,
                                },
                            });
                        }
                        let img = self.current_image.as_ref().unwrap();

                        match self.entry.scaling_mode {
                            ScalingMode::Fit(color) => {
                                Some(crate::scaler::fit(img, &color, width, height))
                            }

                            ScalingMode::Zoom => Some(crate::scaler::zoom(img, width, height)),

                            ScalingMode::Stretch => {
                                Some(crate::scaler::stretch(img, width, height))
                            }
                        }
                    }

                    Source::Color(Color::Single([ref r, ref g, ref b])) => {
                        Some(image::DynamicImage::from(crate::colored::single(
                            [*r, *g, *b],
                            width,
                            height,
                        )))
                    }

                    Source::Color(Color::Gradient(ref gradient)) => {
                        match crate::colored::gradient(gradient, width, height) {
                            Ok(buffer) => Some(image::DynamicImage::from(buffer)),
                            Err(why) => {
                                tracing::error!(
                                    ?gradient,
                                    ?why,
                                    "color gradient in config is invalid"
                                );
                                None
                            }
                        }
                    }
                };
            }

            let image = cur_resized_img.as_ref().unwrap();
            let buffer_result =
                crate::draw::canvas(pool, image, width as i32, height as i32, width as i32 * 4);

            match buffer_result {
                Ok(buffer) => {
                    crate::draw::layer_surface(
                        layer,
                        &self.queue_handle,
                        &buffer,
                        (width as i32, height as i32),
                    );
                    layer.needs_redraw = false;

                    let elapsed = Instant::now().duration_since(start);

                    tracing::debug!(?elapsed, source = ?self.entry.source, "wallpaper draw");
                }

                Err(why) => {
                    tracing::error!(?why, "wallpaper could not be drawn");
                }
            }
        }
    }

    pub fn load_images(&mut self) {
        let mut image_queue = VecDeque::new();

        match self.entry.source {
            Source::Path(ref source) => {
                tracing::debug!(?source, "loading images");

                if let Ok(source) = source.canonicalize() {
                    if source.is_dir() {
                        if source.starts_with("/usr/share/backgrounds/") {
                            // Store paths of wallpapers to be used for the slideshow.
                            for img_path in WalkDir::new(source)
                                .follow_links(true)
                                .into_iter()
                                .filter_map(Result::ok)
                                .filter(|p| p.path().is_file())
                            {
                                image_queue.push_front(img_path.path().into());
                            }
                        } else if let Ok(dir) = source.read_dir() {
                            for entry in dir.filter_map(Result::ok) {
                                let Ok(path) = entry.path().canonicalize() else {
                                    continue;
                                };

                                if path.is_file() {
                                    image_queue.push_front(path);
                                }
                            }
                        }
                    } else if source.is_file() {
                        image_queue.push_front(source);
                    }
                }

                if image_queue.len() > 1 {
                    let image_slice = image_queue.make_contiguous();
                    match self.entry.sampling_method {
                        SamplingMethod::Alphanumeric => {
                            image_slice
                                .sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
                        }
                        SamplingMethod::Random => image_slice.shuffle(&mut thread_rng()),
                    };

                    // If a wallpaper from this slideshow was previously set, resume with that wallpaper.
                    if let Some(Source::Path(last_path)) = current_image(&self.entry.output) {
                        if image_queue.contains(&last_path) {
                            while let Some(path) = image_queue.pop_front() {
                                if path == last_path {
                                    image_queue.push_front(path);
                                    break;
                                }

                                image_queue.push_back(path);
                            }
                        }
                    }
                }

                image_queue.pop_front().map(|current_image_path| {
                    self.current_source = Some(Source::Path(current_image_path.clone()));
                    image_queue.push_back(current_image_path);
                });
            }

            Source::Color(ref c) => {
                self.current_source = Some(Source::Color(c.clone()));
            }
        };
        if let Err(err) = self.save_state() {
            error!("{err}");
        }
        self.image_queue = image_queue;
    }

    fn watch_source(&self, tx: calloop::channel::SyncSender<(String, notify::Event)>) {
        let Source::Path(ref source) = self.entry.source else {
            return;
        };

        let output = self.entry.output.clone();
        let mut watcher = match RecommendedWatcher::new(
            move |res| {
                if let Ok(e) = res {
                    let _ = tx.send((output.clone(), e));
                }
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(_) => return,
        };

        tracing::debug!(output = self.entry.output, "watching source");

        if let Ok(m) = fs::metadata(source) {
            if m.is_dir() {
                let _ = watcher.watch(source, RecursiveMode::Recursive);
            } else if m.is_file() {
                let _ = watcher.watch(source, RecursiveMode::NonRecursive);
            }
        }
    }

    fn register_timer(&mut self) {
        let rotation_freq = self.entry.rotation_frequency;
        let cosmic_bg_clone = self.entry.output.clone();
        // set timer for rotation
        if rotation_freq > 0 {
            self.timer_token = self
                .loop_handle
                .insert_source(
                    Timer::from_duration(Duration::from_secs(rotation_freq)),
                    move |_, _, state: &mut CosmicBg| {
                        let span = tracing::debug_span!("Wallpaper::timer");
                        let _handle = span.enter();

                        let Some(item) = state
                            .wallpapers
                            .iter_mut()
                            .find(|w| w.entry.output == cosmic_bg_clone)
                        else {
                            return TimeoutAction::Drop; // Drop if no item found for this timer
                        };

                        while let Some(next) = item.image_queue.pop_front() {
                            item.current_source = Some(Source::Path(next.clone()));
                            if let Err(err) = item.save_state() {
                                error!("{err}");
                            }

                            item.image_queue.push_back(next);
                            item.clear_image();
                            item.draw();

                            return TimeoutAction::ToDuration(Duration::from_secs(rotation_freq));
                        }

                        TimeoutAction::Drop
                    },
                )
                .ok();
        }
    }

    fn clear_image(&mut self) {
        self.current_image = None;
        for l in &mut self.layers {
            l.needs_redraw = true;
        }
    }
}

fn current_image(output: &str) -> Option<Source> {
    let state = State::state().ok()?;
    let mut wallpapers = State::get_entry(&state)
        .unwrap_or_default()
        .wallpapers
        .into_iter();

    let wallpaper = if output == "all" {
        wallpapers.next()
    } else {
        wallpapers.into_iter().find(|(name, _path)| name == output)
    };

    wallpaper.map(|(_name, path)| path)
}

/// Decodes JPEG XL image files into `image::DynamicImage` via `jxl-oxide`.
fn decode_jpegxl(path: &std::path::Path) -> eyre::Result<DynamicImage> {
    let mut image = JxlImage::builder()
        .open(path)
        .map_err(|why| eyre!("failed to read image header: {why}"))?;

    image.request_color_encoding(EnumColourEncoding::srgb(
        jxl_oxide::RenderingIntent::Relative,
    ));

    let render = image
        .render_frame(0)
        .map_err(|why| eyre!("failed to render image frame: {why}"))?;

    let framebuffer = render.image_all_channels();

    match image.pixel_format() {
        PixelFormat::Graya => GrayAlphaImage::from_raw(
            framebuffer.width() as u32,
            framebuffer.height() as u32,
            framebuffer
                .buf()
                .iter()
                .map(|x| x * 255. + 0.5)
                .map(|x| x as u8)
                .collect::<Vec<_>>(),
        )
        .map(DynamicImage::ImageLumaA8)
        .ok_or_eyre("Can't decode gray alpha buffer"),
        PixelFormat::Gray => GrayImage::from_raw(
            framebuffer.width() as u32,
            framebuffer.height() as u32,
            framebuffer
                .buf()
                .iter()
                .map(|x| x * 255. + 0.5)
                .map(|x| x as u8)
                .collect::<Vec<_>>(),
        )
        .map(DynamicImage::ImageLuma8)
        .ok_or_eyre("Can't decode gray buffer"),
        PixelFormat::Rgba => RgbaImage::from_raw(
            framebuffer.width() as u32,
            framebuffer.height() as u32,
            framebuffer
                .buf()
                .iter()
                .map(|x| x * 255. + 0.5)
                .map(|x| x as u8)
                .collect::<Vec<_>>(),
        )
        .map(DynamicImage::ImageRgba8)
        .ok_or_eyre("Can't decode rgba buffer"),
        PixelFormat::Rgb => RgbImage::from_raw(
            framebuffer.width() as u32,
            framebuffer.height() as u32,
            framebuffer
                .buf()
                .iter()
                .map(|x| x * 255. + 0.5)
                .map(|x| x as u8)
                .collect::<Vec<_>>(),
        )
        .map(DynamicImage::ImageRgb8)
        .ok_or_eyre("Can't decode rgb buffer"),
        //TODO: handle this
        PixelFormat::Cmyk => Err(eyre!("unsupported pixel format: CMYK")),
        PixelFormat::Cmyka => Err(eyre!("unsupported pixel format: CMYKA")),
    }
}
