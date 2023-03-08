// SPDX-License-Identifier: MPL-2.0-only
mod img_source;

use std::{collections::VecDeque, fs, path::PathBuf, time::Duration};

use cosmic_bg_config::{
    CosmicBgConfig, CosmicBgEntry, CosmicBgImgSource, CosmicBgOutput, FilterMethod, SamplingMethod,
    ScalingMode,
};
use cosmic_config::ConfigGet;
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
                    state.apply_config(config);
                }
                calloop::channel::Event::Msg(BgConfigUpdate::NewEntry(entry)) => {
                    if let Some(wallpaper) = state
                        .wallpapers
                        .iter_mut()
                        .find(|w| w.configured_output == entry.output)
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
                    println!("Changed: {:?}", keys);
                    for key in keys.iter() {
                        println!(" - {} = {:?}", key, config_helper.get::<ron::Value>(key));
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
    let wallpapers = config
        .backgrounds
        .into_iter()
        .map(|bg| CosmicBgWallpaper::new(bg, qh.clone(), event_loop.handle(), source_tx.clone()))
        .collect_vec();

    // XXX All entry if it exists, should be placed last in the list of wallpapers
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
    _output_info: OutputInfo,
    pool: Option<SlotPool>,
    first_configure: bool,
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
}

impl CosmicBg {
    fn apply_config(&mut self, mut config: CosmicBgConfig) {
        self.wallpapers.retain_mut(|w| {
            if let Some(pos) = config
                .backgrounds
                .iter_mut()
                .position(|new_w| new_w.output == w.configured_output)
            {
                let _not_new = config.backgrounds.remove(pos);
                true
            } else {
                false
            }
        });

        for w in config.backgrounds {
            self.wallpapers.push(CosmicBgWallpaper::new(
                w,
                self.qh.clone(),
                self.loop_handle.clone(),
                self.source_tx.clone(),
            ));
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
        qh: &QueueHandle<Self>,
        wl_output: wl_output::WlOutput,
    ) {
        let output_info = match self.output_state.info(&wl_output) {
            Some(info) => info,
            None => return,
        };

        match self
            .wallpapers
            .iter_mut()
            .find(|w| match &w.configured_output {
                CosmicBgOutput::All => !w.layers.iter().any(|l| l.wl_output == wl_output),
                CosmicBgOutput::Name(name) => {
                    Some(name) == output_info.name.as_ref()
                        && !w.layers.iter().any(|l| l.wl_output == wl_output)
                }
            }) {
            Some(item) => {
                let (width, height) = output_info.logical_size.unwrap_or((0, 0));
                let (width, height) = (width as u32, height as u32);

                let surface = self.compositor_state.create_surface(qh);

                let layer = self.layer_state.create_layer_surface(
                    qh,
                    surface.clone(),
                    Layer::Background,
                    "wallpaper".into(),
                    Some(&wl_output),
                );
                layer.set_anchor(Anchor::all());
                layer.set_exclusive_zone(-1);
                layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                layer.set_size(width, height);
                surface.commit();
                item.layers.push(CosmicBgLayer {
                    layer,
                    wl_output,
                    _output_info: output_info,
                    width,
                    height,
                    first_configure: false,
                    pool: None,
                });
            }
            None => return,
        };
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
        let output_info = match self.output_state.info(&output) {
            Some(info) => info,
            None => return,
        };

        let item = match self
            .wallpapers
            .iter_mut()
            .find(|w| match &w.configured_output {
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
        qh: &QueueHandle<Self>,
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
    configured_output: CosmicBgOutput,
    layers: Vec<CosmicBgLayer>,
    cur_image: Option<RgbImage>,
    image_queue: VecDeque<PathBuf>,
    source: CosmicBgImgSource,
    sampling_method: SamplingMethod,
    // TODO filter images by whether they seem to match dark / light mode
    // Alternatively only load from light / dark subdirectories given a directory source when this is active
    _filter_by_theme: bool,
    rotation_frequency: u64,
    filter: image::imageops::FilterType,
    scaling: ScalingMode,
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
        let filter = match entry.filter_method {
            FilterMethod::Nearest => image::imageops::Nearest,
            FilterMethod::Linear => image::imageops::Triangle,
            FilterMethod::Lanczos => image::imageops::Lanczos3,
        };

        let mut wallpaper = CosmicBgWallpaper {
            configured_output: entry.output.clone(),
            layers: Vec::new(),
            cur_image: None,
            image_queue: Default::default(),
            source: entry.source,
            _filter_by_theme: entry.filter_by_theme,
            rotation_frequency: entry.rotation_frequency,
            sampling_method: entry.sampling_method,
            new_image: false,
            filter,
            scaling: entry.scaling_mode.clone(),
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
        for layer in self.layers.iter_mut().filter(|l| !l.first_configure) {
            let img = match self.cur_image.as_ref() {
                Some(img) => match self.scaling {
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

                        let ratio = (layer.width as f64 / img.width() as f64)
                            .min(layer.height as f64 / img.height() as f64);
                        let (new_width, new_height) = (
                            (img.width() as f64 * ratio).round() as u32,
                            (img.height() as f64 * ratio).round() as u32,
                        );
                        let new_image =
                            image::imageops::resize(img, new_width, new_height, self.filter);
                        image::imageops::replace(
                            &mut final_image,
                            &new_image,
                            ((layer.width - new_width) / 2).into(),
                            ((layer.height - new_height) / 2).into(),
                        );

                        final_image
                    }
                    ScalingMode::Zoom => {
                        let ratio = (layer.width as f64 / img.width() as f64)
                            .max(layer.height as f64 / img.height() as f64);
                        let (new_width, new_height) = (
                            (img.width() as f64 * ratio).round() as u32,
                            (img.height() as f64 * ratio).round() as u32,
                        );
                        let mut new_image =
                            image::imageops::resize(img, new_width, new_height, self.filter);
                        image::imageops::crop(
                            &mut new_image,
                            (new_width - layer.width) / 2,
                            (new_height - layer.height) / 2,
                            layer.width,
                            layer.height,
                        )
                        .to_image()
                    }
                    ScalingMode::Stretch => {
                        image::imageops::resize(img, layer.width, layer.height, self.filter)
                    }
                },
                None => continue,
            };

            let width = layer.width;
            let height = layer.height;
            let stride = layer.width as i32 * 4;

            let pool = match layer.pool.as_mut() {
                Some(p) => p,
                None => continue,
            };

            let (buffer, canvas) = pool
                .create_buffer(
                    width as i32,
                    height as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                )
                .expect("create buffer");
            // Draw to the window:
            {
                canvas
                    .chunks_exact_mut(4)
                    .zip(img.pixels())
                    .for_each(|(dest, source)| {
                        dest[3] = 0xFF_u8.to_le();
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
        let mut image_queue = VecDeque::new();
        let path_source: PathBuf = self.source.clone().into();
        if path_source.is_dir() {
            for img_path in WalkDir::new(&path_source)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|p| p.path().is_file())
            {
                image_queue.push_front(img_path.path().into());
            }
        } else if path_source.is_file() {
            image_queue.push_front(path_source);
        }
        {
            let image_slice = image_queue.make_contiguous();
            match self.sampling_method {
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
            img.map(|img| img.into_rgb8())
        });

        self.new_image = cur_image.is_some();
        self.cur_image = cur_image;
        self.image_queue = image_queue;
    }

    fn watch_source(&self, tx: calloop::channel::SyncSender<(CosmicBgOutput, notify::Event)>) {
        let output = self.configured_output.clone();
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

        let source: PathBuf = self.source.clone().into();
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
        if config.output == self.configured_output {
            self._filter_by_theme = config.filter_by_theme;
            self.rotation_frequency = config.rotation_frequency;
            self.filter = match config.filter_method {
                FilterMethod::Nearest => image::imageops::Nearest,
                FilterMethod::Linear => image::imageops::Triangle,
                FilterMethod::Lanczos => image::imageops::Lanczos3,
            };
            self.scaling = config.scaling_mode.clone();
            if config.source != self.source {
                self.source = config.source.clone().into();
                self.load_images();
                self.watch_source(tx);
            }
            self.draw();
        }
    }

    fn register_timer(&mut self) {
        let rotation_freq = self.rotation_frequency;
        let cosmic_bg_clone = self.configured_output.clone();
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
                            .find(|w| w.configured_output == cosmic_bg_clone)
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

                            if let Some(image) = img.take() {
                                item.image_queue.push_back(next);
                                item.cur_image.replace(image.into_rgb8());
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
