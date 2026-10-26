//! macOS native printing and page layout preview engine.
//! Ports Pix's image_print extension with paper sizes, scaling, margins, orientation,
//! and integration with macOS Cocoa/Preview system print dialog.

use nodapix_imaging::contact_sheet::blit_buffer;
use nodapix_imaging::transform::resize;
use nodapix_imaging::{ExportFormat, ExportOptions, ImageBackend, NativeBackend, PixelBuffer};
use std::path::PathBuf;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperSize {
    Letter,    // 8.5 x 11.0 in
    A4,        // 8.27 x 11.69 in (210 x 297 mm)
    Photo4x6,  // 4.0 x 6.0 in
    Photo5x7,  // 5.0 x 7.0 in
    Photo8x10, // 8.0 x 10.0 in
}

impl PaperSize {
    pub fn label(self) -> &'static str {
        match self {
            Self::Letter => "US Letter (8.5×11 in)",
            Self::A4 => "A4 (210×297 mm)",
            Self::Photo4x6 => "Photo 4×6 in (10×15 cm)",
            Self::Photo5x7 => "Photo 5×7 in (13×18 cm)",
            Self::Photo8x10 => "Photo 8×10 in (20×25 cm)",
        }
    }

    /// Dimensions in pixels at 300 DPI.
    pub fn pixel_dimensions_300dpi(self) -> (u32, u32) {
        match self {
            Self::Letter => (2550, 3300),
            Self::A4 => (2480, 3508),
            Self::Photo4x6 => (1200, 1800),
            Self::Photo5x7 => (1500, 2100),
            Self::Photo8x10 => (2400, 3000),
        }
    }

    /// Aspect ratio (width / height) in portrait.
    pub fn aspect_ratio(self) -> f32 {
        let (w, h) = self.pixel_dimensions_300dpi();
        w as f32 / h as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintOrientation {
    Auto,
    Portrait,
    Landscape,
}

impl PrintOrientation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Portrait => "Portrait",
            Self::Landscape => "Landscape",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintScaleMode {
    FitToPage,
    FillPage,
    OriginalSize,
}

impl PrintScaleMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::FitToPage => "Fit to Page",
            Self::FillPage => "Fill Page",
            Self::OriginalSize => "100% Size",
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintMargin {
    None,   // 0 mm
    Small,  // 0.25 in (75 px at 300 dpi)
    Normal, // 0.5 in (150 px at 300 dpi)
    Wide,   // 1.0 in (300 px at 300 dpi)
}

impl PrintMargin {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None (0\")",
            Self::Small => "Small (0.25\")",
            Self::Normal => "Normal (0.5\")",
            Self::Wide => "Wide (1.0\")",
        }
    }

    pub fn pixels_300dpi(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Small => 75,
            Self::Normal => 150,
            Self::Wide => 300,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PrintSettings {
    pub paper_size: PaperSize,
    pub orientation: PrintOrientation,
    pub scale_mode: PrintScaleMode,
    pub margin: PrintMargin,
    pub include_edits: bool,
    pub copies: u32,
}

#[derive(Clone, Debug)]
pub struct PrintModalSettings {
    pub is_open: bool,
    pub settings: PrintSettings,
    pub is_printing: bool,
    pub status_message: String,
}

impl Default for PrintModalSettings {
    fn default() -> Self {
        Self {
            is_open: false,
            settings: PrintSettings::default(),
            is_printing: false,
            status_message: String::new(),
        }
    }
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            paper_size: PaperSize::Letter,
            orientation: PrintOrientation::Auto,
            scale_mode: PrintScaleMode::FitToPage,
            margin: PrintMargin::Normal,
            include_edits: true,
            copies: 1,
        }
    }
}

/// Calculate page dimensions taking orientation into account.
pub fn calculate_effective_dimensions(
    settings: &PrintSettings,
    img_width: u32,
    img_height: u32,
) -> (u32, u32) {
    let (mut pw, mut ph) = settings.paper_size.pixel_dimensions_300dpi();
    let is_landscape = match settings.orientation {
        PrintOrientation::Auto => img_width > img_height,
        PrintOrientation::Portrait => false,
        PrintOrientation::Landscape => true,
    };
    if is_landscape && pw < ph {
        std::mem::swap(&mut pw, &mut ph);
    } else if !is_landscape && pw > ph {
        std::mem::swap(&mut pw, &mut ph);
    }
    (pw, ph)
}

