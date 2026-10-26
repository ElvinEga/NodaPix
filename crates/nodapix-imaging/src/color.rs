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
    /// Gamma correction (0.1 to 5.0, default 1.0)
    pub gamma: f32,
    /// Highlights adjustment (-1.0 to 1.0, default 0.0)
    pub highlights: f32,
    /// Shadows adjustment (-1.0 to 1.0, default 0.0)
    pub shadows: f32,
    /// White balance temperature offset (-1.0 to 1.0, default 0.0)
    pub temperature: f32,
    /// White balance tint offset (-1.0 to 1.0, default 0.0)
    pub tint: f32,
    /// RGB midtone color balancing [Cyan-Red, Magenta-Green, Yellow-Blue] (-1.0 to 1.0)
    pub color_level: [f32; 3],
}

impl Default for ColorAdjustments {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            gamma: 1.0,
            highlights: 0.0,
            shadows: 0.0,
            temperature: 0.0,
            tint: 0.0,
            color_level: [0.0, 0.0, 0.0],
        }
    }
}

impl ColorAdjustments {
    pub fn is_identity(&self) -> bool {
        self.exposure == 0.0
            && self.brightness == 0.0
            && self.contrast == 0.0
            && self.saturation == 0.0
            && (self.gamma - 1.0).abs() < 1e-4
            && self.highlights == 0.0
            && self.shadows == 0.0
            && self.temperature == 0.0
            && self.tint == 0.0
            && self.color_level == [0.0, 0.0, 0.0]
    }
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

    // Temperature (Kelvin shift) and Tint multipliers
    // Temperature > 0 shifts warmer (boost red, drop blue), < 0 shifts cooler (boost blue, drop red)
    let r_temp = 1.0 + adj.temperature * 0.2;
    let b_temp = 1.0 - adj.temperature * 0.2;
    // Tint > 0 shifts magenta (boost red+blue or drop green), < 0 shifts green (boost green)
    let g_tint = 1.0 - adj.tint * 0.15;

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

            // 2. Temperature & Tint
            if adj.temperature != 0.0 || adj.tint != 0.0 {
                r *= r_temp;
                g *= g_tint;
                b *= b_temp;
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

            // 8. Saturation (Pix lightness lerp)
            if adj.saturation != 0.0 {
                let max = r.max(g).max(b);
                let min = r.min(g).min(b);
                let lightness = (max + min) * 0.5;

                r = lerp(r, lightness, sat_factor);
                g = lerp(g, lightness, sat_factor);
                b = lerp(b, lightness, sat_factor);
            }

            pixel[0] = r.clamp(0.0, 1.0);
            pixel[1] = g.clamp(0.0, 1.0);
            pixel[2] = b.clamp(0.0, 1.0);
            pixel[3] = a;
        });
}
