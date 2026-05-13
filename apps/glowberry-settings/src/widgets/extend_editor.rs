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

const PADDING: f32 = 20.0;
const MONITOR_BORDER_WIDTH: f32 = 2.0;
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
}

pub struct ExtendEditor<'a, Message> {
    monitors: &'a [MonitorGeometry],
    layers: Vec<LayerView<'a>>,
    on_move: Box<dyn Fn(DefaultKey, f64, f64) -> Message + 'a>,
    on_scale: Box<dyn Fn(DefaultKey, f64) -> Message + 'a>,
    on_select: Box<dyn Fn(Option<DefaultKey>) -> Message + 'a>,
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
            width: Length::Fill,
            height: Length::Fixed(400.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragMode {
    Move,
    ResizeNW,
    ResizeNE,
    ResizeSW,
    ResizeSE,
}

#[derive(Default)]
struct State {
    drag_mode: Option<DragMode>,
    dragging_layer: Option<DefaultKey>,
    drag_start: Point,
    offset_at_drag_start: (f64, f64),
    scale_at_drag_start: f64,
    view_scale: f32,
    view_origin: (f32, f32),
}

impl State {
    fn virtual_to_widget(&self, vx: f64, vy: f64) -> (f32, f32) {
        (
            self.view_origin.0 + vx as f32 * self.view_scale,
            self.view_origin.1 + vy as f32 * self.view_scale,
        )
    }

    fn widget_to_virtual_delta(&self, dx: f32, dy: f32) -> (f64, f64) {
        if self.view_scale > 0.0 {
            (
                dx as f64 / self.view_scale as f64,
                dy as f64 / self.view_scale as f64,
            )
        } else {
            (0.0, 0.0)
        }
    }
}

fn layer_widget_rect(state: &State, layer: &LayerView, bounds: &Rectangle) -> Rectangle {
    let (lx, ly) = state.virtual_to_widget(layer.offset_x, layer.offset_y);
    let lw = layer.image_size.0 as f64 * layer.img_scale;
    let lh = layer.image_size.1 as f64 * layer.img_scale;
    Rectangle {
        x: bounds.x + lx,
        y: bounds.y + ly,
        width: lw as f32 * state.view_scale,
        height: lh as f32 * state.view_scale,
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

fn scene_bounds(monitors: &[MonitorGeometry], layers: &[LayerView]) -> (f64, f64, f64, f64) {
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

        let (scene_x, scene_y, scene_w, scene_h) = scene_bounds(self.monitors, &self.layers);

        let available_w = (size.width - 2.0 * PADDING).max(1.0);
        let available_h = (size.height - 2.0 * PADDING).max(1.0);

        let view_scale = if scene_w > 0.0 && scene_h > 0.0 {
            (available_w as f64 / scene_w).min(available_h as f64 / scene_h) as f32
        } else {
            1.0
        };

        let rendered_w = scene_w as f32 * view_scale;
        let rendered_h = scene_h as f32 * view_scale;
        let origin_x = PADDING + (available_w - rendered_w) / 2.0 - scene_x as f32 * view_scale;
        let origin_y = PADDING + (available_h - rendered_h) / 2.0 - scene_y as f32 * view_scale;

        let state = tree.state.downcast_mut::<State>();
        state.view_scale = view_scale;
        state.view_origin = (origin_x, origin_y);

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
            core::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position_in(bounds) {
                    let state = tree.state.downcast_mut::<State>();
                    let abs_pos = Point {
                        x: bounds.x + position.x,
                        y: bounds.y + position.y,
                    };

                    // First: check resize handles on the selected layer
                    if let Some(selected) = self.layers.iter().find(|l| l.selected) {
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

                    // Then: hit-test layers in reverse z-order for move/select
                    let mut hit = None;
                    let mut sorted: Vec<&LayerView> = self.layers.iter().collect();
                    sorted.sort_by_key(|l| std::cmp::Reverse(l.z_index));

                    for layer in &sorted {
                        let rect = layer_widget_rect(state, layer, &bounds);
                        if rect.contains(abs_pos) {
                            hit = Some(layer.id);
                            break;
                        }
                    }

                    if let Some(layer_id) = hit {
                        if let Some(layer) = self.layers.iter().find(|l| l.id == layer_id) {
                            state.drag_mode = Some(DragMode::Move);
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

            core::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let state = tree.state.downcast_mut::<State>();
                if let Some(layer_id) = state.dragging_layer
                    && let Some(mode) = state.drag_mode
                    && let Some(position) = cursor.position_in(bounds)
                {
                    let dx = position.x - state.drag_start.x;
                    let dy = position.y - state.drag_start.y;

                    match mode {
                        DragMode::Move => {
                            let (vdx, vdy) = state.widget_to_virtual_delta(dx, dy);
                            let new_x = state.offset_at_drag_start.0 + vdx;
                            let new_y = state.offset_at_drag_start.1 + vdy;
                            shell.publish((self.on_move)(layer_id, new_x, new_y));
                        }
                        DragMode::ResizeNW
                        | DragMode::ResizeNE
                        | DragMode::ResizeSW
                        | DragMode::ResizeSE => {
                            if let Some(layer) = self.layers.iter().find(|l| l.id == layer_id) {
                                let orig_scale = state.scale_at_drag_start;
                                let orig_w = layer.image_size.0 as f64 * orig_scale;
                                let orig_h = layer.image_size.1 as f64 * orig_scale;
                                let (vdx, vdy) = state.widget_to_virtual_delta(dx, dy);

                                // Compute new scale based on drag direction
                                // Use the diagonal distance for uniform scaling
                                let (scale_dx, scale_dy) = match mode {
                                    DragMode::ResizeSE => (vdx, vdy),
                                    DragMode::ResizeNW => (-vdx, -vdy),
                                    DragMode::ResizeNE => (vdx, -vdy),
                                    DragMode::ResizeSW => (-vdx, vdy),
                                    _ => (0.0, 0.0),
                                };

                                // Use the axis with the larger delta for scale
                                let ratio = if orig_w.abs() > orig_h.abs() {
                                    (orig_w + scale_dx) / orig_w
                                } else {
                                    (orig_h + scale_dy) / orig_h
                                };
                                let new_scale = (orig_scale * ratio).clamp(0.05, 10.0);
                                let new_w = layer.image_size.0 as f64 * new_scale;
                                let new_h = layer.image_size.1 as f64 * new_scale;

                                // Anchor the opposite corner
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
                            }
                        }
                    }
                    shell.capture_event();
                }
            }

            core::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let state = tree.state.downcast_mut::<State>();
                if state.dragging_layer.is_some() {
                    state.dragging_layer = None;
                    state.drag_mode = None;
                    shell.capture_event();
                }
            }

            core::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if cursor.is_over(bounds)
                    && let Some(selected) = self.layers.iter().find(|l| l.selected)
                {
                    let scroll_y = match delta {
                        mouse::ScrollDelta::Lines { y, .. } => *y,
                        mouse::ScrollDelta::Pixels { y, .. } => *y / 50.0,
                    };
                    let scale_factor = 1.0 + scroll_y as f64 * 0.1;
                    let new_scale = (selected.img_scale * scale_factor).clamp(0.05, 10.0);
                    shell.publish((self.on_scale)(selected.id, new_scale));
                    shell.capture_event();
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

        // During active drag, show the appropriate cursor
        if let Some(mode) = state.drag_mode {
            return match mode {
                DragMode::Move => mouse::Interaction::Grabbing,
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

            // Check handles on selected layer first
            if let Some(selected) = self.layers.iter().find(|l| l.selected) {
                let rect = layer_widget_rect(state, selected, &bounds);
                if hit_test_handles(&rect, abs_pos).is_some() {
                    return mouse::Interaction::ResizingDiagonallyDown;
                }
            }

            // Check if over any layer
            for layer in &self.layers {
                let rect = layer_widget_rect(state, layer, &bounds);
                if rect.contains(abs_pos) {
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

        // Draw layers sorted by z_index (bottom to top)
        let mut sorted: Vec<&LayerView> = self.layers.iter().collect();
        sorted.sort_by_key(|l| l.z_index);

        for layer in &sorted {
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

            // Selection border + handles for selected layer
            if layer.selected {
                if let Some(clipped) = layer_rect.intersection(&bounds) {
                    renderer.fill_quad(
                        Quad {
                            bounds: clipped,
                            border: Border {
                                color: cosmic_theme.accent_color().into(),
                                radius: 0.0.into(),
                                width: SELECTION_BORDER_WIDTH,
                            },
                            shadow: Default::default(),
                            snap: true,
                        },
                        core::Background::Color(core::Color::TRANSPARENT),
                    );
                }

                // Draw corner resize handles
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

        // Draw monitor outlines on top
        for (i, monitor) in self.monitors.iter().enumerate() {
            let (mx, my) =
                state.virtual_to_widget(monitor.position.0 as f64, monitor.position.1 as f64);
            let mw = monitor.logical_size.0 as f32 * state.view_scale;
            let mh = monitor.logical_size.1 as f32 * state.view_scale;

            let mon_rect = Rectangle {
                x: bounds.x + mx,
                y: bounds.y + my,
                width: mw,
                height: mh,
            };

            let mut bg = cosmic_theme.palette.neutral_4;
            bg.alpha = 0.08;

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
                core::Background::Color(bg.into()),
            );

            // Monitor label
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
    }
}

impl<'a, Message: 'static + Clone> From<ExtendEditor<'a, Message>>
    for cosmic::Element<'a, Message>
{
    fn from(editor: ExtendEditor<'a, Message>) -> Self {
        Element::new(editor)
    }
}
