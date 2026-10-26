//! Color and tonal adjustment algorithms inspired by Linux Mint Pix.

use crate::buffer::PixelBuffer;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::f32::consts::FRAC_PI_2;

/// Parametric color adjustment configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorAdjustments {
    /// Exposure adjustment in EV units (-5.0 to +5.0, default 0.0)
    pub exposure: f32,
    /// Brightness adjustment (-1.0 to 1.0, default 0.0)
    pub brightness: f32,
    /// Contrast adjustment (-1.0 to 1.0, default 0.0)
    pub contrast: f32,
    /// Saturation adjustment (-1.0 to 1.0, default 0.0)
    pub saturation: f32,
    /// Hue rotation (-1.0 to 1.0 maps to -180 to +180 degrees, default 0.0)
    pub hue: f32,
    /// Gamma correction (0.1 to 5.0, default 1.0)
    pub gamma: f32,
    /// Highlights adjustment (-1.0 to 1.0, default 0.0)
    pub highlights: f32,
    /// Shadows adjustment (-1.0 to 1.0, default 0.0)
    pub shadows: f32,
    /// Assumed illuminant colour temperature in Kelvin
    /// ([`TEMPERATURE_MIN_KELVIN`] to [`TEMPERATURE_MAX_KELVIN`], neutral
    /// [`TEMPERATURE_NEUTRAL_KELVIN`]). Higher values warm the image up.
    pub temperature: f32,
    /// White balance tint offset (-1.0 to 1.0, default 0.0)
    pub tint: f32,
    /// RGB midtone color balancing [Cyan-Red, Magenta-Green, Yellow-Blue] (-1.0 to 1.0)
    pub color_level: [f32; 3],
    /// Vignette falloff (-1.0 to 1.0, positive darkens corners, negative lightens)
    pub vignette: f32,
    /// 1-click negative color inversion
    pub invert: bool,
    /// Spline tone curves for composite and individual RGB channels
    pub curves: Curves,
    /// Parametric red-eye removal spots (normalized coordinates)
    pub red_eye_fixes: Vec<RedEyeFix>,
    /// Color filter preset
    pub filter_preset: FilterPreset,
}

/// A single red-eye removal spot in normalized [0.0, 1.0] image coordinates.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RedEyeFix {
    pub x: f32,
    pub y: f32,
    /// Radius normalized to image height (e.g. 0.02 for ~2% of height).
    pub radius: f32,
}

/// A 2D control point for a tone curve, normalized to [0.0, 1.0].
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
}

impl CurvePoint {
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
        }
    }
}

/// A natural cubic spline tone curve defined by sorted control points.
/// Uses Fritsch-Carlson monotone cubic spline interpolation to eliminate overshoot/ringing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToneCurve {
    pub points: Vec<CurvePoint>,
}

impl Default for ToneCurve {
    fn default() -> Self {
        Self {
            points: vec![
                CurvePoint { x: 0.0, y: 0.0 },
                CurvePoint { x: 1.0, y: 1.0 },
            ],
        }
    }
}

impl ToneCurve {
    pub fn new(mut points: Vec<CurvePoint>) -> Self {
        if points.is_empty() {
            return Self::default();
        }
        points.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        if points.first().map(|p| p.x > 0.0).unwrap_or(false) {
            points.insert(0, CurvePoint { x: 0.0, y: points[0].y });
        }
        if points.last().map(|p| p.x < 1.0).unwrap_or(false) {
            let last_y = points.last().unwrap().y;
            points.push(CurvePoint { x: 1.0, y: last_y });
        }
        Self { points }
    }

    pub fn is_identity(&self) -> bool {
        if self.points.len() == 2 {
            (self.points[0].x - 0.0).abs() < 1e-4
                && (self.points[0].y - 0.0).abs() < 1e-4
                && (self.points[1].x - 1.0).abs() < 1e-4
                && (self.points[1].y - 1.0).abs() < 1e-4
        } else {
            self.points.iter().all(|p| (p.x - p.y).abs() < 1e-4)
        }
    }

