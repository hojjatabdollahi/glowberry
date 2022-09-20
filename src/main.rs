// SPDX-License-Identifier: MPL-2.0-only

use std::{collections::VecDeque, convert::TryInto, path::PathBuf, time::{Duration, self}};

use cosmic_bg_config::{CosmicBgConfig, CosmicBgOuput};
use image::{io::Reader as ImageReader, DynamicImage};
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputInfo, OutputState},
    reexports::{
        calloop::{self, timer::{Timer, TimeoutAction}},
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

fn main() -> anyhow::Result<()> {
    let conn = Connection::connect_to_env().unwrap();

    let mut event_loop = calloop::EventLoop::try_new()?;
    let event_queue: EventQueue<CosmicBg> = conn.new_event_queue();
    let qh = event_queue.handle();
    let config = CosmicBgConfig::load().unwrap_or_default();

    // initial setup with all imagesf
    let wallpapers = config
        .backgrounds
        .iter()
        .map(|bg| {
            let mut image_queue = VecDeque::new();
            // TODO init the image paths
            let cur_image = image_queue.pop_front().and_then(|cur_image_path| {
                let img = match ImageReader::open(&cur_image_path) {
                    Ok(img) => match img.decode() {
                        Ok(img) => Some(img),
                        Err(_) => return None,
                    },
                    Err(_) => return None,
                };
                image_queue.push_back(cur_image_path);
                img
            });

            let rotation_freq = bg.rotation_frequency;
            let cosmic_bg_clone = bg.output.clone();

            // set timer for rotation
            if rotation_freq > 0 {
                let _ = event_loop.handle().insert_source(Timer::from_duration(Duration::from_secs(rotation_freq)), move |_, _, state: &mut CosmicBg| {
                    let item = match state
                    .wallpapers
                    .iter_mut()
                    .find(|w| w.configured_output == cosmic_bg_clone) {
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
                            item.image_queue.push_back(next.clone());
                            for layer in item.layers.iter_mut().filter(|l| !l.first_configure ) {
                                layer.layer.set_size(image.width(), image.height());
                                layer.layer.wl_surface().commit();
                            }
                            item.cur_image.replace(image);

                            return TimeoutAction::ToDuration(Duration::from_secs(rotation_freq));
                        }
                    }

                    TimeoutAction::Drop
                });
            }
            
            CosmicBgWallpaper {
                configured_output: bg.output.clone(),
                layers: Vec::new(),
                cur_image,
                image_queue,
                filter_by_theme: bg.filter_by_theme,
                rotation_frequency: bg.rotation_frequency,
                pool: None,
            }
        })
        .collect();

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
struct CosmicBgWallpaper {
    configured_output: CosmicBgOuput,
    layers: Vec<CosmicBgLayer>,
    cur_image: Option<DynamicImage>,
    image_queue: VecDeque<PathBuf>,
    filter_by_theme: bool,
    rotation_frequency: u64,
    pool: Option<SlotPool>,
}

#[derive(Debug)]
struct CosmicBgLayer {
    layer: LayerSurface,
    wl_output: WlOutput,
    output_info: OutputInfo,
    first_configure: bool,
    width: u32,
    height: u32,
}

#[derive(Debug)]
struct CosmicBg {
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

        let (width, height) = match item.cur_image.as_ref() {
            Some(img) => (
                img.width(),//.min(output_info.physical_size.0 as u32),
                img.height()//.min(output_info.physical_size.1 as u32),
            ),
            None => (1, 1),
        };

        if item.pool.is_none() {
            let pool = SlotPool::new(width as usize * height as usize * 4, &self.shm_state)
                .expect("Failed to create pool");
            item.pool.replace(pool);
        }

        let surface = self.compositor_state.create_surface(&qh).unwrap();

        let layer = LayerSurface::builder()
            .size((1, 1))
            .keyboard_interactivity(KeyboardInteractivity::None)
            .namespace("wallpaper")
            .map(&qh, &mut self.layer_state, surface, Layer::Background)
            .expect("layer surface creation");

        item.layers.push(CosmicBgLayer {
            layer,
            wl_output: output,
            output_info,
            width,
            height,
            first_configure: false,
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
        for layer in &self.layers {
            if layer.first_configure {
                continue;
            }

            let width = layer.width;
            let height = layer.height;
            let stride = layer.width as i32 * 4;
            let pool = self.pool.as_mut().unwrap();

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
                    .enumerate()
                    .for_each(|(index, chunk)| {
                        let x = (index % width as usize) as u32;
                        let y = (index / width as usize) as u32;

                        let a = 0xFF;
                        let r =
                            u32::min(((width - x) * 0xFF) / width, ((height - y) * 0xFF) / height);
                        let g = u32::min((x * 0xFF) / width, ((height - y) * 0xFF) / height);
                        let b = u32::min(((width - x) * 0xFF) / width, (y * 0xFF) / height);
                        let color = (a << 24) + (r << 16) + (g << 8) + b;

                        let array: &mut [u8; 4] = chunk.try_into().unwrap();
                        *array = color.to_le_bytes();
                    });
            }

            let wl_surface = layer.layer.wl_surface();
            // Damage the entire window
            wl_surface.damage_buffer(0, 0, width as i32, height as i32);

            // Request our next frame
            layer.layer.wl_surface().frame(qh, wl_surface.clone());

            // Attach and commit to present.
            buffer.attach_to(&wl_surface).expect("buffer attach");
            wl_surface.commit();

            // TODO save and reuse buffer when the window size is unchanged.  This is especially
            // useful if you do damage tracking, since you don't need to redraw the undamaged parts
            // of the canvas.
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
    registry_handlers![CompositorState, OutputState, ShmState, LayerState,];
}
