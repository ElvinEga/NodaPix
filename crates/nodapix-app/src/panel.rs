//! The collapsible editing panel containing real-time adjustment sliders.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    canvas, div, fill, point, px, size, Bounds, CursorStyle, ParentElement, Pixels,
    Styled, rgb, rgba,
};
use nodapix_core::EditState;
use nodapix_imaging::{
    ImageHistogram, TEMPERATURE_MAX_KELVIN, TEMPERATURE_MIN_KELVIN, TEMPERATURE_NEUTRAL_KELVIN,
};

pub const PANEL_WIDTH: f32 = 280.0;
pub const TRACK_HEIGHT: f32 = 4.0;
pub const THUMB_RADIUS: f32 = 5.0;

/// Reciprocal colour-temperature unit: mired = 10^6 / Kelvin.
const MIREDS_PER_KELVIN: f32 = 1_000_000.0;

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
    Hue,
    Temperature,
    Tint,
    Sharpness,
    Angle,
    Vignette,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolGroup {
    BrightnessContrast,
    ColorBalance,
    Levels,
    HueSaturation,
    Sharpen,
    Vignette,
    Straighten,
}

#[derive(Clone, Copy)]
pub struct SliderSpec {
    pub id: SliderId,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
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
    pub const HUE: Self = Self {
        id: SliderId::Hue,
        label: "Hue",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        bipolar: true,
    };
    pub const TEMPERATURE: Self = Self {
        id: SliderId::Temperature,
        label: "Temperature",
        min: TEMPERATURE_MIN_KELVIN,
        max: TEMPERATURE_MAX_KELVIN,
        default: TEMPERATURE_NEUTRAL_KELVIN,
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
    pub const ANGLE: Self = Self {
        id: SliderId::Angle,
        label: "Angle",
        min: -45.0,
        max: 45.0,
        default: 0.0,
        bipolar: true,
    };
    pub const VIGNETTE: Self = Self {
        id: SliderId::Vignette,
        label: "Vignette",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        bipolar: true,
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
            SliderId::Temperature => format!("{:.0}K", val),
            SliderId::Angle => {
                if val.abs() < 0.05 {
                    "0.0°".to_string()
                } else {
                    format!("{:+.1}°", val)
                }
            }
            SliderId::Hue => {
                let degrees = val * 180.0;
                if degrees.abs() < 0.5 {
                    "0°".to_string()
                } else {
                    format!("{:+.0}°", degrees)
                }
            }
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
            SliderId::Hue => state.color.hue,
            SliderId::Temperature => state.color.temperature,
            SliderId::Tint => state.color.tint,
            SliderId::Sharpness => state.sharpness,
            SliderId::Angle => state.angle_fine_degrees,
            SliderId::Vignette => state.color.vignette,
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
            SliderId::Hue => state.color.hue = val,
            SliderId::Temperature => state.color.temperature = val,
            SliderId::Tint => state.color.tint = val,
            SliderId::Sharpness => state.sharpness = val,
            SliderId::Angle => state.angle_fine_degrees = val,
            SliderId::Vignette => state.color.vignette = val,
        }
    }

    /// Temperature travels linearly in mireds (10^6/Kelvin) on either side of
    /// neutral, which is how the eye reads colour temperature, while every other
    /// control is linear in its own value.
    fn is_mired_scale(&self) -> bool {
        self.id == SliderId::Temperature
    }

