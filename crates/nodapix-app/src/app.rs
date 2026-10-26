//! The editor view: owns the open document, the viewport interaction state, and the adjustment panel.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use gpui::{
    canvas, div, fill, point, px, size, App, Bounds, Context, Corners, CursorStyle, ExternalPaths,
    FocusHandle, Focusable, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, PinchEvent, Pixels, Point, Render, RenderImage, ScrollDelta, ScrollEvent,
    SharedString, Size, Styled, Window, prelude::*, rgb, rgba,
};
use nodapix_core::{execute_pipeline, render_proxy_preview, EditState, ImageDocument};
use nodapix_imaging::{
    attach_exif, calculate_auto_levels, calculate_white_balance_from_sample, compute_histogram,
    resize, CropRatioPreset, CropRect, ExportFormat, ExportOptions, FilterPreset, ImageBackend,
    ImageHistogram, ImageMetadata, NativeBackend, PixelBuffer, TEMPERATURE_NEUTRAL_KELVIN,
};

use crate::actions::{
    ActualSize, ApplyCrop, AutoEnhance, CancelCrop, Export, Fit, FlipH, FlipV, NextImage, Open,
    PrevImage, Redo, ResetAdjustments, ResetAll, ResetCrop, RotateCCW, RotateCW, Save, SaveAs,
    ToggleCompare, ToggleCrop, ToggleInfo, ToggleResize, ToggleSidebar, ToggleThumbnailPane, Undo,
    ZoomIn, ZoomOut,
};
use crate::canvas::{
    fit_scale, hit_test_crop, CropHandle, Viewport, Zoom, MAX_SCALE, MIN_SCALE,
};
use crate::icon::LucideIcon;
use crate::panel::{
    render_histogram, render_slider_row, SliderId, SliderSpec, SliderState, ToolGroup, ALL_SPECS,
    BORDER_COLOR, PANEL_BG, PANEL_WIDTH, SECTION_BG, TEXT_MUTED, TEXT_PRIMARY,
};
use crate::render::texture_from_rgba8;

const BACKDROP: u32 = 0x14_14_16;
const CHROME: u32 = 0x1f_1f_22;
const TEXT: u32 = 0xe6_e6_ea;
const MUTED: u32 = 0x8a_8a_93;
const ACCENT: u32 = 0x0a_84_ff;

pub const KEY_CONTEXT: &str = "NodaPix";

/// Keeps concurrent temp names unique within this process.
static SAVE_NONCE: AtomicU64 = AtomicU64::new(0);

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// Replace `path` with `bytes` through a sibling temp file, so a crashed or failed
/// write can never leave the original half-written.
fn write_atomically(bytes: &[u8], path: &Path) -> Result<(), String> {
    let nonce = SAVE_NONCE.fetch_add(1, Ordering::SeqCst);
    let tmp = path.with_file_name(format!(
        ".{}.{}.{}.nodapix.tmp",
        file_label(path),
        std::process::id(),
        nonce
    ));
    let result = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("{}: {}", path.display(), e))
}

