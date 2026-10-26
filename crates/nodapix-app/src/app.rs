//! The editor view: owns the open document, the viewport interaction state, and the adjustment panel.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    div, point, px, size, App, Bounds, Context, CursorStyle, ExternalPaths, FocusHandle, Focusable,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PinchEvent, Pixels, Point, Render, RenderImage, ScrollDelta, ScrollEvent, SharedString, Size,
    Styled, Window, prelude::*, rgb, rgba,
};
use nodapix_core::{EditState, ImageDocument};
use nodapix_imaging::{
    resize, CropRect, ExportFormat, ExportOptions, ImageBackend, NativeBackend,
};

use crate::actions::{
    ActualSize, ApplyCrop, CancelCrop, Export, Fit, FlipH, FlipV, Open, Redo, ResetAdjustments,
    ResetAll, ResetCrop, RotateCCW, RotateCW, ToggleCompare, ToggleCrop, ToggleSidebar, Undo,
    ZoomIn, ZoomOut,
};
use crate::canvas::{
    fit_scale, hit_test_crop, CropHandle, Viewport, Zoom, MAX_SCALE, MIN_SCALE,
};
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

struct CropDrag {
    handle: CropHandle,
    start_pos: Point<Pixels>,
    start_crop: CropRect,
}

pub struct ExportSettings {
    pub is_open: bool,
    pub format: ExportFormat,
    pub quality: u8,
    pub scale_percent: u32,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            is_open: false,
            format: ExportFormat::Jpeg,
            quality: 90,
            scale_percent: 100,
        }
    }
}

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
    image_dest: Rc<Cell<Option<Bounds<Pixels>>>>,
    focus_handle: FocusHandle,
    sidebar_open: bool,
    slider_state: SliderState,

    // Crop state
    crop_mode: bool,
    draft_crop: Option<CropRect>,
    crop_drag: Option<CropDrag>,

    // Export sheet state
    export_settings: ExportSettings,
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
            image_dest: Rc::new(Cell::new(None)),
            focus_handle: cx.focus_handle(),
            sidebar_open: true,
            slider_state: SliderState::new(),
            crop_mode: false,
            draft_crop: None,
            crop_drag: None,
            export_settings: ExportSettings::default(),
        }
    }

    // ---------------------------------------------------------------- lifecycle

    pub fn open(&mut self, _: &Open, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = rfd::FileDialog::new().set_title("Open Image").pick_file() {
            self.load(path, cx);
        }
    }

    pub fn load(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match NativeBackend::new().decode_from_path(&path) {
            Ok(source) => {
                self.document = Some(ImageDocument::new(source, Some(path.clone())));
                self.compare = false;
                self.zoom = Zoom::Fit;
                self.pan = point(px(0.0), px(0.0));
                self.crop_mode = false;
                self.draft_crop = None;
                self.crop_drag = None;
                self.export_settings.is_open = false;
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
        self.crop_mode = false;
        self.draft_crop = None;
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

    // ---------------------------------------------------------------- crop tools

    pub fn toggle_crop(&mut self, _: &ToggleCrop, _: &mut Window, cx: &mut Context<Self>) {
        if self.crop_mode {
            self.crop_mode = false;
            self.draft_crop = None;
            self.crop_drag = None;
            self.status = "Crop cancelled".into();
        } else {
            if self.document.is_none() {
                return;
            }
            self.crop_mode = true;
            self.draft_crop = self
                .document
                .as_ref()
                .and_then(|d| d.state.crop)
                .or(Some(CropRect {
                    x: 0.05,
                    y: 0.05,
                    width: 0.9,
                    height: 0.9,
                }));
            self.status = "Crop mode: drag handles to select region. Enter to apply, Esc to cancel.".into();
        }
        cx.notify();
    }

    pub fn apply_crop(&mut self, _: &ApplyCrop, _: &mut Window, cx: &mut Context<Self>) {
        if self.export_settings.is_open {
            self.perform_export(cx);
            return;
        }
        if self.crop_mode {
            if let Some(crop) = self.draft_crop {
                self.edit(|state| {
                    state.crop = Some(crop);
                });
                self.status = "Crop applied".into();
            }
            self.crop_mode = false;
            self.draft_crop = None;
            self.crop_drag = None;
            cx.notify();
        }
    }

    pub fn cancel_crop(&mut self, _: &CancelCrop, _: &mut Window, cx: &mut Context<Self>) {
        if self.export_settings.is_open {
            self.export_settings.is_open = false;
            cx.notify();
            return;
        }
        if self.crop_mode {
            self.crop_mode = false;
            self.draft_crop = None;
            self.crop_drag = None;
            self.status = "Crop cancelled".into();
            cx.notify();
        }
    }

    pub fn reset_crop(&mut self, _: &ResetCrop, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(|state| {
            state.crop = None;
        });
        self.crop_mode = false;
        self.draft_crop = None;
        self.crop_drag = None;
        self.status = "Crop reset".into();
        cx.notify();
    }

    // ---------------------------------------------------------------- export dialog

    pub fn export(&mut self, _: &Export, _: &mut Window, cx: &mut Context<Self>) {
        if self.document.is_none() {
            return;
        }
        self.export_settings.is_open = true;
        cx.notify();
    }

    pub fn perform_export(&mut self, cx: &mut Context<Self>) {
        let Some(document) = &self.document else {
            return;
        };

        let default_name = document.file_name();
        let stem = std::path::Path::new(&default_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled");
        let ext = self.export_settings.format.extension();
        let proposed_name = format!("{}.{}", stem, ext);

        let filter_name = match self.export_settings.format {
            ExportFormat::Png => "PNG Image (*.png)",
            ExportFormat::Jpeg => "JPEG Image (*.jpg, *.jpeg)",
            ExportFormat::WebP => "WebP Image (*.webp)",
            ExportFormat::Tiff => "TIFF Image (*.tiff)",
        };

        let dialog = rfd::FileDialog::new()
            .set_title("Export Image")
            .set_file_name(&proposed_name)
            .add_filter(filter_name, &[ext]);

        if let Some(path) = dialog.save_file() {
            let mut rendered = document.render_export();
            if self.export_settings.scale_percent != 100 {
                let factor = self.export_settings.scale_percent as f32 / 100.0;
                let target_w = ((rendered.width as f32 * factor).round() as u32).max(1);
                let target_h = ((rendered.height as f32 * factor).round() as u32).max(1);
                rendered = resize(&rendered, target_w, target_h);
            }

            match NativeBackend::new().encode_to_path(
                &rendered,
                &path,
                &ExportOptions {
                    format: self.export_settings.format,
                    quality: self.export_settings.quality,
                },
            ) {
                Ok(()) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.display().to_string());
                    self.status = format!(
                        "Exported {} ({}×{}, {:?})",
                        name, rendered.width, rendered.height, self.export_settings.format
                    )
                    .into();
                    self.export_settings.is_open = false;
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
        if self.crop_mode {
            if let (Some(dest), Some(crop)) = (self.image_dest.get(), self.draft_crop) {
                if let Some(handle) = hit_test_crop(event.position, dest, crop) {
                    self.crop_drag = Some(CropDrag {
                        handle,
                        start_pos: event.position,
                        start_crop: crop,
                    });
                    cx.notify();
                    return;
                }
            }
        }
        self.drag_origin = Some((event.position, self.pan));
        cx.notify();
    }

    fn drag_pan(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(drag) = &self.crop_drag {
            let Some(dest) = self.image_dest.get() else {
                return;
            };
            let img_w = f32::from(dest.size.width);
            let img_h = f32::from(dest.size.height);
            if img_w <= 0.0 || img_h <= 0.0 {
                return;
            }

            let dx = (f32::from(event.position.x) - f32::from(drag.start_pos.x)) / img_w;
            let dy = (f32::from(event.position.y) - f32::from(drag.start_pos.y)) / img_h;
            let sc = drag.start_crop;

            let updated = match drag.handle {
                CropHandle::Inside => {
                    let max_x = (1.0 - sc.width).max(0.0);
                    let max_y = (1.0 - sc.height).max(0.0);
                    CropRect {
                        x: (sc.x + dx).clamp(0.0, max_x),
                        y: (sc.y + dy).clamp(0.0, max_y),
                        width: sc.width,
                        height: sc.height,
                    }
                }
                CropHandle::TopLeft => {
                    let max_x = sc.x + sc.width - 0.05;
                    let max_y = sc.y + sc.height - 0.05;
                    let new_x = (sc.x + dx).clamp(0.0, max_x);
                    let new_y = (sc.y + dy).clamp(0.0, max_y);
                    CropRect {
                        x: new_x,
                        y: new_y,
                        width: (sc.x + sc.width) - new_x,
                        height: (sc.y + sc.height) - new_y,
                    }
                }
                CropHandle::Top => {
                    let max_y = sc.y + sc.height - 0.05;
                    let new_y = (sc.y + dy).clamp(0.0, max_y);
                    CropRect {
                        x: sc.x,
                        y: new_y,
                        width: sc.width,
                        height: (sc.y + sc.height) - new_y,
                    }
                }
                CropHandle::TopRight => {
                    let max_y = sc.y + sc.height - 0.05;
                    let new_y = (sc.y + dy).clamp(0.0, max_y);
                    let new_w = (sc.width + dx).clamp(0.05, 1.0 - sc.x);
                    CropRect {
                        x: sc.x,
                        y: new_y,
                        width: new_w,
                        height: (sc.y + sc.height) - new_y,
                    }
                }
                CropHandle::Right => {
                    let new_w = (sc.width + dx).clamp(0.05, 1.0 - sc.x);
                    CropRect {
                        x: sc.x,
                        y: sc.y,
                        width: new_w,
                        height: sc.height,
                    }
                }
                CropHandle::BottomRight => {
                    let new_w = (sc.width + dx).clamp(0.05, 1.0 - sc.x);
                    let new_h = (sc.height + dy).clamp(0.05, 1.0 - sc.y);
                    CropRect {
                        x: sc.x,
                        y: sc.y,
                        width: new_w,
                        height: new_h,
                    }
                }
                CropHandle::Bottom => {
                    let new_h = (sc.height + dy).clamp(0.05, 1.0 - sc.y);
                    CropRect {
                        x: sc.x,
                        y: sc.y,
                        width: sc.width,
                        height: new_h,
                    }
                }
                CropHandle::BottomLeft => {
                    let max_x = sc.x + sc.width - 0.05;
                    let new_x = (sc.x + dx).clamp(0.0, max_x);
                    let new_h = (sc.height + dy).clamp(0.05, 1.0 - sc.y);
                    CropRect {
                        x: new_x,
                        y: sc.y,
                        width: (sc.x + sc.width) - new_x,
                        height: new_h,
                    }
                }
                CropHandle::Left => {
                    let max_x = sc.x + sc.width - 0.05;
                    let new_x = (sc.x + dx).clamp(0.0, max_x);
                    CropRect {
                        x: new_x,
                        y: sc.y,
                        width: (sc.x + sc.width) - new_x,
                        height: sc.height,
                    }
                }
            };

            self.draft_crop = Some(updated);
            cx.notify();
            return;
        }

        let Some((origin, pan_start)) = self.drag_origin else {
            return;
        };
        let pan_x = f32::from(pan_start.x) + (f32::from(event.position.x) - f32::from(origin.x));
        let pan_y = f32::from(pan_start.y) + (f32::from(event.position.y) - f32::from(origin.y));
        self.pan = point(px(pan_x), px(pan_y));
        cx.notify();
    }

    fn end_pan(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.crop_drag = None;
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

    fn render_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
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
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                this.reset_adjustments(&ResetAdjustments, window, cx);
                            }))
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
                    // Section 0: Transform & Crop
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
                                    .child("Transform & Crop"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .flex_1()
                                            .py(px(4.0))
                                            .text_center()
                                            .text_xs()
                                            .rounded_md()
                                            .bg(if self.crop_mode { rgb(ACCENT) } else { rgb(0x2d_2d_36) })
                                            .text_color(if self.crop_mode { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                                this.toggle_crop(&ToggleCrop, window, cx);
                                            }))
                                            .child(if self.crop_mode { "Exit Crop" } else { "✂ Crop" }),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .py(px(4.0))
                                            .text_center()
                                            .text_xs()
                                            .rounded_md()
                                            .bg(rgb(0x2d_2d_36))
                                            .text_color(rgb(TEXT_PRIMARY))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                                this.rotate_cw(&RotateCW, window, cx);
                                            }))
                                            .child("Rotate ↻"),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .py(px(4.0))
                                            .text_center()
                                            .text_xs()
                                            .rounded_md()
                                            .bg(rgb(0x2d_2d_36))
                                            .text_color(rgb(TEXT_PRIMARY))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                                this.flip_h(&FlipH, window, cx);
                                            }))
                                            .child("Flip ⇄"),
                                    ),
                            ),
                    )
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

    fn render_export_modal(&self, cx: &mut Context<Self>) -> gpui::Div {
        let (orig_w, orig_h) = self
            .document
            .as_ref()
            .map(|d| d.output_size())
            .unwrap_or((1, 1));
        let scale = self.export_settings.scale_percent as f32 / 100.0;
        let out_w = ((orig_w as f32 * scale).round() as u32).max(1);
        let out_h = ((orig_h as f32 * scale).round() as u32).max(1);
        let current_fmt = self.export_settings.format;
        let current_quality = self.export_settings.quality;
        let current_scale = self.export_settings.scale_percent;

        let formats = [
            (ExportFormat::Jpeg, "JPEG"),
            (ExportFormat::Png, "PNG"),
            (ExportFormat::WebP, "WebP"),
            (ExportFormat::Tiff, "TIFF"),
        ];

        let scales = [
            (100, "100%"),
            (75, "75%"),
            (50, "50%"),
            (25, "25%"),
        ];

        let qualities = [70, 80, 85, 90, 95, 100];

        div()
            .absolute()
            .size_full()
            .bg(rgba(0x00_00_00_aa))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(460.0))
                    .bg(rgb(0x22_22_28))
                    .border_1()
                    .border_color(rgb(BORDER_COLOR))
                    .rounded_xl()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    // Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Export Image"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(self.document.as_ref().map(|d| d.file_name()).unwrap_or_default()),
                            ),
                    )
                    // Format selection
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(MUTED))
                                    .child("FORMAT"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .children(formats.iter().map(|(fmt, name)| {
                                        let is_selected = *fmt == current_fmt;
                                        let target_fmt = *fmt;
                                        div()
                                            .flex_1()
                                            .py(px(6.0))
                                            .text_center()
                                            .text_xs()
                                            .font_weight(if is_selected { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                                            .rounded_md()
                                            .bg(if is_selected { rgb(ACCENT) } else { rgb(0x2d_2d_36) })
                                            .text_color(if is_selected { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                            .border_1()
                                            .border_color(if is_selected { rgb(ACCENT) } else { rgb(BORDER_COLOR) })
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                this.export_settings.format = target_fmt;
                                                cx.notify();
                                            }))
                                            .child(*name)
                                    })),
                            ),
                    )
                    // Quality selection (JPEG & WebP)
                    .child({
                        let is_lossy = matches!(current_fmt, ExportFormat::Jpeg | ExportFormat::WebP);
                        if is_lossy {
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(rgb(MUTED))
                                                .child("QUALITY"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .text_color(rgb(TEXT_PRIMARY))
                                                .child(format!("{}%", current_quality)),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .children(qualities.iter().map(|&q| {
                                            let is_selected = q == current_quality;
                                            div()
                                                .flex_1()
                                                .py(px(5.0))
                                                .text_center()
                                                .text_xs()
                                                .rounded_md()
                                                .bg(if is_selected { rgb(ACCENT) } else { rgb(0x2d_2d_36) })
                                                .text_color(if is_selected { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                    this.export_settings.quality = q;
                                                    cx.notify();
                                                }))
                                                .child(format!("{}%", q))
                                        })),
                                )
                        } else {
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(if current_fmt == ExportFormat::Png {
                                    "PNG uses lossless compression (maximum fidelity)."
                                } else {
                                    "TIFF preserves uncompressed 32-bit archival data."
                                })
                        }
                    })
                    // Dimensions & Scaling
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(MUTED))
                                            .child("RESOLUTION SCALE"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(0x30_d1_58))
                                            .child(format!("{} × {} px", out_w, out_h)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .children(scales.iter().map(|&(s, label)| {
                                        let is_selected = s == current_scale;
                                        div()
                                            .flex_1()
                                            .py(px(6.0))
                                            .text_center()
                                            .text_xs()
                                            .rounded_md()
                                            .bg(if is_selected { rgb(ACCENT) } else { rgb(0x2d_2d_36) })
                                            .text_color(if is_selected { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                            .border_1()
                                            .border_color(if is_selected { rgb(ACCENT) } else { rgb(BORDER_COLOR) })
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                this.export_settings.scale_percent = s;
                                                cx.notify();
                                            }))
                                            .child(label)
                                    })),
                            ),
                    )
                    // Footer Buttons
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_3()
                            .pt_2()
                            .border_t_1()
                            .border_color(rgb(BORDER_COLOR))
                            .child(
                                div()
                                    .px_4()
                                    .py(px(6.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x34_34_3e))
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                        this.export_settings.is_open = false;
                                        cx.notify();
                                    }))
                                    .child("Cancel"),
                            )
                            .child(
                                div()
                                    .px_4()
                                    .py(px(6.0))
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .rounded_md()
                                    .bg(rgb(ACCENT))
                                    .text_color(rgb(0xff_ff_ff))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                        this.perform_export(cx);
                                    }))
                                    .child("Save Image As…"),
                            ),
                    ),
            )
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> gpui::Div {
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
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                this.toggle_sidebar(&ToggleSidebar, window, cx);
                            }))
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
            crop: self.draft_crop,
            crop_mode: self.crop_mode,
            bounds_out: Some(self.bounds.clone()),
            image_dest_out: Some(self.image_dest.clone()),
        };

        let mut root = div()
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
            .on_action(cx.listener(Self::toggle_crop))
            .on_action(cx.listener(Self::apply_crop))
            .on_action(cx.listener(Self::cancel_crop))
            .on_action(cx.listener(Self::reset_crop))
            .on_action(cx.listener(Self::export));

        // Floating Crop toolbar when active
        if self.crop_mode {
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .py_2()
                    .bg(rgb(0x28_28_32))
                    .border_b_1()
                    .border_color(rgb(BORDER_COLOR))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(0xff_d6_0a))
                                    .child("Crop Mode Active"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("· Drag handles or region to adjust. Press Enter to apply, Esc to cancel."),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .px_3()
                                    .py(px(4.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x3a_3a_44))
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.cancel_crop(&CancelCrop, window, cx);
                                    }))
                                    .child("Cancel"),
                            )
                            .child(
                                div()
                                    .px_3()
                                    .py(px(4.0))
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .rounded_md()
                                    .bg(rgb(ACCENT))
                                    .text_color(rgb(0xff_ff_ff))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.apply_crop(&ApplyCrop, window, cx);
                                    }))
                                    .child("Apply Crop"),
                            ),
                    ),
            );
        }

        root = root.child({
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
                        .cursor(if self.crop_mode { CursorStyle::Crosshair } else { CursorStyle::OpenHand })
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
                        .child(self.render_panel(cx)),
                );
            }
            split
        })
        .child(self.status_bar(cx));

        // Export Modal overlay if active
        if self.export_settings.is_open {
            root = root.child(self.render_export_modal(cx));
        }

        root
    }
}
