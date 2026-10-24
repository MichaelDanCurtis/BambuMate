//! The photos a user adds to a journal entry. Each is re-encoded to a JPEG
//! no larger than 2048 px on its long edge, which also drops whatever
//! metadata the original carried. Refusals are plain sentences.

use std::io::Cursor;
use std::path::Path;

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

/// Largest photo accepted, before re-encoding.
pub const MAX_PHOTO_BYTES: usize = 25 * 1024 * 1024;
/// Longest edge of a stored photo, in pixels.
pub const MAX_EDGE: u32 = 2048;
/// Longest edge of an upload that is decoded at all. A file this small on
/// disk can still claim a vast image; anything wider or taller is refused
/// from its header, before any pixel is decoded.
const MAX_DECODE_SIDE: u32 = 16_384;
/// Most pixels an upload may hold (100 MP covers every phone camera mode).
const MAX_DECODE_PIXELS: u64 = 100_000_000;
/// Most memory the decoder may allocate.
const MAX_DECODE_ALLOC: u64 = 768 * 1024 * 1024;
const JPEG_QUALITY: u8 = 85;
/// Largest stored file `image_data_url` reads.
const MAX_READ_BYTES: u64 = 32 * 1024 * 1024;

pub const TOO_LARGE: &str = "That photo is over 25 MB. Choose a smaller one.";
pub const NOT_AN_IMAGE: &str =
    "That file isn't a photo BambuMate can read. Choose a JPEG, PNG or WebP image.";

/// The photo bytes from the page's base64, refused over 25 MB.
pub fn decode_upload(data_base64: &str) -> Result<Vec<u8>, &'static str> {
    decode_with_limit(data_base64, MAX_PHOTO_BYTES)
}

fn decode_with_limit(data_base64: &str, max: usize) -> Result<Vec<u8>, &'static str> {
    let data = data_base64.trim();
    // Base64 is 4/3 the size: anything longer can't fit, so don't decode it.
    if data.len() > max.div_ceil(3) * 4 {
        return Err(TOO_LARGE);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| NOT_AN_IMAGE)?;
    if bytes.len() > max {
        return Err(TOO_LARGE);
    }
    Ok(bytes)
}

/// Decodes a JPEG, PNG or WebP and re-encodes it as an opaque JPEG with a
/// long edge of at most [`MAX_EDGE`], upright, without the original's
/// metadata. Anything else, and any image whose header claims more than
/// [`MAX_DECODE_PIXELS`], is refused.
pub fn prepare_photo(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    prepare_with_budget(bytes, MAX_DECODE_PIXELS)
}

fn prepare_with_budget(bytes: &[u8], max_pixels: u64) -> Result<Vec<u8>, &'static str> {
    let format = match image::guess_format(bytes) {
        Ok(f @ (ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP)) => f,
        _ => return Err(NOT_AN_IMAGE),
    };
    let mut limits = Limits::no_limits();
    limits.max_image_width = Some(MAX_DECODE_SIDE);
    limits.max_image_height = Some(MAX_DECODE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| NOT_AN_IMAGE)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(NOT_AN_IMAGE);
    }
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|_| NOT_AN_IMAGE)?;
    if let Some(orientation) = orientation {
        img.apply_orientation(orientation);
    }
    let img = if img.width().max(img.height()) > MAX_EDGE {
        img.resize(MAX_EDGE, MAX_EDGE, FilterType::Lanczos3)
    } else {
        img
    };
    let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
    let mut out = Cursor::new(Vec::new());
    rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))
        .map_err(|e| {
            tracing::warn!("journal photo re-encode failed: {e}");
            NOT_AN_IMAGE
        })?;
    Ok(out.into_inner())
}