    /// Evaluates the curve at normalized x in [0.0, 1.0] using Fritsch-Carlson
    /// monotone cubic spline interpolation.
    pub fn evaluate(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let n = self.points.len();
        if n == 0 {
            return x;
        }
        if n == 1 {
            return self.points[0].y;
        }
        if x <= self.points[0].x {
            return self.points[0].y;
        }
        if x >= self.points[n - 1].x {
            return self.points[n - 1].y;
        }

        // 1. Calculate secants (delta)
        let mut delta = Vec::with_capacity(n - 1);
        for i in 0..n - 1 {
            let h = self.points[i + 1].x - self.points[i].x;
            if h <= 1e-6 {
                delta.push(0.0);
            } else {
                delta.push((self.points[i + 1].y - self.points[i].y) / h);
            }
        }

        // 2. Initialize tangents (d)
        let mut d = vec![0.0; n];
        d[0] = delta[0];
        for i in 1..n - 1 {
            if delta[i - 1] * delta[i] <= 0.0 {
                d[i] = 0.0;
            } else {
                d[i] = (delta[i - 1] + delta[i]) * 0.5;
            }
        }
        d[n - 1] = delta[n - 2];

        // 3. Fritsch-Carlson monotonicity condition
        for i in 0..n - 1 {
            if delta[i] == 0.0 {
                d[i] = 0.0;
                d[i + 1] = 0.0;
            } else {
                let alpha = d[i] / delta[i];
                let beta = d[i + 1] / delta[i];
                let hyp = alpha.hypot(beta);
                if hyp > 3.0 {
                    let tau = 3.0 / hyp;
                    d[i] = tau * alpha * delta[i];
                    d[i + 1] = tau * beta * delta[i];
                }
            }
        }

        // 4. Find interval containing x
        let mut idx = 0;
        for i in 0..n - 1 {
            if x >= self.points[i].x && x <= self.points[i + 1].x {
                idx = i;
                break;
            }
        }

        let h = self.points[idx + 1].x - self.points[idx].x;
        if h <= 1e-6 {
            return self.points[idx].y;
        }
        let t = (x - self.points[idx].x) / h;
        let t2 = t * t;
        let t3 = t2 * t;

        // Hermite basis functions
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;

        let y = h00 * self.points[idx].y
            + h10 * h * d[idx]
            + h01 * self.points[idx + 1].y
            + h11 * h * d[idx + 1];

        y.clamp(0.0, 1.0)
    }

    /// Build a 1024-entry lookup table for fast pixel processing.
    pub fn build_lut(&self) -> [f32; 1024] {
        let mut lut = [0.0f32; 1024];
        for i in 0..1024 {
            let x = i as f32 / 1023.0;
            lut[i] = self.evaluate(x);
        }
        lut
    }
}

/// Tone curves for Composite RGB, Red, Green, and Blue channels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Curves {
    pub composite: ToneCurve,
    pub red: ToneCurve,
    pub green: ToneCurve,
    pub blue: ToneCurve,
}

impl Curves {
    pub fn is_identity(&self) -> bool {
        self.composite.is_identity()
            && self.red.is_identity()
            && self.green.is_identity()
            && self.blue.is_identity()
    }

    pub fn build_luts(&self) -> ([f32; 1024], [f32; 1024], [f32; 1024], [f32; 1024]) {
        (
            self.composite.build_lut(),
            self.red.build_lut(),
            self.green.build_lut(),
            self.blue.build_lut(),
        )
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FilterPreset {
    #[default]
    None,
    BlackAndWhite,
    GrayscaleDesaturate,
    GrayscaleAverage,
    Sepia,
    WarmGold,
    CoolNordic,
    Vivid,
    Lomo,
    Vintage,
    CrossProcess,
}

impl FilterPreset {
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::BlackAndWhite => "B&W (Luma)",
            Self::GrayscaleDesaturate => "Desaturate",
            Self::GrayscaleAverage => "Average B&W",
            Self::Sepia => "Sepia",
            Self::WarmGold => "Warm Gold",
            Self::CoolNordic => "Cool Nordic",
            Self::Vivid => "Vivid",
            Self::Lomo => "Lomo",
            Self::Vintage => "Vintage",
            Self::CrossProcess => "Cross-Process",
        }
    }
}

