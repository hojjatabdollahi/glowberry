// SPDX-License-Identifier: MPL-2.0

//! The display canvas: the connected displays drawn at their real positions,
//! with the staged content on top. Clicking a display selects it; free
//! (spanning) image layers can be dragged and resized.

use crate::monitor_query::MonitorGeometry;
use cosmic::Renderer;
use cosmic::iced::core::renderer::Quad;
use cosmic::iced::core::widget::{Tree, tree};
use cosmic::iced::core::{
    self as core, Border, Clipboard, Element, Layout, Length, Rectangle, Renderer as IcedRenderer,
    Shell, Size, Widget,
};
use cosmic::iced::core::{Point, keyboard, layout, mouse, renderer};
use cosmic::iced::widget::image::Handle as ImageHandle;
use slotmap::DefaultKey;

const PADDING: f32 = 10.0;
const MONITOR_CORNER_RADIUS: f32 = 6.0;
/// Width of the background-colored frame painted just outside each display to
/// round off the content clipped to it. Must be at least 0.42 x the radius so
/// the sharp corner underneath is fully covered.
const CORNER_MASK: f32 = 3.0;
const MONITOR_BORDER_WIDTH: f32 = 1.5;
const SELECTION_BORDER_WIDTH: f32 = 2.5;
const HANDLE_SIZE: f32 = 12.0;
const HANDLE_HIT_SIZE: f32 = 16.0;

#[derive(Clone, Debug)]
pub struct LayerView<'a> {
    pub id: DefaultKey,
    pub image_handle: Option<&'a ImageHandle>,
    pub image_size: (u32, u32),
    pub offset_x: f64,
    pub offset_y: f64,
    pub img_scale: f64,
    pub z_index: usize,
    /// Only free layers can be selected; locked ones belong to their display.
    pub selected: bool,
    /// Locked layers fill one display and are not draggable.
    pub locked: bool,
    /// The display a locked layer belongs to; it is clipped to that display.
    pub target_output: Option<&'a str>,
    /// For color items: fill this layer with a solid color or gradient
    /// instead of an image.
    pub color: Option<&'a glowberry_config::Color>,
}

/// Build a fill background for a color item (solid or gradient), applying the
/// given opacity. Mirrors the gradient construction used by the library swatch.
fn color_background(color: &glowberry_config::Color, opacity: f32) -> cosmic::iced::Background {
    use cosmic::iced::{Background, Color as IcedColor, Degrees, Gradient, gradient::Linear};
    match color {
        glowberry_config::Color::Single([r, g, b]) => {
            Background::Color(IcedColor::from_rgba(*r, *g, *b, opacity))
        }
        glowberry_config::Color::Gradient(grad) => {
            let stop_increment = 1.0 / (grad.colors.len().saturating_sub(1).max(1)) as f32;
            let mut stop = 0.0;
            let mut linear = Linear::new(Degrees(grad.radius));
            for &[r, g, b] in &*grad.colors {
                linear = linear.add_stop(stop, IcedColor::from_rgba(r, g, b, opacity));
                stop += stop_increment;
            }
            Background::Gradient(Gradient::Linear(linear))
        }
    }
}

#[allow(clippy::type_complexity)]
pub struct ExtendEditor<'a, Message> {
    monitors: &'a [MonitorGeometry],
    layers: Vec<LayerView<'a>>,
    /// Connector names of the selected displays (empty = all).
    selected_displays: &'a [String],
    on_move: Box<dyn Fn(DefaultKey, f64, f64) -> Message + 'a>,
    on_scale: Box<dyn Fn(DefaultKey, f64) -> Message + 'a>,
    on_select: Box<dyn Fn(Option<DefaultKey>) -> Message + 'a>,
    on_display_click: Option<Box<dyn Fn(String, bool) -> Message + 'a>>,
    on_background_click: Option<Message>,
    on_right_click: Option<Box<dyn Fn(DefaultKey, f32, f32) -> Message + 'a>>,
    fit_requested: bool,
    width: Length,
    height: Length,
}

