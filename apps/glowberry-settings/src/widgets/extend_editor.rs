// SPDX-License-Identifier: MPL-2.0

use crate::monitor_query::MonitorGeometry;
use cosmic::Renderer;
use cosmic::iced::core::renderer::Quad;
use cosmic::iced::core::widget::{Tree, tree};
use cosmic::iced::core::{
    self as core, Border, Clipboard, Element, Layout, Length, Rectangle, Renderer as IcedRenderer,
    Shell, Size, Widget,
};
use cosmic::iced::core::{Point, layout, mouse, renderer};
use cosmic::iced::widget::image::Handle as ImageHandle;
use slotmap::DefaultKey;

const PADDING: f32 = 10.0;
const MONITOR_BORDER_WIDTH: f32 = 3.0;
const MONITOR_CORNER_RADIUS: f32 = 4.0;
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
    pub selected: bool,
    pub locked: bool,
}

pub struct ExtendEditor<'a, Message> {
    monitors: &'a [MonitorGeometry],
    layers: Vec<LayerView<'a>>,
    on_move: Box<dyn Fn(DefaultKey, f64, f64) -> Message + 'a>,
    on_scale: Box<dyn Fn(DefaultKey, f64) -> Message + 'a>,
    on_select: Box<dyn Fn(Option<DefaultKey>) -> Message + 'a>,
    on_right_click: Option<Box<dyn Fn(DefaultKey) -> Message + 'a>>,
    fit_requested: bool,
    width: Length,
    height: Length,
}

impl<'a, Message> ExtendEditor<'a, Message> {
    pub fn new(
        monitors: &'a [MonitorGeometry],
        layers: Vec<LayerView<'a>>,
        on_move: impl Fn(DefaultKey, f64, f64) -> Message + 'a,
        on_scale: impl Fn(DefaultKey, f64) -> Message + 'a,
        on_select: impl Fn(Option<DefaultKey>) -> Message + 'a,
    ) -> Self {
        Self {
            monitors,
            layers,
            on_move: Box::new(on_move),
            on_scale: Box::new(on_scale),
            on_select: Box::new(on_select),
            on_right_click: None,
            fit_requested: false,
            width: Length::Fill,
            height: Length::Fixed(400.0),
        }
    }

    pub fn on_right_click(mut self, f: impl Fn(DefaultKey) -> Message + 'a) -> Self {
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
    // Camera state (persistent, not recomputed each frame)
    camera_zoom: f32,
    camera_pan: (f32, f32),
    camera_initialized: bool,
    camera_pan_start: (f32, f32),
    widget_size: (f32, f32),
}

impl Default for State {
    fn default() -> Self {
        Self {
            drag_mode: None,
            dragging_layer: None,
            drag_start: Point::ORIGIN,
            offset_at_drag_start: (0.0, 0.0),
            scale_at_drag_start: 1.0,
            camera_zoom: 1.0,
            camera_pan: (0.0, 0.0),
            camera_initialized: false,
            camera_pan_start: (0.0, 0.0),
            widget_size: (400.0, 300.0),
        }
    }
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

