//! The editor view: owns the open document and the viewport interaction state.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    point, px, size, App, Bounds, Context, CursorStyle, ExternalPaths, FocusHandle, Focusable,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PinchEvent, Pixels, Point, Render, RenderImage, ScrollDelta, ScrollEvent, SharedString, Size,
    Styled, Window, div, prelude::*, rgb,
};
use nodapix_core::{EditState, ImageDocument};
use nodapix_imaging::{ImageBackend, RustImageBackend};

use crate::actions::{
    ActualSize, Fit, FlipH, FlipV, Open, Redo, ResetAll, RotateCCW, RotateCW, ToggleCompare, Undo,
    ZoomIn, ZoomOut,
};
use crate::canvas::{fit_scale, Viewport, Zoom, MAX_SCALE, MIN_SCALE};
use crate::render::texture_from_rgba8;

const BACKDROP: u32 = 0x14_14_16;
const CHROME: u32 = 0x1f_1f_22;
const TEXT: u32 = 0xe6_e6_ea;
const MUTED: u32 = 0x8a_8a_93;

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
                    name,
                    self.image.width as u32,
                    self.image.height as u32
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

        // Keep the image point under `anchor` stationary as the scale changes:
        // pan' = (p - c) * (1 - k) + pan * k, with k = new / old.
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

    // ---------------------------------------------------------------------- view

    fn status_bar(&self) -> gpui::Div {
        let zoom_text = format!("{:.0}%", self.scale() * 100.0);
        let mode = if self.compare { "Original" } else { "Edited" };

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
                    .gap_2()
                    .child(div().text_color(rgb(TEXT)).child(self.status.clone()))
                    .child(div().text_color(rgb(MUTED)).child(mode)),
            )
            .child(div().text_color(rgb(MUTED)).child(zoom_text))
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
            .child(self.status_bar())
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
            )
    }
}