impl Default for ColorAdjustments {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            hue: 0.0,
            gamma: 1.0,
            highlights: 0.0,
            shadows: 0.0,
            temperature: TEMPERATURE_NEUTRAL_KELVIN,
            tint: 0.0,
            color_level: [0.0, 0.0, 0.0],
            vignette: 0.0,
            invert: false,
            curves: Curves::default(),
            red_eye_fixes: Vec::new(),
            filter_preset: FilterPreset::None,
        }
    }
}

impl ColorAdjustments {
    pub fn is_identity(&self) -> bool {
        self.exposure == 0.0
            && self.brightness == 0.0
            && self.contrast == 0.0
            && self.saturation == 0.0
            && self.hue == 0.0
            && (self.gamma - 1.0).abs() < 1e-4
            && self.highlights == 0.0
            && self.shadows == 0.0
            && self.temperature == TEMPERATURE_NEUTRAL_KELVIN
            && self.tint == 0.0
            && self.color_level == [0.0, 0.0, 0.0]
            && self.vignette == 0.0
            && !self.invert
            && self.curves.is_identity()
            && self.red_eye_fixes.is_empty()
            && self.filter_preset == FilterPreset::None
    }
}

/// Usable range of the white balance temperature control, in Kelvin.
pub const TEMPERATURE_MIN_KELVIN: f32 = 2000.0;
pub const TEMPERATURE_MAX_KELVIN: f32 = 12000.0;
/// Illuminant that produces unity white balance gains.
pub const TEMPERATURE_NEUTRAL_KELVIN: f32 = 6500.0;

/// Second radiation constant hc/k from Planck's law, in metre-Kelvin.
const SECOND_RADIATION_CONSTANT: f32 = 0.014_387_7;
/// Dominant wavelengths of the sRGB primaries, in nanometres.
const CHANNEL_WAVELENGTHS_NM: [f32; 3] = [610.0, 546.1, 435.8];
/// Compresses the physical correction so both ends of the slider stay usable.
const GAIN_RESPONSE: f32 = 0.5;

/// Relative spectral radiance of a Planckian radiator at one wavelength.
fn planck_radiance(wavelength_nm: f32, kelvin: f32) -> f32 {
    let lambda = wavelength_nm * 1e-9;
    let exponent = SECOND_RADIATION_CONSTANT / (lambda * kelvin);
    lambda.powi(-5) / (exponent.exp() - 1.0)
}

/// Channel gains that neutralise an image lit by a blackbody at `kelvin`.
///
/// Gains are the narrow-band response of the sRGB primaries at the neutral
/// illuminant divided by their response at `kelvin`, geometric-mean normalised
/// so the overall image luminance is preserved. Higher Kelvin therefore warms
/// the image (the compensation for a bluer light), and the neutral temperature
/// returns exactly unity gains.
pub fn kelvin_to_white_balance_gains(kelvin: f32) -> [f32; 3] {
    let kelvin = kelvin.clamp(TEMPERATURE_MIN_KELVIN, TEMPERATURE_MAX_KELVIN);

    let raw = CHANNEL_WAVELENGTHS_NM.map(|wavelength| {
        planck_radiance(wavelength, TEMPERATURE_NEUTRAL_KELVIN)
            / planck_radiance(wavelength, kelvin)
    });

    let geometric_mean = (raw[0] * raw[1] * raw[2]).cbrt();
    raw.map(|gain| (gain / geometric_mean).powf(GAIN_RESPONSE))
}

/// Maps a bipolar -1.0..1.0 slider onto the Kelvin range, neutral at 0.0.
pub fn slider_to_kelvin(slider: f32) -> f32 {
    let slider = slider.clamp(-1.0, 1.0);
    if slider >= 0.0 {
        TEMPERATURE_NEUTRAL_KELVIN + slider * (TEMPERATURE_MAX_KELVIN - TEMPERATURE_NEUTRAL_KELVIN)
    } else {
        TEMPERATURE_NEUTRAL_KELVIN + slider * (TEMPERATURE_NEUTRAL_KELVIN - TEMPERATURE_MIN_KELVIN)
    }
}