/// Re-embed the source file's EXIF in freshly encoded bytes. The orientation tag is
/// normalised because decoding already baked it into the pixels.
fn with_exif(encoded: Vec<u8>, format: ExportFormat, block: Option<&[u8]>) -> Vec<u8> {
    let Some(block) = block else {
        return encoded;
    };
    let mut block = block.to_vec();
    ImageMetadata::set_orientation(&mut block, 1);
    attach_exif(&encoded, format, &block)
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

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

#[derive(Clone, Debug)]
pub struct ResizeSettings {
    pub is_open: bool,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
    pub unit_percent: bool,
    pub preserve_aspect_ratio: bool,
    pub destination_folder: Option<PathBuf>,
    pub format: Option<ExportFormat>,
}

impl Default for ResizeSettings {
    fn default() -> Self {
        Self {
            is_open: false,
            width: 1920,
            height: 1080,
            original_width: 1920,
            original_height: 1080,
            unit_percent: false,
            preserve_aspect_ratio: true,
            destination_folder: None,
            format: None,
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
    render_generation: Arc<AtomicU64>,

    // Histogram
    histogram: Option<ImageHistogram>,

    // Crop state
    crop_mode: bool,
    draft_crop: Option<CropRect>,
    crop_preset: CropRatioPreset,
    crop_portrait: bool,
    crop_drag: Option<CropDrag>,
    crop_multiple_of: Option<u32>,

    // Export sheet state
    export_settings: ExportSettings,

    // Resize dialog state
    resize_settings: ResizeSettings,

    // Folder images & carousel
    folder_images: Vec<PathBuf>,
    folder_index: usize,
    thumbnail_cache: HashMap<PathBuf, Arc<RenderImage>>,
    thumbnail_pane_open: bool,
    file_size_bytes: u64,

    // Active tool group in right panel grid
    active_tool_group: Option<ToolGroup>,

    // EXIF metadata inspector
    metadata: Option<ImageMetadata>,
    exif_open: bool,
    /// The source file's own EXIF block, re-embedded into every export.
    exif_block: Option<Vec<u8>>,

    // Last rendered preview buffer for instant eyedropper pixel sampling
    last_rendered_preview: Option<Arc<PixelBuffer<u8>>>,

    // Horizon line straighten tool state
    horizon_mode: bool,
    horizon_start: Option<Point<Pixels>>,
    horizon_current: Option<Point<Pixels>>,

    // Eyedropper / White balance sampler state
    eyedropper_active: bool,
    eyedropper_sample: Option<([u8; 4], String, Point<Pixels>)>,

    /// Last title pushed to the window, so the render loop only re-sets it on change.
    window_title: SharedString,
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
            render_generation: Arc::new(AtomicU64::new(0)),
            histogram: None,
            crop_mode: false,
            draft_crop: None,
            crop_preset: CropRatioPreset::Free,
            crop_portrait: false,
            crop_drag: None,
            crop_multiple_of: None,
            export_settings: ExportSettings::default(),
            resize_settings: ResizeSettings::default(),
            folder_images: Vec::new(),
            folder_index: 0,
            thumbnail_cache: HashMap::new(),
            thumbnail_pane_open: true,
            file_size_bytes: 0,
            active_tool_group: None,
            metadata: None,
            exif_open: false,
            exif_block: None,
            last_rendered_preview: None,
            horizon_mode: false,
            horizon_start: None,
            horizon_current: None,
            eyedropper_active: false,
            eyedropper_sample: None,
            window_title: "NodaPix".into(),
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
                self.metadata = ImageMetadata::read_from_path(&path);
                self.exif_block = ImageMetadata::read_exif_block_from_path(&path);
                self.document = Some(ImageDocument::new(source, Some(path.clone())));
                self.compare = false;
                self.zoom = Zoom::Fit;
                self.pan = point(px(0.0), px(0.0));
                self.crop_mode = false;
                self.draft_crop = None;
                self.crop_preset = CropRatioPreset::Free;
                self.crop_portrait = false;
                self.crop_drag = None;
                self.export_settings.is_open = false;
                self.exif_open = false;
                self.sync_geometry();
                self.rebuild_original_texture();
                self.edited = self.original.clone();
                self.request_preview_render(cx);
                self.file_size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.scan_folder(&path, cx);
                self.update_status();
            }
            Err(err) => {
                self.status = format!("Cannot open {}: {}", path.display(), err).into();
            }
        }
        cx.notify();
    }

    fn update_status(&mut self) {
        let Some(document) = &self.document else {
            self.status = "Open an image to begin".into();
            return;
        };
        let (w, h) = document.output_size();
        let idx_str = if !self.folder_images.is_empty() {
            format!("{}/{} · ", self.folder_index + 1, self.folder_images.len())
        } else {
            String::new()
        };
        let size_str = if self.file_size_bytes > 0 {
            format!(" · {}", format_file_size(self.file_size_bytes))
        } else {
            String::new()
        };
        let zoom_pct = (self.scale() * 100.0).round() as u32;
        self.status = format!("{}{}×{}{size_str} · {}%", idx_str, w, h, zoom_pct).into();
    }

    fn scan_folder(&mut self, current_path: &Path, cx: &mut Context<Self>) {
        let Some(parent) = current_path.parent() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(parent) else {
            return;
        };
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                    let ext = ext.to_lowercase();
                    if matches!(
                        ext.as_str(),
                        "jpg" | "jpeg" | "png" | "webp" | "tiff" | "tif" | "bmp" | "heic" | "heif" | "avif" | "dng"
                    ) {
                        files.push(p);
                    }
                }
            }
        }
        files.sort();
        self.folder_index = files.iter().position(|p| p == current_path).unwrap_or(0);
        self.folder_images = files;
        self.request_thumbnails(cx);
    }

    fn request_thumbnails(&mut self, cx: &mut Context<Self>) {
        let images = self.folder_images.clone();
        let executor = cx.background_executor().clone();
        for path in images {
            if self.thumbnail_cache.contains_key(&path) {
                continue;
            }
            let p = path.clone();
            let p_decode = path.clone();
            let executor = executor.clone();
            cx.spawn(async move |this, cx| {
                let thumb = executor
                    .spawn(async move {
                        NativeBackend::new()
                            .decode_from_path(&p_decode)
                            .ok()
                            .map(|buf| {
                                let small = nodapix_imaging::downsample_to_fit(&buf, 140);
                                texture_from_rgba8(&small.to_u8_srgb())
                            })
                    })
                    .await;
                if let Some(texture) = thumb {
                    let _ = this.update(cx, |editor, cx| {
                        editor.thumbnail_cache.insert(p, texture);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    pub fn prev_image(&mut self, _: &PrevImage, _: &mut Window, cx: &mut Context<Self>) {
        if self.folder_images.is_empty() {
            return;
        }
        if self.folder_index == 0 {
            self.folder_index = self.folder_images.len() - 1;
        } else {
            self.folder_index -= 1;
        }
        let next_path = self.folder_images[self.folder_index].clone();
        self.load(next_path, cx);
    }

    pub fn next_image(&mut self, _: &NextImage, _: &mut Window, cx: &mut Context<Self>) {
        if self.folder_images.is_empty() {
            return;
        }
        if self.folder_index + 1 >= self.folder_images.len() {
            self.folder_index = 0;
        } else {
            self.folder_index += 1;
        }
        let next_path = self.folder_images[self.folder_index].clone();
        self.load(next_path, cx);
    }

    pub fn toggle_thumbnail_pane(
        &mut self,
        _: &ToggleThumbnailPane,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.thumbnail_pane_open = !self.thumbnail_pane_open;
        cx.notify();
    }

    pub fn toggle_resize(&mut self, _: &ToggleResize, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(doc) = &self.document {
            let (w, h) = doc.output_size();
            self.resize_settings.original_width = w;
            self.resize_settings.original_height = h;
            self.resize_settings.width = w;
            self.resize_settings.height = h;
            self.resize_settings.unit_percent = false;
            self.resize_settings.destination_folder = doc
                .file_path
                .as_ref()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()));
            self.resize_settings.is_open = !self.resize_settings.is_open;
            cx.notify();
        }
    }

    pub fn execute_resize(&mut self, cx: &mut Context<Self>) {
        let Some(document) = &mut self.document else {
            return;
        };
        let target_w = if self.resize_settings.unit_percent {
            ((self.resize_settings.original_width as f64 * self.resize_settings.width as f64
                / 100.0)
                .round() as u32)
                .max(1)
        } else {
            self.resize_settings.width.max(1)
        };
        let target_h = if self.resize_settings.unit_percent {
            ((self.resize_settings.original_height as f64 * self.resize_settings.height as f64
                / 100.0)
                .round() as u32)
                .max(1)
        } else {
            self.resize_settings.height.max(1)
        };

        let exported = document.render_export();
        let resized = nodapix_imaging::resize(&exported, target_w, target_h);

        if let Some(dest_dir) = &self.resize_settings.destination_folder {
            let orig_name = document.file_name();
            let base_name = Path::new(&orig_name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "image".to_string());
            let fmt = self.resize_settings.format.unwrap_or(ExportFormat::Png);
            let ext = fmt.extension();
            let out_name = format!("{}_{}x{}.{}", base_name, target_w, target_h, ext);
            let out_path = dest_dir.join(out_name);

            let options = ExportOptions {
                format: fmt,
                quality: 90,
            };
            if let Ok(bytes) = NativeBackend::new().encode_to_bytes(&resized, &options) {
                let final_bytes = with_exif(bytes, fmt, self.exif_block.as_deref());
                let _ = std::fs::write(&out_path, final_bytes);
            }
        }

        self.document = Some(ImageDocument::new(resized, document.file_path.clone()));
        self.sync_geometry();
        self.rebuild_original_texture();
        self.edited = self.original.clone();
        self.request_preview_render(cx);
        self.resize_settings.is_open = false;
        self.update_status();
        cx.notify();
    }

    fn sync_geometry(&mut self) {
        if let Some(document) = &self.document {
            let (width, height) = document.output_size();
            self.image = size(width as f32, height as f32);
        }
    }

    fn rebuild_original_texture(&mut self) {
        self.original = self
            .document
            .as_ref()
            .map(|document| texture_from_rgba8(&document.render_original_preview()));
    }

    fn request_preview_render(&mut self, cx: &mut Context<Self>) {
        let generation = self.render_generation.fetch_add(1, Ordering::SeqCst) + 1;

        let Some(document) = &self.document else {
            self.edited = None;
            self.histogram = None;
            return;
        };
        let proxy = document.proxy_source.clone();
        let state = document.state.clone();

        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let rendered = executor
                .spawn(async move { render_proxy_preview(&proxy, &state) })
                .await;

            let hist = compute_histogram(&rendered);

            let _ = this.update(cx, |editor, cx| {
                if editor.render_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                editor.histogram = Some(hist);
                editor.edited = Some(texture_from_rgba8(&rendered));
                editor.last_rendered_preview = Some(Arc::new(rendered));
                cx.notify();
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------ editing

    fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut EditState)) {
        let Some(document) = &mut self.document else {
            return;
        };
        document.commit_checkpoint();
        change(&mut document.state);
        self.sync_geometry();
        self.request_preview_render(cx);
    }

    pub fn rotate_cw(&mut self, _: &RotateCW, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |state| state.rotate_cw());
        cx.notify();
    }

    pub fn rotate_ccw(&mut self, _: &RotateCCW, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |state| state.rotate_ccw());
        cx.notify();
    }

    pub fn flip_h(&mut self, _: &FlipH, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |state| state.toggle_flip_h());
        cx.notify();
    }

    pub fn flip_v(&mut self, _: &FlipV, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |state| state.toggle_flip_v());
        cx.notify();
    }

    pub fn reset_all(&mut self, _: &ResetAll, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |state| state.reset_all());
        self.zoom = Zoom::Fit;
        self.pan = point(px(0.0), px(0.0));
        self.crop_mode = false;
        self.draft_crop = None;
        self.crop_preset = CropRatioPreset::Free;
        self.crop_portrait = false;
        cx.notify();
    }

    pub fn reset_adjustments(
        &mut self,
        _: &ResetAdjustments,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |state| state.reset_adjustments());
        self.horizon_mode = false;
        self.horizon_start = None;
        self.horizon_current = None;
        self.eyedropper_active = false;
        self.eyedropper_sample = None;
        cx.notify();
    }

    pub fn auto_enhance(&mut self, _: &AutoEnhance, _: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = &self.document else {
            return;
        };
        let preview = document.render_preview();
        let (exp_ev, contrast) = calculate_auto_levels(&preview);
        self.edit(cx, |state| {
            state.color.exposure = exp_ev;
            state.color.contrast = contrast;
        });
        self.status = format!(
            "Auto Enhance applied: Exposure {:+.1} EV, Contrast {:+.0}%",
            exp_ev,
            contrast * 100.0
        )
        .into();
        cx.notify();
    }

    pub fn toggle_info(&mut self, _: &ToggleInfo, _: &mut Window, cx: &mut Context<Self>) {
        if self.document.is_none() {
            return;
        }
        self.exif_open = !self.exif_open;
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
        if self.exif_open {
            self.exif_open = false;
            cx.notify();
            return;
        }
        if self.crop_mode {
            if let Some(crop) = self.draft_crop {
                self.edit(cx, |state| {
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
        if self.exif_open {
            self.exif_open = false;
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
        self.edit(cx, |state| {
            state.crop = None;
        });
        self.crop_mode = false;
        self.draft_crop = None;
        self.crop_drag = None;
        self.crop_preset = CropRatioPreset::Free;
        self.status = "Crop reset".into();
        cx.notify();
    }

    pub fn select_crop_preset(&mut self, preset: CropRatioPreset, cx: &mut Context<Self>) {
        self.crop_preset = preset;
        if preset == CropRatioPreset::Free {
            self.draft_crop = Some(CropRect {
                x: 0.05,
                y: 0.05,
                width: 0.9,
                height: 0.9,
            });
        } else if let Some(ratio) = preset.target_ratio(self.image.width, self.image.height, self.crop_portrait) {
            self.draft_crop = Some(CropRect::centered_with_ratio(ratio, self.image.width, self.image.height));
        }
        cx.notify();
    }

    pub fn toggle_crop_orientation(&mut self, cx: &mut Context<Self>) {
        self.crop_portrait = !self.crop_portrait;
        if self.crop_preset != CropRatioPreset::Free {
            if let Some(ratio) = self.crop_preset.target_ratio(self.image.width, self.image.height, self.crop_portrait) {
                self.draft_crop = Some(CropRect::centered_with_ratio(ratio, self.image.width, self.image.height));
            }
        }
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

            let encoded = NativeBackend::new()
                .encode_to_bytes(
                    &rendered,
                    &ExportOptions {
                        format: self.export_settings.format,
                        quality: self.export_settings.quality,
                    },
                )
                .map_err(|e| e.to_string())
                .map(|bytes| {
                    with_exif(
                        bytes,
                        self.export_settings.format,
                        self.exif_block.as_deref(),
                    )
                });

            match encoded.and_then(|bytes| write_atomically(&bytes, &path)) {
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

    // ----------------------------------------------------------- save / save as

    /// Replace `path` with `bytes` through a sibling temp file, so a crashed or
    /// failed write can never leave the original half-written.
    pub fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        let (dirty, path) = match &self.document {
            Some(document) => (document.is_dirty, document.file_path.clone()),
            None => return,
        };
        if !dirty {
            self.status = "No changes to save".into();
            cx.notify();
            return;
        }
        let format = path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .and_then(ExportFormat::from_extension);
        let (Some(path), Some(format)) = (path, format) else {
            // Nothing to overwrite in place, or the original's format is one we
            // cannot encode: let the user choose a writable destination.
            self.save_as_dialog(cx);
            return;
        };

        let name = file_label(&path);
        let confirmed = rfd::MessageDialog::new()
            .set_title("Replace Original?")
            .set_description(format!(
                "Saving overwrites {name} with the edited version, re-encoded as {format:?}. \
                 Use Export to keep the original untouched. Replace it?"
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if confirmed != rfd::MessageDialogResult::Yes {
            self.status = format!("Save cancelled — {name} left untouched").into();
            cx.notify();
            return;
        }

        self.begin_save(path, format, cx);
    }

    pub fn save_as(&mut self, _: &SaveAs, _: &mut Window, cx: &mut Context<Self>) {
        self.save_as_dialog(cx);
    }

    fn save_as_dialog(&mut self, cx: &mut Context<Self>) {
        let proposed = match &self.document {
            Some(document) => document.file_name(),
            None => "Untitled.jpg".to_string(),
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save As")
            .set_file_name(&proposed)
            .save_file()
        else {
            return;
        };

        let Some(format) = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(ExportFormat::from_extension)
        else {
            self.status = format!(
                "Cannot save as {}: supported types are JPEG, PNG, WebP and TIFF",
                file_label(&path)
            )
            .into();
            cx.notify();
            return;
        };

        self.begin_save(path, format, cx);
    }

    /// Render and encode on a worker thread: a full-resolution TIFF can take long
    /// enough that doing it inline would freeze the sliders.
    fn begin_save(&mut self, path: PathBuf, format: ExportFormat, cx: &mut Context<Self>) {
        let Some(document) = &self.document else {
            return;
        };
        let source = document.source.clone();
        let state = document.state.clone();
        let quality = self.export_settings.quality;
        let exif = self.exif_block.clone();
        let label = file_label(&path);

        self.status = format!("Saving {label}…").into();
        cx.notify();

        let executor = cx.background_executor().clone();
        let destination = path.clone();
        cx.spawn(async move |this, cx| {
            let outcome = executor
                .spawn(async move {
                    let rendered = execute_pipeline(&source, &state);
                    NativeBackend::new()
                        .encode_to_bytes(&rendered, &ExportOptions { format, quality })
                        .map_err(|e| e.to_string())
                        .map(|bytes| with_exif(bytes, format, exif.as_deref()))
                        .and_then(|bytes| write_atomically(&bytes, &destination))
                        .map(|_| state)
                })
                .await;

            let _ = this.update(cx, |editor, cx| {
                editor.status = match outcome {
                    Ok(saved_state) => {
                        if let Some(document) = &mut editor.document {
                            document.file_path = Some(path);
                            // Edits made while the encoder ran are still unsaved.
                            document.is_dirty = document.state != saved_state;
                        }
                        format!("Saved {label}").into()
                    }
                    Err(error) => format!("Save failed: {error}").into(),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Filename plus the edited marker, mirrored into the titlebar.
    fn title(&self) -> SharedString {
        match &self.document {
            Some(document) if document.is_dirty => {
                format!("{} — Edited", document.file_name()).into()
            }
            Some(document) => document.file_name().into(),
            None => "NodaPix".into(),
        }
    }

    pub fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(document) = &mut self.document {
            if document.undo() {
                self.sync_geometry();
                self.request_preview_render(cx);
            }
        }
        cx.notify();
    }

    pub fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(document) = &mut self.document {
            if document.redo() {
                self.sync_geometry();
                self.request_preview_render(cx);
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
        if self.eyedropper_active {
            if let (Some(dest), Some(rendered)) = (self.image_dest.get(), &self.last_rendered_preview) {
                let bx = f32::from(dest.origin.x);
                let by = f32::from(dest.origin.y);
                let bw = f32::from(dest.size.width);
                let bh = f32::from(dest.size.height);
                let mx = f32::from(event.position.x);
                let my = f32::from(event.position.y);
                if mx >= bx && mx < bx + bw && my >= by && my < by + bh && bw > 0.0 && bh > 0.0 {
                    let nx = (mx - bx) / bw;
                    let ny = (my - by) / bh;
                    let px_coord = ((nx * rendered.width as f32) as u32).min(rendered.width.saturating_sub(1));
                    let py_coord = ((ny * rendered.height as f32) as u32).min(rendered.height.saturating_sub(1));
                    let idx = (py_coord as usize * rendered.width as usize + px_coord as usize) * 4;
                    let r = rendered.data[idx] as f32 / 255.0;
                    let g = rendered.data[idx + 1] as f32 / 255.0;
                    let b = rendered.data[idx + 2] as f32 / 255.0;

                    let (kelvin, tint) = calculate_white_balance_from_sample(r, g, b);
                    self.edit(cx, |state| {
                        state.color.temperature = kelvin;
                        state.color.tint = tint;
                    });
                    self.status = format!("White balance adjusted: {:.0}K, tint {:+.2}", kelvin, tint).into();
                    self.eyedropper_active = false;
                    self.eyedropper_sample = None;
                    cx.notify();
                    return;
                }
            }
        }

        if self.horizon_mode {
            self.horizon_start = Some(event.position);
            self.horizon_current = Some(event.position);
            cx.notify();
            return;
        }

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
        if self.eyedropper_active {
            if let (Some(dest), Some(rendered)) = (self.image_dest.get(), &self.last_rendered_preview) {
                let bx = f32::from(dest.origin.x);
                let by = f32::from(dest.origin.y);
                let bw = f32::from(dest.size.width);
                let bh = f32::from(dest.size.height);
                let mx = f32::from(event.position.x);
                let my = f32::from(event.position.y);
                if mx >= bx && mx < bx + bw && my >= by && my < by + bh && bw > 0.0 && bh > 0.0 {
                    let nx = (mx - bx) / bw;
                    let ny = (my - by) / bh;
                    let px_coord = ((nx * rendered.width as f32) as u32).min(rendered.width.saturating_sub(1));
                    let py_coord = ((ny * rendered.height as f32) as u32).min(rendered.height.saturating_sub(1));
                    let idx = (py_coord as usize * rendered.width as usize + px_coord as usize) * 4;
                    let r = rendered.data[idx];
                    let g = rendered.data[idx + 1];
                    let b = rendered.data[idx + 2];
                    let a = rendered.data[idx + 3];
                    let hex = format!("#{:02X}{:02X}{:02X}", r, g, b);
                    self.eyedropper_sample = Some(([r, g, b, a], hex, event.position));
                    cx.notify();
                    return;
                } else {
                    self.eyedropper_sample = None;
                    cx.notify();
                }
            }
        }

        if self.horizon_mode {
            if self.horizon_start.is_some() {
                self.horizon_current = Some(event.position);
                cx.notify();
                return;
            }
        }

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

            let ratio_opt = self.crop_preset.target_ratio(img_w, img_h, self.crop_portrait);
            let r_norm = ratio_opt.map(|r| r * (img_h / img_w));

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
                CropHandle::BottomRight => {
                    if let Some(r) = r_norm {
                        let raw_w = (sc.width + dx).clamp(0.05, 1.0 - sc.x);
                        let raw_h = (raw_w / r).clamp(0.05, 1.0 - sc.y);
                        let final_w = (raw_h * r).clamp(0.05, 1.0 - sc.x);
                        CropRect { x: sc.x, y: sc.y, width: final_w, height: raw_h }
                    } else {
                        let new_w = (sc.width + dx).clamp(0.05, 1.0 - sc.x);
                        let new_h = (sc.height + dy).clamp(0.05, 1.0 - sc.y);
                        CropRect { x: sc.x, y: sc.y, width: new_w, height: new_h }
                    }
                }
                CropHandle::TopLeft => {
                    if let Some(r) = r_norm {
                        let max_w = sc.x + sc.width;
                        let max_h = sc.y + sc.height;
                        let raw_w = (sc.width - dx).clamp(0.05, max_w);
                        let raw_h = (raw_w / r).clamp(0.05, max_h);
                        let final_w = (raw_h * r).clamp(0.05, max_w);
                        let new_x = max_w - final_w;
                        let new_y = max_h - raw_h;
                        CropRect { x: new_x, y: new_y, width: final_w, height: raw_h }
                    } else {
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
                }
                CropHandle::TopRight => {
                    if let Some(r) = r_norm {
                        let max_h = sc.y + sc.height;
                        let raw_w = (sc.width + dx).clamp(0.05, 1.0 - sc.x);
                        let raw_h = (raw_w / r).clamp(0.05, max_h);
                        let final_w = (raw_h * r).clamp(0.05, 1.0 - sc.x);
                        let new_y = max_h - raw_h;
                        CropRect { x: sc.x, y: new_y, width: final_w, height: raw_h }
                    } else {
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
                }
                CropHandle::BottomLeft => {
                    if let Some(r) = r_norm {
                        let max_w = sc.x + sc.width;
                        let raw_w = (sc.width - dx).clamp(0.05, max_w);
                        let raw_h = (raw_w / r).clamp(0.05, 1.0 - sc.y);
                        let final_w = (raw_h * r).clamp(0.05, max_w);
                        let new_x = max_w - final_w;
                        CropRect { x: new_x, y: sc.y, width: final_w, height: raw_h }
                    } else {
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
                }
                CropHandle::Top => {
                    let max_y = sc.y + sc.height - 0.05;
                    let new_y = (sc.y + dy).clamp(0.0, max_y);
                    let new_h = (sc.y + sc.height) - new_y;
                    if let Some(r) = r_norm {
                        let new_w = (new_h * r).clamp(0.05, 1.0);
                        let cx = sc.x + sc.width / 2.0;
                        let new_x = (cx - new_w / 2.0).clamp(0.0, 1.0 - new_w);
                        CropRect { x: new_x, y: new_y, width: new_w, height: new_h }
                    } else {
                        CropRect { x: sc.x, y: new_y, width: sc.width, height: new_h }
                    }
                }
                CropHandle::Bottom => {
                    let new_h = (sc.height + dy).clamp(0.05, 1.0 - sc.y);
                    if let Some(r) = r_norm {
                        let new_w = (new_h * r).clamp(0.05, 1.0);
                        let cx = sc.x + sc.width / 2.0;
                        let new_x = (cx - new_w / 2.0).clamp(0.0, 1.0 - new_w);
                        CropRect { x: new_x, y: sc.y, width: new_w, height: new_h }
                    } else {
                        CropRect { x: sc.x, y: sc.y, width: sc.width, height: new_h }
                    }
                }
                CropHandle::Left => {
                    let max_x = sc.x + sc.width - 0.05;
                    let new_x = (sc.x + dx).clamp(0.0, max_x);
                    let new_w = (sc.x + sc.width) - new_x;
                    if let Some(r) = r_norm {
                        let new_h = (new_w / r).clamp(0.05, 1.0);
                        let cy = sc.y + sc.height / 2.0;
                        let new_y = (cy - new_h / 2.0).clamp(0.0, 1.0 - new_h);
                        CropRect { x: new_x, y: new_y, width: new_w, height: new_h }
                    } else {
                        CropRect { x: new_x, y: sc.y, width: new_w, height: sc.height }
                    }
                }
                CropHandle::Right => {
                    let new_w = (sc.width + dx).clamp(0.05, 1.0 - sc.x);
                    if let Some(r) = r_norm {
                        let new_h = (new_w / r).clamp(0.05, 1.0);
                        let cy = sc.y + sc.height / 2.0;
                        let new_y = (cy - new_h / 2.0).clamp(0.0, 1.0 - new_h);
                        CropRect { x: sc.x, y: new_y, width: new_w, height: new_h }
                    } else {
                        CropRect { x: sc.x, y: sc.y, width: new_w, height: sc.height }
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
        if self.horizon_mode {
            if let (Some(p1), Some(p2)) = (self.horizon_start, self.horizon_current) {
                let dx = f32::from(p2.x) - f32::from(p1.x);
                let dy = f32::from(p2.y) - f32::from(p1.y);
                if dx.hypot(dy) > 10.0 {
                    let angle_rad = if dx.abs() >= dy.abs() {
                        -dy.atan2(dx)
                    } else if dy > 0.0 {
                        dx.atan2(dy)
                    } else {
                        (-dx).atan2(-dy)
                    };
                    let mut angle_deg = angle_rad * 180.0 / std::f32::consts::PI;
                    while angle_deg > 45.0 {
                        angle_deg -= 90.0;
                    }
                    while angle_deg < -45.0 {
                        angle_deg += 90.0;
                    }
                    let angle_deg = angle_deg.clamp(-45.0, 45.0);
                    self.edit(cx, |state| state.angle_fine_degrees = angle_deg);
                    self.status = format!("Aligned to horizon line: {:+.2}°", angle_deg).into();
                }
            }
            self.horizon_start = None;
            self.horizon_current = None;
            self.horizon_mode = false;
            cx.notify();
            return;
        }

        self.crop_drag = None;
        self.drag_origin = None;
        cx.notify();
    }

    // ------------------------------------------------------------- slider events

    fn update_slider(&mut self, id: SliderId, val: f32, cx: &mut Context<Self>) {
        let Some(document) = &mut self.document else {
            return;
        };
        let Some(spec) = ALL_SPECS.iter().find(|s| s.id == id) else {
            return;
        };
        spec.set_value(&mut document.state, val);
        self.sync_geometry();
        self.request_preview_render(cx);
    }

    fn on_slider_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.crop_mode || self.active_tool_group.is_none() {
            return;
        }

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
            self.update_slider(id, val, cx);
            cx.notify();
        }
    }

    fn on_slider_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.crop_mode || self.active_tool_group.is_none() {
            return;
        }

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
            self.update_slider(id, val, cx);
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
        // Clear slider bounds so only currently painted sliders can be hit-tested
        self.slider_state.bounds.borrow_mut().clear();

        if self.crop_mode {
            self.render_crop_sidebar(cx)
        } else if let Some(group) = self.active_tool_group {
            self.render_tool_sidebar(group, cx)
        } else {
            self.render_tools_grid_panel(cx)
        }
    }

    fn render_crop_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(document) = &self.document else {
            return div();
        };

        let (img_w, img_h) = document.output_size();
        let crop = self.draft_crop.unwrap_or(CropRect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        });
        let (crop_x, crop_y, crop_w, crop_h) = crop.to_pixel_coords(img_w, img_h);
        let step = self.crop_multiple_of.unwrap_or(1);
        let current_preset = self.crop_preset;
        let is_portrait = self.crop_portrait;
        let multiple_of = self.crop_multiple_of;

        let presets = [
            CropRatioPreset::Free,
            CropRatioPreset::Square,
            CropRatioPreset::Original,
            CropRatioPreset::Ratio3x2,
            CropRatioPreset::Ratio4x3,
            CropRatioPreset::Ratio16x9,
            CropRatioPreset::Ratio16x10,
        ];

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
                            .child("Crop"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(ACCENT))
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                this.cancel_crop(&CancelCrop, window, cx);
                            }))
                            .child("Cancel"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .overflow_hidden()
                    // Selection group
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
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Selection"),
                            )
                            // Width spinbox
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child("Width:"))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .w(px(70.0))
                                                    .text_center()
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x18_18_1c))
                                                    .border_1()
                                                    .border_color(rgb(BORDER_COLOR))
                                                    .child(format!("{} px", crop_w)),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let new_w = crop_w.saturating_sub(step).max(step);
                                                        let target_ratio = this.crop_preset.target_ratio(img_w as f32, img_h as f32, this.crop_portrait);
                                                        let new_h = if let Some(r) = target_ratio {
                                                            ((new_w as f32 / r).round() as u32).min(img_h - crop_y)
                                                        } else {
                                                            crop_h
                                                        };
                                                        this.draft_crop = Some(CropRect::from_pixel_coords(crop_x, crop_y, new_w, new_h, img_w, img_h));
                                                        cx.notify();
                                                    }))
                                                    .child("−"),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let new_w = (crop_w + step).min(img_w - crop_x);
                                                        let target_ratio = this.crop_preset.target_ratio(img_w as f32, img_h as f32, this.crop_portrait);
                                                        let new_h = if let Some(r) = target_ratio {
                                                            ((new_w as f32 / r).round() as u32).min(img_h - crop_y)
                                                        } else {
                                                            crop_h
                                                        };
                                                        this.draft_crop = Some(CropRect::from_pixel_coords(crop_x, crop_y, new_w, new_h, img_w, img_h));
                                                        cx.notify();
                                                    }))
                                                    .child("+"),
                                            ),
                                    ),
                            )
                            // Height spinbox
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child("Height:"))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .w(px(70.0))
                                                    .text_center()
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x18_18_1c))
                                                    .border_1()
                                                    .border_color(rgb(BORDER_COLOR))
                                                    .child(format!("{} px", crop_h)),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let new_h = crop_h.saturating_sub(step).max(step);
                                                        let target_ratio = this.crop_preset.target_ratio(img_w as f32, img_h as f32, this.crop_portrait);
                                                        let new_w = if let Some(r) = target_ratio {
                                                            ((new_h as f32 * r).round() as u32).min(img_w - crop_x)
                                                        } else {
                                                            crop_w
                                                        };
                                                        this.draft_crop = Some(CropRect::from_pixel_coords(crop_x, crop_y, new_w, new_h, img_w, img_h));
                                                        cx.notify();
                                                    }))
                                                    .child("−"),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let new_h = (crop_h + step).min(img_h - crop_y);
                                                        let target_ratio = this.crop_preset.target_ratio(img_w as f32, img_h as f32, this.crop_portrait);
                                                        let new_w = if let Some(r) = target_ratio {
                                                            ((new_h as f32 * r).round() as u32).min(img_w - crop_x)
                                                        } else {
                                                            crop_w
                                                        };
                                                        this.draft_crop = Some(CropRect::from_pixel_coords(crop_x, crop_y, new_w, new_h, img_w, img_h));
                                                        cx.notify();
                                                    }))
                                                    .child("+"),
                                            ),
                                    ),
                            )
                            // Position X spinbox
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child("Position X:"))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .w(px(70.0))
                                                    .text_center()
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x18_18_1c))
                                                    .border_1()
                                                    .border_color(rgb(BORDER_COLOR))
                                                    .child(format!("{} px", crop_x)),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let new_x = crop_x.saturating_sub(step);
                                                        this.draft_crop = Some(CropRect::from_pixel_coords(new_x, crop_y, crop_w, crop_h, img_w, img_h));
                                                        cx.notify();
                                                    }))
                                                    .child("−"),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let new_x = (crop_x + step).min(img_w.saturating_sub(crop_w));
                                                        this.draft_crop = Some(CropRect::from_pixel_coords(new_x, crop_y, crop_w, crop_h, img_w, img_h));
                                                        cx.notify();
                                                    }))
                                                    .child("+"),
                                            ),
                                    ),
                            )
                            // Position Y spinbox
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child("Position Y:"))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .w(px(70.0))
                                                    .text_center()
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x18_18_1c))
                                                    .border_1()
                                                    .border_color(rgb(BORDER_COLOR))
                                                    .child(format!("{} px", crop_y)),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let new_y = crop_y.saturating_sub(step);
                                                        this.draft_crop = Some(CropRect::from_pixel_coords(crop_x, new_y, crop_w, crop_h, img_w, img_h));
                                                        cx.notify();
                                                    }))
                                                    .child("−"),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(2.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let new_y = (crop_y + step).min(img_h.saturating_sub(crop_h));
                                                        this.draft_crop = Some(CropRect::from_pixel_coords(crop_x, new_y, crop_w, crop_h, img_w, img_h));
                                                        cx.notify();
                                                    }))
                                                    .child("+"),
                                            ),
                                    ),
                            )
                            // Maximize & Center action buttons
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .pt_1()
                                    .child(
                                        div()
                                            .flex_1()
                                            .py(px(4.0))
                                            .text_center()
                                            .text_xs()
                                            .rounded_sm()
                                            .bg(rgb(0x2a_2a_33))
                                            .text_color(rgb(TEXT_PRIMARY))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                let target_ratio = this.crop_preset.target_ratio(img_w as f32, img_h as f32, this.crop_portrait);
                                                if let Some(r) = target_ratio {
                                                    this.draft_crop = Some(CropRect::centered_with_ratio(r, img_w as f32, img_h as f32));
                                                } else {
                                                    this.draft_crop = Some(CropRect { x: 0.0, y: 0.0, width: 1.0, height: 1.0 });
                                                }
                                                cx.notify();
                                            }))
                                            .child("Maximize"),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .py(px(4.0))
                                            .text_center()
                                            .text_xs()
                                            .rounded_sm()
                                            .bg(rgb(0x2a_2a_33))
                                            .text_color(rgb(TEXT_PRIMARY))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                let new_x = (img_w.saturating_sub(crop_w)) / 2;
                                                let new_y = (img_h.saturating_sub(crop_h)) / 2;
                                                this.draft_crop = Some(CropRect::from_pixel_coords(new_x, new_y, crop_w, crop_h, img_w, img_h));
                                                cx.notify();
                                            }))
                                            .child("Center"),
                                    ),
                            ),
                    )
                    // Aspect ratio group
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
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Aspect ratio"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_1()
                                    .children(presets.iter().map(|&preset| {
                                        let is_sel = preset == current_preset;
                                        div()
                                            .px_2()
                                            .py(px(2.0))
                                            .text_xs()
                                            .rounded_sm()
                                            .bg(if is_sel { rgb(ACCENT) } else { rgb(0x2a_2a_33) })
                                            .text_color(if is_sel { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                this.crop_preset = preset;
                                                if let Some(ratio) = preset.target_ratio(img_w as f32, img_h as f32, this.crop_portrait) {
                                                    this.draft_crop = Some(CropRect::centered_with_ratio(ratio, img_w as f32, img_h as f32));
                                                }
                                                cx.notify();
                                            }))
                                            .child(preset.label())
                                    })),
                            )
                            // Invert aspect ratio toggle
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                        this.crop_portrait = !this.crop_portrait;
                                        if let Some(ratio) = this.crop_preset.target_ratio(img_w as f32, img_h as f32, this.crop_portrait) {
                                            this.draft_crop = Some(CropRect::centered_with_ratio(ratio, img_w as f32, img_h as f32));
                                        }
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .w(px(14.0))
                                            .h(px(14.0))
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(rgb(BORDER_COLOR))
                                            .bg(if is_portrait { rgb(ACCENT) } else { rgb(0x18_18_1c) })
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .children(if is_portrait {
                                                Some(LucideIcon::Check.element(10.0, rgb(0xff_ff_ff)))
                                            } else {
                                                None
                                            }),
                                    )
                                    .child(div().text_xs().text_color(rgb(TEXT_PRIMARY)).child("Invert aspect ratio")),
                            )
                            // Multiple of N px toggle
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                                this.crop_multiple_of = if this.crop_multiple_of.is_some() { None } else { Some(8) };
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .w(px(14.0))
                                                    .h(px(14.0))
                                                    .rounded_sm()
                                                    .border_1()
                                                    .border_color(rgb(BORDER_COLOR))
                                                    .bg(if multiple_of.is_some() { rgb(ACCENT) } else { rgb(0x18_18_1c) })
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .children(if multiple_of.is_some() {
                                                        Some(LucideIcon::Check.element(10.0, rgb(0xff_ff_ff)))
                                                    } else {
                                                        None
                                                    }),
                                            )
                                            .child(div().text_xs().text_color(rgb(TEXT_PRIMARY)).child("Multiple of:")),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(1.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x18_18_1c))
                                                    .border_1()
                                                    .border_color(rgb(BORDER_COLOR))
                                                    .child(format!("{} px", multiple_of.unwrap_or(8))),
                                            ),
                                    ),
                            ),
                    )
                    // Apply & Cancel buttons
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .pt_2()
                            .child(
                                div()
                                    .flex_1()
                                    .py(px(6.0))
                                    .text_center()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .rounded_md()
                                    .bg(rgb(ACCENT))
                                    .text_color(rgb(0xff_ff_ff))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.apply_crop(&ApplyCrop, window, cx);
                                    }))
                                    .child("Accept (↵)"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .py(px(6.0))
                                    .text_center()
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x34_34_3e))
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.cancel_crop(&CancelCrop, window, cx);
                                    }))
                                    .child("Cancel (Esc)"),
                            ),
                    ),
            )
    }

    fn render_tools_grid_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(_document) = &self.document else {
            return div();
        };

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
                // Panel Header Quick Actions: Save, Save As, Undo, Redo
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
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.0))
                                    .rounded_sm()
                                    .bg(rgb(0x2a_2a_33))
                                    .text_xs()
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.save(&Save, window, cx);
                                    }))
                                    .child(LucideIcon::Save.element(13.0, rgb(TEXT_PRIMARY)))
                                    .child("Save"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.0))
                                    .rounded_sm()
                                    .bg(rgb(0x2a_2a_33))
                                    .text_xs()
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.save_as(&SaveAs, window, cx);
                                    }))
                                    .child(LucideIcon::Download.element(13.0, rgb(TEXT_PRIMARY)))
                                    .child("Save As"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(26.0))
                                    .h(px(22.0))
                                    .rounded_sm()
                                    .bg(rgb(0x2a_2a_33))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.undo(&Undo, window, cx);
                                    }))
                                    .child(LucideIcon::Undo2.element(13.0, rgb(TEXT_PRIMARY))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(26.0))
                                    .h(px(22.0))
                                    .rounded_sm()
                                    .bg(rgb(0x2a_2a_33))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.redo(&Redo, window, cx);
                                    }))
                                    .child(LucideIcon::Redo2.element(13.0, rgb(TEXT_PRIMARY))),
                            ),
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
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .overflow_hidden()
                    // Live Histogram
                    .child(render_histogram(self.histogram.as_ref()))
                    // Section: Colors (4x2 Grid of tools)
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
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Colors"),
                            )
                            // 4x2 grid of buttons
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    // Row 1: Brightness/Contrast, Color Balance, Levels, Equalize
                                    .child(
                                        div()
                                            .flex()
                                            .gap_1()
                                            .child(self.render_grid_tool_btn(
                                                LucideIcon::Sun,
                                                "Light",
                                                false,
                                                cx.listener(|this, _, _, cx| {
                                                    this.active_tool_group = Some(ToolGroup::BrightnessContrast);
                                                    cx.notify();
                                                }),
                                            ))
                                            .child(self.render_grid_tool_btn(
                                                LucideIcon::Palette,
                                                "Balance",
                                                false,
                                                cx.listener(|this, _, _, cx| {
                                                    this.active_tool_group = Some(ToolGroup::ColorBalance);
                                                    cx.notify();
                                                }),
                                            ))
                                            .child(self.render_grid_tool_btn(
                                                LucideIcon::Sliders,
                                                "Levels",
                                                false,
                                                cx.listener(|this, _, _, cx| {
                                                    this.active_tool_group = Some(ToolGroup::Levels);
                                                    cx.notify();
                                                }),
                                            ))
                                            .child(self.render_grid_tool_btn(
                                                LucideIcon::Contrast,
                                                "Equalize",
                                                false,
                                                cx.listener(|this, _, window, cx| {
                                                    this.auto_enhance(&AutoEnhance, window, cx);
                                                }),
                                            )),
                                    )
                                    // Row 2: Curves/Hue, Auto Enhance, Sharpen, Vignette
                                    .child(
                                        div()
                                            .flex()
                                            .gap_1()
                                            .child(self.render_grid_tool_btn(
                                                LucideIcon::Activity,
                                                "Hue/Sat",
                                                false,
                                                cx.listener(|this, _, _, cx| {
                                                    this.active_tool_group = Some(ToolGroup::HueSaturation);
                                                    cx.notify();
                                                }),
                                            ))
                                            .child(self.render_grid_tool_btn(
                                                LucideIcon::Wand2,
                                                "Auto",
                                                false,
                                                cx.listener(|this, _, window, cx| {
                                                    this.auto_enhance(&AutoEnhance, window, cx);
                                                }),
                                            ))
                                            .child(self.render_grid_tool_btn(
                                                LucideIcon::Zap,
                                                "Sharpen",
                                                false,
                                                cx.listener(|this, _, _, cx| {
                                                    this.active_tool_group = Some(ToolGroup::Sharpen);
                                                    cx.notify();
                                                }),
                                            ))
                                            .child(self.render_grid_tool_btn(
                                                LucideIcon::Eye,
                                                "Vignette",
                                                false,
                                                cx.listener(|this, _, _, cx| {
                                                    this.active_tool_group = Some(ToolGroup::Vignette);
                                                    cx.notify();
                                                }),
                                            )),
                                    ),
                            ),
                    )
                    // Section: Rotation (4x1 Grid + Straighten)
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
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Rotation"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(self.render_grid_tool_btn(
                                        LucideIcon::RotateCcw,
                                        "Left",
                                        false,
                                        cx.listener(|this, _, window, cx| {
                                            this.rotate_ccw(&RotateCCW, window, cx);
                                        }),
                                    ))
                                    .child(self.render_grid_tool_btn(
                                        LucideIcon::RotateCw,
                                        "Right",
                                        false,
                                        cx.listener(|this, _, window, cx| {
                                            this.rotate_cw(&RotateCW, window, cx);
                                        }),
                                    ))
                                    .child(self.render_grid_tool_btn(
                                        LucideIcon::FlipHorizontal,
                                        "Flip H",
                                        false,
                                        cx.listener(|this, _, window, cx| {
                                            this.flip_h(&FlipH, window, cx);
                                        }),
                                    ))
                                    .child(self.render_grid_tool_btn(
                                        LucideIcon::FlipVertical,
                                        "Flip V",
                                        false,
                                        cx.listener(|this, _, window, cx| {
                                            this.flip_v(&FlipV, window, cx);
                                        }),
                                    )),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .h(px(32.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_1()
                                    .rounded_sm()
                                    .bg(rgb(0x2a_2a_33))
                                    .border_1()
                                    .border_color(rgb(BORDER_COLOR))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                        this.active_tool_group = Some(ToolGroup::Straighten);
                                        cx.notify();
                                    }))
                                    .child(LucideIcon::SlidersHorizontal.element(14.0, rgb(TEXT_PRIMARY)))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(TEXT_PRIMARY))
                                            .child("Straighten…"),
                                    ),
                            ),
                    )
                    // Section: Format (Resize & Crop)
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
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Format"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .py(px(8.0))
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .justify_center()
                                            .gap_1()
                                            .rounded_sm()
                                            .bg(rgb(0x2a_2a_33))
                                            .border_1()
                                            .border_color(rgb(BORDER_COLOR))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                                this.toggle_resize(&ToggleResize, window, cx);
                                            }))
                                            .child(LucideIcon::Scaling.element(18.0, rgb(TEXT_PRIMARY)))
                                            .child(div().text_xs().text_color(rgb(TEXT_PRIMARY)).child("Resize…")),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .py(px(8.0))
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .justify_center()
                                            .gap_1()
                                            .rounded_sm()
                                            .bg(if self.crop_mode { rgb(ACCENT) } else { rgb(0x2a_2a_33) })
                                            .border_1()
                                            .border_color(if self.crop_mode { rgb(ACCENT) } else { rgb(BORDER_COLOR) })
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                                this.toggle_crop(&ToggleCrop, window, cx);
                                            }))
                                            .child(LucideIcon::Crop.element(18.0, if self.crop_mode { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) }))
                                            .child(div().text_xs().text_color(if self.crop_mode { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) }).child("Crop")),
                                    ),
                            ),
                    ),
            )
    }

    fn render_tool_sidebar(&self, group: ToolGroup, cx: &mut Context<Self>) -> gpui::Div {
        let Some(document) = &self.document else {
            return div();
        };

        let state = &document.state;
        let bounds = self.slider_state.bounds.clone();
        let current_filter = state.color.filter_preset;

        let filter_presets = [
            FilterPreset::None,
            FilterPreset::BlackAndWhite,
            FilterPreset::GrayscaleDesaturate,
            FilterPreset::GrayscaleAverage,
            FilterPreset::Sepia,
            FilterPreset::WarmGold,
            FilterPreset::CoolNordic,
            FilterPreset::Vivid,
            FilterPreset::Lomo,
            FilterPreset::Vintage,
            FilterPreset::CrossProcess,
        ];

        let (title, icon, has_histogram) = match group {
            ToolGroup::BrightnessContrast => ("Light & Contrast", LucideIcon::Sun, true),
            ToolGroup::ColorBalance => ("Color Balance", LucideIcon::Palette, true),
            ToolGroup::Levels => ("Levels & Exposure", LucideIcon::Sliders, true),
            ToolGroup::HueSaturation => ("Hue & Saturation", LucideIcon::Activity, false),
            ToolGroup::Sharpen => ("Sharpen", LucideIcon::Zap, false),
            ToolGroup::Vignette => ("Vignette & Filters", LucideIcon::Eye, false),
            ToolGroup::Straighten => ("Straighten & Rotate", LucideIcon::SlidersHorizontal, false),
        };

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
                // Header: Back button, Title, Reset button
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
                            .flex()
                            .items_center()
                            .gap_1()
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                this.active_tool_group = None;
                                cx.notify();
                            }))
                            .child(LucideIcon::ChevronLeft.element(14.0, rgb(TEXT_PRIMARY)))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Back"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(icon.element(14.0, rgb(ACCENT)))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(ACCENT))
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                match group {
                                    ToolGroup::BrightnessContrast => this.edit(cx, |state| {
                                        state.color.brightness = 0.0;
                                        state.color.contrast = 0.0;
                                        state.color.invert = false;
                                    }),
                                    ToolGroup::ColorBalance => {
                                        this.edit(cx, |state| {
                                            state.color.temperature = TEMPERATURE_NEUTRAL_KELVIN;
                                            state.color.tint = 0.0;
                                        });
                                        this.eyedropper_active = false;
                                        this.eyedropper_sample = None;
                                    }
                                    ToolGroup::Levels => this.edit(cx, |state| {
                                        state.color.exposure = 0.0;
                                        state.color.highlights = 0.0;
                                        state.color.shadows = 0.0;
                                    }),
                                    ToolGroup::HueSaturation => this.edit(cx, |state| {
                                        state.color.saturation = 0.0;
                                        state.color.hue = 0.0;
                                    }),
                                    ToolGroup::Sharpen => this.edit(cx, |state| {
                                        state.sharpness = 0.0;
                                    }),
                                    ToolGroup::Vignette => this.edit(cx, |state| {
                                        state.color.vignette = 0.0;
                                        state.color.filter_preset = FilterPreset::None;
                                    }),
                                    ToolGroup::Straighten => {
                                        this.edit(cx, |state| {
                                            state.angle_fine_degrees = 0.0;
                                        });
                                        this.horizon_mode = false;
                                        this.horizon_start = None;
                                        this.horizon_current = None;
                                    }
                                }
                            }))
                            .child("Reset"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .overflow_hidden()
                    // Optional Live Histogram
                    .when(has_histogram, |el| {
                        el.child(render_histogram(self.histogram.as_ref()))
                    })
                    // Tool Controls Card
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .p_3()
                            .rounded_md()
                            .bg(rgb(SECTION_BG))
                            .border_1()
                            .border_color(rgb(BORDER_COLOR))
                            .when(group == ToolGroup::BrightnessContrast, |el| {
                                el.child(render_slider_row(
                                    SliderSpec::BRIGHTNESS,
                                    SliderSpec::BRIGHTNESS.get_value(state),
                                    bounds.clone(),
                                ))
                                .child(render_slider_row(
                                    SliderSpec::CONTRAST,
                                    SliderSpec::CONTRAST.get_value(state),
                                    bounds.clone(),
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .pt_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(TEXT_PRIMARY))
                                                .child("Invert (Negative)"),
                                        )
                                        .child(
                                            div()
                                                .px_2()
                                                .py(px(3.0))
                                                .text_xs()
                                                .rounded_sm()
                                                .bg(if state.color.invert { rgb(ACCENT) } else { rgb(0x2a_2a_33) })
                                                .text_color(if state.color.invert { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                                .border_1()
                                                .border_color(if state.color.invert { rgb(ACCENT) } else { rgb(BORDER_COLOR) })
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                                    this.edit(cx, |state| state.color.invert = !state.color.invert);
                                                }))
                                                .child(if state.color.invert { "Inverted" } else { "Normal" }),
                                        ),
                                )
                            })
                            .when(group == ToolGroup::ColorBalance, |el| {
                                el.child(render_slider_row(
                                    SliderSpec::TEMPERATURE,
                                    SliderSpec::TEMPERATURE.get_value(state),
                                    bounds.clone(),
                                ))
                                .child(render_slider_row(
                                    SliderSpec::TINT,
                                    SliderSpec::TINT.get_value(state),
                                    bounds.clone(),
                                ))
                                .child(
                                    div()
                                        .w_full()
                                        .py(px(6.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .gap_1()
                                        .rounded_sm()
                                        .bg(if self.eyedropper_active { rgb(ACCENT) } else { rgb(0x2a_2a_33) })
                                        .text_color(if self.eyedropper_active { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                        .border_1()
                                        .border_color(if self.eyedropper_active { rgb(ACCENT) } else { rgb(BORDER_COLOR) })
                                        .cursor(CursorStyle::PointingHand)
                                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                            this.eyedropper_active = !this.eyedropper_active;
                                            this.eyedropper_sample = None;
                                            cx.notify();
                                        }))
                                        .child(LucideIcon::Pipette.element(14.0, if self.eyedropper_active { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) }))
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .child(if self.eyedropper_active { "Click Neutral Gray to Balance…" } else { "Sample White Balance" }),
                                        ),
                                )
                            })
                            .when(group == ToolGroup::Levels, |el| {
                                el.child(render_slider_row(
                                    SliderSpec::EXPOSURE,
                                    SliderSpec::EXPOSURE.get_value(state),
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
                                ))
                            })
                            .when(group == ToolGroup::HueSaturation, |el| {
                                el.child(render_slider_row(
                                    SliderSpec::SATURATION,
                                    SliderSpec::SATURATION.get_value(state),
                                    bounds.clone(),
                                ))
                                .child(render_slider_row(
                                    SliderSpec::HUE,
                                    SliderSpec::HUE.get_value(state),
                                    bounds.clone(),
                                ))
                            })
                            .when(group == ToolGroup::Sharpen, |el| {
                                el.child(render_slider_row(
                                    SliderSpec::SHARPNESS,
                                    SliderSpec::SHARPNESS.get_value(state),
                                    bounds.clone(),
                                ))
                            })
                            .when(group == ToolGroup::Vignette, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .text_color(rgb(TEXT_PRIMARY))
                                                .child("Preset Filters"),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .flex_wrap()
                                                .gap_1()
                                                .children(filter_presets.iter().map(|&preset| {
                                                    let is_sel = preset == current_filter;
                                                    div()
                                                        .px_2()
                                                        .py(px(3.0))
                                                        .text_xs()
                                                        .rounded_sm()
                                                        .bg(if is_sel { rgb(ACCENT) } else { rgb(0x2a_2a_33) })
                                                        .text_color(if is_sel { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                                        .cursor(CursorStyle::PointingHand)
                                                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                            this.edit(cx, |state| state.color.filter_preset = preset);
                                                        }))
                                                        .child(preset.label())
                                                })),
                                        ),
                                )
                                .child(
                                    div()
                                        .pt_2()
                                        .border_t_1()
                                        .border_color(rgb(BORDER_COLOR))
                                        .child(render_slider_row(
                                            SliderSpec::VIGNETTE,
                                            SliderSpec::VIGNETTE.get_value(state),
                                            bounds.clone(),
                                        )),
                                )
                            })
                            .when(group == ToolGroup::Straighten, |el| {
                                el.child(render_slider_row(
                                    SliderSpec::ANGLE,
                                    SliderSpec::ANGLE.get_value(state),
                                    bounds.clone(),
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .pt_1()
                                        .child(
                                            div()
                                                .px_2()
                                                .py(px(3.0))
                                                .text_xs()
                                                .rounded_sm()
                                                .bg(rgb(0x2a_2a_33))
                                                .text_color(rgb(TEXT_PRIMARY))
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                                    this.edit(cx, |state| state.angle_fine_degrees = 0.0);
                                                }))
                                                .child("Reset Angle (0°)"),
                                        ),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .py(px(6.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .gap_1()
                                        .rounded_sm()
                                        .bg(if self.horizon_mode { rgb(ACCENT) } else { rgb(0x2a_2a_33) })
                                        .text_color(if self.horizon_mode { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                        .border_1()
                                        .border_color(if self.horizon_mode { rgb(ACCENT) } else { rgb(BORDER_COLOR) })
                                        .cursor(CursorStyle::PointingHand)
                                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                            this.horizon_mode = !this.horizon_mode;
                                            this.horizon_start = None;
                                            this.horizon_current = None;
                                            cx.notify();
                                        }))
                                        .child(LucideIcon::Activity.element(14.0, if self.horizon_mode { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) }))
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .child(if self.horizon_mode { "Drag Horizon Line on Image…" } else { "Draw Alignment Line…" }),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .pt_2()
                                        .border_t_1()
                                        .border_color(rgb(BORDER_COLOR))
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .text_color(rgb(TEXT_PRIMARY))
                                                .child("Rotate & Flip"),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .gap_1()
                                                .child(self.render_grid_tool_btn(
                                                    LucideIcon::RotateCcw,
                                                    "Left 90°",
                                                    false,
                                                    cx.listener(|this, _, window, cx| {
                                                        this.rotate_ccw(&RotateCCW, window, cx);
                                                    }),
                                                ))
                                                .child(self.render_grid_tool_btn(
                                                    LucideIcon::RotateCw,
                                                    "Right 90°",
                                                    false,
                                                    cx.listener(|this, _, window, cx| {
                                                        this.rotate_cw(&RotateCW, window, cx);
                                                    }),
                                                ))
                                                .child(self.render_grid_tool_btn(
                                                    LucideIcon::FlipHorizontal,
                                                    "Flip H",
                                                    false,
                                                    cx.listener(|this, _, window, cx| {
                                                        this.flip_h(&FlipH, window, cx);
                                                    }),
                                                ))
                                                .child(self.render_grid_tool_btn(
                                                    LucideIcon::FlipVertical,
                                                    "Flip V",
                                                    false,
                                                    cx.listener(|this, _, window, cx| {
                                                        this.flip_v(&FlipV, window, cx);
                                                    }),
                                                )),
                                        ),
                                )
                            }),
                    ),
            )
            .child(
                // Bottom Done bar
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(BORDER_COLOR))
                    .bg(rgb(CHROME))
                    .child(
                        div()
                            .w_full()
                            .py(px(6.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .rounded_md()
                            .bg(rgb(ACCENT))
                            .text_color(rgb(0xff_ff_ff))
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                this.active_tool_group = None;
                                cx.notify();
                            }))
                            .child(LucideIcon::Check.element(14.0, rgb(0xff_ff_ff)))
                            .child("Done"),
                    ),
            )
    }

    fn render_grid_tool_btn(
        &self,
        icon: LucideIcon,
        label: &'static str,
        is_active: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> gpui::Div {
        div()
            .flex_1()
            .h(px(48.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(2.0))
            .rounded_sm()
            .bg(if is_active { rgb(ACCENT) } else { rgb(0x2a_2a_33) })
            .border_1()
            .border_color(if is_active { rgb(ACCENT) } else { rgb(BORDER_COLOR) })
            .cursor(CursorStyle::PointingHand)
            .on_mouse_down(MouseButton::Left, on_click)
            .child(icon.element(17.0, if is_active { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) }))
            .child(
                div()
                    .text_xs()
                    .text_color(if is_active { rgb(0xff_ff_ff) } else { rgb(TEXT_MUTED) })
                    .child(label),
            )
    }

    fn render_resize_modal(&self, cx: &mut Context<Self>) -> gpui::Div {
        let (orig_w, orig_h) = self.document.as_ref().map(|d| d.output_size()).unwrap_or((1, 1));
        let width = self.resize_settings.width;
        let height = self.resize_settings.height;
        let unit_pct = self.resize_settings.unit_percent;
        let keep_aspect = self.resize_settings.preserve_aspect_ratio;
        let dest_folder = self.resize_settings.destination_folder.clone();
        let current_fmt = self.resize_settings.format;

        let dest_name = dest_folder
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Original image folder".to_string());

        let formats = [
            (None, "Keep original format"),
            (Some(ExportFormat::Png), "PNG"),
            (Some(ExportFormat::Jpeg), "JPEG"),
            (Some(ExportFormat::WebP), "WebP"),
            (Some(ExportFormat::Tiff), "TIFF"),
        ];

        div()
            .absolute()
            .size_full()
            .bg(rgba(0x00_00_00_a0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(420.0))
                    .bg(rgb(0x23_23_2a))
                    .border_1()
                    .border_color(rgb(BORDER_COLOR))
                    .rounded_lg()
                    .shadow_xl()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    // Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(rgb(BORDER_COLOR))
                            .pb_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Resize Images"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(TEXT_MUTED))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                        this.resize_settings.is_open = false;
                                        cx.notify();
                                    }))
                                    .child(LucideIcon::X.element(14.0, rgb(TEXT_MUTED))),
                            ),
                    )
                    // New Dimensions
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("New dimensions"),
                            )
                            // Width row
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child("Width:"))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(3.0))
                                                    .w(px(70.0))
                                                    .text_center()
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x18_18_1c))
                                                    .border_1()
                                                    .border_color(rgb(BORDER_COLOR))
                                                    .child(if unit_pct { format!("{}%", width) } else { format!("{}", width) }),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(3.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let step = if unit_pct { 5 } else { 10 };
                                                        this.resize_settings.width = width.saturating_sub(step).max(1);
                                                        if this.resize_settings.preserve_aspect_ratio {
                                                            this.resize_settings.height = ((this.resize_settings.width as f64 * orig_h as f64 / orig_w as f64).round() as u32).max(1);
                                                        }
                                                        cx.notify();
                                                    }))
                                                    .child("−"),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(3.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let step = if unit_pct { 5 } else { 10 };
                                                        this.resize_settings.width = (width + step).min(99999);
                                                        if this.resize_settings.preserve_aspect_ratio {
                                                            this.resize_settings.height = ((this.resize_settings.width as f64 * orig_h as f64 / orig_w as f64).round() as u32).max(1);
                                                        }
                                                        cx.notify();
                                                    }))
                                                    .child("+"),
                                            )
                                            // Unit selector
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(3.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .border_1()
                                                    .border_color(rgb(BORDER_COLOR))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                                        this.resize_settings.unit_percent = !this.resize_settings.unit_percent;
                                                        if this.resize_settings.unit_percent {
                                                            this.resize_settings.width = 100;
                                                            this.resize_settings.height = 100;
                                                        } else {
                                                            this.resize_settings.width = this.resize_settings.original_width;
                                                            this.resize_settings.height = this.resize_settings.original_height;
                                                        }
                                                        cx.notify();
                                                    }))
                                                    .child(if unit_pct { "% ▾" } else { "pixel ▾" }),
                                            ),
                                    ),
                            )
                            // Height row
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child("Height:"))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(3.0))
                                                    .w(px(70.0))
                                                    .text_center()
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x18_18_1c))
                                                    .border_1()
                                                    .border_color(rgb(BORDER_COLOR))
                                                    .child(if unit_pct { format!("{}%", height) } else { format!("{}", height) }),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(3.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let step = if unit_pct { 5 } else { 10 };
                                                        this.resize_settings.height = height.saturating_sub(step).max(1);
                                                        if this.resize_settings.preserve_aspect_ratio {
                                                            this.resize_settings.width = ((this.resize_settings.height as f64 * orig_w as f64 / orig_h as f64).round() as u32).max(1);
                                                        }
                                                        cx.notify();
                                                    }))
                                                    .child("−"),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(3.0))
                                                    .text_xs()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2a_2a_33))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                        let step = if unit_pct { 5 } else { 10 };
                                                        this.resize_settings.height = (height + step).min(99999);
                                                        if this.resize_settings.preserve_aspect_ratio {
                                                            this.resize_settings.width = ((this.resize_settings.height as f64 * orig_w as f64 / orig_h as f64).round() as u32).max(1);
                                                        }
                                                        cx.notify();
                                                    }))
                                                    .child("+"),
                                            ),
                                    ),
                            )
                            // Preserve aspect ratio checkbox
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                        this.resize_settings.preserve_aspect_ratio = !this.resize_settings.preserve_aspect_ratio;
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .w(px(14.0))
                                            .h(px(14.0))
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(rgb(BORDER_COLOR))
                                            .bg(if keep_aspect { rgb(ACCENT) } else { rgb(0x18_18_1c) })
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .children(if keep_aspect {
                                                Some(LucideIcon::Check.element(10.0, rgb(0xff_ff_ff)))
                                            } else {
                                                None
                                            }),
                                    )
                                    .child(div().text_xs().text_color(rgb(TEXT_PRIMARY)).child("Preserve original aspect ratios")),
                            ),
                    )
                    // Destination group
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Destination"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .p_2()
                                    .rounded_md()
                                    .bg(rgb(0x1c_1c_22))
                                    .border_1()
                                    .border_color(rgb(BORDER_COLOR))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                            this.resize_settings.destination_folder = Some(folder);
                                            cx.notify();
                                        }
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(LucideIcon::FolderOpen.element(14.0, rgb(ACCENT)))
                                            .child(div().text_xs().text_color(rgb(TEXT_PRIMARY)).child(dest_name)),
                                    )
                                    .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child("Change…")),
                            ),
                    )
                    // Format group
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child("Format"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_1()
                                    .children(formats.iter().map(|&(fmt, label)| {
                                        let is_sel = fmt == current_fmt;
                                        div()
                                            .px_2()
                                            .py(px(3.0))
                                            .text_xs()
                                            .rounded_sm()
                                            .bg(if is_sel { rgb(ACCENT) } else { rgb(0x2a_2a_33) })
                                            .text_color(if is_sel { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                this.resize_settings.format = fmt;
                                                cx.notify();
                                            }))
                                            .child(label)
                                    })),
                            ),
                    )
                    // Footer buttons
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
                                        this.resize_settings.is_open = false;
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
                                        this.execute_resize(cx);
                                    }))
                                    .child("Execute"),
                            ),
                    ),
            )
    }

    fn render_thumbnail_carousel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.thumbnail_pane_open || self.folder_images.is_empty() {
            return div().into_any_element();
        }

        let current_idx = self.folder_index;
        div()
            .id("thumbnail-carousel")
            .w_full()
            .h(px(84.0))
            .bg(rgb(0x16_16_1a))
            .border_t_1()
            .border_color(rgb(BORDER_COLOR))
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .overflow_x_scroll()
            .gap_2()
            .children(self.folder_images.iter().enumerate().map(|(idx, path)| {
                let is_active = idx == current_idx;
                let thumb = self.thumbnail_cache.get(path).cloned();
                let path_clone = path.clone();

                div()
                    .flex_shrink_0()
                    .w(px(96.0))
                    .h(px(68.0))
                    .rounded_sm()
                    .overflow_hidden()
                    .relative()
                    .bg(rgb(0x22_22_28))
                    .border_2()
                    .border_color(if is_active { rgb(ACCENT) } else { rgb(0x38_38_44) })
                    .cursor(CursorStyle::PointingHand)
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        if this.folder_index != idx {
                            this.folder_index = idx;
                            this.load(path_clone.clone(), cx);
                        }
                    }))
                    .child(
                        if let Some(texture) = thumb {
                            canvas(
                                move |_, _, _| {},
                                move |bounds, _, window, _| {
                                    let _ = window.paint_image(
                                        bounds,
                                        bounds,
                                        Corners::all(px(0.0)),
                                        texture,
                                        0,
                                        false,
                                    );
                                },
                            )
                            .size_full()
                            .into_any_element()
                        } else {
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_xs()
                                .text_color(rgb(TEXT_MUTED))
                                .child(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
                                .into_any_element()
                        }
                    )
            }))
            .into_any_element()
    }

    fn render_exif_modal(&self, cx: &mut Context<Self>) -> gpui::Div {
        let meta = self.metadata.clone().unwrap_or_default();
        let (w, h) = self.document.as_ref().map(|d| d.output_size()).unwrap_or((1, 1));
        let file_name = self.document.as_ref().map(|d| d.file_name()).unwrap_or_default();

        let camera_str = match (&meta.make, &meta.model) {
            (Some(make), Some(model)) => {
                if model.starts_with(make) {
                    model.clone()
                } else {
                    format!("{} {}", make, model)
                }
            }
            (None, Some(model)) => model.clone(),
            (Some(make), None) => make.clone(),
            (None, None) => "Unknown Camera".to_string(),
        };

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
                                    .child("File Info & EXIF"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(file_name),
                            ),
                    )
                    // Camera & Lens Card
                    .child(
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(rgb(0x2c_2c_36))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child(camera_str),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(meta.lens.unwrap_or_else(|| "Standard Optical Lens".to_string())),
                            ),
                    )
                    // Exposure Badges
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .p_2()
                                    .rounded_md()
                                    .bg(rgb(0x28_28_32))
                                    .text_center()
                                    .child(div().text_xs().text_color(rgb(MUTED)).child("Aperture"))
                                    .child(div().text_sm().font_weight(gpui::FontWeight::BOLD).text_color(rgb(TEXT_PRIMARY)).child(meta.f_number.unwrap_or_else(|| "—".to_string()))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .p_2()
                                    .rounded_md()
                                    .bg(rgb(0x28_28_32))
                                    .text_center()
                                    .child(div().text_xs().text_color(rgb(MUTED)).child("Shutter"))
                                    .child(div().text_sm().font_weight(gpui::FontWeight::BOLD).text_color(rgb(TEXT_PRIMARY)).child(meta.exposure_time.unwrap_or_else(|| "—".to_string()))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .p_2()
                                    .rounded_md()
                                    .bg(rgb(0x28_28_32))
                                    .text_center()
                                    .child(div().text_xs().text_color(rgb(MUTED)).child("ISO"))
                                    .child(div().text_sm().font_weight(gpui::FontWeight::BOLD).text_color(rgb(TEXT_PRIMARY)).child(meta.iso.unwrap_or_else(|| "—".to_string()))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .p_2()
                                    .rounded_md()
                                    .bg(rgb(0x28_28_32))
                                    .text_center()
                                    .child(div().text_xs().text_color(rgb(MUTED)).child("Focal"))
                                    .child(div().text_sm().font_weight(gpui::FontWeight::BOLD).text_color(rgb(TEXT_PRIMARY)).child(meta.focal_length.unwrap_or_else(|| "—".to_string()))),
                            ),
                    )
                    // Geometry & Date
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!("Dimensions: {} × {} px", w, h))
                            .child(meta.date_time.unwrap_or_default()),
                    )
                    // Close button
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .pt_2()
                            .border_t_1()
                            .border_color(rgb(BORDER_COLOR))
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
                                        this.exif_open = false;
                                        cx.notify();
                                    }))
                                    .child("Done"),
                            ),
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
        let panel_btn_text = if self.sidebar_open { "Hide Tools" } else { "Tools" };

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
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(rgb(ACCENT))
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                this.toggle_info(&ToggleInfo, window, cx);
                            }))
                            .child(LucideIcon::Info.element(12.0, rgb(ACCENT)))
                            .child("Info"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(if self.thumbnail_pane_open { rgb(ACCENT) } else { rgb(MUTED) })
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                this.toggle_thumbnail_pane(&ToggleThumbnailPane, window, cx);
                            }))
                            .child(LucideIcon::Film.element(12.0, if self.thumbnail_pane_open { rgb(ACCENT) } else { rgb(MUTED) }))
                            .child(if self.thumbnail_pane_open { "Hide Thumbnails" } else { "Thumbnails" }),
                    )
                    .child(div().text_color(rgb(MUTED)).child(zoom_text))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(rgb(ACCENT))
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                this.toggle_sidebar(&ToggleSidebar, window, cx);
                            }))
                            .child(LucideIcon::SlidersHorizontal.element(12.0, rgb(ACCENT)))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The dirty marker belongs in the titlebar, but re-setting it every frame
        // would poke AppKit needlessly.
        let title = self.title();
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }

        let texture = if self.compare {
            self.original.clone().or_else(|| self.edited.clone())
        } else {
            self.edited.clone().or_else(|| self.original.clone())
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
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
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
            .on_action(cx.listener(Self::auto_enhance))
            .on_action(cx.listener(Self::toggle_info))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_crop))
            .on_action(cx.listener(Self::apply_crop))
            .on_action(cx.listener(Self::cancel_crop))
            .on_action(cx.listener(Self::reset_crop))
            .on_action(cx.listener(Self::export))
            .on_action(cx.listener(Self::prev_image))
            .on_action(cx.listener(Self::next_image))
            .on_action(cx.listener(Self::toggle_thumbnail_pane))
            .on_action(cx.listener(Self::toggle_resize));

        // Top Toolbar
        if self.crop_mode {
            // Dedicated Crop Top Bar matching gthumb-046_orig.jpg
            let presets = [
                CropRatioPreset::Free,
                CropRatioPreset::Square,
                CropRatioPreset::Original,
                CropRatioPreset::Ratio3x2,
                CropRatioPreset::Ratio4x3,
                CropRatioPreset::Ratio16x9,
                CropRatioPreset::Ratio16x10,
            ];
            let active_preset = self.crop_preset;
            let is_portrait = self.crop_portrait;

            root = root.child(
                div()
                    .flex()
                    .flex_col()
                    .bg(rgb(CHROME))
                    .border_b_1()
                    .border_color(rgb(BORDER_COLOR))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .h(px(38.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py(px(3.0))
                                            .rounded_md()
                                            .bg(rgb(0x32_32_3c))
                                            .text_color(rgb(TEXT_PRIMARY))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                                this.cancel_crop(&CancelCrop, window, cx);
                                            }))
                                            .child(LucideIcon::ChevronLeft.element(14.0, rgb(TEXT_PRIMARY)))
                                            .child(div().text_xs().child("Back")),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(TEXT_PRIMARY))
                                            .child("Crop"),
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
                                            .py(px(3.0))
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
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_3()
                                            .py(px(3.0))
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .rounded_md()
                                            .bg(rgb(ACCENT))
                                            .text_color(rgb(0xff_ff_ff))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                                this.apply_crop(&ApplyCrop, window, cx);
                                            }))
                                            .child(LucideIcon::Check.element(14.0, rgb(0xff_ff_ff)))
                                            .child("Accept"),
                                    ),
                            ),
                    )
                    // Aspect ratio presets sub-bar
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_4()
                            .py(px(4.0))
                            .bg(rgb(0x22_22_2a))
                            .border_t_1()
                            .border_color(rgb(BORDER_COLOR))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .mr_2()
                                    .child("Aspect Ratio:"),
                            )
                            .children(presets.iter().map(|&p| {
                                let is_sel = p == active_preset;
                                div()
                                    .px_2()
                                    .py(px(2.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(if is_sel { rgb(ACCENT) } else { rgb(0x32_32_3c) })
                                    .text_color(if is_sel { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                        this.select_crop_preset(p, cx);
                                    }))
                                    .child(p.label())
                            }))
                            .child(
                                div()
                                    .ml_2()
                                    .px_2()
                                    .py(px(2.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(if is_portrait { rgb(0x4a_4a_58) } else { rgb(0x32_32_3c) })
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                        this.toggle_crop_orientation(cx);
                                    }))
                                    .child(if is_portrait { "Orientation: Vertical ↕" } else { "Orientation: Horizontal ↔" }),
                            ),
                    ),
            );
        } else {
            // Standard Pix Viewer top toolbar
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .h(px(38.0))
                    .bg(rgb(CHROME))
                    .border_b_1()
                    .border_color(rgb(BORDER_COLOR))
                    .child(
                        // Left: Navigation & File actions
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x32_32_3c))
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.open(&Open, window, cx);
                                    }))
                                    .child(LucideIcon::FolderOpen.element(13.0, rgb(TEXT_PRIMARY)))
                                    .child("Open…"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x32_32_3c))
                                    .text_color(if self.folder_images.len() > 1 { rgb(TEXT_PRIMARY) } else { rgb(MUTED) })
                                    .cursor(if self.folder_images.len() > 1 { CursorStyle::PointingHand } else { CursorStyle::Arrow })
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.prev_image(&PrevImage, window, cx);
                                    }))
                                    .child(LucideIcon::ChevronLeft.element(13.0, if self.folder_images.len() > 1 { rgb(TEXT_PRIMARY) } else { rgb(MUTED) }))
                                    .child("Prev"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x32_32_3c))
                                    .text_color(if self.folder_images.len() > 1 { rgb(TEXT_PRIMARY) } else { rgb(MUTED) })
                                    .cursor(if self.folder_images.len() > 1 { CursorStyle::PointingHand } else { CursorStyle::Arrow })
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.next_image(&NextImage, window, cx);
                                    }))
                                    .child("Next")
                                    .child(LucideIcon::ChevronRight.element(13.0, if self.folder_images.len() > 1 { rgb(TEXT_PRIMARY) } else { rgb(MUTED) })),
                            )
                            .child(div().w(px(1.0)).h(px(16.0)).bg(rgb(BORDER_COLOR)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x32_32_3c))
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.save(&Save, window, cx);
                                    }))
                                    .child(LucideIcon::Save.element(13.0, rgb(TEXT_PRIMARY)))
                                    .child("Save"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x32_32_3c))
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.export(&Export, window, cx);
                                    }))
                                    .child(LucideIcon::Download.element(13.0, rgb(TEXT_PRIMARY)))
                                    .child("Export…"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x32_32_3c))
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.toggle_resize(&ToggleResize, window, cx);
                                    }))
                                    .child(LucideIcon::Scaling.element(13.0, rgb(TEXT_PRIMARY)))
                                    .child("Resize…"),
                            ),
                    )
                    // Right: View controls & Tools sidebar toggle
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(24.0))
                                    .h(px(22.0))
                                    .rounded_md()
                                    .bg(rgb(0x32_32_3c))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.zoom_out(&ZoomOut, window, cx);
                                    }))
                                    .child(LucideIcon::Minus.element(13.0, rgb(TEXT_PRIMARY))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x32_32_3c))
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.fit(&Fit, window, cx);
                                    }))
                                    .child(LucideIcon::Minimize2.element(12.0, rgb(TEXT_PRIMARY)))
                                    .child("Fit"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.0))
                                    .text_xs()
                                    .rounded_md()
                                    .bg(rgb(0x32_32_3c))
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.actual_size(&ActualSize, window, cx);
                                    }))
                                    .child(LucideIcon::ZoomIn.element(12.0, rgb(TEXT_PRIMARY)))
                                    .child("100%"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(24.0))
                                    .h(px(22.0))
                                    .rounded_md()
                                    .bg(rgb(0x32_32_3c))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.zoom_in(&ZoomIn, window, cx);
                                    }))
                                    .child(LucideIcon::Plus.element(13.0, rgb(TEXT_PRIMARY))),
                            )
                            .child(div().w(px(1.0)).h(px(16.0)).bg(rgb(BORDER_COLOR)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_3()
                                    .py(px(3.0))
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .rounded_md()
                                    .bg(if self.sidebar_open { rgb(ACCENT) } else { rgb(0x32_32_3c) })
                                    .text_color(if self.sidebar_open { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) })
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                        this.toggle_sidebar(&ToggleSidebar, window, cx);
                                    }))
                                    .child(LucideIcon::SlidersHorizontal.element(13.0, if self.sidebar_open { rgb(0xff_ff_ff) } else { rgb(TEXT_PRIMARY) }))
                                    .child("Tools"),
                            ),
                    ),
            );
        }

        root = root.child({
            let center_area = div()
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child({
                    let mut canvas_container = div()
                        .flex_1()
                        .relative()
                        .overflow_hidden()
                        .cursor(if self.crop_mode || self.horizon_mode || self.eyedropper_active { CursorStyle::Crosshair } else { CursorStyle::OpenHand })
                        .on_scroll(cx.listener(Self::on_scroll))
                        .on_pinch(cx.listener(Self::on_pinch))
                        .on_drop(cx.listener(Self::on_drop))
                        .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_pan))
                        .on_mouse_up(MouseButton::Left, cx.listener(Self::end_pan))
                        .on_mouse_move(cx.listener(Self::drag_pan))
                        .child(viewport.element());

                    // Floating Next/Prev arrows on canvas sides (gthumb-053_orig.jpg)
                    if self.folder_images.len() > 1 && !self.crop_mode {
                        canvas_container = canvas_container
                            .child(
                                div()
                                    .absolute()
                                    .left(px(16.0))
                                    .top(px(0.0))
                                    .bottom(px(0.0))
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .w(px(36.0))
                                            .h(px(36.0))
                                            .rounded_full()
                                            .bg(rgb(0x22_22_2a))
                                            .border_1()
                                            .border_color(rgb(BORDER_COLOR))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor(CursorStyle::PointingHand)
                                            .hover(|s| s.bg(rgb(0x38_38_46)))
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                                this.prev_image(&PrevImage, window, cx);
                                            }))
                                            .child(LucideIcon::ChevronLeft.element(20.0, rgb(TEXT_PRIMARY))),
                                    ),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .right(px(16.0))
                                    .top(px(0.0))
                                    .bottom(px(0.0))
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .w(px(36.0))
                                            .h(px(36.0))
                                            .rounded_full()
                                            .bg(rgb(0x22_22_2a))
                                            .border_1()
                                            .border_color(rgb(BORDER_COLOR))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor(CursorStyle::PointingHand)
                                            .hover(|s| s.bg(rgb(0x38_38_46)))
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                                this.next_image(&NextImage, window, cx);
                                            }))
                                            .child(LucideIcon::ChevronRight.element(20.0, rgb(TEXT_PRIMARY))),
                                    ),
                            );
                    }

                    // Alignment guide line overlay on canvas when dragging in Horizon mode
                    if let (Some(p1), Some(p2)) = (self.horizon_start, self.horizon_current) {
                        let p1_x = f32::from(p1.x);
                        let p1_y = f32::from(p1.y);
                        let p2_x = f32::from(p2.x);
                        let p2_y = f32::from(p2.y);
                        let dx = p2_x - p1_x;
                        let dy = p2_y - p1_y;
                        let angle_rad = if dx.abs() >= dy.abs() {
                            -dy.atan2(dx)
                        } else if dy > 0.0 {
                            dx.atan2(dy)
                        } else {
                            (-dx).atan2(-dy)
                        };
                        let mut angle_deg = angle_rad * 180.0 / std::f32::consts::PI;
                        while angle_deg > 45.0 {
                            angle_deg -= 90.0;
                        }
                        while angle_deg < -45.0 {
                            angle_deg += 90.0;
                        }
                        let angle_deg = angle_deg.clamp(-45.0, 45.0);

                        canvas_container = canvas_container.child(
                            div()
                                .absolute()
                                .inset_0()
                                .child(canvas(
                                    move |_, _, _| {},
                                    move |_, _, window, _| {
                                        let dist = dx.hypot(dy);
                                        let steps = (dist / 2.0).max(1.0) as usize;
                                        for i in 0..=steps {
                                            let t = i as f32 / steps as f32;
                                            let x = p1_x + dx * t;
                                            let y = p1_y + dy * t;
                                            window.paint_quad(fill(
                                                Bounds {
                                                    origin: point(px(x - 1.0), px(y - 1.0)),
                                                    size: size(px(2.0), px(2.0)),
                                                },
                                                rgb(0x38_bd_f8),
                                            ));
                                        }
                                        // Endpoints
                                        window.paint_quad(fill(
                                            Bounds {
                                                origin: point(px(p1_x - 5.0), px(p1_y - 5.0)),
                                                size: size(px(10.0), px(10.0)),
                                            },
                                            rgb(0x38_bd_f8),
                                        ));
                                        window.paint_quad(fill(
                                            Bounds {
                                                origin: point(px(p1_x - 2.0), px(p1_y - 2.0)),
                                                size: size(px(4.0), px(4.0)),
                                            },
                                            rgb(0xff_ff_ff),
                                        ));
                                        window.paint_quad(fill(
                                            Bounds {
                                                origin: point(px(p2_x - 5.0), px(p2_y - 5.0)),
                                                size: size(px(10.0), px(10.0)),
                                            },
                                            rgb(0x38_bd_f8),
                                        ));
                                        window.paint_quad(fill(
                                            Bounds {
                                                origin: point(px(p2_x - 2.0), px(p2_y - 2.0)),
                                                size: size(px(4.0), px(4.0)),
                                            },
                                            rgb(0xff_ff_ff),
                                        ));
                                    },
                                ))
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(p2_x + 12.0))
                                        .top(px(p2_y - 12.0))
                                        .px_2()
                                        .py(px(2.0))
                                        .rounded_sm()
                                        .bg(rgb(0x18_18_20))
                                        .border_1()
                                        .border_color(rgb(0x38_bd_f8))
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(rgb(0x38_bd_f8))
                                        .child(format!("{:+.2}°", angle_deg)),
                                ),
                        );
                    }

                    // Floating pixel color loupe when Eyedropper mode is active
                    if self.eyedropper_active {
                        if let Some(([r, g, b, _], hex, pos)) = &self.eyedropper_sample {
                            let lx = (f32::from(pos.x) + 16.0).max(10.0);
                            let ly = (f32::from(pos.y) - 50.0).max(10.0);
                            let hex_str = hex.clone();
                            let color_rgb = ((*r as u32) << 16) | ((*g as u32) << 8) | (*b as u32);
                            canvas_container = canvas_container.child(
                                div()
                                    .absolute()
                                    .left(px(lx))
                                    .top(px(ly))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .py(px(4.0))
                                    .rounded_md()
                                    .bg(rgb(0x18_18_22))
                                    .border_1()
                                    .border_color(rgb(BORDER_COLOR))
                                    .child(
                                        div()
                                            .w(px(20.0))
                                            .h(px(20.0))
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(rgb(0xff_ff_ff))
                                            .bg(rgb(color_rgb)),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .text_color(rgb(TEXT_PRIMARY))
                                                    .child(hex_str),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(TEXT_MUTED))
                                                    .child(format!("RGB: {}, {}, {}", r, g, b)),
                                            ),
                                    ),
                            );
                        }
                    }

                    canvas_container
                })
                .when(self.thumbnail_pane_open && !self.folder_images.is_empty(), |this| {
                    this.child(self.render_thumbnail_carousel(cx))
                });

            let mut split = div()
                .flex_1()
                .relative()
                .flex()
                .flex_row()
                .overflow_hidden()
                .child(center_area);

            if (self.sidebar_open || self.crop_mode) && self.document.is_some() {
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

        // Resize Modal overlay if active (gthumb-054_orig.jpg)
        if self.resize_settings.is_open {
            root = root.child(self.render_resize_modal(cx));
        }

        // Export Modal overlay if active
        if self.export_settings.is_open {
            root = root.child(self.render_export_modal(cx));
        }

        // EXIF Metadata Inspector Modal overlay if active
        if self.exif_open {
            root = root.child(self.render_exif_modal(cx));
        }

        root
    }
}

#[cfg(test)]
mod tests {
    use super::write_atomically;

    #[test]
    fn atomic_replace_overwrites_without_leaving_temporaries() {
        let dir = std::env::temp_dir().join(format!("nodapix-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("photo.jpg");

        write_atomically(b"original", &path).unwrap();
        write_atomically(b"edited", &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"edited");

        // A missing directory fails without touching the destination.
        let missing = dir.join("sub/photo.jpg");
        assert!(write_atomically(b"nope", &missing).is_err());
        assert!(!missing.exists());

        let debris: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(debris.is_empty(), "temporary files left behind: {debris:?}");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