        // Initialize camera or handle fit request
        if !state.camera_initialized || self.fit_requested {
            state.fit_to_view(self.monitors, &self.layers);
            state.camera_initialized = true;
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
            // Left click: select/move/resize layers
            core::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position_in(bounds) {
                    let state = tree.state.downcast_mut::<State>();
                    let abs_pos = Point {
                        x: bounds.x + position.x,
                        y: bounds.y + position.y,
                    };

                    // Check resize handles on selected unlocked layer
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

                    // Hit-test layers
                    let locked: Vec<&LayerView> = self.layers.iter().filter(|l| l.locked).collect();
                    let mut unlocked: Vec<&LayerView> =
                        self.layers.iter().filter(|l| !l.locked).collect();
                    unlocked.sort_by_key(|l| std::cmp::Reverse(l.z_index));

                    let mut hit = None;
                    for layer in locked.iter().chain(unlocked.iter()) {
                        let rect = layer_widget_rect(state, layer, &bounds);
                        if rect.contains(abs_pos) {
                            hit = Some((layer.id, layer.locked));
                            break;
                        }
                    }

                    if let Some((layer_id, is_locked)) = hit {
                        if !is_locked
                            && let Some(layer) = self.layers.iter().find(|l| l.id == layer_id)
                        {
                            state.drag_mode = Some(DragMode::MoveLayer);
                            state.dragging_layer = Some(layer_id);
                            state.drag_start = position;
                            state.offset_at_drag_start = (layer.offset_x, layer.offset_y);
                            state.scale_at_drag_start = layer.img_scale;
                        }
                        shell.publish((self.on_select)(Some(layer_id)));
                    } else {
                        state.drag_mode = None;
                        state.dragging_layer = None;
                        shell.publish((self.on_select)(None));
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

                    match mode {
                        DragMode::PanCamera => {
                            state.camera_pan =
                                (state.camera_pan_start.0 + dx, state.camera_pan_start.1 + dy);
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
                        let rect = layer_widget_rect(state, layer, &bounds);
                        if rect.contains(abs_pos) {
                            shell.publish(on_right_click(layer.id));
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
                DragMode::MoveLayer => mouse::Interaction::Grabbing,
                DragMode::PanCamera => mouse::Interaction::Grabbing,
                DragMode::ResizeNW | DragMode::ResizeSE => {
                    mouse::Interaction::ResizingDiagonallyDown
                }
                DragMode::ResizeNE | DragMode::ResizeSW => {
                    mouse::Interaction::ResizingDiagonallyDown
                }
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

            for layer in &self.layers {
                let rect = layer_widget_rect(state, layer, &bounds);
                if rect.contains(abs_pos) {
                    if layer.locked {
                        return mouse::Interaction::NotAllowed;
                    }
                    return mouse::Interaction::Grab;
                }
            }
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

        // Widget background
        renderer.fill_quad(
            Quad {
                bounds,
                border: Border {
                    color: cosmic_theme.palette.neutral_5.into(),
                    radius: 8.0.into(),
                    width: 1.0,
                },
                shadow: Default::default(),
                snap: true,
            },
            core::Background::Color(cosmic_theme.palette.neutral_2.into()),
        );

        // Clip all content to widget bounds
        renderer.with_layer(bounds, |renderer| {
            // Draw unlocked layers first (by z_index), then locked on top
            let mut unlocked: Vec<&LayerView> = self.layers.iter().filter(|l| !l.locked).collect();
            unlocked.sort_by_key(|l| l.z_index);
            let locked: Vec<&LayerView> = self.layers.iter().filter(|l| l.locked).collect();

            // 1. Draw layer images (unlocked by z, then locked)
            for layer in unlocked.iter().chain(locked.iter()) {
                let layer_rect = layer_widget_rect(state, layer, &bounds);
                let opacity = if layer.selected { 1.0 } else { 0.75 };

                if let Some(handle) = layer.image_handle {
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
                        bounds,
                    );
                } else if let Some(clipped) = layer_rect.intersection(&bounds) {
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

            // 2. Draw selection borders, resize handles, and lock badges
            for layer in unlocked.iter().chain(locked.iter()) {
                let layer_rect = layer_widget_rect(state, layer, &bounds);

                if layer.selected {
                    let border_color = if layer.locked {
                        cosmic_theme.palette.neutral_7
                    } else {
                        cosmic_theme.accent_color()
                    };
                    if let Some(clipped) = layer_rect.intersection(&bounds) {
                        renderer.fill_quad(
                            Quad {
                                bounds: clipped,
                                border: Border {
                                    color: border_color.into(),
                                    radius: 0.0.into(),
                                    width: SELECTION_BORDER_WIDTH,
                                },
                                shadow: Default::default(),
                                snap: true,
                            },
                            core::Background::Color(core::Color::TRANSPARENT),
                        );
                    }

                    if !layer.locked {
                        let accent = cosmic_theme.accent_color();
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
                }

                if layer.locked {
                    let badge_size = 18.0_f32;
                    let badge_rect = Rectangle {
                        x: layer_rect.x + layer_rect.width - badge_size - 4.0,
                        y: layer_rect.y + 4.0,
                        width: badge_size,
                        height: badge_size,
                    };
                    if bounds.contains(Point::new(badge_rect.center_x(), badge_rect.center_y())) {
                        renderer.fill_quad(
                            Quad {
                                bounds: badge_rect,
                                border: Border {
                                    radius: 4.0.into(),
                                    ..Default::default()
                                },
                                shadow: Default::default(),
                                snap: true,
                            },
                            core::Background::Color({
                                let mut c = cosmic_theme.palette.neutral_1;
                                c.alpha = 0.85;
                                c.into()
                            }),
                        );
                        core::text::Renderer::fill_text(
                            renderer,
                            core::Text {
                                content: String::from("\u{1F512}"),
                                size: core::Pixels(11.0),
                                line_height: core::text::LineHeight::Relative(1.2),
                                font: cosmic::font::default(),
                                bounds: badge_rect.size(),
                                align_x: cosmic::iced::core::text::Alignment::Center,
                                align_y: cosmic::iced::core::alignment::Vertical::Center,
                                shaping: core::text::Shaping::Advanced,
                                wrapping: core::text::Wrapping::None,
                                ellipsize: core::text::Ellipsize::None,
                            },
                            core::Point {
                                x: badge_rect.center_x(),
                                y: badge_rect.center_y(),
                            },
                            cosmic_theme.palette.neutral_10.into(),
                            bounds,
                        );
                    }
                }
            }
        }); // end with_layer for images + selection

        // Monitor outlines in a separate layer so they're always on top
        renderer.with_layer(bounds, |renderer| {
            for (i, monitor) in self.monitors.iter().enumerate() {
                let (mx, my) =
                    state.virtual_to_widget(monitor.position.0 as f64, monitor.position.1 as f64);
                let mw = monitor.logical_size.0 as f32 * state.camera_zoom;
                let mh = monitor.logical_size.1 as f32 * state.camera_zoom;

                let mon_rect = Rectangle {
                    x: bounds.x + mx,
                    y: bounds.y + my,
                    width: mw,
                    height: mh,
                };

                renderer.fill_quad(
                    Quad {
                        bounds: mon_rect,
                        border: Border {
                            color: cosmic_theme.accent_color().into(),
                            radius: MONITOR_CORNER_RADIUS.into(),
                            width: MONITOR_BORDER_WIDTH,
                        },
                        shadow: Default::default(),
                        snap: true,
                    },
                    core::Background::Color(core::Color::TRANSPARENT),
                );

                let label = format!("{}", i + 1);
                let label_bg = Rectangle {
                    x: mon_rect.x + mon_rect.width / 2.0 - 12.0,
                    y: mon_rect.y + mon_rect.height / 2.0 - 10.0,
                    width: 24.0,
                    height: 20.0,
                };

                renderer.fill_quad(
                    Quad {
                        bounds: label_bg,
                        border: Border {
                            radius: 10.0.into(),
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
                        size: core::Pixels(14.0),
                        line_height: core::text::LineHeight::Relative(1.2),
                        font: cosmic::font::bold(),
                        bounds: label_bg.size(),
                        align_x: cosmic::iced::core::text::Alignment::Center,
                        align_y: cosmic::iced::core::alignment::Vertical::Center,
                        shaping: core::text::Shaping::Basic,
                        wrapping: core::text::Wrapping::Word,
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
