//! Generates the master high-resolution 1024x1024 Retina application icon for NodaPix.

use image::{Rgba, RgbaImage};
use std::f32::consts::PI;
use std::path::Path;

fn main() {
    let size = 1024;
    let mut img = RgbaImage::new(size, size);

    let cx = 512.0_f32;
    let cy = 512.0_f32;

    // Apple squircle parameters
    let half_w = 410.0_f32;
    let corner_r = 180.0_f32;
    let inner_box = half_w - corner_r;

    for y in 0..size {
        for x in 0..size {
            let px = x as f32;
            let py = y as f32;

            // Squircle Signed Distance Field (SDF)
            let qx = (px - cx).abs() - inner_box;
            let qy = (py - cy).abs() - inner_box;
            let d_outer = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt()
                + qx.max(qy).min(0.0)
                - corner_r;

            // Antialiased mask for outer squircle
            let alpha = (0.5 - d_outer).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }

            // Squircle background gradient: midnight graphite
            let t = py / 1024.0;
            let bg_r = 26.0 * (1.0 - t) + 14.0 * t;
            let bg_g = 28.0 * (1.0 - t) + 15.0 * t;
            let bg_b = 38.0 * (1.0 - t) + 22.0 * t;

            let mut r = bg_r;
            let mut g = bg_g;
            let mut b = bg_b;

            // Outer rim highlight
            if d_outer >= -6.0 && d_outer <= 0.0 {
                let rim_t = (d_outer + 6.0) / 6.0;
                let top_bias = (1.0 - t).max(0.2);
                let rim_glow = (1.0 - rim_t) * top_bias * 60.0;
                r += rim_glow;
                g += rim_glow;
                b += rim_glow * 1.2;
            }

            // Lens & Aperture disc
            let dist_center = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();

            // Outer metallic bezel (radius 270 to 290)
            if dist_center <= 290.0 && dist_center >= 265.0 {
                let angle = (py - cy).atan2(px - cx);
                let metallic = 45.0 + 25.0 * (angle * 2.0).sin().abs();
                r = metallic;
                g = metallic + 4.0;
                b = metallic + 12.0;
            } else if dist_center < 265.0 {
                // Glass lens interior
                let lens_norm = dist_center / 265.0;

                // Deep optical sapphire coating
                let mut lr = 12.0 + 8.0 * (1.0 - lens_norm);
                let mut lg = 18.0 + 16.0 * (1.0 - lens_norm);
                let mut lb = 36.0 + 60.0 * (1.0 - lens_norm);

                // Cyan/blue glare highlight at top-left
                let glare_dist = ((px - (cx - 100.0)).powi(2) + (py - (cy - 100.0)).powi(2)).sqrt();
                if glare_dist < 180.0 {
                    let glare = (1.0 - glare_dist / 180.0).powi(2) * 140.0;
                    lr += glare * 0.1;
                    lg += glare * 0.6;
                    lb += glare * 1.0;
                }

                // Magenta/warm optical reflection at bottom-right
                let warm_dist = ((px - (cx + 90.0)).powi(2) + (py - (cy + 90.0)).powi(2)).sqrt();
                if warm_dist < 160.0 {
                    let warm = (1.0 - warm_dist / 160.0).powi(2) * 90.0;
                    lr += warm * 1.0;
                    lg += warm * 0.2;
                    lb += warm * 0.7;
                }

                // Aperture blades (6 blades)
                let angle = (py - cy).atan2(px - cx) + PI / 6.0;
                let blade_phase = (angle * 3.0).sin();
                if dist_center < 160.0 && dist_center > 70.0 {
                    let blade_tint = blade_phase.abs() * 18.0;
                    lr = (lr - blade_tint).max(0.0);
                    lg = (lg - blade_tint).max(0.0);
                    lb = (lb - blade_tint).max(0.0);
                }

                // Center aperture opening
                if dist_center <= 70.0 {
                    let center_fade = dist_center / 70.0;
                    lr = lr * center_fade + 5.0 * (1.0 - center_fade);
                    lg = lg * center_fade + 8.0 * (1.0 - center_fade);
                    lb = lb * center_fade + 16.0 * (1.0 - center_fade);
                }

                r = lr;
                g = lg;
                b = lb;
            }

            // Stylized Crop Framing Overlay (crisp white brackets at corners)
            // Left bracket: x around 340, Top around 340
            let in_bracket_tl = (px >= 330.0 && px <= 410.0 && (py - 330.0).abs() <= 6.0)
                || (py >= 330.0 && py <= 410.0 && (px - 330.0).abs() <= 6.0);
            let in_bracket_tr = (px >= 614.0 && px <= 694.0 && (py - 330.0).abs() <= 6.0)
                || (py >= 330.0 && py <= 410.0 && (px - 694.0).abs() <= 6.0);
            let in_bracket_bl = (px >= 330.0 && px <= 410.0 && (py - 694.0).abs() <= 6.0)
                || (py >= 614.0 && py <= 694.0 && (px - 330.0).abs() <= 6.0);
            let in_bracket_br = (px >= 614.0 && px <= 694.0 && (py - 694.0).abs() <= 6.0)
                || (py >= 614.0 && py <= 694.0 && (px - 694.0).abs() <= 6.0);

            if in_bracket_tl || in_bracket_tr || in_bracket_bl || in_bracket_br {
                r = 255.0;
                g = 255.0;
                b = 255.0;
            }

            let final_r = (r.clamp(0.0, 255.0) * alpha) as u8;
            let final_g = (g.clamp(0.0, 255.0) * alpha) as u8;
            let final_b = (b.clamp(0.0, 255.0) * alpha) as u8;
            let final_a = (alpha * 255.0) as u8;

            img.put_pixel(x, y, Rgba([final_r, final_g, final_b, final_a]));
        }
    }

    let out_dir = Path::new("bundle");
    std::fs::create_dir_all(out_dir).expect("failed to create bundle dir");
    let out_path = out_dir.join("icon_1024.png");
    img.save(&out_path).expect("failed to save icon_1024.png");
    println!("Successfully generated master icon at {}", out_path.display());
}
