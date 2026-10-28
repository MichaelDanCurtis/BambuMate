//! Frames as Channel messages: byte 0 is the tag, byte 1 a flag, then the
//! payload. Config: u16 BE codec length, the codec, the avcC record.
//! Video: the AVC access unit, flag 1 for a keyframe. JPEG: the picture.

use super::Frame;

pub const TAG_CONFIG: u8 = 1;
pub const TAG_VIDEO: u8 = 2;
pub const TAG_JPEG: u8 = 3;

pub fn encode(frame: &Frame) -> Vec<u8> {
    match frame {
        Frame::Config { codec, avcc } => {
            let codec = codec.as_bytes();
            let mut out = Vec::with_capacity(4 + codec.len() + avcc.len());
            out.extend_from_slice(&[TAG_CONFIG, 0]);
            out.extend_from_slice(&(codec.len() as u16).to_be_bytes());
            out.extend_from_slice(codec);
            out.extend_from_slice(avcc);
            out
        }
        Frame::Video { key, data } => {
            let mut out = Vec::with_capacity(2 + data.len());
            out.extend_from_slice(&[TAG_VIDEO, u8::from(*key)]);
            out.extend_from_slice(data);
            out
        }
        Frame::Jpeg(data) => {
            let mut out = Vec::with_capacity(2 + data.len());
            out.extend_from_slice(&[TAG_JPEG, 0]);
            out.extend_from_slice(data);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_frame_kind_has_its_layout() {
        let config = Frame::Config {
            codec: "avc1.640028".into(),
            avcc: vec![1, 2, 3].into(),
        };
        let mut want = vec![1, 0, 0, 11];
        want.extend_from_slice(b"avc1.640028");
        want.extend_from_slice(&[1, 2, 3]);
        assert_eq!(encode(&config), want);
        assert_eq!(
            encode(&Frame::Video {
                key: true,
                data: vec![9, 8].into()
            }),
            vec![2, 1, 9, 8]
        );
        assert_eq!(
            encode(&Frame::Video {
                key: false,
                data: vec![7].into()
            }),
            vec![2, 0, 7]
        );
        assert_eq!(
            encode(&Frame::Jpeg(vec![0xFF, 0xD8].into())),
            vec![3, 0, 0xFF, 0xD8]
        );
    }
}