/// Render the image onto a printable virtual page buffer at 300 DPI.
pub fn render_print_page(
    image: &PixelBuffer<f32>,
    settings: &PrintSettings,
) -> PixelBuffer<f32> {
    let (page_w, page_h) = calculate_effective_dimensions(settings, image.width, image.height);
    let mut page = PixelBuffer::<f32>::new(page_w, page_h);

    // Fill page white
    for y in 0..page_h {
        for x in 0..page_w {
            page.set_pixel(x, y, [1.0, 1.0, 1.0, 1.0]);
        }
    }

    let margin_px = settings.margin.pixels_300dpi();
    let avail_w = page_w.saturating_sub(margin_px * 2);
    let avail_h = page_h.saturating_sub(margin_px * 2);

    if avail_w == 0 || avail_h == 0 {
        return page;
    }

    match settings.scale_mode {
        PrintScaleMode::FitToPage => {
            let scale_w = avail_w as f32 / image.width as f32;
            let scale_h = avail_h as f32 / image.height as f32;
            let scale = scale_w.min(scale_h);

            let target_w = ((image.width as f32 * scale).round() as u32).max(1);
            let target_h = ((image.height as f32 * scale).round() as u32).max(1);

            let resized = resize(image, target_w, target_h);

            let dst_x = margin_px + (avail_w.saturating_sub(target_w)) / 2;
            let dst_y = margin_px + (avail_h.saturating_sub(target_h)) / 2;

            blit_buffer(&mut page, &resized, dst_x as usize, dst_y as usize);
        }
        PrintScaleMode::FillPage => {
            let scale_w = avail_w as f32 / image.width as f32;
            let scale_h = avail_h as f32 / image.height as f32;
            let scale = scale_w.max(scale_h);

            let target_w = ((image.width as f32 * scale).round() as u32).max(1);
            let target_h = ((image.height as f32 * scale).round() as u32).max(1);

            let resized = resize(image, target_w, target_h);

            // Center and crop within printable area
            let offset_x = (target_w.saturating_sub(avail_w)) / 2;
            let offset_y = (target_h.saturating_sub(avail_h)) / 2;

            for y in 0..avail_h {
                let sy = offset_y + y;
                if sy >= resized.height {
                    break;
                }
                for x in 0..avail_w {
                    let sx = offset_x + x;
                    if sx >= resized.width {
                        break;
                    }
                    let px = resized.get_pixel(sx, sy);
                    page.set_pixel(margin_px + x, margin_px + y, px);
                }
            }
        }
        PrintScaleMode::OriginalSize => {
            // 1:1 image pixels centered
            let target_w = image.width.min(avail_w);
            let target_h = image.height.min(avail_h);

            let dst_x = margin_px + (avail_w.saturating_sub(target_w)) / 2;
            let dst_y = margin_px + (avail_h.saturating_sub(target_h)) / 2;

            blit_buffer(&mut page, image, dst_x as usize, dst_y as usize);
        }
    }

    page
}

/// Invokes macOS native printing dialog for a rendered page or image.
pub fn launch_macos_print_system(
    page: &PixelBuffer<f32>,
) -> Result<PathBuf, String> {
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!("nodapix_print_{}.png", std::process::id()));

    let backend = NativeBackend::default();
    let options = ExportOptions {
        format: ExportFormat::Png,
        quality: 100,
    };

    backend
        .encode_to_path(page, &temp_file, &options)
        .map_err(|e| format!("Failed to prepare printable image: {}", e))?;

    // Invoke macOS Preview print dialog via osascript
    let script = format!(
        "tell application \"Preview\"\nactivate\nopen POSIX file \"{}\"\nprint POSIX file \"{}\" with print dialog\nend tell",
        temp_file.display(),
        temp_file.display()
    );

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output();

    match output {
        Ok(out) if out.status.success() => Ok(temp_file),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Fallback to simple open
            let _ = std::process::Command::new("open")
                .arg(&temp_file)
                .spawn();
            Err(format!("AppleScript print dialog warning: {}", stderr))
        }
        Err(e) => {
            let _ = std::process::Command::new("open")
                .arg(&temp_file)
                .spawn();
            Err(format!("Failed to invoke osascript: {}", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_page_dimensions_and_orientation() {
        let mut settings = PrintSettings::default();
        settings.paper_size = PaperSize::Letter;
        settings.orientation = PrintOrientation::Auto;

        // Landscape image -> Landscape paper
        let (pw1, ph1) = calculate_effective_dimensions(&settings, 800, 600);
        assert_eq!(pw1, 3300);
        assert_eq!(ph1, 2550);

        // Portrait image -> Portrait paper
        let (pw2, ph2) = calculate_effective_dimensions(&settings, 600, 800);
        assert_eq!(pw2, 2550);
        assert_eq!(ph2, 3300);
    }

    #[test]
    fn test_render_print_page_fit() {
        let mut img = PixelBuffer::<f32>::new(200, 100);
        for y in 0..100 {
            for x in 0..200 {
                img.set_pixel(x, y, [0.0, 1.0, 0.0, 1.0]); // green
            }
        }

        let mut settings = PrintSettings::default();
        settings.paper_size = PaperSize::Photo4x6;
        settings.scale_mode = PrintScaleMode::FitToPage;
        settings.margin = PrintMargin::Small;

        let page = render_print_page(&img, &settings);
        assert!(page.width > 0 && page.height > 0);

        // Should have white borders and green photo inside
        let corner_px = page.get_pixel(0, 0);
        assert_eq!(corner_px, [1.0, 1.0, 1.0, 1.0]);

        let center_px = page.get_pixel(page.width / 2, page.height / 2);
        assert!(center_px[1] > 0.9, "Center pixel should be green");
    }
}
