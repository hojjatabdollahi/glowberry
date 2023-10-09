// SPDX-License-Identifier: MPL-2.0-only

mod colored;
mod draw;
mod img_source;
mod scaler;
mod wallpaper;

use cosmic_bg_config::{state::State, Config, Entry};
use cosmic_config::{calloop::ConfigWatchSource, ConfigGet, CosmicConfigEntry};
use eyre::Context;
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputInfo, OutputState},
    reexports::{
        calloop,
        calloop_wayland_source::WaylandSource,
        client::{
            globals::registry_queue_init,
            protocol::{
                wl_output::{self, WlOutput},
                wl_surface,
            },
            Connection, QueueHandle,
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::wlr_layer::{
        Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
        LayerSurfaceConfigure,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use tracing::error;
use tracing_subscriber::prelude::*;
use wallpaper::Wallpaper;

#[derive(Debug)]
pub struct CosmicBgLayer {
    layer: LayerSurface,
    wl_output: WlOutput,
    output_info: OutputInfo,
    pool: Option<SlotPool>,
    first_configure: bool,
    width: u32,
    height: u32,
    last_draw: Option<u64>,
}

#[allow(clippy::too_many_lines)]
fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    if std::env::var("RUST_SPANTRACE").is_err() {
        std::env::set_var("RUST_SPANTRACE", "0");
    }

    init_logger();

    let conn = Connection::connect_to_env().wrap_err("wayland client connection failed")?;

    let mut event_loop: calloop::EventLoop<'static, CosmicBg> =
        calloop::EventLoop::try_new().wrap_err("failed to create event loop")?;

    let (globals, event_queue) =
        registry_queue_init(&conn).wrap_err("failed to initialize registry queue")?;

    let qh = event_queue.handle();

    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .wrap_err("failed to insert main EventLoop into WaylandSource")?;

    let config_helper = Config::helper();

    let config = match config_helper.as_ref() {
        Ok(helper) => {
            let source =
                ConfigWatchSource::new(helper).expect("failed to create ConfigWatchSource");

            event_loop
                .handle()
                .insert_source(source, |(config_helper, keys), (), state| {
                    let mut changes_applied = false;

                    for key in &keys {
                        match key.as_str() {
                            cosmic_bg_config::BACKGROUNDS => {
                                tracing::debug!("updating backgrounds");
                                state.config.load_backgrounds(&config_helper);
                                changes_applied = true;
                            }

                            cosmic_bg_config::DEFAULT_BACKGROUND => {
                                tracing::debug!("updating default background");
                                let entry = Config::load_default_background(&config_helper);

                                if state.config.default_background != entry {
                                    state.config.default_background = entry;
                                    changes_applied = true;
                                }
                            }

                            cosmic_bg_config::SAME_ON_ALL => {
                                tracing::debug!("updating same_on_all");
                                state.config.same_on_all = Config::load_same_on_all(&config_helper);

                                if state.config.same_on_all {
                                    state.config.outputs.clear();
                                } else {
                                    state.config.load_backgrounds(&config_helper);
                                }
                                state.config.outputs.clear();
                                changes_applied = true;
                            }

                            _ => {
                                tracing::debug!(key, "key modified");

                                if let Some(output) = key.strip_prefix("output.") {
                                    if let Ok(new_entry) = config_helper.get::<Entry>(key) {
                                        if let Some(existing) = state.config.entry_mut(output) {
                                            *existing = new_entry;
                                            changes_applied = true;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if changes_applied {
                        state.apply_backgrounds();

                        tracing::debug!(
                            same_on_all = state.config.same_on_all,
                            outputs = ?state.config.outputs,
                            backgrounds = ?state.config.backgrounds,
                            default_background = ?state.config.default_background.source,
                            "new state"
                        );
                    }
                })
                .expect("failed to insert config watching source into event loop");

            Config::load(helper).unwrap_or_else(|why| {
                tracing::error!(?why, "Config file error, falling back to defaults");
                Config::default()
            })
        }
        Err(why) => {
            tracing::error!(?why, "Config file error, falling back to defaults");
            Config::default()
        }
    };

    let source_tx = img_source::img_source(&event_loop.handle());

    // initial setup with all images
    let wallpapers = {
        let mut wallpapers = Vec::with_capacity(config.backgrounds.len() + 1);

        wallpapers.extend({
            config.backgrounds.iter().map(|bg| {
                Wallpaper::new(
                    bg.clone(),
                    qh.clone(),
                    event_loop.handle(),
                    source_tx.clone(),
                )
            })
        });

        wallpapers.sort_by(|a, b| a.entry.output.cmp(&b.entry.output));

        wallpapers.push(Wallpaper::new(
            config.default_background.clone(),
            qh.clone(),
            event_loop.handle(),
            source_tx.clone(),
        ));

        wallpapers
    };

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
pub struct CosmicBg {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm_state: Shm,
    layer_state: LayerShell,
    qh: QueueHandle<CosmicBg>,
    source_tx: calloop::channel::SyncSender<(String, notify::Event)>,
    loop_handle: calloop::LoopHandle<'static, CosmicBg>,
    exit: bool,
    wallpapers: Vec<Wallpaper>,
    config: Config,
    active_outputs: Vec<WlOutput>,
}

impl CosmicBg {
    fn apply_backgrounds(&mut self) {
        self.wallpapers.clear();

        let mut all_wallpaper = Wallpaper::new(
            self.config.default_background.clone(),
            self.qh.clone(),
            self.loop_handle.clone(),
            self.source_tx.clone(),
        );

        let mut backgrounds = self.config.backgrounds.clone();
        backgrounds.sort_by(|a, b| a.output.cmp(&b.output));

        'outer: for output in &self.active_outputs {
            let Some(output_info) = self.output_state.info(output) else {
                continue;
            };

            let o_name = output_info.name.clone().unwrap_or_default();
            for background in &backgrounds {
                if background.output == o_name {
                    let mut new_wallpaper = Wallpaper::new(
                        background.clone(),
                        self.qh.clone(),
                        self.loop_handle.clone(),
                        self.source_tx.clone(),
                    );

                    new_wallpaper
                        .layers
                        .push(self.new_layer(output.clone(), output_info));

                    self.wallpapers.push(new_wallpaper);

                    continue 'outer;
                }
            }

            all_wallpaper
                .layers
                .push(self.new_layer(output.clone(), output_info));
        }

        self.wallpapers.push(all_wallpaper);
    }

    #[must_use]
    pub fn new_layer(&self, output: WlOutput, output_info: OutputInfo) -> CosmicBgLayer {
        let (width, height) = output_info
            .logical_size
            .map_or((0, 0), |(w, h)| (w as u32, h as u32));

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
            pool: None,
            last_draw: None,
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

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // TODO
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
        let Some(output_info) = self.output_state.info(&wl_output) else {
            return;
        };

        if let Some(pos) = self
            .wallpapers
            .iter()
            .position(|w| match w.entry.output.as_str() {
                "all" => !w.layers.iter().any(|l| l.wl_output == wl_output),
                name => {
                    Some(name) == output_info.name.as_deref()
                        && !w.layers.iter().any(|l| l.wl_output == wl_output)
                }
            })
        {
            let layer = self.new_layer(wl_output, output_info);
            self.wallpapers[pos].layers.push(layer);
            if let Err(err) = self.wallpapers[pos].save_state() {
                tracing::error!("{err}");
            }
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
        let Some(output_info) = self.output_state.info(&output) else {
            return;
        };

        // state cleanup
        if let Ok(state_helper) = State::state() {
            let mut state = State::get_entry(&state_helper).unwrap_or_default();
            state
                .wallpapers
                .retain(|(o_name, _source)| Some(o_name) != output_info.name.as_ref());
            if let Err(err) = state.write_entry(&state_helper) {
                error!("{err}");
            }
        }

        let Some(output_wallpaper) =
            self.wallpapers
                .iter_mut()
                .find(|w| match w.entry.output.as_str() {
                    "all" => true,
                    name => Some(name) == output_info.name.as_deref(),
                })
        else {
            return;
        };

        let Some(layer_position) = output_wallpaper
            .layers
            .iter()
            .position(|bg_layer| bg_layer.wl_output == output)
        else {
            return;
        };

        output_wallpaper.layers.remove(layer_position);
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
        let span = tracing::debug_span!("<CosmicBg as LayerShellHandler>::configure");
        let _handle = span.enter();

        for wallpaper in &mut self.wallpapers {
            let (w, h) = configure.new_size;
            if let Some(w_layer) = wallpaper.layers.iter_mut().find(|l| &l.layer == layer) {
                w_layer.width = w;
                w_layer.height = h;

                if let Some(pool) = w_layer.pool.as_mut() {
                    if let Err(why) = pool.resize(w as usize * h as usize * 4) {
                        tracing::error!(?why, "failed to resize pool");
                        continue;
                    }
                } else {
                    match SlotPool::new(w as usize * h as usize * 4, &self.shm_state) {
                        Ok(pool) => {
                            w_layer.pool.replace(pool);
                        }

                        Err(why) => {
                            tracing::error!(?why, "failed to create pool");
                            continue;
                        }
                    }
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

fn init_logger() {
    let log_level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|level| level.parse::<tracing::Level>().ok())
        .unwrap_or(tracing::Level::INFO);

    let log_format = tracing_subscriber::fmt::format()
        .pretty()
        .without_time()
        .with_line_number(true)
        .with_file(true)
        .with_target(false)
        .with_thread_names(true);

    let log_filter = tracing_subscriber::fmt::Layer::default()
        .with_writer(std::io::stderr)
        .event_format(log_format)
        .with_filter(tracing_subscriber::filter::filter_fn(move |metadata| {
            metadata.level() == &tracing::Level::ERROR
                || (metadata.target().starts_with("cosmic_bg") && metadata.level() <= &log_level)
        }));

    tracing_subscriber::registry().with(log_filter).init();
}
