//! The printer's camera over the local network: live view and stills.
//!
//! H2, X1 and P2S printers stream H.264 over RTSPS on port 322; P1 and A1
//! printers send JPEG frames over TLS on port 6000. Both log in with the
//! LAN access code and are checked with the same certificate verifier as
//! MQTT. Nothing here ever sends the printer a command: the connections
//! only do the stream handshake.

use std::sync::Arc;

use serde::Serialize;

pub mod transport;

/// The event carrying `CameraState`.
pub const STATE_EVENT: &str = "camera://state";
/// Preference keys; both are on unless set to `"false"`.
pub const LIVE_VIEW_PREF: &str = "camera_live_view";
pub const SNAPSHOTS_PREF: &str = "camera_snapshots";
/// The largest frame, JPEG or channel message accepted.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;

/// One unit from the camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// The H.264 decoder setup: the RFC 6381 codec string (`avc1.640028`)
    /// and the avcC record. Comes before the first video frame and again
    /// whenever it changes.
    Config { codec: String, avcc: Arc<[u8]> },
    /// One H.264 access unit, AVC length-prefixed with 4-byte lengths.
    Video { key: bool, data: Arc<[u8]> },
    /// One JPEG picture (port 6000).
    Jpeg(Arc<[u8]>),
}

/// Why a camera session ended or never started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraError {
    /// The printer closed the stream before any frame: on port 6000 that
    /// is a wrong access code; on RTSPS a 401.
    Auth,
    /// The printer's certificate failed the check.
    Tls,
    /// Port 322 refused or closed the connection at once: LAN Mode
    /// Liveview is off, or another viewer has the camera.
    Refused,
    /// Connected, but no frame came in time: the printer is serving as
    /// many viewers as it can.
    Busy,
    /// Anything else, as a sentence for the page. Never holds the code.
    Other { message: String },
}

impl CameraError {
    /// The sentence the page shows.
    pub fn message(&self) -> String {
        match self {
            CameraError::Auth => {
                "The printer refused the camera login. Check the access code in Settings.".into()
            }
            CameraError::Tls => "Couldn't verify the printer's camera.".into(),
            CameraError::Refused => "The printer didn't open its camera. Turn on LAN Mode \
                 Liveview on the printer, or close the live view in another app."
                .into(),
            CameraError::Busy => "The camera is in use by another app. Close the live view \
                 in Bambu Studio or Handy, then retry."
                .into(),
            CameraError::Other { message } => message.clone(),
        }
    }

    /// Auth, Tls, Refused and Busy wait for Retry; anything else retries
    /// on its own with backoff.
    pub fn needs_retry(&self) -> bool {
        !matches!(self, CameraError::Other { .. })
    }
}
