//! The editor view: owns the open document, the viewport interaction state, and the adjustment panel.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    div, point, px, size, App, Bounds, Context, CursorStyle, ExternalPaths, FocusHandle, Focusable,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PinchEvent, Pixels, Point, Render, RenderImage, ScrollDelta, ScrollEvent, SharedString, Size,
    Styled, Window, prelude::*, rgb,
};
use nodapix_core::{EditState, ImageDocument};
use nodapix_imaging::{ExportFormat, ExportOptions, ImageBackend, RustImageBackend};

use crate::actions::{
    ActualSize, Export, Fit, FlipH, FlipV, Open, Redo, ResetAdjustments, ResetAll, RotateCCW,
    RotateCW, ToggleCompare, ToggleSidebar, Undo, ZoomIn, ZoomOut,
};
use crate::canvas::{fit_scale, Viewport, Zoom, MAX_SCALE, MIN_SCALE};
use crate::panel::{
    render_slider_row, SliderId, SliderSpec, SliderState, ALL_SPECS, BORDER_COLOR, PANEL_BG,
    PANEL_WIDTH, SECTION_BG, TEXT_PRIMARY,
};
use crate::render::texture_from_rgba8;

const BACKDROP: u32 = 0x14_14_16;
const CHROME: u32 = 0x1f_1f_22;
const TEXT: u32 = 0xe6_e6_ea;
const MUTED: u32 = 0x8a_8a_93;
const ACCENT: u32 = 0x0a_84_ff;

pub const KEY_CONTEXT: &str = "NodaPix";

pub struct Editor {
    document: Option<ImageDocument>,
    edited: Option<Arc<RenderImage>>,
    original: Option<Arc<RenderImage>>,
    image: Size<f32>,
    zoom: Zoom,
    pan: Point<Pixels>,
    compare: bool,
    drag_origin: Option<(Point<Pixels>, Point<Pixels>)>,
    status: SharedString,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    focus_handle: FocusHandle,
    sidebar_open: bool,
    slider_state: SliderState,
}

impl Editor {
    pub fn new(cx: &mut App) -> Self {
        Self {
            document: None,
            edited: None,
            original: None,
            image: size(1.0, 1.0),
            zoom: Zoom::Fit,
            pan: point(px(0.0), px(0.0)),
            compare: false,
            drag_origin: None,
            status: "Open an image to begin".into(),
            bounds: Rc::new(Cell::new(None)),
            focus_handle: cx.focus_handle(),
            sidebar_open: true,
            slider_state: SliderState::new(),
        }
    }

    // ---------------------------------------------------------------- lifecycle

    pub fn open(&mut self, _: &Open, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = rfd::FileDialog::new().set_title("Open Image").pick_file() {
            self.load(path, cx);
        }
    }

    pub fn load(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match RustImageBackend::new().decode_from_path(&path) {
            Ok(source) => {
                self.document = Some(ImageDocument::new(source, Some(path.clone())));
                self.compare = false;
                self.zoom = Zoom::Fit;
                self.pan = point(px(0.0), px(0.0));
                self.sync_geometry();
                self.rebuild_textures();
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                self.status = format!(
                    "{} · {}×{}",
                    name, self.image.width as u32, self.image.height as u32
                )
                .into();
            }
            Err(err) => {
                self.status = format!("Cannot open {}: {}", path.display(), err).into();
            }
        }
        cx.notify();
    }

    fn sync_geometry(&mut self) {
        if let Some(document) = &self.document {
            let (width, height) = document.output_size();
            self.image = size(width as f32, height as f32);
        }
    }

    /// Re-render the preview textures from the current edit state.
    fn rebuild_textures(&mut self) {
        let Some(document) = &self.document else {
            self.edited = None;
            self.original = None;
            return;
        };
        self.edited = Some(texture_from_rgba8(&document.render_preview()));
        self.original = Some(texture_from_rgba8(&document.render_original_preview()));
    }

    // ------------------------------------------------------------------ editing

    fn edit(&mut self, change: impl FnOnce(&mut EditState)) {
        let Some(document) = &mut self.document else {
            return;
        };
        document.commit_checkpoint();
        change(&mut document.state);
        self.sync_geometry();
        self.rebuild_textures();
    }

    pub fn rotate_cw(&mut self, _: &RotateCW, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(|state| state.rotate_cw());
        cx.notify();
    }

    pub fn rotate_ccw(&mut self, _: &RotateCCW, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(|state| state.rotate_ccw());
        cx.notify();
    }