/// RGB -> luminance/chrominance matrix used for hue rotation (NTSC Y'IQ').
const RGB_TO_YIQ: [[f32; 3]; 3] = [
    [0.299, 0.587, 0.114],
    [0.595_716, -0.274_453, -0.321_263],
    [0.211_539, -0.522_817, 0.311_278],
];

type Mat3 = [[f32; 3]; 3];

fn mat3_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (r, out_row) in out.iter_mut().enumerate() {
        for (c, cell) in out_row.iter_mut().enumerate() {
            *cell = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    out
}

fn mat3_inverse(m: &Mat3) -> Mat3 {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let inv_det = 1.0 / det;

    [
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv_det,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv_det,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv_det,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det,
        ],
    ]
}

/// Builds the linear RGB transform that rotates hue by `degrees`.
///
/// Rotating the I/Q chroma vector leaves luminance untouched, so hue shifts
/// without the lightness and saturation drift an HSL round-trip would cause.
fn hue_rotation_matrix(degrees: f32) -> Mat3 {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let rotate_iq = [[1.0, 0.0, 0.0], [0.0, cos, sin], [0.0, -sin, cos]];

    mat3_mul(&mat3_inverse(&RGB_TO_YIQ), &mat3_mul(&rotate_iq, &RGB_TO_YIQ))
}

/// Linear interpolation (lerp).
#[inline(always)]
pub fn lerp(original: f32, reference: f32, distance: f32) -> f32 {
    distance * reference + (1.0 - distance) * original
}

/// Computes parabolic midtone distance weighting factor.
/// Peak at 0.5 (midtone), tapering to 0 at 0.0 (shadows) and 1.0 (highlights).
#[inline(always)]
pub fn midtone_weight(v: f32) -> f32 {
    let normalized = ((v - 0.5) / 0.5).clamp(-1.0, 1.0);
    0.667 * (1.0 - normalized * normalized)
}