impl<'a, Message> ExtendEditor<'a, Message> {
    pub fn new(
        monitors: &'a [MonitorGeometry],
        layers: Vec<LayerView<'a>>,
        selected_displays: &'a [String],
        on_move: impl Fn(DefaultKey, f64, f64) -> Message + 'a,
        on_scale: impl Fn(DefaultKey, f64) -> Message + 'a,
        on_select: impl Fn(Option<DefaultKey>) -> Message + 'a,
    ) -> Self {
        Self {
            monitors,
            layers,
            selected_displays,
            on_move: Box::new(on_move),
            on_scale: Box::new(on_scale),
            on_select: Box::new(on_select),
            on_display_click: None,
            on_background_click: None,
            on_right_click: None,
            fit_requested: false,
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    /// Called when a display is clicked: `(connector, additive)`. `additive`
    /// is true while Ctrl or Shift is held.
    pub fn on_display_click(mut self, f: impl Fn(String, bool) -> Message + 'a) -> Self {
        self.on_display_click = Some(Box::new(f));
        self
    }

    /// Emitted when the empty canvas is clicked.
    pub fn on_background_click(mut self, message: Message) -> Self {
        self.on_background_click = Some(message);
        self
    }

    pub fn on_right_click(mut self, f: impl Fn(DefaultKey, f32, f32) -> Message + 'a) -> Self {
        self.on_right_click = Some(Box::new(f));
        self
    }

    pub fn fit_requested(mut self, requested: bool) -> Self {
        self.fit_requested = requested;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragMode {
    MoveLayer,
    ResizeNW,
    ResizeNE,
    ResizeSW,
    ResizeSE,
    PanCamera,
}

struct State {
    drag_mode: Option<DragMode>,
    dragging_layer: Option<DefaultKey>,
    drag_start: Point,
    offset_at_drag_start: (f64, f64),
    scale_at_drag_start: f64,
    modifiers: keyboard::Modifiers,
    // Camera state (persistent, not recomputed each frame)
    camera_zoom: f32,
    camera_pan: (f32, f32),
    camera_initialized: bool,
    camera_pan_start: (f32, f32),
    widget_size: (f32, f32),
    // Signature of the scene content last fitted to. When the monitor layout or
    // the set of layers changes (e.g. monitors are queried after the window
    // opens, or an image is added), the view re-fits automatically so the user
    // doesn't have to press the fit button. Moving/scaling a layer does not
    // change the signature, so it won't fight manual edits.
    last_content_sig: u64,
    // Widget size at the last fit. The window settles to its final size shortly
    // after opening, so we also re-fit when the size changes — keeping the scene
    // centered — until the user manually pans/zooms.
    last_fit_size: (f32, f32),
    user_adjusted: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            drag_mode: None,
            dragging_layer: None,
            drag_start: Point::ORIGIN,
            offset_at_drag_start: (0.0, 0.0),
            scale_at_drag_start: 1.0,
            modifiers: keyboard::Modifiers::default(),
            camera_zoom: 1.0,
            camera_pan: (0.0, 0.0),
            camera_initialized: false,
            camera_pan_start: (0.0, 0.0),
            widget_size: (400.0, 300.0),
            last_content_sig: 0,
            last_fit_size: (0.0, 0.0),
            user_adjusted: false,
        }
    }
}

/// Signature of the scene's structural content (monitor geometry, and the set
/// and intrinsic size of layers) — but not layer offset/scale, so editing a
/// layer doesn't trigger a re-fit.
fn content_signature(monitors: &[MonitorGeometry], layers: &[LayerView]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    monitors.len().hash(&mut hasher);
    for m in monitors {
        m.position.hash(&mut hasher);
        m.logical_size.hash(&mut hasher);
    }
    layers.len().hash(&mut hasher);
    for l in layers {
        l.id.hash(&mut hasher);
        l.image_size.hash(&mut hasher);
    }
    hasher.finish()
}

impl State {
    fn virtual_to_widget(&self, vx: f64, vy: f64) -> (f32, f32) {
        (
            self.camera_pan.0 + vx as f32 * self.camera_zoom,
            self.camera_pan.1 + vy as f32 * self.camera_zoom,
        )
    }

    fn widget_to_virtual_delta(&self, dx: f32, dy: f32) -> (f64, f64) {
        if self.camera_zoom > 0.0 {
            (
                dx as f64 / self.camera_zoom as f64,
                dy as f64 / self.camera_zoom as f64,
            )
        } else {
            (0.0, 0.0)
        }
    }

    fn fit_to_view(&mut self, monitors: &[MonitorGeometry], layers: &[LayerView]) {
        let (sx, sy, sw, sh) = scene_content_bounds(monitors, layers);
        let available_w = (self.widget_size.0 - 2.0 * PADDING).max(1.0);
        let available_h = (self.widget_size.1 - 2.0 * PADDING).max(1.0);

        self.camera_zoom = if sw > 0.0 && sh > 0.0 {
            (available_w as f64 / sw).min(available_h as f64 / sh) as f32
        } else {
            1.0
        };

        let rendered_w = sw as f32 * self.camera_zoom;
        let rendered_h = sh as f32 * self.camera_zoom;
        self.camera_pan = (
            PADDING + (available_w - rendered_w) / 2.0 - sx as f32 * self.camera_zoom,
            PADDING + (available_h - rendered_h) / 2.0 - sy as f32 * self.camera_zoom,
        );
    }
}

fn layer_widget_rect(state: &State, layer: &LayerView, bounds: &Rectangle) -> Rectangle {
    let (lx, ly) = state.virtual_to_widget(layer.offset_x, layer.offset_y);
    let lw = layer.image_size.0 as f64 * layer.img_scale;
    let lh = layer.image_size.1 as f64 * layer.img_scale;
    Rectangle {
        x: bounds.x + lx,
        y: bounds.y + ly,
        width: lw as f32 * state.camera_zoom,
        height: lh as f32 * state.camera_zoom,
    }
}

fn monitor_widget_rect(state: &State, monitor: &MonitorGeometry, bounds: &Rectangle) -> Rectangle {
    let (mx, my) = state.virtual_to_widget(monitor.position.0 as f64, monitor.position.1 as f64);
    Rectangle {
        x: bounds.x + mx,
        y: bounds.y + my,
        width: monitor.logical_size.0 as f32 * state.camera_zoom,
        height: monitor.logical_size.1 as f32 * state.camera_zoom,
    }
}

/// Where a layer is drawn and hit-tested: free layers use their own rect,
/// locked ones the rect of the display they fill.
fn layer_hit_rect(
    state: &State,
    layer: &LayerView,
    monitors: &[MonitorGeometry],
    bounds: &Rectangle,
) -> Rectangle {
    if layer.locked
        && let Some(m) = layer
            .target_output
            .and_then(|name| monitors.iter().find(|m| m.name == name))
    {
        return monitor_widget_rect(state, m, bounds);
    }
    layer_widget_rect(state, layer, bounds)
}

fn corner_hit_rect(cx: f32, cy: f32) -> Rectangle {
    Rectangle {
        x: cx - HANDLE_HIT_SIZE / 2.0,
        y: cy - HANDLE_HIT_SIZE / 2.0,
        width: HANDLE_HIT_SIZE,
        height: HANDLE_HIT_SIZE,
    }
}

fn hit_test_handles(rect: &Rectangle, abs_pos: Point) -> Option<DragMode> {
    let corners = [
        (rect.x, rect.y, DragMode::ResizeNW),
        (rect.x + rect.width, rect.y, DragMode::ResizeNE),
        (rect.x, rect.y + rect.height, DragMode::ResizeSW),
        (
            rect.x + rect.width,
            rect.y + rect.height,
            DragMode::ResizeSE,
        ),
    ];
    for (cx, cy, mode) in corners {
        if corner_hit_rect(cx, cy).contains(abs_pos) {
            return Some(mode);
        }
    }
    None
}

fn scene_content_bounds(
    monitors: &[MonitorGeometry],
    layers: &[LayerView],
) -> (f64, f64, f64, f64) {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    for m in monitors {
        let x = m.position.0 as f64;
        let y = m.position.1 as f64;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + m.logical_size.0 as f64);
        max_y = max_y.max(y + m.logical_size.1 as f64);
    }

    for l in layers {
        let w = l.image_size.0 as f64 * l.img_scale;
        let h = l.image_size.1 as f64 * l.img_scale;
        min_x = min_x.min(l.offset_x);
        min_y = min_y.min(l.offset_y);
        max_x = max_x.max(l.offset_x + w);
        max_y = max_y.max(l.offset_y + h);
    }

    if min_x > max_x {
        return (0.0, 0.0, 1920.0, 1080.0);
    }

    (min_x, min_y, max_x - min_x, max_y - min_y)
}

fn draw_corner_handle(
    renderer: &mut Renderer,
    cx: f32,
    cy: f32,
    color: cosmic_theme::palette::Srgba,
) {
    renderer.fill_quad(
        Quad {
            bounds: Rectangle {
                x: cx - HANDLE_SIZE / 2.0,
                y: cy - HANDLE_SIZE / 2.0,
                width: HANDLE_SIZE,
                height: HANDLE_SIZE,
            },
            border: Border {
                radius: (HANDLE_SIZE / 2.0).into(),
                width: 2.0,
                color: core::Color::WHITE,
            },
            shadow: Default::default(),
            snap: true,
        },
        core::Background::Color(color.into()),
    );
}

use cosmic::cosmic_theme;

impl<Message: Clone> Widget<Message, cosmic::Theme, Renderer> for ExtendEditor<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width).height(self.height);
        let size = limits.resolve(self.width, self.height, Size::ZERO);

        let state = tree.state.downcast_mut::<State>();
        state.widget_size = (size.width, size.height);

        // Fit on first layout, on an explicit fit request, or — until the user
        // manually pans/zooms — whenever the scene's structural content changes
        // (monitors queried after open, a layer added) or the widget is resized
        // (the window settling to its final size after opening). This keeps the
        // scene fitted and centered on open without fighting later manual edits.
        let sig = content_signature(self.monitors, &self.layers);
        let size_changed = (state.widget_size.0 - state.last_fit_size.0).abs() > 0.5
            || (state.widget_size.1 - state.last_fit_size.1).abs() > 0.5;
        let auto_fit = !state.user_adjusted && (sig != state.last_content_sig || size_changed);
        if !state.camera_initialized || self.fit_requested || auto_fit {
            state.fit_to_view(self.monitors, &self.layers);
            state.camera_initialized = true;
            state.last_content_sig = sig;
            state.last_fit_size = state.widget_size;
            // An explicit fit re-enables automatic centering.
            if self.fit_requested {
                state.user_adjusted = false;
            }
        }

        layout::Node::new(size)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &cosmic::iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();

        match event {
            core::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                tree.state.downcast_mut::<State>().modifiers = *modifiers;
            }

            // Left click: select a display, or grab a free layer
            core::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position_in(bounds) {
                    let state = tree.state.downcast_mut::<State>();
                    let abs_pos = Point {
                        x: bounds.x + position.x,
                        y: bounds.y + position.y,
                    };

                    // Resize handles on the selected free layer win over everything.
                    if let Some(selected) = self.layers.iter().find(|l| l.selected && !l.locked) {
                        let rect = layer_widget_rect(state, selected, &bounds);
                        if let Some(mode) = hit_test_handles(&rect, abs_pos) {
                            state.drag_mode = Some(mode);
                            state.dragging_layer = Some(selected.id);
                            state.drag_start = position;
                            state.offset_at_drag_start = (selected.offset_x, selected.offset_y);
                            state.scale_at_drag_start = selected.img_scale;
                            shell.capture_event();
                            return;
                        }
                    }

                    let display = self
                        .monitors
                        .iter()
                        .find(|m| monitor_widget_rect(state, m, &bounds).contains(abs_pos))
                        .map(|m| m.name.clone());
                    let over_locked = display.as_deref().is_some_and(|d| {
                        self.layers
                            .iter()
                            .any(|l| l.locked && l.target_output == Some(d))
                    });
                    // Topmost free layer under the cursor, unless a locked item hides it.
                    let mut free: Vec<&LayerView> =
                        self.layers.iter().filter(|l| !l.locked).collect();
                    free.sort_by_key(|l| std::cmp::Reverse(l.z_index));
                    let grabbed = if over_locked {
                        None
                    } else {
                        free.iter()
                            .find(|l| layer_widget_rect(state, l, &bounds).contains(abs_pos))
                            .map(|l| (l.id, l.offset_x, l.offset_y, l.img_scale))
                    };
                    let additive = state.modifiers.control() || state.modifiers.shift();

                    if let Some(name) = &display
                        && let Some(on_display_click) = &self.on_display_click
                    {
                        shell.publish(on_display_click(name.clone(), additive));
                    }
                    match grabbed {
                        Some((id, ox, oy, scale)) => {
                            state.drag_mode = Some(DragMode::MoveLayer);
                            state.dragging_layer = Some(id);
                            state.drag_start = position;
                            state.offset_at_drag_start = (ox, oy);
                            state.scale_at_drag_start = scale;
                            shell.publish((self.on_select)(Some(id)));
                        }
                        None if display.is_some() => {
                            shell.publish((self.on_select)(None));
                        }
                        None => {
                            // Empty area: back to all displays and start panning
                            // so a click-drag on the background moves the view.
                            state.drag_mode = Some(DragMode::PanCamera);
                            state.dragging_layer = None;
                            state.drag_start = position;
                            state.camera_pan_start = state.camera_pan;
                            if let Some(msg) = &self.on_background_click {
                                shell.publish(msg.clone());
                            }
                        }
                    }
                    shell.capture_event();
                }
            }

            // Middle click: pan camera
            core::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)) => {
                if let Some(position) = cursor.position_in(bounds) {
                    let state = tree.state.downcast_mut::<State>();
                    state.drag_mode = Some(DragMode::PanCamera);
                    state.drag_start = position;
                    state.camera_pan_start = state.camera_pan;
                    shell.capture_event();
                }
            }

            core::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let state = tree.state.downcast_mut::<State>();
                if let Some(mode) = state.drag_mode
                    && let Some(position) = cursor.position_in(bounds)
                {
                    let dx = position.x - state.drag_start.x;
                    let dy = position.y - state.drag_start.y;
                    // Any drag counts as a manual adjustment; stop auto-fitting
                    // on resize so we don't fight the user.
                    state.user_adjusted = true;

                    match mode {
                        DragMode::PanCamera => {
                            state.camera_pan =
                                (state.camera_pan_start.0 + dx, state.camera_pan_start.1 + dy);
                            shell.request_redraw();
                            shell.capture_event();
                        }
                        DragMode::MoveLayer => {
                            if let Some(layer_id) = state.dragging_layer {
                                let (vdx, vdy) = state.widget_to_virtual_delta(dx, dy);
                                let new_x = state.offset_at_drag_start.0 + vdx;
                                let new_y = state.offset_at_drag_start.1 + vdy;
                                shell.publish((self.on_move)(layer_id, new_x, new_y));
                                shell.capture_event();
                            }
                        }
                        DragMode::ResizeNW
                        | DragMode::ResizeNE
                        | DragMode::ResizeSW
                        | DragMode::ResizeSE => {
                            if let Some(layer_id) = state.dragging_layer
                                && let Some(layer) = self.layers.iter().find(|l| l.id == layer_id)
                            {
                                let orig_scale = state.scale_at_drag_start;
                                let orig_w = layer.image_size.0 as f64 * orig_scale;
                                let orig_h = layer.image_size.1 as f64 * orig_scale;
                                let (vdx, vdy) = state.widget_to_virtual_delta(dx, dy);

                                let (scale_dx, scale_dy) = match mode {
                                    DragMode::ResizeSE => (vdx, vdy),
                                    DragMode::ResizeNW => (-vdx, -vdy),
                                    DragMode::ResizeNE => (vdx, -vdy),
                                    DragMode::ResizeSW => (-vdx, vdy),
                                    _ => (0.0, 0.0),
                                };

                                let ratio = if orig_w.abs() > orig_h.abs() {
                                    (orig_w + scale_dx) / orig_w
                                } else {
                                    (orig_h + scale_dy) / orig_h
                                };
                                let new_scale = (orig_scale * ratio).clamp(0.05, 10.0);
                                let new_w = layer.image_size.0 as f64 * new_scale;
                                let new_h = layer.image_size.1 as f64 * new_scale;

                                let orig_offset = state.offset_at_drag_start;
                                let new_offset = match mode {
                                    DragMode::ResizeSE => orig_offset,
                                    DragMode::ResizeNW => (
                                        orig_offset.0 + orig_w - new_w,
                                        orig_offset.1 + orig_h - new_h,
                                    ),
                                    DragMode::ResizeNE => {
                                        (orig_offset.0, orig_offset.1 + orig_h - new_h)
                                    }
                                    DragMode::ResizeSW => {
                                        (orig_offset.0 + orig_w - new_w, orig_offset.1)
                                    }
                                    _ => orig_offset,
                                };

                                shell.publish((self.on_scale)(layer_id, new_scale));
                                shell.publish((self.on_move)(layer_id, new_offset.0, new_offset.1));
                                shell.capture_event();
                            }
                        }
                    }
                }
            }

            core::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | core::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)) => {
                let state = tree.state.downcast_mut::<State>();
                if state.drag_mode.is_some() {
                    state.dragging_layer = None;
                    state.drag_mode = None;
                    shell.capture_event();
                }
            }

            // Scroll: zoom camera centered on cursor
            core::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if let Some(position) = cursor.position_in(bounds) {
                    let state = tree.state.downcast_mut::<State>();
                    let scroll_y = match delta {
                        mouse::ScrollDelta::Lines { y, .. } => *y,
                        mouse::ScrollDelta::Pixels { y, .. } => *y / 50.0,
                    };
                    let factor = 1.0 + scroll_y * 0.1;
                    let new_zoom = (state.camera_zoom * factor).clamp(0.01, 50.0);

                    // Zoom centered on cursor position
                    let cursor_x = position.x;
                    let cursor_y = position.y;
                    state.camera_pan.0 =
                        cursor_x - (cursor_x - state.camera_pan.0) * (new_zoom / state.camera_zoom);
                    state.camera_pan.1 =
                        cursor_y - (cursor_y - state.camera_pan.1) * (new_zoom / state.camera_zoom);
                    state.camera_zoom = new_zoom;
                    state.user_adjusted = true;

                    shell.request_redraw();
                    shell.capture_event();
                }
            }

            // Right click on layers
            core::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(position) = cursor.position_in(bounds)
                    && let Some(on_right_click) = &self.on_right_click
                {
                    let state = tree.state.downcast_ref::<State>();
                    let abs_pos = Point {
                        x: bounds.x + position.x,
                        y: bounds.y + position.y,
                    };

                    let locked: Vec<&LayerView> = self.layers.iter().filter(|l| l.locked).collect();
                    let mut unlocked: Vec<&LayerView> =
                        self.layers.iter().filter(|l| !l.locked).collect();
                    unlocked.sort_by_key(|l| std::cmp::Reverse(l.z_index));

                    for layer in locked.iter().chain(unlocked.iter()) {
                        let rect = layer_hit_rect(state, layer, self.monitors, &bounds);
                        if rect.contains(abs_pos) {
                            shell.publish(on_right_click(layer.id, position.x, position.y));
                            shell.capture_event();
                            break;
                        }
                    }
                }
            }

            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();

        if let Some(mode) = state.drag_mode {
            return match mode {
                DragMode::MoveLayer | DragMode::PanCamera => mouse::Interaction::Grabbing,
                DragMode::ResizeNW
                | DragMode::ResizeSE
                | DragMode::ResizeNE
                | DragMode::ResizeSW => mouse::Interaction::ResizingDiagonallyDown,
            };
        }

        if let Some(position) = cursor.position_in(bounds) {
            let abs_pos = Point {
                x: bounds.x + position.x,
                y: bounds.y + position.y,
            };

            if let Some(selected) = self.layers.iter().find(|l| l.selected && !l.locked) {
                let rect = layer_widget_rect(state, selected, &bounds);
                if hit_test_handles(&rect, abs_pos).is_some() {
                    return mouse::Interaction::ResizingDiagonallyDown;
                }
            }

            let over_locked = self.layers.iter().any(|l| {
                l.locked && layer_hit_rect(state, l, self.monitors, &bounds).contains(abs_pos)
            });
            if !over_locked
                && self
                    .layers
                    .iter()
                    .any(|l| !l.locked && layer_widget_rect(state, l, &bounds).contains(abs_pos))
            {
                return mouse::Interaction::Grab;
            }
            if self
                .monitors
                .iter()
                .any(|m| monitor_widget_rect(state, m, &bounds).contains(abs_pos))
            {
                return mouse::Interaction::Pointer;
            }
            // Empty area is pannable.
            return mouse::Interaction::Grab;
        }
        mouse::Interaction::Idle
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &cosmic::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let theme = cosmic::theme::active();
        let cosmic_theme = theme.cosmic();

        // Clip all content to widget bounds
        renderer.with_layer(bounds, |renderer| {
            // Draw free layers first (by z_index), then locked on top
            let mut unlocked: Vec<&LayerView> = self.layers.iter().filter(|l| !l.locked).collect();
            unlocked.sort_by_key(|l| l.z_index);
            let locked: Vec<&LayerView> = self.layers.iter().filter(|l| l.locked).collect();

            // 1. Layer images
            for layer in unlocked.iter().chain(locked.iter()) {
                let layer_rect = layer_widget_rect(state, layer, &bounds);
                let opacity = if layer.locked || layer.selected {
                    1.0
                } else {
                    0.85
                };
                // Locked items are cover-fit, so they overflow their display;
                // clip them to it.
                let clip = if layer.locked {
                    layer_hit_rect(state, layer, self.monitors, &bounds)
                        .intersection(&bounds)
                        .unwrap_or_default()
                } else {
                    bounds
                };

                if let Some(color) = layer.color {
                    if let Some(clipped) = layer_rect.intersection(&clip) {
                        renderer.fill_quad(
                            Quad {
                                bounds: clipped,
                                border: Border::default(),
                                shadow: Default::default(),
                                snap: true,
                            },
                            color_background(color, opacity),
                        );
                    }
                } else if let Some(handle) = layer.image_handle {
                    use core::image::Renderer as ImageRenderer;
                    ImageRenderer::draw_image(
                        renderer,
                        core::Image {
                            handle: handle.clone(),
                            filter_method: core::image::FilterMethod::Linear,
                            rotation: core::Radians(0.0),
                            border_radius: Default::default(),
                            opacity,
                            snap: true,
                        },
                        layer_rect,
                        clip,
                    );
                } else if let Some(clipped) = layer_rect.intersection(&clip) {
                    let mut c = cosmic_theme.palette.neutral_6;
                    c.alpha = opacity * 0.5;
                    renderer.fill_quad(
                        Quad {
                            bounds: clipped,
                            border: Border::default(),
                            shadow: Default::default(),
                            snap: true,
                        },
                        core::Background::Color(c.into()),
                    );
                }
            }

            // 2. Selection border and resize handles on the selected free layer
            for layer in unlocked.iter().filter(|l| l.selected) {
                let layer_rect = layer_widget_rect(state, layer, &bounds);
                let accent = cosmic_theme.accent_color();
                if let Some(clipped) = layer_rect.intersection(&bounds) {
                    renderer.fill_quad(
                        Quad {
                            bounds: clipped,
                            border: Border {
                                color: accent.into(),
                                radius: 0.0.into(),
                                width: SELECTION_BORDER_WIDTH,
                            },
                            shadow: Default::default(),
                            snap: true,
                        },
                        core::Background::Color(core::Color::TRANSPARENT),
                    );
                }
                let corners = [
                    (layer_rect.x, layer_rect.y),
                    (layer_rect.x + layer_rect.width, layer_rect.y),
                    (layer_rect.x, layer_rect.y + layer_rect.height),
                    (
                        layer_rect.x + layer_rect.width,
                        layer_rect.y + layer_rect.height,
                    ),
                ];
                for (cx, cy) in corners {
                    draw_corner_handle(renderer, cx, cy, accent);
                }
            }
        }); // end with_layer for images + selection

        // Display frames in a separate layer so they're always on top
        renderer.with_layer(bounds, |renderer| {
            // Rounded corners: iced clips rectangularly, so a frame in the
            // window background color, drawn just outside each display with a
            // rounded inner edge, covers the corners of the content beneath.
            // All masks go first so a neighbour's band can't cover a border.
            let bg: core::Color = cosmic_theme.bg_color().into();
            for monitor in self.monitors.iter() {
                let r = monitor_widget_rect(state, monitor, &bounds);
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle {
                            x: r.x - CORNER_MASK,
                            y: r.y - CORNER_MASK,
                            width: r.width + 2.0 * CORNER_MASK,
                            height: r.height + 2.0 * CORNER_MASK,
                        },
                        border: Border {
                            color: bg,
                            radius: (MONITOR_CORNER_RADIUS + CORNER_MASK).into(),
                            width: CORNER_MASK,
                        },
                        shadow: Default::default(),
                        snap: true,
                    },
                    core::Background::Color(core::Color::TRANSPARENT),
                );
            }

            for monitor in self.monitors.iter() {
                let mon_rect = monitor_widget_rect(state, monitor, &bounds);
                let selected = self.selected_displays.contains(&monitor.name);

                let (color, width) = if selected {
                    (cosmic_theme.accent_color(), SELECTION_BORDER_WIDTH)
                } else {
                    (cosmic_theme.palette.neutral_7, MONITOR_BORDER_WIDTH)
                };
                renderer.fill_quad(
                    Quad {
                        bounds: mon_rect,
                        border: Border {
                            color: color.into(),
                            radius: MONITOR_CORNER_RADIUS.into(),
                            width,
                        },
                        shadow: Default::default(),
                        snap: true,
                    },
                    core::Background::Color(core::Color::TRANSPARENT),
                );

                // Display name: a pill in the bottom-left corner, hidden when
                // the display is drawn too small to hold it.
                let label = monitor.name.clone();
                let label_w = label.len() as f32 * 9.5 + 24.0;
                let label_h = 30.0;
                if mon_rect.width < label_w + 24.0 || mon_rect.height < label_h + 24.0 {
                    continue;
                }
                let label_bg = Rectangle {
                    x: mon_rect.x + 12.0,
                    y: mon_rect.y + mon_rect.height - label_h - 12.0,
                    width: label_w,
                    height: label_h,
                };

                renderer.fill_quad(
                    Quad {
                        bounds: label_bg,
                        border: Border {
                            radius: (label_h / 2.0).into(),
                            ..Default::default()
                        },
                        shadow: Default::default(),
                        snap: true,
                    },
                    core::Background::Color({
                        let mut c = cosmic_theme.palette.neutral_1;
                        c.alpha = 0.8;
                        c.into()
                    }),
                );

                core::text::Renderer::fill_text(
                    renderer,
                    core::Text {
                        content: label,
                        size: core::Pixels(16.0),
                        line_height: core::text::LineHeight::Relative(1.2),
                        font: cosmic::font::bold(),
                        bounds: label_bg.size(),
                        align_x: cosmic::iced::core::text::Alignment::Center,
                        align_y: cosmic::iced::core::alignment::Vertical::Center,
                        shaping: core::text::Shaping::Basic,
                        wrapping: core::text::Wrapping::None,
                        ellipsize: core::text::Ellipsize::None,
                    },
                    core::Point {
                        x: label_bg.center_x(),
                        y: label_bg.center_y(),
                    },
                    cosmic_theme.palette.neutral_10.into(),
                    bounds,
                );
            }
        }); // end with_layer for monitors
    }
}

impl<'a, Message: 'static + Clone> From<ExtendEditor<'a, Message>>
    for cosmic::Element<'a, Message>
{
    fn from(editor: ExtendEditor<'a, Message>) -> Self {
        Element::new(editor)
    }
}
