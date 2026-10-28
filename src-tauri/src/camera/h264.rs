//! One H.264 keyframe to a JPEG, for journal snapshots and stills. The
//! stream arrives as AVC (length-prefixed NAL units) with an avcC record;
//! openh264 wants Annex B (start codes) with SPS and PPS in front.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, RgbImage};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

pub const JPEG_QUALITY: u8 = 85;
/// Longest side of a snapshot.
pub const MAX_EDGE: u32 = 1920;

/// What an avcC record holds that decoding needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvcConfig {
    /// Bytes in each NAL length prefix (1, 2 or 4).
    pub length_size: usize,
    pub sps: Vec<Vec<u8>>,
    pub pps: Vec<Vec<u8>>,
}

/// Reads an `AVCDecoderConfigurationRecord` (ISO 14496-15 5.2.4.1).
pub fn parse_avcc(b: &[u8]) -> Result<AvcConfig, String> {
    let bad = || "The camera's video setup couldn't be read.".to_string();
    if b.len() < 7 || b[0] != 1 {
        return Err(bad());
    }
    let length_size = usize::from(b[4] & 0x03) + 1;
    if length_size == 3 {
        return Err(bad());
    }
    let mut at = 5;
    let sets = |count: usize, at: &mut usize| -> Result<Vec<Vec<u8>>, String> {
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let len = usize::from(u16::from_be_bytes(
                b.get(*at..*at + 2).ok_or_else(bad)?.try_into().unwrap(),
            ));
            *at += 2;
            out.push(b.get(*at..*at + len).ok_or_else(bad)?.to_vec());
            *at += len;
        }
        Ok(out)
    };
    let sps_count = usize::from(b[at] & 0x1F);
    at += 1;
    let sps = sets(sps_count, &mut at)?;
    let pps_count = usize::from(*b.get(at).ok_or_else(bad)?);
    at += 1;
    let pps = sets(pps_count, &mut at)?;
    if sps.is_empty() || pps.is_empty() {
        return Err(bad());
    }
    Ok(AvcConfig {
        length_size,
        sps,
        pps,
    })
}

/// SPS, PPS, then each NAL unit of `sample`, each after a 4-byte start code.
pub fn to_annexb(cfg: &AvcConfig, sample: &[u8]) -> Result<Vec<u8>, String> {
    const START: [u8; 4] = [0, 0, 0, 1];
    let mut out = Vec::with_capacity(sample.len() + 128);
    for set in cfg.sps.iter().chain(&cfg.pps) {
        out.extend_from_slice(&START);
        out.extend_from_slice(set);
    }
    let mut at = 0;
    while at < sample.len() {
        let len_bytes = sample
            .get(at..at + cfg.length_size)
            .ok_or("The camera sent a cut-off video frame.")?;
        let len = len_bytes
            .iter()
            .fold(0usize, |n, b| (n << 8) | usize::from(*b));
        at += cfg.length_size;
        let nal = sample
            .get(at..at + len)
            .ok_or("The camera sent a cut-off video frame.")?;
        out.extend_from_slice(&START);
        out.extend_from_slice(nal);
        at += len;
    }
    Ok(out)
}

/// Decodes the keyframe `sample` and encodes it as a JPEG.
pub fn keyframe_jpeg(avcc: &[u8], sample: &[u8]) -> Result<Vec<u8>, String> {
    let cfg = parse_avcc(avcc)?;
    let annexb = to_annexb(&cfg, sample)?;
    let failed = || "The camera's picture couldn't be decoded.".to_string();
    let mut decoder = Decoder::new().map_err(|_| failed())?;
    let yuv = decoder
        .decode(&annexb)
        .map_err(|_| failed())?
        .ok_or_else(failed)?;
    let (w, h) = yuv.dimensions();
    let mut rgb = vec![0u8; w * h * 3];
    yuv.write_rgb8(&mut rgb);
    encode_jpeg(&rgb, w as u32, h as u32)
}