/// Apply a full suite of color adjustments to an f32 linear buffer.
pub fn apply_color_adjustments(buffer: &mut PixelBuffer<f32>, adj: &ColorAdjustments) {
    if adj.is_identity() {
        return;
    }

    // Precalculate factors
    let exp_factor = 2.0_f32.powf(adj.exposure);

    let sat_factor = if adj.saturation < 0.0 {
        (adj.saturation * FRAC_PI_2).tan()
    } else {
        adj.saturation
    };

    let contrast_factor = if adj.contrast < 0.0 {
        (adj.contrast * FRAC_PI_2).tan()
    } else {
        adj.contrast
    };

    let gamma_inv = if adj.gamma > 0.01 {
        1.0 / adj.gamma
    } else {
        1.0
    };

    // White balance channel gains and the hue rotation matrix
    let [wb_r, wb_g, wb_b] = kelvin_to_white_balance_gains(adj.temperature);
    // Tint > 0 shifts magenta (boost red+blue or drop green), < 0 shifts green (boost green)
    let g_tint = 1.0 - adj.tint * 0.15;
    let hue_matrix = hue_rotation_matrix(adj.hue * 360.0);
    let hue_active = adj.hue != 0.0;

    let curves_active = !adj.curves.is_identity();
    let (lut_c, lut_r, lut_g, lut_b) = if curves_active {
        adj.curves.build_luts()
    } else {
        ([0.0; 1024], [0.0; 1024], [0.0; 1024], [0.0; 1024])
    };

    buffer
        .data
        .par_chunks_exact_mut(4)
        .for_each(|pixel| {
            let mut r = pixel[0];
            let mut g = pixel[1];
            let mut b = pixel[2];
            let a = pixel[3];

            // 1. Exposure
            if adj.exposure != 0.0 {
                r *= exp_factor;
                g *= exp_factor;
                b *= exp_factor;
            }

            // 2. White balance temperature & tint
            if adj.temperature != TEMPERATURE_NEUTRAL_KELVIN || adj.tint != 0.0 {
                r *= wb_r;
                g *= wb_g * g_tint;
                b *= wb_b;
            }

            // 3. Highlights and Shadows (tonal compression / lift)
            if adj.highlights != 0.0 || adj.shadows != 0.0 {
                let l = (r.max(g).max(b)).clamp(0.0, 1.0);
                if adj.shadows != 0.0 {
                    let shadow_mask = (1.0 - l).powi(2);
                    let shadow_boost = adj.shadows * shadow_mask * 0.5;
                    r += shadow_boost;
                    g += shadow_boost;
                    b += shadow_boost;
                }
                if adj.highlights != 0.0 {
                    let highlight_mask = l.powi(2);
                    let highlight_cut = adj.highlights * highlight_mask * 0.5;
                    r += highlight_cut;
                    g += highlight_cut;
                    b += highlight_cut;
                }
            }

            // 4. Gamma Correction
            if (adj.gamma - 1.0).abs() > 1e-4 {
                r = if r > 0.0 { r.powf(gamma_inv) } else { 0.0 };
                g = if g > 0.0 { g.powf(gamma_inv) } else { 0.0 };
                b = if b > 0.0 { b.powf(gamma_inv) } else { 0.0 };
            }

            // 5. Brightness (Pix lerp towards 1.0 or 0.0)
            if adj.brightness > 0.0 {
                r = lerp(r, 1.0, adj.brightness);
                g = lerp(g, 1.0, adj.brightness);
                b = lerp(b, 1.0, adj.brightness);
            } else if adj.brightness < 0.0 {
                r = lerp(r, 0.0, -adj.brightness);
                g = lerp(g, 0.0, -adj.brightness);
                b = lerp(b, 0.0, -adj.brightness);
            }

            // 6. Contrast (Pix lerp towards 0.5 midpoint)
            if adj.contrast != 0.0 {
                r = lerp(r, 0.5, contrast_factor);
                g = lerp(g, 0.5, contrast_factor);
                b = lerp(b, 0.5, contrast_factor);
            }

            // 7. Channel Midtone Balance
            if adj.color_level[0] != 0.0 {
                r += adj.color_level[0] * midtone_weight(r);
            }
            if adj.color_level[1] != 0.0 {
                g += adj.color_level[1] * midtone_weight(g);
            }
            if adj.color_level[2] != 0.0 {
                b += adj.color_level[2] * midtone_weight(b);
            }

            // 8. Hue rotation (luminance-preserving chroma rotation)
            if hue_active {
                let rotated_r = hue_matrix[0][0] * r + hue_matrix[0][1] * g + hue_matrix[0][2] * b;
                let rotated_g = hue_matrix[1][0] * r + hue_matrix[1][1] * g + hue_matrix[1][2] * b;
                let rotated_b = hue_matrix[2][0] * r + hue_matrix[2][1] * g + hue_matrix[2][2] * b;
                r = rotated_r;
                g = rotated_g;
                b = rotated_b;
            }

            // 9. Saturation (Pix lightness lerp)
            if adj.saturation != 0.0 {
                let max = r.max(g).max(b);
                let min = r.min(g).min(b);
                let lightness = (max + min) * 0.5;

                r = lerp(r, lightness, sat_factor);
                g = lerp(g, lightness, sat_factor);
                b = lerp(b, lightness, sat_factor);
            }

            // 10. Filter Presets
            match adj.filter_preset {
                FilterPreset::None => {}
                FilterPreset::BlackAndWhite => {
                    // Pix ITU-R BT.709 standard luminance weights
                    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    r = luma;
                    g = luma;
                    b = luma;
                }
                FilterPreset::GrayscaleDesaturate => {
                    // Pix Desaturate algorithm: (max + min) / 2
                    let l = (r.max(g).max(b) + r.min(g).min(b)) * 0.5;
                    r = l;
                    g = l;
                    b = l;
                }
                FilterPreset::GrayscaleAverage => {
                    // Pix Average algorithm: (r + g + b) / 3
                    let avg = (r + g + b) / 3.0;
                    r = avg;
                    g = avg;
                    b = avg;
                }
                FilterPreset::Sepia => {
                    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    r = (luma * 1.15).min(1.0);
                    g = luma * 0.95;
                    b = luma * 0.75;
                }
                FilterPreset::WarmGold => {
                    r = (r * 1.1 + 0.02).min(1.0);
                    g = (g * 1.02).min(1.0);
                    b = b * 0.92;
                }
                FilterPreset::CoolNordic => {
                    r = r * 0.92;
                    g = (g * 1.02).min(1.0);
                    b = (b * 1.12 + 0.03).min(1.0);
                }
                FilterPreset::Vivid => {
                    r = lerp(r, 0.5, 0.12) * 1.04;
                    g = lerp(g, 0.5, 0.12) * 1.04;
                    b = lerp(b, 0.5, 0.12) * 1.04;
                }
                FilterPreset::Lomo => {
                    // Lomo: saturation boost + strong contrast S-curve
                    let max = r.max(g).max(b);
                    let min = r.min(g).min(b);
                    let l = (max + min) * 0.5;
                    r = lerp(r, l, 0.35);
                    g = lerp(g, l, 0.35);
                    b = lerp(b, l, 0.35);
                    r = lerp(r, 0.5, 0.22);
                    g = lerp(g, 0.5, 0.22);
                    b = lerp(b, 0.5, 0.22);
                }
                FilterPreset::Vintage => {
                    // Vintage: lifted black floor, faded highlights, warm golden wash
                    r = (r * 0.88 + 0.09).min(1.0);
                    g = (g * 0.85 + 0.06).min(1.0);
                    b = (b * 0.74 + 0.04).min(1.0);
                }
                FilterPreset::CrossProcess => {
                    // Cross-process: cyan shadows, yellow highlights
                    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    let shadow_mask = (1.0 - luma).powi(2);
                    let highlight_mask = luma.powi(2);
                    b = (b + shadow_mask * 0.12).min(1.0);
                    g = (g + shadow_mask * 0.06).min(1.0);
                    r = (r + highlight_mask * 0.10).min(1.0);
                    g = (g + highlight_mask * 0.08).min(1.0);
                }
            }

            // 11. Invert / Negative
            if adj.invert {
                r = 1.0 - r;
                g = 1.0 - g;
                b = 1.0 - b;
            }

            // 12. Spline Tone Curves
            if curves_active {
                if !adj.curves.red.is_identity() {
                    let idx = (r.clamp(0.0, 1.0) * 1023.0).round() as usize;
                    r = lut_r[idx];
                }
                if !adj.curves.green.is_identity() {
                    let idx = (g.clamp(0.0, 1.0) * 1023.0).round() as usize;
                    g = lut_g[idx];
                }
                if !adj.curves.blue.is_identity() {
                    let idx = (b.clamp(0.0, 1.0) * 1023.0).round() as usize;
                    b = lut_b[idx];
                }
                if !adj.curves.composite.is_identity() {
                    let idx_r = (r.clamp(0.0, 1.0) * 1023.0).round() as usize;
                    let idx_g = (g.clamp(0.0, 1.0) * 1023.0).round() as usize;
                    let idx_b = (b.clamp(0.0, 1.0) * 1023.0).round() as usize;
                    r = lut_c[idx_r];
                    g = lut_c[idx_g];
                    b = lut_c[idx_b];
                }
            }

            pixel[0] = r.clamp(0.0, 1.0);
            pixel[1] = g.clamp(0.0, 1.0);
            pixel[2] = b.clamp(0.0, 1.0);
            pixel[3] = a;
        });

    // 11. Vignette Radial Falloff
    if adj.vignette != 0.0 {
        let width = buffer.width;
        let height = buffer.height;
        let cx = width as f32 / 2.0;
        let cy = height as f32 / 2.0;
        let vignette = adj.vignette;

        buffer
            .data
            .par_chunks_exact_mut(width as usize * 4)
            .enumerate()
            .for_each(|(y_idx, row)| {
                let y = y_idx as f32;
                let ny = (y - cy) / cy;
                for x_idx in 0..width as usize {
                    let x = x_idx as f32;
                    let nx = (x - cx) / cx;
                    let d = (nx * nx + ny * ny).sqrt();

                    let t = ((d - 0.45) / 0.85).clamp(0.0, 1.0);
                    let s = t * t * (3.0 - 2.0 * t);

                    let factor = if vignette > 0.0 {
                        1.0 - vignette * s * 0.85
                    } else {
                        1.0 + (-vignette) * s * 0.5
                    };

                    let px_idx = x_idx * 4;
                    row[px_idx] = (row[px_idx] * factor).clamp(0.0, 1.0);
                    row[px_idx + 1] = (row[px_idx + 1] * factor).clamp(0.0, 1.0);
                    row[px_idx + 2] = (row[px_idx + 2] * factor).clamp(0.0, 1.0);
                }
            });
    }

    // 13. Red-Eye Removal Fixes
    if !adj.red_eye_fixes.is_empty() {
        for fix in &adj.red_eye_fixes {
            apply_red_eye_fix_to_buffer(buffer, *fix);
        }
    }
}

