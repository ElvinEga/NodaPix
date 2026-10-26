//! ImageDocument: owns source image, proxy buffer, editing state, and undo history.

use crate::edit_state::EditState;
use crate::history::HistoryStack;
use crate::pipeline::execute_pipeline;
use nodapix_imaging::{downsample_to_fit, straightened_size, PixelBuffer};
use std::path::PathBuf;
use std::sync::Arc;

pub const PROXY_MAX_DIMENSION: u32 = 2048;

/// Render a display preview from a detached proxy buffer.
///
/// Kept separate from [`ImageDocument::render_preview`] so the app can hand the
/// proxy and a state snapshot to a worker thread without touching the document.
pub fn render_proxy_preview(proxy: &PixelBuffer<f32>, state: &EditState) -> PixelBuffer<u8> {
    execute_pipeline(proxy, state).to_u8_srgb()
}

#[derive(Clone, Debug)]
pub struct ImageDocument {
    /// Full-resolution original decoded image in linear f32 RGBA.
    pub source: PixelBuffer<f32>,
    /// Display-resolution proxy image for 60fps slider updates.
    /// Shared so interactive renders can be dispatched off the main thread.
    pub proxy_source: Arc<PixelBuffer<f32>>,
    /// Active parametric edit state.
    pub state: EditState,
    /// Committed checkpoints for Undo/Redo.
    pub history: HistoryStack<EditState>,
    /// Source file path if opened from disk.
    pub file_path: Option<PathBuf>,
    /// Whether unsaved edits exist.
    pub is_dirty: bool,
}

impl ImageDocument {
    pub fn new(source: PixelBuffer<f32>, file_path: Option<PathBuf>) -> Self {
        let proxy_source = Arc::new(downsample_to_fit(&source, PROXY_MAX_DIMENSION));
        Self {
            source,
            proxy_source,
            state: EditState::default(),
            history: HistoryStack::default(),
            file_path,
            is_dirty: false,
        }
    }

    /// Render interactive display preview (using downsampled proxy for speed).
    pub fn render_preview(&self) -> PixelBuffer<u8> {
        render_proxy_preview(&self.proxy_source, &self.state)
    }

    /// Render unedited original display preview for comparison toggle.
    pub fn render_original_preview(&self) -> PixelBuffer<u8> {
        self.proxy_source.to_u8_srgb()
    }

    /// Render full-resolution output for saving/export.
    pub fn render_export(&self) -> PixelBuffer<f32> {
        execute_pipeline(&self.source, &self.state)
    }

    /// Checkpoint the current state before or after a discrete edit operation.
    pub fn commit_checkpoint(&mut self) {
        self.history.push_checkpoint(self.state.clone());
        self.is_dirty = true;
    }

    /// Undo last committed operation.
    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.history.undo(self.state.clone()) {
            self.state = prev;
            self.is_dirty = true;
            true
        } else {
            false
        }
    }

    /// Redo previously undone operation.
    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.history.redo(self.state.clone()) {
            self.state = next;
            self.is_dirty = true;
            true
        } else {
            false
        }
    }

    pub fn file_name(&self) -> String {
        self.file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled.png".to_string())
    }

    /// Calculate the resulting output dimensions after rotation, straightening, and crop.
    pub fn output_size(&self) -> (u32, u32) {
        let (mut w, mut h) = (self.source.width, self.source.height);
        if matches!(
            self.state.rotation,
            nodapix_imaging::OrthogonalRotation::Rotate90
                | nodapix_imaging::OrthogonalRotation::Rotate270
        ) {
            std::mem::swap(&mut w, &mut h);
        }
        let (w, h) = straightened_size(w, h, self.state.angle_fine_degrees);
        if let Some(crop) = self.state.crop {
            let (_, _, cw, ch) = crop.to_pixel_coords(w, h);
            (cw, ch)
        } else {
            (w, h)
        }
    }
}
