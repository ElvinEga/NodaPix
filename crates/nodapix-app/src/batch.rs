//! Batch operations and file utilities for NodaPix.
//! Includes Batch Format Conversion and Pattern-based Series Renaming.

use nodapix_imaging::ExportFormat;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseTransform {
    Preserve,
    Lowercase,
    Uppercase,
}

impl CaseTransform {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preserve => "Preserve Case",
            Self::Lowercase => "lowercase",
            Self::Uppercase => "UPPERCASE",
        }
    }

    pub fn apply(self, text: &str) -> String {
        match self {
            Self::Preserve => text.to_string(),
            Self::Lowercase => text.to_lowercase(),
            Self::Uppercase => text.to_uppercase(),
        }
    }
}

/// Evaluates a renaming template string against an individual file.
///
/// Supported tokens:
/// - `#` runs (e.g. `###` -> `001`, `####` -> `0001`, or a single `#` -> `1`)
/// - `%0Nd` (e.g. `%03d` -> `001`, `%04d` -> `0001`)
/// - `%F` or `%f`: Original filename without extension
/// - `%E` or `%e`: Original file extension
/// - `%Y`: 4-digit year (from EXIF or current time)
/// - `%m`: 2-digit month
/// - `%d`: 2-digit day
pub fn evaluate_rename_template(
    template: &str,
    number: usize,
    original_path: &Path,
    exif_date: Option<&str>,
    case_mode: CaseTransform,
) -> String {
    let stem = original_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let ext = original_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    // Date components: extract from EXIF string "YYYY:MM:DD HH:MM:SS" or fallback
    let (year, month, day) = if let Some(d) = exif_date {
        let parts: Vec<&str> = d.split(|c| c == ':' || c == '-' || c == ' ').collect();
        if parts.len() >= 3 {
            (parts[0], parts[1], parts[2])
        } else {
            ("2026", "01", "01")
        }
    } else {
        ("2026", "01", "01")
    };

    let mut result = template.to_string();

    // Replace %F or %f with stem
    result = result.replace("%F", stem).replace("%f", stem);

    // Replace %E or %e with ext
    result = result.replace("%E", ext).replace("%e", ext);

    // Replace date tokens
    result = result.replace("%Y", year);
    result = result.replace("%m", month);
    result = result.replace("%d", day);

    // Replace printf-style %0Nd (e.g., %03d, %04d)
    let re_printf = regex_printf(&result, number);
    result = re_printf;

    // Replace hash sequences like "###", "####", "#"
    let re_hashes = replace_hashes(&result, number);
    result = re_hashes;

    // Apply casing
    let transformed_name = case_mode.apply(&result);

    // Ensure extension is retained if template doesn't specify one
    if !ext.is_empty() && !transformed_name.contains('.') {
        format!("{}.{}", transformed_name, case_mode.apply(ext))
    } else {
        transformed_name
    }
}

/// Helper to replace sequences of '#' with zero-padded number.
fn replace_hashes(text: &str, number: usize) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '#' {
            let mut count = 1;
            while let Some(&'#') = chars.peek() {
                chars.next();
                count += 1;
            }
            let formatted = format!("{:0width$}", number, width = count);
            out.push_str(&formatted);
        } else {
            out.push(ch);
        }
    }
    out
}

