// SPDX-License-Identifier: MPL-2.0-only

use crate::{CosmicBg, CosmicBgLayer};

use std::{
    collections::{hash_map::DefaultHasher, VecDeque},
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    time::{Duration, Instant},
};

use cosmic_bg_config::{state::State, Color, Entry, SamplingMethod, ScalingMode, Source};
use cosmic_config::CosmicConfigEntry;
use image::{io::Reader as ImageReader, DynamicImage};
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
    current_image: Option<DynamicImage>,
    current_source: Option<Source>,
    timer_token: Option<RegistrationToken>,
    new_image: bool,
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
            current_image: None,
            current_source: None,
            image_queue: VecDeque::default(),
            new_image: false,
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

        let hash = self.current_image.as_ref().map(|image| {
            let mut hasher = DefaultHasher::new();
            image.as_bytes().hash(&mut hasher);
            hasher.finish()
        });

        for layer in self
            .layers
            .iter_mut()
            .filter(|layer| !layer.first_configure || layer.last_draw != hash)
        {
            let Some(pool) = layer.pool.as_mut() else {
                continue;
            };

            let original_image = self.current_image.as_ref().filter(|original| {
                original.width() == layer.width && original.height() == layer.height
            });

            let image = if let Some(original_image) = original_image {
                original_image
            } else {
                if cur_resized_img.as_ref().map_or(true, |img| {
                    img.width() != layer.width || img.height() != layer.height
                }) {
                    let CosmicBgLayer { width, height, .. } = *layer;

                    cur_resized_img = match self.entry.source {
                        Source::Path(_) => {
                            let Some(img) = self.current_image.as_ref() else {
                                continue;
                            };

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

                cur_resized_img.as_ref().unwrap()
            };

            layer.last_draw = hash;

            let buffer_result = crate::draw::canvas(
                pool,
                image,
                layer.width as i32,
                layer.height as i32,
                layer.width as i32 * 4,
            );

            match buffer_result {
                Ok(buffer) => {
                    crate::draw::layer_surface(layer, &self.queue_handle, &buffer);

                    let elapsed = Instant::now().duration_since(start);

                    tracing::debug!(?elapsed, source = ?self.entry.source, "wallpaper draw");
                }

                Err(why) => {
                    tracing::error!(?why, "wallpaper could not be drawn");
                }
            }
        }
    }

    fn load_images(&mut self) {
        let mut image_queue = VecDeque::new();

        let current_image = match self.entry.source {
            Source::Path(ref source) => {
                tracing::debug!(?source, "loading images");

                if source.is_dir() {
                    // Store paths of wallpapers to be used for the slideshow.
                    for img_path in WalkDir::new(source)
                        .follow_links(true)
                        .into_iter()
                        .filter_map(Result::ok)
                        .filter(|p| p.path().is_file())
                    {
                        image_queue.push_front(img_path.path().into());
                    }
                } else if source.is_file() {
                    image_queue.push_front(source.clone());
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
                    if let Ok(context) = cosmic_bg_config::context() {
                        if let Ok(last_path) = context.current_image(&self.entry.output) {
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
                }

                image_queue.pop_front().and_then(|current_image_path| {
                    set_current_image(&self.entry.output, current_image_path.clone());
                    self.current_source = Some(Source::Path(current_image_path.clone()));
                    let img = match ImageReader::open(&current_image_path) {
                        Ok(img) => match img.decode() {
                            Ok(img) => Some(img),
                            Err(_) => return None,
                        },
                        Err(_) => return None,
                    };
                    image_queue.push_back(current_image_path);
                    img
                })
            }

            Source::Color(ref c) => {
                self.current_source = Some(Source::Color(c.clone()));
                None
            }
        };
        if let Err(err) = self.save_state() {
            error!("{err}");
        }
        self.new_image = current_image.is_some();
        self.current_image = current_image;
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
            let output = self.entry.output.clone();
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

                            set_current_image(&output, next.clone());

                            let image = match ImageReader::open(&next) {
                                Ok(image) => match image.decode() {
                                    Ok(image) => image,
                                    Err(_) => continue,
                                },
                                Err(_) => continue,
                            };

                            item.image_queue.push_back(next);
                            item.current_image.replace(image);
                            item.new_image = true;
                            item.draw();

                            return TimeoutAction::ToDuration(Duration::from_secs(rotation_freq));
                        }

                        TimeoutAction::Drop
                    },
                )
                .ok();
        }
    }
}

fn set_current_image(output: &str, image: PathBuf) {
    if let Ok(context) = cosmic_bg_config::context() {
        let _res = context.set_current_image(output, image);
    }
}
