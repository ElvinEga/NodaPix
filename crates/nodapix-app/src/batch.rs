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
}