/// Apply a single red-eye removal fix directly to an f32 PixelBuffer.
/// Implements Pix/GIMP red pupil neutralization: R' = (G * GREEN_FACTOR + B * BLUE_FACTOR) / (2.0 * RED_FACTOR)
/// with smooth radial edge feathering.
pub fn apply_red_eye_fix_to_buffer(buffer: &mut PixelBuffer<f32>, fix: RedEyeFix) {
    if fix.radius <= 0.0 {
        return;
    }
    const RED_FACTOR: f32 = 0.5133333;
    const GREEN_FACTOR: f32 = 1.0;
    const BLUE_FACTOR: f32 = 0.1933333;

    let width = buffer.width;
    let height = buffer.height;

    let cx = fix.x * width as f32;
    let cy = fix.y * height as f32;
    let radius = (fix.radius * height as f32).max(1.0);

    let min_x = ((cx - radius).floor() as i32).max(0) as u32;
    let max_x = ((cx + radius).ceil() as i32).min(width as i32 - 1) as u32;
    let min_y = ((cy - radius).floor() as i32).max(0) as u32;
    let max_y = ((cy + radius).ceil() as i32).min(height as i32 - 1) as u32;

    let r_sq = radius * radius;
    let inner_r = radius * 0.65;
    let inner_r_sq = inner_r * inner_r;

    for y in min_y..=max_y {
        let dy = y as f32 - cy;
        let dy_sq = dy * dy;
        for x in min_x..=max_x {
            let dx = x as f32 - cx;
            let d_sq = dx * dx + dy_sq;
            if d_sq > r_sq {
                continue;
            }

            let idx = ((y * width + x) * 4) as usize;
            let r = buffer.data[idx];
            let g = buffer.data[idx + 1];
            let b = buffer.data[idx + 2];

            let ad_red = r * RED_FACTOR;
            let ad_green = g * GREEN_FACTOR;
            let ad_blue = b * BLUE_FACTOR;

            if ad_red >= ad_green && ad_red >= ad_blue {
                let target_r = ((ad_green + ad_blue) / (2.0 * RED_FACTOR)).clamp(0.0, 1.0);
                let factor = if d_sq <= inner_r_sq {
                    1.0
                } else {
                    let d = d_sq.sqrt();
                    1.0 - ((d - inner_r) / (radius - inner_r)).clamp(0.0, 1.0)
                };
                buffer.data[idx] = r * (1.0 - factor) + target_r * factor;
            }
        }
    }
}