/// A stored JPEG or PNG as a `data:` URL; `None` when the file is missing,
/// too large or not one of those.
pub fn image_data_url(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_READ_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let mime = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else {
        return None;
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use base64::Engine;
    use image::{DynamicImage, ImageFormat};

    use super::*;

    fn encoded(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(width, height)
            .write_to(&mut out, format)
            .unwrap();
        out.into_inner()
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        encoded(width, height, ImageFormat::Png)
    }

    #[test]
    fn a_large_photo_becomes_a_jpeg_with_a_2048_px_long_edge() {
        let jpeg = prepare_photo(&png(3000, 1000)).unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
        let back = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(back.width(), MAX_EDGE);
        assert!((682..=683).contains(&back.height()), "{}", back.height());
    }

    #[test]
    fn a_small_photo_keeps_its_size() {
        let back = image::load_from_memory(&prepare_photo(&png(640, 480)).unwrap()).unwrap();
        assert_eq!((back.width(), back.height()), (640, 480));
    }

    #[test]
    fn a_transparent_png_becomes_an_opaque_jpeg() {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(300, 200)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        assert!(prepare_photo(&out.into_inner()).is_ok());
    }

    #[test]
    fn jpeg_and_webp_photos_are_accepted_too() {
        let jpeg = encoded(64, 48, ImageFormat::Jpeg);
        assert!(prepare_photo(&jpeg).is_ok());
        let webp = encoded(64, 48, ImageFormat::WebP);
        assert!(prepare_photo(&webp).is_ok());
    }

    #[test]
    fn non_images_are_refused_with_a_plain_sentence() {
        assert_eq!(prepare_photo(b"not an image"), Err(NOT_AN_IMAGE));
        assert_eq!(decode_upload("@@@ not base64"), Err(NOT_AN_IMAGE));
    }

    #[test]
    fn images_that_are_not_jpeg_png_or_webp_are_refused() {
        assert_eq!(
            prepare_photo(&encoded(16, 16, ImageFormat::Gif)),
            Err(NOT_AN_IMAGE)
        );
        assert_eq!(
            prepare_photo(&encoded(16, 16, ImageFormat::Bmp)),
            Err(NOT_AN_IMAGE)
        );
    }

    #[test]
    fn a_tiny_file_cannot_expand_into_a_huge_image() {
        // 16 385 px wide is over the side limit; the file itself is tiny.
        let wide = png(MAX_DECODE_SIDE + 1, 2);
        assert!(wide.len() < 10_000);
        assert_eq!(prepare_photo(&wide), Err(NOT_AN_IMAGE));
        // The pixel budget is checked from the header, before decoding.
        assert_eq!(
            prepare_with_budget(&png(400, 300), 100_000),
            Err(NOT_AN_IMAGE)
        );
        assert!(prepare_with_budget(&png(400, 300), 120_000).is_ok());
    }

    #[test]
    fn uploads_over_the_cap_are_refused_before_and_after_decoding() {
        let b64 = base64::engine::general_purpose::STANDARD;
        assert_eq!(decode_with_limit(&"A".repeat(20), 8), Err(TOO_LARGE));
        assert_eq!(
            decode_with_limit(&b64.encode([1u8; 8]), 8).unwrap().len(),
            8
        );
        assert_eq!(decode_with_limit(&b64.encode([1u8; 9]), 8), Err(TOO_LARGE));
    }

    #[test]
    fn data_urls_are_built_only_for_jpeg_and_png_files() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg = dir.path().join("a.jpg");
        std::fs::write(&jpeg, prepare_photo(&png(10, 10)).unwrap()).unwrap();
        let p = dir.path().join("b.png");
        std::fs::write(&p, png(4, 4)).unwrap();
        let text = dir.path().join("c.jpg");
        std::fs::write(&text, b"hello").unwrap();
        assert!(image_data_url(&jpeg)
            .unwrap()
            .starts_with("data:image/jpeg;base64,/9j/"));
        assert!(image_data_url(&p)
            .unwrap()
            .starts_with("data:image/png;base64,iVBOR"));
        assert_eq!(image_data_url(&text), None);
        assert_eq!(image_data_url(&dir.path().join("missing.jpg")), None);
    }
}
