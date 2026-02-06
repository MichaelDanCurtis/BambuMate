//! Image loading, resizing, and base64 encoding for vision APIs.
//!
//! All images are resized to max 1024px on longest edge to control
//! API costs (requirement FNDN-05).

use base64::{engine::general_purpose::STANDARD, Engine};
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;
use tracing::info;

/// Maximum dimension (width or height) for images sent to vision APIs.
/// Requirement FNDN-05: Photos resized to max 1024px before sending.
pub const MAX_IMAGE_DIMENSION: u32 = 1024;

/// Minimum dimension for valid analysis (too small = poor detection).
pub const MIN_IMAGE_DIMENSION: u32 = 200;

/// Prepare an image for vision API: load, validate, resize, encode.
///
/// # Arguments
/// * `image_bytes` - Raw image bytes (JPEG, PNG, WebP, etc.)
///
/// # Returns
/// Base64-encoded JPEG string ready for API payload.
///
/// # Errors
/// - Image cannot be decoded
/// - Image too small (< 200px on shortest side)
pub fn prepare_image(image_bytes: &[u8]) -> Result<String, String> {
    // Load image
    let img = image::load_from_memory(image_bytes)
        .map_err(|e| format!("Failed to load image: {}. Ensure it's a valid JPEG/PNG/WebP.", e))?;

    let (width, height) = (img.width(), img.height());
    info!("Loaded image: {}x{}", width, height);

    // Validate minimum size
    let min_side = width.min(height);
    if min_side < MIN_IMAGE_DIMENSION {
        return Err(format!(
            "Image too small for reliable analysis: {}x{}. Minimum dimension is {}px.",
            width, height, MIN_IMAGE_DIMENSION
        ));
    }

    // Resize if needed (maintain aspect ratio)
    let resized = resize_if_needed(img, MAX_IMAGE_DIMENSION);
    info!("Resized to: {}x{}", resized.width(), resized.height());

    // Encode to JPEG
    let jpeg_bytes = encode_to_jpeg(&resized)?;
    info!("Encoded to JPEG: {} bytes", jpeg_bytes.len());

    // Base64 encode
    let base64_string = STANDARD.encode(&jpeg_bytes);

    Ok(base64_string)
}

/// Resize image if either dimension exceeds max, maintaining aspect ratio.
fn resize_if_needed(img: DynamicImage, max_dimension: u32) -> DynamicImage {
    let (width, height) = (img.width(), img.height());

    if width <= max_dimension && height <= max_dimension {
        return img;
    }

    // Calculate scale factor
    let scale = max_dimension as f32 / width.max(height) as f32;
    let new_width = (width as f32 * scale) as u32;
    let new_height = (height as f32 * scale) as u32;

    img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3)
}

/// Encode DynamicImage to JPEG bytes.
fn encode_to_jpeg(img: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut buffer = Cursor::new(Vec::new());
    img.write_to(&mut buffer, ImageFormat::Jpeg)
        .map_err(|e| format!("Failed to encode image to JPEG: {}", e))?;
    Ok(buffer.into_inner())
}

/// Get the media type for vision API payloads.
pub fn image_media_type() -> &'static str {
    "image/jpeg"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prepare_image_rejects_too_small() {
        // Create a valid but tiny image (50x50 - below 200px minimum)
        let img = DynamicImage::new_rgb8(50, 50);
        let mut buffer = Cursor::new(Vec::new());
        img.write_to(&mut buffer, ImageFormat::Png).unwrap();
        let small_png = buffer.into_inner();

        let result = prepare_image(&small_png);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too small"));
    }

    #[test]
    fn test_prepare_image_rejects_invalid() {
        let result = prepare_image(b"not an image");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to load"));
    }

    #[test]
    fn test_resize_if_needed_no_resize() {
        // Create a small test image
        let img = DynamicImage::new_rgb8(500, 300);
        let resized = resize_if_needed(img, 1024);
        assert_eq!(resized.width(), 500);
        assert_eq!(resized.height(), 300);
    }

    #[test]
    fn test_resize_if_needed_resize_width() {
        let img = DynamicImage::new_rgb8(2000, 1000);
        let resized = resize_if_needed(img, 1024);
        assert_eq!(resized.width(), 1024);
        assert_eq!(resized.height(), 512);
    }

    #[test]
    fn test_resize_if_needed_resize_height() {
        let img = DynamicImage::new_rgb8(1000, 2000);
        let resized = resize_if_needed(img, 1024);
        assert_eq!(resized.width(), 512);
        assert_eq!(resized.height(), 1024);
    }

    #[test]
    fn test_image_media_type() {
        assert_eq!(image_media_type(), "image/jpeg");
    }

    #[test]
    fn test_encode_to_jpeg_success() {
        let img = DynamicImage::new_rgb8(100, 100);
        let result = encode_to_jpeg(&img);
        assert!(result.is_ok());
        let bytes = result.unwrap();
        // JPEG magic bytes
        assert!(bytes.len() > 2);
        assert_eq!(bytes[0], 0xFF);
        assert_eq!(bytes[1], 0xD8);
    }

    #[test]
    fn test_prepare_image_valid_image() {
        // Create a 300x300 RGB image
        let img = DynamicImage::new_rgb8(300, 300);
        let mut buffer = Cursor::new(Vec::new());
        img.write_to(&mut buffer, ImageFormat::Png).unwrap();
        let png_bytes = buffer.into_inner();

        let result = prepare_image(&png_bytes);
        assert!(result.is_ok());

        // Verify it's valid base64
        let base64_str = result.unwrap();
        let decoded = STANDARD.decode(&base64_str);
        assert!(decoded.is_ok());

        // Verify decoded is JPEG
        let jpeg_bytes = decoded.unwrap();
        assert_eq!(jpeg_bytes[0], 0xFF);
        assert_eq!(jpeg_bytes[1], 0xD8);
    }
}
