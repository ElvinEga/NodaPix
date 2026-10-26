//! Image EXIF metadata inspection using `kamadak-exif`.

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageMetadata {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub f_number: Option<String>,
    pub exposure_time: Option<String>,
    pub iso: Option<String>,
    pub focal_length: Option<String>,
    pub date_time: Option<String>,
}

impl ImageMetadata {
    pub fn read_from_path(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let mut reader = BufReader::new(file);
        let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;

        let get_field = |tag: exif::Tag| {
            exif.get_field(tag, exif::In::PRIMARY)
                .map(|f| f.display_value().to_string().trim_matches('"').to_string())
        };

        Some(Self {
            make: get_field(exif::Tag::Make),
            model: get_field(exif::Tag::Model),
            lens: get_field(exif::Tag::LensModel),
            f_number: get_field(exif::Tag::FNumber),
            exposure_time: get_field(exif::Tag::ExposureTime),
            iso: get_field(exif::Tag::PhotographicSensitivity)
                .or_else(|| get_field(exif::Tag::ISOSpeed)),
            focal_length: get_field(exif::Tag::FocalLength),
            date_time: get_field(exif::Tag::DateTimeOriginal)
                .or_else(|| get_field(exif::Tag::DateTime)),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.make.is_none()
            && self.model.is_none()
            && self.lens.is_none()
            && self.f_number.is_none()
            && self.exposure_time.is_none()
            && self.iso.is_none()
            && self.focal_length.is_none()
            && self.date_time.is_none()
    }
}