    pub fn flip_h(&mut self, _: &FlipH, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(|state| state.toggle_flip_h());
        cx.notify();
    }

    pub fn flip_v(&mut self, _: &FlipV, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(|state| state.toggle_flip_v());
        cx.notify();
    }

    pub fn reset_all(&mut self, _: &ResetAll, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(|state| state.reset_all());
        self.zoom = Zoom::Fit;
        self.pan = point(px(0.0), px(0.0));
        cx.notify();
    }

    pub fn reset_adjustments(
        &mut self,
        _: &ResetAdjustments,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(|state| state.reset_adjustments());
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        cx.notify();
    }

    pub fn export(&mut self, _: &Export, _: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = &self.document else {
            return;
        };

        let default_name = document.file_name();
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Export Image")
            .set_file_name(&default_name)
            .add_filter("PNG Image (*.png)", &["png"])
            .add_filter("JPEG Image (*.jpg, *.jpeg)", &["jpg", "jpeg"])
            .add_filter("WebP Image (*.webp)", &["webp"])
            .save_file()
        {
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("png");
            let format = ExportFormat::from_extension(ext).unwrap_or(ExportFormat::Png);
            let rendered = document.render_export();
            match RustImageBackend::new().encode_to_path(
                &rendered,
                &path,
                &ExportOptions {
                    format,
                    quality: 90,
                },
            ) {
                Ok(()) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.display().to_string());
                    self.status = format!("Exported {}", name).into();
                }
                Err(e) => {
                    self.status = format!("Export failed: {}", e).into();
                }
            }
            cx.notify();
        }
    }

    pub fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(document) = &mut self.document {
            if document.undo() {
                self.sync_geometry();
                self.rebuild_textures();
            }
        }
        cx.notify();
    }

    pub fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(document) = &mut self.document {
            if document.redo() {
                self.sync_geometry();
                self.rebuild_textures();
            }
        }
        cx.notify();
    }

    pub fn toggle_compare(&mut self, _: &ToggleCompare, _: &mut Window, cx: &mut Context<Self>) {
        self.compare = !self.compare;
        cx.notify();
    }

    // ----------------------------------------------------------------- viewport

    fn available(&self) -> Size<f32> {
        self.bounds
            .get()
            .map(|b| size(f32::from(b.size.width), f32::from(b.size.height)))
            .unwrap_or(self.image)
    }

    fn scale(&self) -> f32 {
        match self.zoom {
            Zoom::Percent(scale) => scale,
            Zoom::Fit => fit_scale(self.image, self.available()),
        }
    }

    fn zoom_to(&mut self, target: f32, anchor: Option<Point<Pixels>>) {
        let old = self.scale();
        let new = target.clamp(MIN_SCALE, MAX_SCALE);
        if (new - old).abs() < f32::EPSILON {
            return;
        }

        if let (Some(anchor), Some(bounds)) = (anchor, self.bounds.get()) {
            let k = new / old;
            let origin_x = f32::from(bounds.origin.x);
            let origin_y = f32::from(bounds.origin.y);
            let size_w = f32::from(bounds.size.width);
            let size_h = f32::from(bounds.size.height);
            let centre_x = origin_x + size_w / 2.0;
            let centre_y = origin_y + size_h / 2.0;
            let anchor_x = f32::from(anchor.x);
            let anchor_y = f32::from(anchor.y);
            let pan_x = f32::from(self.pan.x);
            let pan_y = f32::from(self.pan.y);
            self.pan = point(
                px((anchor_x - centre_x) * (1.0 - k) + pan_x * k),
                px((anchor_y - centre_y) * (1.0 - k) + pan_y * k),
            );
        }

        self.zoom = Zoom::Percent(new);
    }

    pub fn zoom_in(&mut self, _: &ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = window.mouse_position();
        self.zoom_to(self.scale() * 1.25, Some(anchor));
        cx.notify();
    }

    pub fn zoom_out(&mut self, _: &ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = window.mouse_position();
        self.zoom_to(self.scale() / 1.25, Some(anchor));
        cx.notify();
    }

    pub fn fit(&mut self, _: &Fit, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom = Zoom::Fit;
        self.pan = point(px(0.0), px(0.0));
        cx.notify();
    }

    pub fn actual_size(&mut self, _: &ActualSize, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_to(1.0, None);
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (dx, dy) = match event.delta {
            ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
            ScrollDelta::Lines(delta) => (delta.x * 16.0, delta.y * 16.0),
        };

        if event.modifiers.platform || event.modifiers.control {
            let factor = (-dy * 0.01).exp2().clamp(0.8, 1.25);
            self.zoom_to(self.scale() * factor, Some(event.position));
        } else {
            let pan_x = f32::from(self.pan.x);
            let pan_y = f32::from(self.pan.y);
            self.pan = point(px(pan_x - dx), px(pan_y - dy));
        }

        cx.notify();
    }

    fn on_pinch(&mut self, event: &PinchEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_to(self.scale() * (1.0 + event.delta), Some(event.position));
        cx.notify();
    }

    fn on_drop(&mut self, paths: &ExternalPaths, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = paths.paths().first() {
            self.load(path.clone(), cx);
        }
    }

    fn begin_pan(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.drag_origin = Some((event.position, self.pan));
        cx.notify();
    }

    fn drag_pan(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((origin, pan_start)) = self.drag_origin else {
            return;
        };
        let pan_x = f32::from(pan_start.x) + (f32::from(event.position.x) - f32::from(origin.x));
        let pan_y = f32::from(pan_start.y) + (f32::from(event.position.y) - f32::from(origin.y));
        self.pan = point(px(pan_x), px(pan_y));
        cx.notify();
    }

    fn end_pan(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.drag_origin = None;
        cx.notify();
    }

    // ------------------------------------------------------------- slider events

    fn update_slider(&mut self, id: SliderId, val: f32) {
        if let Some(document) = &mut self.document {
            if let Some(spec) = ALL_SPECS.iter().find(|s| s.id == id) {
                spec.set_value(&mut document.state, val);
                self.rebuild_textures();
            }
        }
    }

    fn on_slider_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let px_pos = f32::from(event.position.x);
        let py_pos = f32::from(event.position.y);

        let mut hit = None;
        {
            let bounds_map = self.slider_state.bounds.borrow();
            for spec in ALL_SPECS {
                if let Some(bounds) = bounds_map.get(&spec.id) {
                    let bx = f32::from(bounds.origin.x);
                    let by = f32::from(bounds.origin.y);
                    let bw = f32::from(bounds.size.width);
                    let bh = f32::from(bounds.size.height);

                    if px_pos >= bx && px_pos <= bx + bw && py_pos >= by - 6.0 && py_pos <= by + bh + 6.0 {
                        let norm = ((px_pos - bx) / bw).clamp(0.0, 1.0);
                        let val = spec.from_normalized(norm);
                        hit = Some((spec.id, val));
                        break;
                    }
                }
            }
        }

        if let Some((id, val)) = hit {
            self.slider_state.active_slider = Some(id);
            if let Some(doc) = &mut self.document {
                doc.commit_checkpoint();
            }
            self.update_slider(id, val);
            cx.notify();
        }
    }

    fn on_slider_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut update = None;
        if let Some(id) = self.slider_state.active_slider {
            if let Some(spec) = ALL_SPECS.iter().find(|s| s.id == id) {
                if let Some(bounds) = self.slider_state.bounds.borrow().get(&id).copied() {
                    let bx = f32::from(bounds.origin.x);
                    let bw = f32::from(bounds.size.width);
                    let px_pos = f32::from(event.position.x);
                    let norm = ((px_pos - bx) / bw).clamp(0.0, 1.0);
                    let val = spec.from_normalized(norm);
                    update = Some((id, val));
                }
            }
        }
        if let Some((id, val)) = update {
            self.update_slider(id, val);
            cx.notify();
        }
    }

    fn on_slider_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.slider_state.active_slider.is_some() {
            self.slider_state.active_slider = None;
            cx.notify();
        }
    }

    // ---------------------------------------------------------------------- view

    fn render_panel(&self) -> gpui::Div {
        let Some(document) = &self.document else {
            return div();
        };

        let state = &document.state;
        let bounds = self.slider_state.bounds.clone();

        div()
            .w(px(PANEL_WIDTH))
            .h_full()
            .bg(rgb(PANEL_BG))
            .border_l_1()
            .border_color(rgb(BORDER_COLOR))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                // Panel Header
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(BORDER_COLOR))
                    .bg(rgb(CHROME))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(TEXT_PRIMARY))
                            .child("Adjustments"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(ACCENT))
                            .cursor(CursorStyle::PointingHand)
                            .child("Reset"),
                    ),
            )
            .child(
                // Scrollable Controls Body
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_3()
                    .overflow_hidden()
                    // Section 1: Light
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_2()
                            .rounded_md()
                            .bg(rgb(SECTION_BG))
                            .border_1()
                            .border_color(rgb(BORDER_COLOR))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Light"),
                            )
                            .child(render_slider_row(
                                SliderSpec::EXPOSURE,
                                SliderSpec::EXPOSURE.get_value(state),
                                bounds.clone(),
                            ))
                            .child(render_slider_row(
                                SliderSpec::BRIGHTNESS,
                                SliderSpec::BRIGHTNESS.get_value(state),
                                bounds.clone(),
                            ))
                            .child(render_slider_row(
                                SliderSpec::CONTRAST,
                                SliderSpec::CONTRAST.get_value(state),
                                bounds.clone(),
                            ))
                            .child(render_slider_row(
                                SliderSpec::HIGHLIGHTS,
                                SliderSpec::HIGHLIGHTS.get_value(state),
                                bounds.clone(),
                            ))
                            .child(render_slider_row(
                                SliderSpec::SHADOWS,
                                SliderSpec::SHADOWS.get_value(state),
                                bounds.clone(),
                            )),
                    )
                    // Section 2: Color
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_2()
                            .rounded_md()
                            .bg(rgb(SECTION_BG))
                            .border_1()
                            .border_color(rgb(BORDER_COLOR))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Color"),
                            )
                            .child(render_slider_row(
                                SliderSpec::SATURATION,
                                SliderSpec::SATURATION.get_value(state),
                                bounds.clone(),
                            ))
                            .child(render_slider_row(
                                SliderSpec::TEMPERATURE,
                                SliderSpec::TEMPERATURE.get_value(state),
                                bounds.clone(),
                            ))
                            .child(render_slider_row(
                                SliderSpec::TINT,
                                SliderSpec::TINT.get_value(state),
                                bounds.clone(),
                            )),
                    )
                    // Section 3: Detail
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_2()
                            .rounded_md()
                            .bg(rgb(SECTION_BG))
                            .border_1()
                            .border_color(rgb(BORDER_COLOR))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Detail"),
                            )
                            .child(render_slider_row(
                                SliderSpec::SHARPNESS,
                                SliderSpec::SHARPNESS.get_value(state),
                                bounds.clone(),
                            )),
                    ),
            )
    }

    fn status_bar(&self) -> gpui::Div {
        let zoom_text = format!("{:.0}%", self.scale() * 100.0);
        let mode = if self.compare { "Original" } else { "Edited" };
        let panel_btn_text = if self.sidebar_open { "Hide Panel" } else { "Show Panel" };

        div()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .h(px(28.0))
            .bg(rgb(CHROME))
            .text_xs()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().text_color(rgb(TEXT)).child(self.status.clone()))
                    .child(div().text_color(rgb(MUTED)).child(mode)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().text_color(rgb(MUTED)).child(zoom_text))
                    .child(
                        div()
                            .text_color(rgb(ACCENT))
                            .cursor(CursorStyle::PointingHand)
                            .child(panel_btn_text),
                    ),
            )
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let texture = if self.compare {
            self.original.clone().or_else(|| self.edited.clone())
        } else {
            self.edited.clone()
        };

        let viewport = Viewport {
            texture,
            image: self.image,
            zoom: self.zoom,
            pan: self.pan,
            backdrop: BACKDROP,
            frame: self.document.is_some(),
            bounds_out: Some(self.bounds.clone()),
        };

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(BACKDROP))
            .text_color(rgb(TEXT))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::toggle_compare))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::fit))
            .on_action(cx.listener(Self::actual_size))
            .on_action(cx.listener(Self::rotate_cw))
            .on_action(cx.listener(Self::rotate_ccw))
            .on_action(cx.listener(Self::flip_h))
            .on_action(cx.listener(Self::flip_v))
            .on_action(cx.listener(Self::reset_all))
            .on_action(cx.listener(Self::reset_adjustments))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::export))
            .child({
                let mut split = div()
                    .flex_1()
                    .relative()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_1()
                            .relative()
                            .overflow_hidden()
                            .cursor(CursorStyle::OpenHand)
                            .on_scroll(cx.listener(Self::on_scroll))
                            .on_pinch(cx.listener(Self::on_pinch))
                            .on_drop(cx.listener(Self::on_drop))
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_pan))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::end_pan))
                            .on_mouse_move(cx.listener(Self::drag_pan))
                            .child(viewport.element()),
                    );

                if self.sidebar_open && self.document.is_some() {
                    split = split.child(
                        div()
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_slider_mouse_down))
                            .on_mouse_move(cx.listener(Self::on_slider_mouse_move))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_slider_mouse_up))
                            .child(self.render_panel()),
                    );
                }
                split
            })
            .child(self.status_bar())
    }
}
