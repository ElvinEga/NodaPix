//! Non-destructive parametric edit state for NodaPix.

use nodapix_imaging::{ColorAdjustments, CropRect, OrthogonalRotation};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditState {
    pub rotation: OrthogonalRotation,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub crop: Option<CropRect>,
    pub color: ColorAdjustments,
    pub sharpness: f32, // 0.0 to 1.0
}

impl Default for EditState {
    fn default() -> Self {
        Self {
            rotation: OrthogonalRotation::None,
            flip_horizontal: false,
            flip_vertical: false,
            crop: None,
            color: ColorAdjustments::default(),
            sharpness: 0.0,
        }
    }
}

impl EditState {
    pub fn is_identity(&self) -> bool {
        self.rotation == OrthogonalRotation::None
            && !self.flip_horizontal
            && !self.flip_vertical
            && self.crop.is_none()
            && self.color.is_identity()
            && self.sharpness == 0.0
    }

    pub fn rotate_cw(&mut self) {
        self.rotation = self.rotation.next_cw();
    }

    pub fn rotate_ccw(&mut self) {
        self.rotation = self.rotation.next_ccw();
    }

    pub fn toggle_flip_h(&mut self) {
        self.flip_horizontal = !self.flip_horizontal;
    }

    pub fn toggle_flip_v(&mut self) {
        self.flip_vertical = !self.flip_vertical;
    }

    pub fn reset_adjustments(&mut self) {
        self.color = ColorAdjustments::default();
        self.sharpness = 0.0;
    }

    pub fn reset_all(&mut self) {
        *self = Self::default();
    }
}