/// 256-bin histogram for Red, Green, Blue, and Luminance channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageHistogram {
    pub r: [u32; 256],
    pub g: [u32; 256],
    pub b: [u32; 256],
    pub luma: [u32; 256],
    pub max_count: u32,
    pub shadow_clipped: bool,
    pub highlight_clipped: bool,
}

impl Default for ImageHistogram {
    fn default() -> Self {
        Self {
            r: [0; 256],
            g: [0; 256],
            b: [0; 256],
            luma: [0; 256],
            max_count: 1,
            shadow_clipped: false,
            highlight_clipped: false,
        }
    }
}

/// Compute 256-bin histogram for RGB and Rec.709 Luma channels.
pub fn compute_histogram(buffer: &PixelBuffer<u8>) -> ImageHistogram {
    let mut r = [0u32; 256];
    let mut g = [0u32; 256];
    let mut b = [0u32; 256];
    let mut luma = [0u32; 256];

    let total = (buffer.width as usize) * (buffer.height as usize);
    if total == 0 {
        return ImageHistogram::default();
    }

    for chunk in buffer.data.chunks_exact(4) {
        let cr = chunk[0];
        let cg = chunk[1];
        let cb = chunk[2];
        r[cr as usize] += 1;
        g[cg as usize] += 1;
        b[cb as usize] += 1;
        let y = ((cr as f32 * 0.2126 + cg as f32 * 0.7152 + cb as f32 * 0.0722).round() as usize)
            .min(255);
        luma[y] += 1;
    }

    let mut max_count = 1u32;
    for i in 1..255 {
        max_count = max_count.max(r[i]).max(g[i]).max(b[i]).max(luma[i]);
    }

    let threshold = (total / 120) as u32; // >0.8% of pixels at extremes indicates clipping
    let shadow_clipped = luma[0] > threshold;
    let highlight_clipped = luma[255] > threshold;

    ImageHistogram {
        r,
        g,
        b,
        luma,
        max_count,
        shadow_clipped,
        highlight_clipped,
    }
}

