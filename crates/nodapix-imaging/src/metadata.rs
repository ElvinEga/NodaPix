//! Image EXIF metadata inspection and preservation using `kamadak-exif` plus raw
//! APP1/eXIf block handling.

use crate::backend::ExportFormat;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// EXIF lives in the first megabyte of every sane JPEG/PNG.
const EXIF_SCAN_LIMIT: u64 = 1 << 20;
const EXIF_HEADER: &[u8] = b"Exif\0\0";
const ORIENTATION_TAG: u16 = 0x0112;
const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

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
    pub color_space: Option<String>,
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

        let cs_str = get_field(exif::Tag::ColorSpace).map(|val| match val.as_str() {
            "1" | "sRGB" => "sRGB".to_string(),
            "2" | "Adobe RGB" => "Adobe RGB (1998)".to_string(),
            "65535" | "Uncalibrated" => "Wide Gamut / Uncalibrated".to_string(),
            other => other.to_string(),
        });

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
            color_space: cs_str,
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
            && self.color_space.is_none()
    }

    /// The camera's own EXIF block, byte for byte: `Exif\0\0` followed by the TIFF
    /// structure. Copying the block preserves every tag, not just the ones the
    /// inspector shows.
    pub fn read_exif_block(bytes: &[u8]) -> Option<Vec<u8>> {
        if bytes.starts_with(&[0xFF, 0xD8]) {
            return jpeg_app1(bytes);
        }
        if bytes.starts_with(PNG_SIGNATURE) {
            return png_exif_chunk(bytes);
        }
        None
    }

    /// Same, without slurping a whole RAW file off disk first.
    pub fn read_exif_block_from_path(path: &Path) -> Option<Vec<u8>> {
        let file = File::open(path).ok()?;
        let size = file.metadata().ok()?.len().min(EXIF_SCAN_LIMIT);
        let mut head = Vec::with_capacity(size as usize);
        file.take(EXIF_SCAN_LIMIT).read_to_end(&mut head).ok()?;
        Self::read_exif_block(&head)
    }

    /// The display orientation the file asks for, 1 when the tag is absent.
    pub fn orientation(block: &[u8]) -> u32 {
        let Some(ifd) = Ifd0::parse(block) else {
            return 1;
        };
        let Some(entry) = ifd.find(ORIENTATION_TAG) else {
            return 1;
        };
        match ifd.u16(entry + 2) {
            Some(3) => ifd.u16(entry + 8).unwrap_or(1) as u32,
            _ => 1,
        }
    }

    /// Rewrite the orientation tag, used when the exported pixels no longer need
    /// the rotation the source file asked for.
    pub fn set_orientation(block: &mut Vec<u8>, value: u32) {
        let Some(ifd) = Ifd0::parse(block).and_then(|ifd| {
            ifd.find(ORIENTATION_TAG)
                .map(|entry| (ifd, entry))
        }) else {
            return;
        };
        let (ifd, entry) = ifd;
        if ifd.u16(entry + 2) != Some(3) || ifd.u32(entry + 4) != Some(1) {
            return;
        }
        let offset = EXIF_HEADER.len() + entry + 8;
        let bytes = (value as u16).to_ne_bytes();
        let (lo, hi) = if ifd.little { (bytes[0], bytes[1]) } else { (bytes[1], bytes[0]) };
        // A zero count would make the tag meaningless, so only touch the value field.
        if let Some(slot) = block.get_mut(offset..offset + 2) {
            slot[0] = lo;
            slot[1] = hi;
        }
    }
}

/// Re-embed an EXIF block into encoded output. JPEG and PNG carry it in stream;
/// WebP and TIFF need container-level surgery and are returned untouched.
pub fn attach_exif(encoded: &[u8], format: ExportFormat, block: &[u8]) -> Vec<u8> {
    let tiff = block.strip_prefix(EXIF_HEADER).unwrap_or(block);
    match format {
        ExportFormat::Jpeg if encoded.starts_with(&[0xFF, 0xD8]) => {
            let payload = EXIF_HEADER.len() + tiff.len();
            let Some(length) = u16::try_from(payload + 2).ok() else {
                return encoded.to_vec();
            };
            let mut out = Vec::with_capacity(encoded.len() + payload + 4);
            out.extend_from_slice(&encoded[..2]);
            out.extend_from_slice(&[0xFF, 0xE1]);
            out.extend_from_slice(&length.to_be_bytes());
            out.extend_from_slice(EXIF_HEADER);
            out.extend_from_slice(tiff);
            out.extend_from_slice(&encoded[2..]);
            out
        }
        ExportFormat::Png if encoded.starts_with(PNG_SIGNATURE) => {
            // IHDR is always the first chunk and always 13 bytes.
            let insert_at = match PNG_SIGNATURE.len() + 12 + 13 {
                i if encoded.len() >= i => i,
                _ => return encoded.to_vec(),
            };
            let mut out = Vec::with_capacity(encoded.len() + tiff.len() + 12);
            out.extend_from_slice(&encoded[..insert_at]);
            out.extend_from_slice(&(tiff.len() as u32).to_be_bytes());
            out.extend_from_slice(b"eXIf");
            out.extend_from_slice(tiff);
            let mut crc_input = b"eXIf".to_vec();
            crc_input.extend_from_slice(tiff);
            out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
            out.extend_from_slice(&encoded[insert_at..]);
            out
        }
        _ => encoded.to_vec(),
    }
}

fn jpeg_app1(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        // Standalone markers carry no length; SOS ends the segment run.
        if marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
            i += 2;
            continue;
        }
        if marker == 0xDA {
            break;
        }
        let length = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let end = i + 2 + length;
        if length < 2 || end > bytes.len() {
            break;
        }
        let segment = &bytes[i + 4..end];
        if marker == 0xE1 && segment.starts_with(EXIF_HEADER) {
            return Some(segment.to_vec());
        }
        i = end;
    }
    None
}

fn png_exif_chunk(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut i = PNG_SIGNATURE.len();
    while i + 12 <= bytes.len() {
        let length = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let kind = &bytes[i + 4..i + 8];
        let end = i + 12 + length;
        if end > bytes.len() {
            break;
        }
        if kind == b"eXIf" {
            let mut block = EXIF_HEADER.to_vec();
            block.extend_from_slice(&bytes[i + 8..i + 8 + length]);
            return Some(block);
        }
        if kind == b"IDAT" {
            break;
        }
        i = end;
    }
    None
}

/// Minimal reader for the first IFD of an `Exif\0\0` block.
struct Ifd0<'a> {
    tiff: &'a [u8],
    little: bool,
}

impl<'a> Ifd0<'a> {
    fn parse(block: &'a [u8]) -> Option<Self> {
        let tiff = block.get(EXIF_HEADER.len()..)?;
        let little = match tiff.first()? {
            b'I' => true,
            b'M' => false,
            _ => return None,
        };
        Some(Self { tiff, little })
    }

    fn u16(&self, i: usize) -> Option<u16> {
        let b = self.tiff.get(i..i + 2)?;
        Some(if self.little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }

    fn u32(&self, i: usize) -> Option<u32> {
        let b = self.tiff.get(i..i + 4)?;
        Some(if self.little {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    }

    /// Offset of the 12-byte directory entry for `tag`, relative to the TIFF block.
    fn find(&self, tag: u16) -> Option<usize> {
        let ifd = self.u32(4)? as usize;
        let count = self.u16(ifd)? as usize;
        (0..count).find_map(|n| {
            let entry = ifd + 2 + n * 12;
            (self.u16(entry)? == tag).then_some(entry)
        })
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (!(crc & 1)).wrapping_add(1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
