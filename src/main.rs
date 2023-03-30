// SPDX-License-Identifier: MPL-2.0-only
mod img_source;

use std::{
    collections::{hash_map::DefaultHasher, VecDeque},
    fs,
    hash::Hash,
    hash::Hasher,
    num::NonZeroU32,
    path::PathBuf,
    time::Duration,
};

use cosmic_bg_config::{
    CosmicBgConfig, CosmicBgEntry, CosmicBgOutput, FilterMethod, SamplingMethod, ScalingMode,
};
use cosmic_config::ConfigGet;
use fast_image_resize as fr;
use image::{io::Reader as ImageReader, Pixel, RgbImage};
use itertools::Itertools;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rand::{seq::SliceRandom, thread_rng};
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputInfo, OutputState},
    reexports::{
        calloop::{
            self,
            timer::{TimeoutAction, Timer},
            EventLoop, RegistrationToken,
        },
        client::{
            globals::registry_queue_init,
            protocol::{
                wl_output::{self, WlOutput},
                wl_shm, wl_surface,
            },
            Connection, QueueHandle, WaylandSource,
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub enum BgConfigUpdate {
    NewConfig(CosmicBgConfig),
    NewEntry(CosmicBgEntry),
}

fn main() -> anyhow::Result<()> {
    let conn = Connection::connect_to_env().unwrap();

    let mut event_loop: EventLoop<'static, CosmicBg> = calloop::EventLoop::try_new()?;

    let (globals, event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();
    WaylandSource::new(event_queue)
        .unwrap()
        .insert(event_loop.handle())
        .unwrap();

    let config_helper = CosmicBgConfig::helper();

    let (cfg_tx, cfg_rx) = calloop::channel::sync_channel(20);

    event_loop
        .handle()
        .insert_source(cfg_rx, |e, _, state| {
            match e {
                calloop::channel::Event::Msg(BgConfigUpdate::NewConfig(config)) => {
                    if state.config != config {
                        state.apply_config(config);
                    }
                }
                calloop::channel::Event::Msg(BgConfigUpdate::NewEntry(entry)) => {
                    if let Some(wallpaper) = state
                        .wallpapers
                        .iter_mut()
                        .find(|w| w.entry.output == entry.output)
                    {
                        wallpaper.apply_entry(entry, state.source_tx.clone());
                    }
                }
                calloop::channel::Event::Closed => {
                    // TODO log drop
                }
            }
        })
        .unwrap();

    // TODO: this could be so nice with `inspect_err`, but that is behind the unstable feature `result_option_inspect` right now
    let (config, _watcher) = match config_helper.as_ref() {
        Ok(helper) => {
            let watcher = helper
                .watch(move |config_helper, keys| {
                    for key in keys.iter() {
                        if key == cosmic_bg_config::BG_KEY {
                            let new_config = CosmicBgConfig::load(config_helper).unwrap();
                            cfg_tx.send(BgConfigUpdate::NewConfig(new_config)).unwrap();
                        } else if let Ok(entry) = config_helper.get::<CosmicBgEntry>(key) {
                            cfg_tx.send(BgConfigUpdate::NewEntry(entry)).unwrap();
                        }
                    }
                })
                .unwrap();

            (
                match CosmicBgConfig::load(helper) {
                    Ok(conf) => conf,
                    Err(err) => {
                        eprintln!("Config file error, falling back to defaults: {err:?}");
                        CosmicBgConfig::default()
                    }
                },
                Some(watcher),
            )
        }
        Err(err) => {
            eprintln!("Config file error, falling back to defaults: {err:?}");
            (CosmicBgConfig::default(), None)
        }
    };

    let source_tx = img_source::img_source(event_loop.handle());
    // initial setup with all images
    let mut wallpapers = config
        .backgrounds
        .iter()
        .map(|bg| {
            CosmicBgWallpaper::new(
                bg.clone(),
                qh.clone(),
                event_loop.handle(),
                source_tx.clone(),
            )
        })
        .collect_vec();
    // XXX All entry if it exists, should be placed last in the list of wallpapers
    wallpapers.sort_by(|a, b| a.entry.output.cmp(&b.entry.output));

    let mut bg_state = CosmicBg {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state: CompositorState::bind(&globals, &qh).unwrap(),
        shm_state: Shm::bind(&globals, &qh).unwrap(),
        layer_state: LayerShell::bind(&globals, &qh).unwrap(),
        qh,
        source_tx,
        loop_handle: event_loop.handle(),
        exit: false,
        wallpapers,
        config,
        active_outputs: Vec::new(),
    };

    loop {
        event_loop.dispatch(None, &mut bg_state)?;

        if bg_state.exit {
            break;
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct CosmicBgLayer {
    layer: LayerSurface,
    wl_output: WlOutput,
    output_info: OutputInfo,
    pool: Option<SlotPool>,
    first_configure: bool,
    last_draw: Option<u64>,
    width: u32,
    height: u32,
}

#[derive(Debug)]
pub struct CosmicBg {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm_state: Shm,
    layer_state: LayerShell,
    qh: QueueHandle<CosmicBg>,
    source_tx: calloop::channel::SyncSender<(CosmicBgOutput, notify::Event)>,
    loop_handle: calloop::LoopHandle<'static, CosmicBg>,
    exit: bool,
    wallpapers: Vec<CosmicBgWallpaper>,
    config: CosmicBgConfig,
    active_outputs: Vec<WlOutput>,
}

impl CosmicBg {
    fn apply_config(&mut self, mut config: CosmicBgConfig) {
        let mut existing_layers = Vec::new();
        self.wallpapers.retain_mut(|w| {
            if let Some(pos) = config
                .backgrounds
                .iter_mut()
                .position(|new_w| new_w.output == w.entry.output)
            {
                let _not_new = config.backgrounds.remove(pos);
                true
            } else {
                existing_layers.append(&mut w.layers);
                false
            }
        });

        for w in config.backgrounds {
            let mut new_wallpaper = CosmicBgWallpaper::new(
                w,
                self.qh.clone(),
                self.loop_handle.clone(),
                self.source_tx.clone(),
            );
            // reuse existing layers from the `All` wallpaper if possible
            if let Some(l) = self.wallpapers.last_mut().and_then(|w| {
                if let Some(pos) = w.layers.iter().position(|l| {
                    let o_name = l.output_info.name.clone().unwrap_or_default();
                    &new_wallpaper.entry.output == &CosmicBgOutput::Name(o_name)
                }) {
                    Some(w.layers.remove(pos))
                } else {
                    None
                }
            }) {
                new_wallpaper.layers.push(l);
            // create a new layer if there is an existing output that matches the added wallpaper
            } else if let Some((output, output_info)) = self.active_outputs.iter().find_map(|o| {
                let output_info = match self.output_state.info(&o) {
                    Some(info) => info,
                    None => return None,
                };
                let o_name = output_info.name.clone().unwrap_or_default();
                if &new_wallpaper.entry.output == &CosmicBgOutput::Name(o_name) {
                    Some((o.clone(), output_info.clone()))
                } else {
                    None
                }
            }) {
                new_wallpaper
                    .layers
                    .push(self.new_layer(output, output_info));
            };
            self.wallpapers.push(new_wallpaper);
            self.wallpapers
                .sort_by(|a, b| a.entry.output.cmp(&b.entry.output));
        }
    }

    pub fn new_layer(&self, output: WlOutput, output_info: OutputInfo) -> CosmicBgLayer {
        let (width, height) = output_info.logical_size.unwrap_or((0, 0));
        let (width, height) = (width as u32, height as u32);

        let surface = self.compositor_state.create_surface(&self.qh);

        let layer = self.layer_state.create_layer_surface(
            &self.qh,
            surface.clone(),
            Layer::Background,
            "wallpaper".into(),
            Some(&output),
        );
        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(width, height);
        surface.commit();
        CosmicBgLayer {
            layer,
            wl_output: output,
            output_info,
            width,
            height,
            first_configure: false,
            last_draw: None,
            pool: None,
        }
    }
}

impl CompositorHandler for CosmicBg {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
        // Not needed for this example.
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }
}

impl OutputHandler for CosmicBg {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        wl_output: wl_output::WlOutput,
    ) {
        self.active_outputs.push(wl_output.clone());
        let output_info = match self.output_state.info(&wl_output) {
            Some(info) => info,
            None => return,
        };

        if let Some(pos) = self.wallpapers.iter().position(|w| match &w.entry.output {
            CosmicBgOutput::All => !w.layers.iter().any(|l| l.wl_output == wl_output),
            CosmicBgOutput::Name(name) => {
                Some(name) == output_info.name.as_ref()
                    && !w.layers.iter().any(|l| l.wl_output == wl_output)
            }
        }) {
            let layer = self.new_layer(wl_output, output_info);
            self.wallpapers[pos].layers.push(layer);
        }
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        // TODO
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.active_outputs.retain(|o| o != &output);
        let output_info = match self.output_state.info(&output) {
            Some(info) => info,
            None => return,
        };

        let item = match self.wallpapers.iter_mut().find(|w| match &w.entry.output {
            CosmicBgOutput::All => true,
            CosmicBgOutput::Name(name) => Some(name) == output_info.name.as_ref(),
        }) {
            Some(item) => item,
            None => return,
        };

        let layer_position = match item
            .layers
            .iter()
            .position(|bg_layer| bg_layer.wl_output == output)
        {
            Some(layer) => layer,
            None => return,
        };
        item.layers.remove(layer_position);
    }
}

impl LayerShellHandler for CosmicBg {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        for wallpaper in self.wallpapers.iter_mut() {
            let (w, h) = configure.new_size;
            if let Some(w_layer) = wallpaper.layers.iter_mut().find(|l| &l.layer == layer) {
                w_layer.width = w;
                w_layer.height = h;
                if let Some(pool) = w_layer.pool.as_mut() {
                    pool.resize(w as usize * h as usize * 4)
                        .expect("failed to resize the pool");
                } else {
                    w_layer.pool.replace(
                        SlotPool::new(w as usize * h as usize * 4, &self.shm_state)
                            .expect("Failed to create pool"),
                    );
                }
                if w_layer.first_configure {
                    w_layer.first_configure = false;
                }
                if wallpaper.layers.iter().all(|l| !l.first_configure) {
                    wallpaper.draw();
                }
                break;
            }
        }
    }
}

impl ShmHandler for CosmicBg {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

#[derive(Debug)]
pub struct CosmicBgWallpaper {
    layers: Vec<CosmicBgLayer>,
    cur_image: Option<fr::Image<'static>>,
    image_queue: VecDeque<PathBuf>,
    entry: CosmicBgEntry,
    // TODO filter images by whether they seem to match dark / light mode
    // Alternatively only load from light / dark subdirectories given a directory source when this is active
    new_image: bool,
    _watcher: Option<RecommendedWatcher>,
    timer_token: Option<RegistrationToken>,
    loop_handle: calloop::LoopHandle<'static, CosmicBg>,
    qh: QueueHandle<CosmicBg>,
}

impl Drop for CosmicBgWallpaper {
    fn drop(&mut self) {
        if let Some(token) = self.timer_token.take() {
            self.loop_handle.remove(token);
        }
    }
}

impl CosmicBgWallpaper {
    pub fn new(
        entry: CosmicBgEntry,
        qh: QueueHandle<CosmicBg>,
        loop_handle: calloop::LoopHandle<'static, CosmicBg>,
        source_tx: calloop::channel::SyncSender<(CosmicBgOutput, notify::Event)>,
    ) -> Self {
        let mut wallpaper = CosmicBgWallpaper {
            entry: entry.clone(),
            layers: Vec::new(),
            cur_image: None,
            image_queue: Default::default(),
            new_image: false,
            _watcher: None,
            timer_token: None,
            loop_handle,
            qh,
        };
        wallpaper.load_images();
        wallpaper.register_timer();
        wallpaper.watch_source(source_tx.clone());
        wallpaper
    }

    pub fn draw(&mut self) {
        let mut cur_img: Option<RgbImage> = None;
        let hash = self.cur_image.as_ref().map(|img| {
            let mut hasher = DefaultHasher::new();
            img.buffer().hash(&mut hasher);
            hasher.finish()
        });
        let mut resizer = fr::Resizer::new(match self.entry.filter_method {
            FilterMethod::Nearest => fr::ResizeAlg::Nearest,
            FilterMethod::Linear => fr::ResizeAlg::Convolution(fr::FilterType::Bilinear),
            FilterMethod::Lanczos => fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3),
        });
        for layer in self
            .layers
            .iter_mut()
            .filter(|l| !l.first_configure && l.last_draw != hash)
        {
            let pool = match layer.pool.as_mut() {
                Some(p) => p,
                None => continue,
            };
            if cur_img
                .as_ref()
                .map(|img| img.width() != layer.width as u32 || img.height() != layer.height as u32)
                .unwrap_or(true)
            {
                cur_img = match self.cur_image.as_ref() {
                    Some(img) => match self.entry.scaling_mode {
                        ScalingMode::Fit(color) => {
                            let u8_color = [
                                (u8::MAX as f32 * color[0]).round() as u8,
                                (u8::MAX as f32 * color[1]).round() as u8,
                                (u8::MAX as f32 * color[2]).round() as u8,
                            ];
                            let mut final_image = image::ImageBuffer::from_pixel(
                                layer.width,
                                layer.height,
                                *image::Rgb::from_slice(&u8_color),
                            );
                            let (w, h) = (img.width().get(), img.height().get());

                            let ratio =
                                (layer.width as f64 / w as f64).min(layer.height as f64 / h as f64);
                            let (new_width, new_height) = (
                                (w as f64 * ratio).round() as u32,
                                (h as f64 * ratio).round() as u32,
                            );

                            let mut dst_image = fr::Image::new(
                                NonZeroU32::new(new_width).unwrap(),
                                NonZeroU32::new(new_width).unwrap(),
                                fr::PixelType::U8x3,
                            );

                            let mut dst_view = dst_image.view_mut();
                            resizer.resize(&img.view(), &mut dst_view).unwrap();

                            let dst_image =
                                RgbImage::from_raw(layer.width, layer.height, dst_image.into_vec())
                                    .unwrap();
                            image::imageops::replace(
                                &mut final_image,
                                &dst_image,
                                ((layer.width - new_width) / 2).into(),
                                ((layer.height - new_height) / 2).into(),
                            );

                            Some(final_image)
                        }
                        ScalingMode::Zoom => {
                            let (w, h) = (img.width().get(), img.height().get());
                            let ratio =
                                (layer.width as f64 / w as f64).max(layer.height as f64 / h as f64);
                            let (new_width, new_height) = (
                                (w as f64 * ratio).round() as u32,
                                (h as f64 * ratio).round() as u32,
                            );
                            let mut dst_image = fr::Image::new(
                                NonZeroU32::new(new_width).unwrap(),
                                NonZeroU32::new(new_height).unwrap(),
                                fr::PixelType::U8x3,
                            );
                            let mut dst_view = dst_image.view_mut();
                            resizer.resize(&img.view(), &mut dst_view).unwrap();

                            let mut dst_image =
                                RgbImage::from_raw(new_width, new_height, dst_image.into_vec())
                                    .unwrap();

                            Some(
                                image::imageops::crop(
                                    &mut dst_image,
                                    (new_width - layer.width) / 2,
                                    (new_height - layer.height) / 2,
                                    layer.width,
                                    layer.height,
                                )
                                .to_image(),
                            )
                        }
                        ScalingMode::Stretch => {
                            let mut dst_image = fr::Image::new(
                                NonZeroU32::new(layer.width).unwrap(),
                                NonZeroU32::new(layer.height).unwrap(),
                                fr::PixelType::U8x3,
                            );
                            let mut dst_view = dst_image.view_mut();
                            resizer.resize(&img.view(), &mut dst_view).unwrap();

                            let dst_image =
                                RgbImage::from_raw(layer.width, layer.height, dst_image.into_vec())
                                    .unwrap();
                            Some(dst_image)
                        }
                    },
                    None => continue,
                };
            }

            let img = cur_img.as_ref().unwrap();
            let width = layer.width;
            let height = layer.height;
            let stride = layer.width as i32 * 4;
            layer.last_draw = hash;

            let (buffer, canvas) = pool
                .create_buffer(
                    width as i32,
                    height as i32,
                    stride,
                    wl_shm::Format::Xrgb8888,
                )
                .expect("create buffer");
            // Draw to the window:
            {
                canvas
                    .chunks_exact_mut(4)
                    .zip(img.pixels())
                    .for_each(|(dest, source)| {
                        dest[2] = source.0[0].to_le();
                        dest[1] = source.0[1].to_le();
                        dest[0] = source.0[2].to_le();
                    });
            }

            let wl_surface = layer.layer.wl_surface();
            // Damage the entire window
            wl_surface.damage_buffer(0, 0, width as i32, height as i32);

            // Request our next frame
            layer.layer.wl_surface().frame(&self.qh, wl_surface.clone());

            // Attach and commit to present.
            buffer.attach_to(wl_surface).expect("buffer attach");
            wl_surface.commit();
        }
    }

    fn load_images(&mut self) {
        println!("Loading images from {:?}", self.entry.source);
        let mut image_queue = VecDeque::new();
        if self.entry.source.is_dir() {
            for img_path in WalkDir::new(&self.entry.source)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|p| p.path().is_file())
            {
                image_queue.push_front(img_path.path().into());
            }
        } else if self.entry.source.is_file() {
            image_queue.push_front(self.entry.source.clone());
        }
        {
            let image_slice = image_queue.make_contiguous();
            match self.entry.sampling_method {
                SamplingMethod::Alphanumeric => {
                    image_slice.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()))
                }
                SamplingMethod::Random => image_slice.shuffle(&mut thread_rng()),
            };
        }

        let cur_image = image_queue.pop_front().and_then(|cur_image_path| {
            let img = match ImageReader::open(&cur_image_path) {
                Ok(img) => match img.decode() {
                    Ok(img) => Some(img),
                    Err(_) => return None,
                },
                Err(_) => return None,
            };
            image_queue.push_back(cur_image_path);
            img.map(|img| img.into_rgb8()).and_then(|img| {
                fr::Image::from_vec_u8(
                    NonZeroU32::new(img.width()).unwrap(),
                    NonZeroU32::new(img.height()).unwrap(),
                    img.into_vec(),
                    fr::PixelType::U8x3,
                )
                .ok()
            })
        });

        self.new_image = cur_image.is_some();
        self.cur_image = cur_image;
        self.image_queue = image_queue;
    }

    fn watch_source(&self, tx: calloop::channel::SyncSender<(CosmicBgOutput, notify::Event)>) {
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

        let source = self.entry.source.as_path();
        if let Ok(m) = fs::metadata(&source) {
            if m.is_dir() {
                let _ = watcher.watch(&source, RecursiveMode::Recursive);
            } else if m.is_file() {
                let _ = watcher.watch(&source, RecursiveMode::NonRecursive);
            }
        }
    }

    fn apply_entry(
        &mut self,
        config: CosmicBgEntry,
        tx: calloop::channel::SyncSender<(CosmicBgOutput, notify::Event)>,
    ) {
        if config.output == self.entry.output && self.entry != config {
            let src_changed = config.source != self.entry.source;
            self.entry = config;
            if src_changed {
                self.load_images();
                self.watch_source(tx);
            }
            self.draw();
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
                        let item = match state
                            .wallpapers
                            .iter_mut()
                            .find(|w| w.entry.output == cosmic_bg_clone)
                        {
                            Some(item) => item,
                            None => return TimeoutAction::Drop, // Drop if no item found for this timer
                        };

                        let mut img = None;

                        while img.is_none() && item.image_queue.front().is_some() {
                            let next = item.image_queue.pop_front().unwrap();

                            img = match ImageReader::open(&next) {
                                Ok(img) => match img.decode() {
                                    Ok(img) => Some(img),
                                    Err(_) => None,
                                },
                                Err(_) => None,
                            };
                            if let Some(image) =
                                img.take().map(|img| img.into_rgb8()).and_then(|img| {
                                    fr::Image::from_vec_u8(
                                        NonZeroU32::new(img.width()).unwrap(),
                                        NonZeroU32::new(img.height()).unwrap(),
                                        img.into_vec(),
                                        fr::PixelType::U8x3,
                                    )
                                    .ok()
                                })
                            {
                                item.image_queue.push_back(next);
                                item.cur_image.replace(image);
                                item.new_image = true;
                                item.draw();
                                return TimeoutAction::ToDuration(Duration::from_secs(
                                    rotation_freq,
                                ));
                            }
                        }

                        TimeoutAction::Drop
                    },
                )
                .ok();
        }
    }
}

delegate_compositor!(CosmicBg);
delegate_output!(CosmicBg);
delegate_shm!(CosmicBg);

delegate_layer!(CosmicBg);

delegate_registry!(CosmicBg);

impl ProvidesRegistryState for CosmicBg {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}