/// Auto-levels dynamic range estimation inspired by Pix's `adjust-contrast.c`.
/// Returns recommended `(exposure_ev_offset, contrast_factor)`.
pub fn calculate_auto_levels(buffer: &PixelBuffer<u8>) -> (f32, f32) {
    let total = (buffer.width as usize) * (buffer.height as usize);
    if total == 0 {
        return (0.0, 0.0);
    }
    let hist = compute_histogram(buffer);
    let clip_count = (total as f32 * 0.005) as u32; // 0.5% cutoff

    let mut accum = 0u32;
    let mut black_point = 0usize;
    for i in 0..256 {
        accum += hist.luma[i];
        if accum > clip_count {
            black_point = i;
            break;
        }
    }

    accum = 0;
    let mut white_point = 255usize;
    for i in (0..256).rev() {
        accum += hist.luma[i];
        if accum > clip_count {
            white_point = i;
            break;
        }
    }

    if white_point <= black_point {
        return (0.0, 0.0);
    }

    let span = (white_point - black_point) as f32 / 255.0;
    let contrast_boost = ((1.0 / span) - 1.0).clamp(0.0, 0.35);

    let mid = (black_point + white_point) as f32 / 2.0;
    let exp_ev = ((128.0 - mid) / 128.0 * 0.7).clamp(-0.7, 0.7);

    (exp_ev, contrast_boost)
}

/// Neutralize color cast by sampling a known neutral white or gray pixel (r, g, b in 0.0..=1.0).
/// Returns recommended `(temperature_kelvin, tint_offset)`.
pub fn calculate_white_balance_from_sample(r: f32, g: f32, b: f32) -> (f32, f32) {
    let y = (r + g + b) / 3.0;
    if y < 0.02 {
        return (TEMPERATURE_NEUTRAL_KELVIN, 0.0);
    }

    // Temperature ratio: if r > b, illuminant was warm, so lower Kelvin to cool image.
    // If b > r, illuminant was cool, so increase Kelvin to warm image.
    let temp_ratio = (b / r.max(0.01)).clamp(0.2, 5.0);
    let kelvin = (TEMPERATURE_NEUTRAL_KELVIN * temp_ratio.powf(0.55))
        .clamp(TEMPERATURE_MIN_KELVIN, TEMPERATURE_MAX_KELVIN);

    // Tint: green deviation from red-blue midpoint
    let expected_g = (r + b) * 0.5;
    let tint = ((g - expected_g) / y * 2.5).clamp(-1.0, 1.0);

    (kelvin, tint)
}