    pub fn normalized(&self, val: f32) -> f32 {
        if self.is_mired_scale() {
            let mired = MIREDS_PER_KELVIN / val.max(1.0);
            let neutral = MIREDS_PER_KELVIN / self.default;
            // Left half: coldest endpoint has the most mireds.
            if mired >= neutral {
                let coldest = MIREDS_PER_KELVIN / self.min;
                return (1.0 - (mired - neutral) / (coldest - neutral)) * 0.5;
            }
            let warmest = MIREDS_PER_KELVIN / self.max;
            return 0.5 + (neutral - mired) / (neutral - warmest) * 0.5;
        }
        ((val - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    pub fn from_normalized(&self, norm: f32) -> f32 {
        if self.is_mired_scale() {
            let neutral = MIREDS_PER_KELVIN / self.default;
            let mired = if norm <= 0.5 {
                let coldest = MIREDS_PER_KELVIN / self.min;
                neutral + (1.0 - norm * 2.0) * (coldest - neutral)
            } else {
                let warmest = MIREDS_PER_KELVIN / self.max;
                neutral - (norm * 2.0 - 1.0) * (neutral - warmest)
            };
            return (MIREDS_PER_KELVIN / mired).clamp(self.min, self.max);
        }
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
    SliderSpec::HUE,
    SliderSpec::TEMPERATURE,
    SliderSpec::TINT,
    SliderSpec::SHARPNESS,
    SliderSpec::ANGLE,
    SliderSpec::VIGNETTE,
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
                            let center_x = bx + bw * spec.normalized(spec.default);
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

/// Render a 256-bin RGB and Luminance live histogram.
pub fn render_histogram(hist: Option<&ImageHistogram>) -> gpui::Div {
    let hist_data = hist.cloned().unwrap_or_default();
    div()
        .w_full()
        .h(px(72.0))
        .bg(rgb(0x13_13_17))
        .border_1()
        .border_color(rgb(BORDER_COLOR))
        .rounded_md()
        .relative()
        .overflow_hidden()
        .child(
            canvas(
                move |_, _, _| {},
                move |bounds, _, window, _| {
                    let w = f32::from(bounds.size.width);
                    let h = f32::from(bounds.size.height);
                    let ox = f32::from(bounds.origin.x);
                    let oy = f32::from(bounds.origin.y);

                    // Grid lines (25%, 50%, 75%)
                    let grid_col = rgba(0xff_ff_ff_12);
                    for step in [0.25, 0.50, 0.75] {
                        let gx = ox + w * step;
                        window.paint_quad(fill(
                            Bounds {
                                origin: point(px(gx), px(oy)),
                                size: size(px(1.0), px(h)),
                            },
                            grid_col,
                        ));
                    }

                    if hist_data.max_count == 0 {
                        return;
                    }

                    let max_c = hist_data.max_count as f32;
                    let red_col = rgba(0xff_45_3a_55);
                    let green_col = rgba(0x30_d1_58_55);
                    let blue_col = rgba(0x0a_84_ff_55);
                    let luma_col = rgba(0xff_ff_ff_70);

                    // Draw 256 bins
                    let bar_w = (w / 256.0).max(1.0);
                    for i in 0..256 {
                        let bx = ox + (i as f32 / 256.0) * w;

                        // Red
                        let rh = (hist_data.r[i] as f32 / max_c * h).min(h);
                        if rh > 0.5 {
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(bx), px(oy + h - rh)),
                                    size: size(px(bar_w), px(rh)),
                                },
                                red_col,
                            ));
                        }

                        // Green
                        let gh = (hist_data.g[i] as f32 / max_c * h).min(h);
                        if gh > 0.5 {
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(bx), px(oy + h - gh)),
                                    size: size(px(bar_w), px(gh)),
                                },
                                green_col,
                            ));
                        }

                        // Blue
                        let bh = (hist_data.b[i] as f32 / max_c * h).min(h);
                        if bh > 0.5 {
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(bx), px(oy + h - bh)),
                                    size: size(px(bar_w), px(bh)),
                                },
                                blue_col,
                            ));
                        }

                        // Luma top dot
                        let lh = (hist_data.luma[i] as f32 / max_c * h).min(h);
                        if lh > 0.5 {
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(bx), px(oy + h - lh)),
                                    size: size(px(bar_w), px(1.0)),
                                },
                                luma_col,
                            ));
                        }
                    }

                    // Shadow clipping indicator (bottom left blue dot)
                    if hist_data.shadow_clipped {
                        window.paint_quad(fill(
                            Bounds {
                                origin: point(px(ox + 4.0), px(oy + h - 8.0)),
                                size: size(px(5.0), px(5.0)),
                            },
                            rgb(0x0a_84_ff),
                        ));
                    }

                    // Highlight clipping indicator (bottom right red dot)
                    if hist_data.highlight_clipped {
                        window.paint_quad(fill(
                            Bounds {
                                origin: point(px(ox + w - 9.0), px(oy + h - 8.0)),
                                size: size(px(5.0), px(5.0)),
                            },
                            rgb(0xff_45_3a),
                        ));
                    }
                },
            )
            .size_full(),
        )
}
