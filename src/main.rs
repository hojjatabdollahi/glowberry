// SPDX-License-Identifier: MPL-2.0-only
mod img_source;

use std::{collections::VecDeque, convert::TryInto, path::PathBuf, time::Duration};

use cosmic_bg_config::{CosmicBgConfig, CosmicBgImgSource, CosmicBgOuput};
use image::{io::Reader as ImageReader, DynamicImage, ImageBuffer, RgbImage};
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    event_loop::WaylandSource,
    output::{OutputHandler, OutputInfo, OutputState},
    reexports::{
        calloop::{
            self,
            timer::{TimeoutAction, Timer},
        },
        client::{
            protocol::{
                wl_output::{self, WlOutput},
                wl_shm, wl_surface,
            },
            Connection, EventQueue, QueueHandle,
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::layer::{
        KeyboardInteractivity, Layer, LayerHandler, LayerState, LayerSurface, LayerSurfaceConfigure,
    },
    shm::{slot::SlotPool, ShmHandler, ShmState},
};
use walkdir::WalkDir;

fn main() -> anyhow::Result<()> {
    let conn = Connection::connect_to_env().unwrap();

    let mut event_loop = calloop::EventLoop::try_new()?;

    let event_queue: EventQueue<CosmicBg> = conn.new_event_queue();
    let qh = event_queue.handle();
    WaylandSource::new(event_queue)
        .unwrap()
        .insert(event_loop.handle())
        .unwrap();
    let config = CosmicBgConfig::load().unwrap_or_default();

    // initial setup with all imagesf
    let wallpapers = config
        .backgrounds
        .iter()
        .map(|bg| {
            let mut image_queue = VecDeque::new();
            let res: Result<PathBuf, _> = bg.source.clone().try_into();
            if let Ok(path_source) = res {
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

            let rotation_freq = bg.rotation_frequency;
            let cosmic_bg_clone = bg.output.clone();

            // set timer for rotation
            if rotation_freq > 0 {
                let _ = event_loop.handle().insert_source(
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
                                for layer in item.layers.iter_mut().filter(|l| !l.first_configure) {
                                    layer.layer.set_size(image.width(), image.height());
                                    layer.layer.wl_surface().commit();
                                }
                                item.cur_image.replace(image.into_rgb8());
                                item.new_image = true;
                                item.draw(&state.qh);
                                return TimeoutAction::ToDuration(Duration::from_secs(
                                    rotation_freq,
                                ));
                            }
                        }

                        TimeoutAction::Drop
                    },
                );
            }
            let new_image = cur_image.is_some();

            CosmicBgWallpaper {
                configured_output: bg.output.clone(),
                layers: Vec::new(),
                cur_image,
                image_queue,
                source: bg.source.clone(),
                filter_by_theme: bg.filter_by_theme,
                rotation_frequency: bg.rotation_frequency,
                new_image,
            }
        })
        .collect();

    let _source_txs = img_source::img_source(
        config
            .backgrounds
            .iter()
            .map(|c| c.source.clone())
            .collect(),
        event_loop.handle(),
    );

    // XXX All entry if it exists, should be placed last in the list of wallpapers
    let mut bg_state = CosmicBg {
        registry_state: RegistryState::new(&conn, &qh),
        output_state: OutputState::new(),
        compositor_state: CompositorState::new(),
        shm_state: ShmState::new(),
        layer_state: LayerState::new(),
        qh,

        exit: false,
        wallpapers,
        config,
    };

    while !bg_state.registry_state.ready() {
        event_loop
            .dispatch(Duration::from_millis(16), &mut bg_state)
            .unwrap();
    }

    loop {
        event_loop.dispatch(Duration::from_millis(16), &mut bg_state)?;

        if bg_state.exit {
            println!("exiting example");
            break;
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct CosmicBgWallpaper {
    configured_output: CosmicBgOuput,
    layers: Vec<CosmicBgLayer>,
    cur_image: Option<RgbImage>,
    image_queue: VecDeque<PathBuf>,
    source: CosmicBgImgSource,
    // TODO filter images by whether they seem to match dark / light mode
    // Alternatively only load from light / dark subdirectories given a directory source when this is active
    filter_by_theme: bool,
    rotation_frequency: u64,
    new_image: bool,
}

#[derive(Debug)]
pub struct CosmicBgLayer {
    layer: LayerSurface,
    wl_output: WlOutput,
    output_info: OutputInfo,
    pool: SlotPool,
    first_configure: bool,
    width: u32,
    height: u32,
}

#[derive(Debug)]
pub struct CosmicBg {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm_state: ShmState,
    layer_state: LayerState,
    qh: QueueHandle<CosmicBg>,

    exit: bool,
    wallpapers: Vec<CosmicBgWallpaper>,
    config: CosmicBgConfig,
}

impl CompositorHandler for CosmicBg {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

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
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        for wallpaper in &mut self.wallpapers {
            wallpaper.draw(qh);
        }
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

        let item = match self
            .wallpapers
            .iter_mut()
            .find(|w| match &w.configured_output {
                CosmicBgOuput::All => !w.layers.iter().any(|l| l.wl_output == wl_output),
                CosmicBgOuput::MakeModel { make, model } => {
                    make == &output_info.make
                        && model == &output_info.model
                        && !w.layers.iter().any(|l| l.wl_output == wl_output)
                }
            }) {
            Some(item) => item,
            None => return,
        };

        // let (width, height) = match item.cur_image.as_ref() {
        //     Some(img) => (
        //         output_info as u32,
        //         output_info.physical_size.1 as u32,
        //     ),
        //     None => (1, 1),
        // };

        let (width, height) = match output_info.modes.iter().find(|mode| mode.current) {
            Some(mode) => (mode.dimensions.0 as u32, mode.dimensions.1 as u32),
            None => (1, 1),
        };

        let surface = self.compositor_state.create_surface(qh).unwrap();

        let layer = LayerSurface::builder()
            .size((width, height))
            .keyboard_interactivity(KeyboardInteractivity::None)
            .namespace("wallpaper")
            .output(&wl_output)
            .map(qh, &self.layer_state, surface, Layer::Background)
            .expect("layer surface creation");

        item.layers.push(CosmicBgLayer {
            layer,
            wl_output,
            output_info,
            width,
            height,
            first_configure: false,
            pool: SlotPool::new(width as usize * height as usize * 4, &self.shm_state)
                .expect("Failed to create pool"),
        });
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
                CosmicBgOuput::All => true,
                CosmicBgOuput::MakeModel { make, model } => {
                    make == &output_info.make && model == &output_info.model
                }
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

impl LayerHandler for CosmicBg {
    fn layer_state(&mut self) -> &mut LayerState {
        &mut self.layer_state
    }

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
        for wallpaper in &mut self.wallpapers {
            let mut draw = false;
            for w_layer in &mut wallpaper.layers {
                if &w_layer.layer == layer && configure.new_size.0 != 0 && configure.new_size.1 != 0
                {
                    w_layer.width = configure.new_size.0;
                    w_layer.height = configure.new_size.1;
                    draw = true;
                    if w_layer.first_configure {
                        w_layer.first_configure = false;
                    }
                }
            }
            if draw {
                wallpaper.draw(qh);
            }
        }
    }
}

impl ShmHandler for CosmicBg {
    fn shm_state(&mut self) -> &mut ShmState {
        &mut self.shm_state
    }
}

impl CosmicBgWallpaper {
    pub fn draw(&mut self, qh: &QueueHandle<CosmicBg>) {
        if !self.new_image {
            return;
        }

        for layer in self.layers.iter_mut().filter(|l| !l.first_configure) {
            // println!("drawing {:?}", self.cur_image.as_ref().map(|img| img.as_raw()));

            let img = match self.cur_image.as_ref().map(|img| {
                image::imageops::resize(
                    img,
                    layer.width,
                    layer.height,
                    image::imageops::FilterType::Nearest,
                )
            }) {
                Some(img) => img,
                None => continue,
            };

            let width = layer.width;
            let height = layer.height;
            let stride = layer.width as i32 * 4;
            let pool = &mut layer.pool;

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
            layer.layer.wl_surface().frame(qh, wl_surface.clone());

            // Attach and commit to present.
            buffer.attach_to(wl_surface).expect("buffer attach");
            wl_surface.commit();
        }
        self.new_image = false;
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
    registry_handlers![CompositorState, OutputState, ShmState, LayerState,];
}
