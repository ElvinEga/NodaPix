//! The collapsible editing panel containing real-time adjustment sliders.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    canvas, div, fill, point, px, size, Bounds, CursorStyle, ParentElement, Pixels,
    Styled, rgb,
};
use nodapix_core::EditState;

pub const PANEL_WIDTH: f32 = 280.0;
pub const TRACK_HEIGHT: f32 = 4.0;
pub const THUMB_RADIUS: f32 = 5.0;

pub const PANEL_BG: u32 = 0x18_18_1c;
pub const SECTION_BG: u32 = 0x20_20_25;
pub const BORDER_COLOR: u32 = 0x2d_2d_35;
pub const TEXT_PRIMARY: u32 = 0xee_ee_f2;
pub const TEXT_MUTED: u32 = 0x8a_8a_96;
pub const TRACK_BG: u32 = 0x34_34_3e;
pub const ACCENT_COLOR: u32 = 0x0a_84_ff;
pub const THUMB_COLOR: u32 = 0xff_ff_ff;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SliderId {
    Exposure,
    Brightness,
    Contrast,
    Highlights,
    Shadows,
    Saturation,
    Temperature,
    Tint,
    Sharpness,
}

#[derive(Clone, Copy)]
pub struct SliderSpec {
    pub id: SliderId,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    #[allow(dead_code)]
    pub default: f32,
    pub bipolar: bool,
}

impl SliderSpec {
    pub const EXPOSURE: Self = Self {
        id: SliderId::Exposure,
        label: "Exposure",
        min: -3.0,
        max: 3.0,
        default: 0.0,
        bipolar: true,
    };
    pub const BRIGHTNESS: Self = Self {
        id: SliderId::Brightness,
        label: "Brightness",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        bipolar: true,
    };
    pub const CONTRAST: Self = Self {
        id: SliderId::Contrast,
        label: "Contrast",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        bipolar: true,
    };
    pub const HIGHLIGHTS: Self = Self {
        id: SliderId::Highlights,
        label: "Highlights",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        bipolar: true,
    };
    pub const SHADOWS: Self = Self {
        id: SliderId::Shadows,
        label: "Shadows",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        bipolar: true,
    };
    pub const SATURATION: Self = Self {
        id: SliderId::Saturation,
        label: "Saturation",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        bipolar: true,
    };
    pub const TEMPERATURE: Self = Self {
        id: SliderId::Temperature,
        label: "Temperature",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        bipolar: true,
    };
    pub const TINT: Self = Self {
        id: SliderId::Tint,
        label: "Tint",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        bipolar: true,
    };
    pub const SHARPNESS: Self = Self {
        id: SliderId::Sharpness,
        label: "Sharpness",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        bipolar: false,
    };

    pub fn format_value(&self, val: f32) -> String {
        match self.id {
            SliderId::Exposure => {
                if val.abs() < 0.01 {
                    "0.0 EV".to_string()
                } else if val > 0.0 {
                    format!("+{:.1} EV", val)
                } else {
                    format!("{:.1} EV", val)
                }
            }
            SliderId::Sharpness => format!("{:.0}%", val * 100.0),
            _ => {
                if val.abs() < 0.01 {
                    "0%".to_string()
                } else if val > 0.0 {
                    format!("+{:.0}%", val * 100.0)
                } else {
                    format!("{:.0}%", val * 100.0)
                }
            }
        }
    }

    pub fn get_value(&self, state: &EditState) -> f32 {
        match self.id {
            SliderId::Exposure => state.color.exposure,
            SliderId::Brightness => state.color.brightness,
            SliderId::Contrast => state.color.contrast,
            SliderId::Highlights => state.color.highlights,
            SliderId::Shadows => state.color.shadows,
            SliderId::Saturation => state.color.saturation,
            SliderId::Temperature => state.color.temperature,
            SliderId::Tint => state.color.tint,
            SliderId::Sharpness => state.sharpness,
        }
    }

