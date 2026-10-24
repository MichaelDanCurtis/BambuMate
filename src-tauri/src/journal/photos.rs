//! The photos a user adds to a journal entry. Each is re-encoded to a JPEG
//! no larger than 2048 px on its long edge, which also drops whatever
//! metadata the original carried. Refusals are plain sentences.

use std::io::Cursor;
use std::path::Path;
use std::sync::Mutex;

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
/// Most pixels an upload may hold. 64 MP covers 12, 48 and 50 MP phone
/// photos and DSLRs up to 61 MP (a 108 or 200 MP mode is refused). At
/// 3 bytes a pixel the decoded image is at most about 190 MB. This is the
/// bound for JPEG and WebP, whose decoders ignore `max_alloc`.
const MAX_DECODE_PIXELS: u64 = 64_000_000;
/// Backstop for the memory a decoder may allocate. Only the PNG decoder
/// honours it (a 16-bit RGBA PNG at 64 MP would be 512 MB, so it is refused).
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;
const JPEG_QUALITY: u8 = 85;
/// Largest stored photo `image_data_url` reads: the upload cap and a margin.
const MAX_PHOTO_READ_BYTES: u64 = MAX_PHOTO_BYTES as u64 + 1024 * 1024;
/// Largest plate thumbnail read (a small PNG from the slice's cache).
pub const MAX_THUMBNAIL_READ_BYTES: u64 = 2 * 1024 * 1024;

/// One photo is prepared at a time: a decode can take hundreds of MB, and
/// the page may send several photos together.
static PREPARE_LOCK: Mutex<()> = Mutex::new(());

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
    let _one_at_a_time = PREPARE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    if !within_budget(width, height, max_pixels) {
        return Err(NOT_AN_IMAGE);
    }
    let orientation = decoder.orientation().ok();
    let img = DynamicImage::from_decoder(decoder).map_err(|_| NOT_AN_IMAGE)?;
    // Shrink first: turning the small image upright is cheap, and no second
    // full-size buffer is needed.
    let mut img = if img.width().max(img.height()) > MAX_EDGE {
        img.resize(MAX_EDGE, MAX_EDGE, FilterType::Lanczos3)
    } else {
        img
    };
    if let Some(orientation) = orientation {
        img.apply_orientation(orientation);
    }
    let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
    let mut out = Cursor::new(Vec::new());
    rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))
        .map_err(|e| {
            tracing::warn!("journal photo re-encode failed: {e}");
            NOT_AN_IMAGE
        })?;
    Ok(out.into_inner())
}

/// Whether an image the header says is `width` x `height` may be decoded.
fn within_budget(width: u32, height: u32, max_pixels: u64) -> bool {
    u64::from(width) * u64::from(height) <= max_pixels
}

/// A stored JPEG or PNG as a `data:` URL; `None` when the file is missing,
/// too large or not one of those.
pub fn image_data_url(path: &Path) -> Option<String> {
    image_data_url_within(path, MAX_PHOTO_READ_BYTES)
}

/// [`image_data_url`] for a file of at most `max_bytes`.
pub fn image_data_url_within(path: &Path, max_bytes: u64) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > max_bytes {
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

    /// A real JPEG whose SOF header claims `width` x `height`.
    fn jpeg_claiming(width: u16, height: u16) -> Vec<u8> {
        let mut jpeg = encoded(16, 16, ImageFormat::Jpeg);
        let sof = jpeg
            .windows(2)
            .position(|w| w == [0xFF, 0xC0] || w == [0xFF, 0xC2])
            .expect("a start-of-frame marker");
        jpeg[sof + 5..sof + 7].copy_from_slice(&height.to_be_bytes());
        jpeg[sof + 7..sof + 9].copy_from_slice(&width.to_be_bytes());
        jpeg
    }

    fn header_size(bytes: &[u8]) -> (u32, u32) {
        use image::ImageDecoder;
        ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg)
            .into_decoder()
            .unwrap()
            .dimensions()
    }

    #[test]
    fn a_jpeg_whose_header_is_over_64_megapixels_is_refused_from_the_header() {
        assert_eq!(MAX_DECODE_PIXELS, 64_000_000);
        let over = jpeg_claiming(8001, 8000);
        assert!(over.len() < 10_000);
        let (w, h) = header_size(&over);
        assert_eq!((w, h), (8001, 8000));
        assert!(!within_budget(w, h, MAX_DECODE_PIXELS));
        assert_eq!(prepare_photo(&over), Err(NOT_AN_IMAGE));
        // The same header at exactly 64 MP is inside the budget.
        let (w, h) = header_size(&jpeg_claiming(8000, 8000));
        assert!(within_budget(w, h, MAX_DECODE_PIXELS));
    }

    #[test]
    fn a_truncated_image_with_a_valid_header_is_refused() {
        let png = png(300, 300);
        assert_eq!(prepare_photo(&png[..png.len() / 2]), Err(NOT_AN_IMAGE));
        let jpeg = encoded(300, 300, ImageFormat::Jpeg);
        // Only the start of a JPEG: its frame header is there, the scan is not.
        assert_eq!(
            prepare_photo(&jpeg[..jpeg.len().min(30)]),
            Err(NOT_AN_IMAGE)
        );
    }

    #[test]
    fn the_real_25_mb_cap_is_exact() {
        let b64 = base64::engine::general_purpose::STANDARD;
        let at_cap = b64.encode(vec![0u8; MAX_PHOTO_BYTES]);
        assert_eq!(decode_upload(&at_cap).unwrap().len(), MAX_PHOTO_BYTES);
        let over = b64.encode(vec![0u8; MAX_PHOTO_BYTES + 1]);
        assert_eq!(decode_upload(&over), Err(TOO_LARGE));
    }

    #[test]
    fn stored_files_over_their_read_cap_are_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.png");
        std::fs::write(&p, png(40, 40)).unwrap();
        assert!(image_data_url_within(&p, 1_000_000).is_some());
        assert_eq!(image_data_url_within(&p, 10), None);
    }

    #[test]
    fn a_panic_while_preparing_does_not_wedge_later_photos() {
        let _ = std::thread::spawn(|| {
            let _held = PREPARE_LOCK.lock().unwrap();
            panic!("a decoder panicked");
        })
        .join();
        assert!(prepare_photo(&png(20, 20)).is_ok());
    }
}