/// Helper to replace `%0Nd` or `%d` patterns
fn regex_printf(text: &str, number: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'd' {
                out.push_str(&number.to_string());
                i += 2;
                continue;
            }
            if bytes[i + 1] == b'0' && i + 3 < bytes.len() && bytes[i + 2].is_ascii_digit() && bytes[i + 3] == b'd' {
                let width = (bytes[i + 2] - b'0') as usize;
                out.push_str(&format!("{:0width$}", number, width = width));
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchConvertSettings {
    pub is_open: bool,
    pub format: ExportFormat,
    pub quality: u8,
    pub destination_folder: Option<PathBuf>,
    pub is_running: bool,
    pub current_index: usize,
    pub total_count: usize,
    pub current_filename: String,
    pub status_message: String,
    pub completed: bool,
}

impl Default for BatchConvertSettings {
    fn default() -> Self {
        Self {
            is_open: false,
            format: ExportFormat::Jpeg,
            quality: 90,
            destination_folder: None,
            is_running: false,
            current_index: 0,
            total_count: 0,
            current_filename: String::new(),
            status_message: String::new(),
            completed: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchRenameSettings {
    pub is_open: bool,
    pub template: String,
    pub start_at: usize,
    pub case_mode: CaseTransform,
    pub is_running: bool,
    pub status_message: String,
    pub completed: bool,
}

impl Default for BatchRenameSettings {
    fn default() -> Self {
        Self {
            is_open: false,
            template: "%F_###".to_string(),
            start_at: 1,
            case_mode: CaseTransform::Preserve,
            is_running: false,
            status_message: String::new(),
            completed: false,
        }
    }
}

// ---------------------------------------------------------------- EXIF Date Adjuster

/// Days from civil date algorithm (Howard Hinnant)
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era as i64 * 146097 + doe as i64 - 719468
}

/// Civil date from days since Unix epoch
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64 + era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

/// Parses an EXIF date string `"YYYY:MM:DD HH:MM:SS"` or `"YYYY-MM-DD HH:MM:SS"`.
pub fn parse_exif_date_string(s: &str) -> Option<(i32, u32, u32, u32, u32, u32)> {
    let parts: Vec<&str> = s.split(|c| c == ':' || c == '-' || c == ' ').collect();
    if parts.len() < 6 {
        return None;
    }
    let y: i32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    let h: u32 = parts[3].parse().ok()?;
    let min: u32 = parts[4].parse().ok()?;
    let sec: u32 = parts[5].parse().ok()?;
    Some((y, m, d, h, min, sec))
}

/// Formats civil date and time back into standard EXIF format `"YYYY:MM:DD HH:MM:SS"`.
pub fn format_exif_date_string(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> String {
    format!("{:04}:{:02}:{:02} {:02}:{:02}:{:02}", y, m, d, h, min, s)
}

/// Offsets an EXIF date string by a signed number of seconds (e.g. +3600 for +1h, -7200 for -2h).
pub fn adjust_date_string(date_str: &str, offset_seconds: i64) -> Option<String> {
    let (y, m, d, h, min, sec) = parse_exif_date_string(date_str)?;
    let days = days_from_civil(y, m, d);
    let day_seconds = (h as i64 * 3600) + (min as i64 * 60) + (sec as i64);
    let total_seconds = days * 86400 + day_seconds + offset_seconds;

    let new_days = total_seconds.div_euclid(86400);
    let rem_seconds = total_seconds.rem_euclid(86400);

    let (new_y, new_m, new_d) = civil_from_days(new_days);
    let new_h = (rem_seconds / 3600) as u32;
    let new_min = ((rem_seconds % 3600) / 60) as u32;
    let new_sec = (rem_seconds % 60) as u32;

    Some(format_exif_date_string(new_y, new_m, new_d, new_h, new_min, new_sec))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateAdjustMode {
    Offset,
    SetSpecific,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchDateSettings {
    pub is_open: bool,
    pub mode: DateAdjustMode,
    pub offset_hours: i32,
    pub offset_minutes: i32,
    pub specific_date: String,
    pub touch_mtime: bool,
    pub is_running: bool,
    pub status_message: String,
    pub completed: bool,
}

impl Default for BatchDateSettings {
    fn default() -> Self {
        Self {
            is_open: false,
            mode: DateAdjustMode::Offset,
            offset_hours: 0,
            offset_minutes: 0,
            specific_date: "2026:01:01 12:00:00".to_string(),
            touch_mtime: true,
            is_running: false,
            status_message: String::new(),
            completed: false,
        }
    }
}

// ---------------------------------------------------------------- Duplicate Finder

/// 64-bit Perceptual Difference Hash (dHash) computed on a downsampled 9x8 luminance grid.
pub fn compute_dhash(buf: &nodapix_imaging::PixelBuffer<f32>) -> u64 {
    if buf.width == 0 || buf.height == 0 {
        return 0;
    }
    // Downsample to 9 columns x 8 rows grayscale
    let mut grid = [0.0f32; 9 * 8];
    let cell_w = buf.width as f32 / 9.0;
    let cell_h = buf.height as f32 / 8.0;

    for gy in 0..8 {
        for gx in 0..9 {
            let start_x = (gx as f32 * cell_w) as u32;
            let end_x = (((gx + 1) as f32 * cell_w) as u32).min(buf.width);
            let start_y = (gy as f32 * cell_h) as u32;
            let end_y = (((gy + 1) as f32 * cell_h) as u32).min(buf.height);

            let mut sum = 0.0f32;
            let mut count = 0;
            for y in start_y..end_y {
                for x in start_x..end_x {
                    let px = buf.get_pixel(x, y);
                    let lum = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
                    sum += lum;
                    count += 1;
                }
            }
            grid[gy * 9 + gx] = if count > 0 { sum / count as f32 } else { 0.0 };
        }
    }

    // Compare each adjacent column: row by row, 8 bits per row
    let mut hash = 0u64;
    for gy in 0..8 {
        for gx in 0..8 {
            let left = grid[gy * 9 + gx];
            let right = grid[gy * 9 + gx + 1];
            if left > right {
                hash |= 1 << (gy * 8 + gx);
            }
        }
    }
    hash
}

/// Hamming distance between two 64-bit hashes.
pub fn hamming_distance(h1: u64, h2: u64) -> u32 {
    (h1 ^ h2).count_ones()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateMatchMode {
    Visual, // dHash distance <= 5
    Exact,  // Identical file size + byte checksum
}

impl DuplicateMatchMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Visual => "Visual Match (Perceptual dHash)",
            Self::Exact => "Exact Match (Identical Bytes)",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DuplicateFileItem {
    pub path: PathBuf,
    pub filename: String,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub dhash: u64,
    pub selected_for_removal: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DuplicateGroup {
    pub group_id: usize,
    pub items: Vec<DuplicateFileItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DuplicateFinderSettings {
    pub is_open: bool,
    pub match_mode: DuplicateMatchMode,
    pub is_scanning: bool,
    pub progress_msg: String,
    pub groups: Vec<DuplicateGroup>,
    pub status_message: String,
}

impl Default for DuplicateFinderSettings {
    fn default() -> Self {
        Self {
            is_open: false,
            match_mode: DuplicateMatchMode::Visual,
            is_scanning: false,
            progress_msg: String::new(),
            groups: Vec::new(),
            status_message: "Scan folder for duplicate photos".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_hash_replacement() {
        let p = Path::new("vacation.jpg");
        let name = evaluate_rename_template("Trip_###", 5, p, None, CaseTransform::Preserve);
        assert_eq!(name, "Trip_005.jpg");

        let name4 = evaluate_rename_template("Photo_####", 42, p, None, CaseTransform::Preserve);
        assert_eq!(name4, "Photo_0042.jpg");

        let name1 = evaluate_rename_template("Pic_#", 7, p, None, CaseTransform::Preserve);
        assert_eq!(name1, "Pic_7.jpg");
    }

    #[test]
    fn test_printf_token_replacement() {
        let p = Path::new("sunset.png");
        let name = evaluate_rename_template("Sun_%03d", 12, p, None, CaseTransform::Preserve);
        assert_eq!(name, "Sun_012.png");
    }

    #[test]
    fn test_original_filename_and_ext() {
        let p = Path::new("Mountain_View.TIFF");
        let name = evaluate_rename_template("%F_edit_###.%E", 1, p, None, CaseTransform::Preserve);
        assert_eq!(name, "Mountain_View_edit_001.TIFF");

        let name_lower = evaluate_rename_template("%F_###", 1, p, None, CaseTransform::Lowercase);
        assert_eq!(name_lower, "mountain_view_001.tiff");

        let name_upper = evaluate_rename_template("%F_###", 1, p, None, CaseTransform::Uppercase);
        assert_eq!(name_upper, "MOUNTAIN_VIEW_001.TIFF");
    }

    #[test]
    fn test_date_tokens() {
        let p = Path::new("scan.jpg");
        let name = evaluate_rename_template("%Y-%m-%d_%F_###", 2, p, Some("2024:08:25 14:30:00"), CaseTransform::Preserve);
        assert_eq!(name, "2024-08-25_scan_002.jpg");
    }

    #[test]
    fn test_exif_date_offset() {
        let original = "2024:08:15 14:30:00";
        // +1 hour
        let plus_1h = adjust_date_string(original, 3600).unwrap();
        assert_eq!(plus_1h, "2024:08:15 15:30:00");

        // -2 hours
        let minus_2h = adjust_date_string(original, -7200).unwrap();
        assert_eq!(minus_2h, "2024:08:15 12:30:00");

        // Across midnight
        let near_midnight = "2024:08:15 23:45:00";
        let next_day = adjust_date_string(near_midnight, 1800).unwrap();
        assert_eq!(next_day, "2024:08:16 00:15:00");

        // Across leap year day
        let leap_eve = "2024:02:28 23:00:00";
        let leap_day = adjust_date_string(leap_eve, 7200).unwrap();
        assert_eq!(leap_day, "2024:02:29 01:00:00");
    }

    #[test]
    fn test_dhash_and_hamming_distance() {
        let mut buf1 = nodapix_imaging::PixelBuffer::<f32>::new(50, 50);
        for y in 0..50 {
            for x in 0..50 {
                // Gradient pattern
                let val = (x as f32) / 50.0;
                buf1.set_pixel(x, y, [val, val, val, 1.0]);
            }
        }

        let mut buf2 = nodapix_imaging::PixelBuffer::<f32>::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                // Same gradient pattern, different resolution
                let val = (x as f32) / 100.0;
                buf2.set_pixel(x, y, [val, val, val, 1.0]);
            }
        }

        let h1 = compute_dhash(&buf1);
        let h2 = compute_dhash(&buf2);
        let dist = hamming_distance(h1, h2);
        assert_eq!(dist, 0, "Identical pattern at different resolutions must have dHash distance 0");

        let mut buf3 = nodapix_imaging::PixelBuffer::<f32>::new(50, 50);
        for y in 0..50 {
            for x in 0..50 {
                // Inverted gradient pattern
                let val = 1.0 - (x as f32) / 50.0;
                buf3.set_pixel(x, y, [val, val, val, 1.0]);
            }
        }
        let h3 = compute_dhash(&buf3);
        let dist_diff = hamming_distance(h1, h3);
        assert!(dist_diff >= 30, "Opposite gradient should have very large hamming distance");
    }
}