    pub fn set_value(&self, state: &mut EditState, val: f32) {
        let val = val.clamp(self.min, self.max);
        match self.id {
            SliderId::Exposure => state.color.exposure = val,
            SliderId::Brightness => state.color.brightness = val,
            SliderId::Contrast => state.color.contrast = val,
            SliderId::Highlights => state.color.highlights = val,
            SliderId::Shadows => state.color.shadows = val,
            SliderId::Saturation => state.color.saturation = val,
            SliderId::Temperature => state.color.temperature = val,
            SliderId::Tint => state.color.tint = val,
            SliderId::Sharpness => state.sharpness = val,
        }
    }

    pub fn normalized(&self, val: f32) -> f32 {
        ((val - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    pub fn from_normalized(&self, norm: f32) -> f32 {
        self.min + norm * (self.max - self.min)
    }
}

pub const ALL_SPECS: &[SliderSpec] = &[
    SliderSpec::EXPOSURE,
    SliderSpec::BRIGHTNESS,
    SliderSpec::CONTRAST,
    SliderSpec::HIGHLIGHTS,
    SliderSpec::SHADOWS,
    SliderSpec::SATURATION,
    SliderSpec::TEMPERATURE,
    SliderSpec::TINT,
    SliderSpec::SHARPNESS,
];

pub struct SliderState {
    pub bounds: Rc<RefCell<HashMap<SliderId, Bounds<Pixels>>>>,
    pub active_slider: Option<SliderId>,
}

impl Default for SliderState {
    fn default() -> Self {
        Self {
            bounds: Rc::new(RefCell::new(HashMap::new())),
            active_slider: None,
        }
    }
}

impl SliderState {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn render_slider_row(
    spec: SliderSpec,
    current_value: f32,
    slider_bounds: Rc<RefCell<HashMap<SliderId, Bounds<Pixels>>>>,
) -> gpui::Div {
    let id = spec.id;
    let norm = spec.normalized(current_value);
    let bipolar = spec.bipolar;
    let val_str = spec.format_value(current_value);

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .text_xs()
                .child(
                    div()
                        .text_color(rgb(TEXT_PRIMARY))
                        .child(spec.label),
                )
                .child(
                    div()
                        .text_color(rgb(TEXT_MUTED))
                        .child(val_str),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(14.0))
                .relative()
                .cursor(CursorStyle::PointingHand)
                .child(canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, _| {
                        slider_bounds.borrow_mut().insert(id, bounds);

                        let bx = f32::from(bounds.origin.x);
                        let by = f32::from(bounds.origin.y);
                        let bw = f32::from(bounds.size.width);
                        let bh = f32::from(bounds.size.height);

                        let track_y = by + (bh - TRACK_HEIGHT) / 2.0;
                        let track_bounds = Bounds {
                            origin: point(px(bx), px(track_y)),
                            size: size(px(bw), px(TRACK_HEIGHT)),
                        };

                        // Track background
                        window.paint_quad(fill(track_bounds, rgb(TRACK_BG)));

                        // Active fill
                        let thumb_x = bx + norm * bw;
                        let fill_bounds = if bipolar {
                            let center_x = bx + bw * 0.5;
                            let (start_x, fill_w) = if thumb_x >= center_x {
                                (center_x, thumb_x - center_x)
                            } else {
                                (thumb_x, center_x - thumb_x)
                            };
                            Bounds {
                                origin: point(px(start_x), px(track_y)),
                                size: size(px(fill_w), px(TRACK_HEIGHT)),
                            }
                        } else {
                            Bounds {
                                origin: point(px(bx), px(track_y)),
                                size: size(px(norm * bw), px(TRACK_HEIGHT)),
                            }
                        };
                        window.paint_quad(fill(fill_bounds, rgb(ACCENT_COLOR)));

                        // Circular thumb
                        let thumb_bounds = Bounds {
                            origin: point(px(thumb_x - THUMB_RADIUS), px(by + (bh / 2.0) - THUMB_RADIUS)),
                            size: size(px(THUMB_RADIUS * 2.0), px(THUMB_RADIUS * 2.0)),
                        };
                        window.paint_quad(fill(thumb_bounds, rgb(THUMB_COLOR)));
                    },
                )),
        )
}
