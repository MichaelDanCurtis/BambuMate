//! Mirrors of the backend's camera state (`camera::session`), the page's
//! reading of it, and the Channel message header. Tested against
//! `wire.json`, which the backend writes.

use serde::Deserialize;

/// Under the "Turn on LAN Mode Liveview" title.
pub const LIVEVIEW_STEPS: &str =
    "On the printer screen: Settings → General (or LAN Only) → LAN Mode Liveview.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Rtsps,
    Jpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffReason {
    NotConfigured,
    NotConnected,
    Waiting,
    NoCamera,
    LiveviewOff,
    Unsupported,
    Disabled,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraErrorKind {
    Auth,
    Tls,
    Refused,
    Busy,
    Other {
        message: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraStatus {
    Idle,
    Connecting,
    Streaming,
    Off {
        reason: OffReason,
        message: String,
    },
    Error {
        error: CameraErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CameraState {
    pub transport: Option<Transport>,
    pub status: CameraStatus,
    pub last_frame_at: Option<i64>,
}

impl Default for CameraState {
    fn default() -> Self {
        CameraState {
            transport: None,
            status: CameraStatus::Idle,
            last_frame_at: None,
        }
    }
}

/// What the panel shows for a state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    /// `[CONNECTING…]`.
    Connecting,
    /// The picture.
    Live,
    /// The title and [`LIVEVIEW_STEPS`].
    LiveviewOff,
    /// A plain sentence.
    Note(String),
    /// `[ERROR: …]` with Retry.
    Error(String),
}

pub fn shown(s: &CameraState) -> Shown {
    match &s.status {
        CameraStatus::Idle | CameraStatus::Connecting => Shown::Connecting,
        CameraStatus::Streaming => Shown::Live,
        CameraStatus::Off {
            reason: OffReason::LiveviewOff,
            ..
        } => Shown::LiveviewOff,
        CameraStatus::Off { message, .. } => Shown::Note(message.clone()),
        CameraStatus::Error { message, .. } => Shown::Error(message.clone()),
    }
}

impl CameraState {
    /// No picture can come until something changes on the printer or in
    /// Settings: the panel stops watching.
    pub fn is_off(&self) -> bool {
        matches!(self.status, CameraStatus::Off { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Config,
    Video,
    Jpeg,
}

/// A Channel message's header: what it is, and where its payload starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub kind: Kind,
    pub key: bool,
    pub payload: usize,
    pub codec: Option<String>,
}

/// Longest message accepted (the backend's `MAX_MESSAGE` plus the header).
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024 + 300;

/// How many leading bytes of a message [`parse`] needs: the config header
/// with the longest codec string a u16 length could name in practice (256
/// bytes), plus one payload byte so a config's payload is seen to exist.
pub const HEADER_COPY: usize = 4 + 256 + 1;

/// Reads the header of `b` (only its first bytes are needed). `None` for
/// anything unknown, cut short or too large.
pub fn parse(b: &[u8]) -> Option<Parsed> {
    let (&tag, rest) = b.split_first()?;
    let (&flag, _) = rest.split_first()?;
    match tag {
        1 => {
            let len = usize::from(u16::from_be_bytes([*b.get(2)?, *b.get(3)?]));
            let codec = std::str::from_utf8(b.get(4..4 + len)?).ok()?.to_string();
            if b.len() <= 4 + len || !codec.is_ascii() {
                return None;
            }
            Some(Parsed {
                kind: Kind::Config,
                key: false,
                payload: 4 + len,
                codec: Some(codec),
            })
        }
        2 | 3 if b.len() > 2 => Some(Parsed {
            kind: if tag == 2 { Kind::Video } else { Kind::Jpeg },
            key: tag == 2 && flag == 1,
            payload: 2,
            codec: None,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("wire.json")).unwrap()
    }

    #[test]
    fn every_backend_state_deserializes() {
        let f = fixture();
        for key in ["idle", "connecting", "streaming_rtsps", "streaming_jpeg"] {
            let s: CameraState = serde_json::from_value(f[key].clone()).unwrap();
            assert_eq!(s.last_frame_at, Some(1_761_600_000), "{key}");
        }
        let offs: Vec<CameraState> = serde_json::from_value(f["off"].clone()).unwrap();
        assert_eq!(offs.len(), 7);
        assert!(offs
            .iter()
            .all(|s| matches!(s.status, CameraStatus::Off { .. })));
        let errors: Vec<CameraState> = serde_json::from_value(f["error"].clone()).unwrap();
        assert_eq!(errors.len(), 5);
    }

    #[test]
    fn each_state_reads_as_the_spec_says() {
        let f = fixture();
        let offs: Vec<CameraState> = serde_json::from_value(f["off"].clone()).unwrap();
        let texts: Vec<Shown> = offs.iter().map(shown).collect();
        assert_eq!(texts[0], Shown::Note("No printer is set up.".into()));
        assert_eq!(texts[1], Shown::Note("The printer isn't connected.".into()));
        assert_eq!(
            texts[2],
            Shown::Note("Waiting for the printer's camera details…".into())
        );
        assert_eq!(texts[3], Shown::Note("This printer has no camera.".into()));
        assert_eq!(texts[4], Shown::LiveviewOff);
        assert_eq!(
            texts[5],
            Shown::Note("BambuMate can't show this printer's camera yet.".into())
        );
        assert_eq!(
            texts[6],
            Shown::Note("Live view is off in Settings.".into())
        );
        let errors: Vec<CameraState> = serde_json::from_value(f["error"].clone()).unwrap();
        assert_eq!(
            shown(&errors[3]),
            Shown::Error("The camera is in use by another app. Close the live view in Bambu Studio or Handy, then retry.".into())
        );
        let idle: CameraState = serde_json::from_value(f["idle"].clone()).unwrap();
        assert_eq!(shown(&idle), Shown::Connecting);
        let live: CameraState = serde_json::from_value(f["streaming_jpeg"].clone()).unwrap();
        assert_eq!(shown(&live), Shown::Live);
    }

    #[test]
    fn channel_messages_parse_like_the_backend_writes_them() {
        let mut config = vec![1, 0, 0, 11];
        config.extend_from_slice(b"avc1.640028");
        config.extend_from_slice(&[1, 2, 3]);
        assert_eq!(
            parse(&config),
            Some(Parsed {
                kind: Kind::Config,
                key: false,
                payload: 15,
                codec: Some("avc1.640028".into())
            })
        );
        assert_eq!(
            parse(&[2, 1, 9, 8]),
            Some(Parsed {
                kind: Kind::Video,
                key: true,
                payload: 2,
                codec: None
            })
        );
        assert_eq!(
            parse(&[2, 0, 7]),
            Some(Parsed {
                kind: Kind::Video,
                key: false,
                payload: 2,
                codec: None
            })
        );
        assert_eq!(
            parse(&[3, 0, 0xFF, 0xD8]),
            Some(Parsed {
                kind: Kind::Jpeg,
                key: false,
                payload: 2,
                codec: None
            })
        );
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&[9, 0, 1]), None);
        assert_eq!(
            parse(&[1, 0, 0, 200, b'a']),
            None,
            "codec longer than the message"
        );
        assert_eq!(parse(&[2, 0]), None, "no payload");
    }
}