/// RGB8 to a JPEG at [`JPEG_QUALITY`], scaled so neither side is over
/// [`MAX_EDGE`].
pub fn encode_jpeg(rgb: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let img =
        RgbImage::from_raw(w, h, rgb.to_vec()).ok_or("The camera's picture had the wrong size.")?;
    let mut img = DynamicImage::ImageRgb8(img);
    if w.max(h) > MAX_EDGE {
        img = img.resize(MAX_EDGE, MAX_EDGE, FilterType::Triangle);
    }
    let mut out = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&img.to_rgb8())
        .map_err(|_| "The picture couldn't be saved as a JPEG.".to_string())?;
    Ok(out.into_inner())
}

/// An avcC record and one AVC keyframe sample, made with openh264's
/// encoder from a gradient: the tests' stand-in for a printer's stream.
#[cfg(test)]
pub(crate) fn test_stream(w: usize, h: usize) -> (Vec<u8>, Vec<u8>) {
    use openh264::encoder::Encoder;
    use openh264::formats::{RgbSliceU8, YUVBuffer};
    let rgb: Vec<u8> = (0..w * h)
        .flat_map(|i| [(i % 251) as u8, 90, 160])
        .collect();
    let yuv = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb, (w, h)));
    let mut enc = Encoder::new().unwrap();
    let annexb = enc.encode(&yuv).unwrap().to_vec();
    let nals: Vec<&[u8]> = openh264::nal_units(&annexb)
        .map(|n| {
            let skip = if n.starts_with(&[0, 0, 0, 1]) { 4 } else { 3 };
            &n[skip..]
        })
        .collect();
    let sps = nals.iter().find(|n| n[0] & 0x1F == 7).unwrap();
    let pps = nals.iter().find(|n| n[0] & 0x1F == 8).unwrap();
    let mut avcc = vec![1, sps[1], sps[2], sps[3], 0xFF, 0xE1];
    avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(sps);
    avcc.push(1);
    avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(pps);
    let mut sample = Vec::new();
    for n in nals.iter().filter(|n| !matches!(n[0] & 0x1F, 7 | 8)) {
        sample.extend_from_slice(&(n.len() as u32).to_be_bytes());
        sample.extend_from_slice(n);
    }
    (avcc, sample)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_encoded_keyframe_comes_back_as_a_jpeg_of_the_same_size() {
        let (avcc, sample) = test_stream(64, 48);
        let jpeg = keyframe_jpeg(&avcc, &sample).unwrap();
        let img = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (64, 48));
    }

    #[test]
    fn avcc_names_its_parameter_sets() {
        let (avcc, _) = test_stream(64, 48);
        let cfg = parse_avcc(&avcc).unwrap();
        assert_eq!(cfg.length_size, 4);
        assert_eq!(cfg.sps.len(), 1);
        assert_eq!(cfg.pps.len(), 1);
        assert_eq!(cfg.sps[0][0] & 0x1F, 7);
        assert_eq!(cfg.pps[0][0] & 0x1F, 8);
    }

    #[test]
    fn annexb_puts_the_parameter_sets_first() {
        let (avcc, sample) = test_stream(64, 48);
        let cfg = parse_avcc(&avcc).unwrap();
        let b = to_annexb(&cfg, &sample).unwrap();
        assert_eq!(&b[..4], &[0, 0, 0, 1]);
        assert_eq!(b[4] & 0x1F, 7);
    }

    #[test]
    fn broken_input_is_an_error_not_a_panic() {
        assert!(parse_avcc(&[]).is_err());
        assert!(parse_avcc(&[1, 0x64, 0, 0x28, 0xFF, 0xE1, 0, 50]).is_err());
        let (avcc, _) = test_stream(64, 48);
        let cfg = parse_avcc(&avcc).unwrap();
        assert!(to_annexb(&cfg, &[0, 0, 0, 99, 1, 2]).is_err());
        assert!(keyframe_jpeg(&avcc, &[0, 0, 0, 3, 0x65, 0, 0]).is_err());
    }

    #[test]
    fn a_large_picture_is_scaled_to_the_longest_edge() {
        let rgb = vec![200u8; 2400 * 1200 * 3];
        let jpeg = encode_jpeg(&rgb, 2400, 1200).unwrap();
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (1920, 960));
    }
}
