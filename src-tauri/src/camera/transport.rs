//! Which camera stream a printer offers, decided from its model and what
//! `print.ipcam` reports. Pure: the hub and the commands call it.

use serde::Serialize;

use crate::printer::model::model_key;
use crate::printer::state::{IpCam, LanLiveview};

pub const RTSPS_PORT: u16 = 322;
pub const JPEG_PORT: u16 = 6000;
pub const RTSPS_PATH: &str = "/streaming/live/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// H.264 over RTSPS on port 322 (H2, X1, P2S).
    Rtsps,
    /// JPEG frames over TLS on port 6000 (P1, A1).
    Jpeg,
}

impl Transport {
    pub fn port(self) -> u16 {
        match self {
            Transport::Rtsps => RTSPS_PORT,
            Transport::Jpeg => JPEG_PORT,
        }
    }
}

/// Why there is no camera to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OffReason {
    NotConfigured,
    NotConnected,
    /// Connected, but the printer hasn't said enough yet.
    Waiting,
    NoCamera,
    /// `rtsp_url` is `"disable"`.
    LiveviewOff,
    /// A model BambuMate doesn't know the camera of.
    Unsupported,
    /// Live view is switched off in Settings.
    Disabled,
}

impl OffReason {
    /// The page's sentence. `LiveviewOff` is the title; the page adds the
    /// steps under it.
    pub fn message(self) -> &'static str {
        match self {
            OffReason::NotConfigured => "No printer is set up.",
            OffReason::NotConnected => "The printer isn't connected.",
            OffReason::Waiting => "Waiting for the printer's camera details…",
            OffReason::NoCamera => "This printer has no camera.",
            OffReason::LiveviewOff => "Turn on LAN Mode Liveview",
            OffReason::Unsupported => "BambuMate can't show this printer's camera yet.",
            OffReason::Disabled => "Live view is off in Settings.",
        }
    }
}

const JPEG_MODELS: &[&str] = &["P1P", "P1S", "A1", "A1MINI"];
const RTSPS_MODELS: &[&str] = &[
    "X1", "X1CARBON", "X1E", "H2D", "H2DPRO", "H2S", "H2C", "P2S",
];

/// The stream to open for `model`, or why there is none. In order: no
/// camera, liveview off, an RTSP URL reported, the model's known family,
/// then waiting (no `ipcam` yet) or unsupported.
pub fn choose(model: &str, ipcam: Option<&IpCam>) -> Result<Transport, OffReason> {
    if ipcam.is_some_and(|c| c.present == Some(false)) {
        return Err(OffReason::NoCamera);
    }
    match ipcam.map(|c| c.liveview) {
        Some(LanLiveview::Off) => return Err(OffReason::LiveviewOff),
        Some(LanLiveview::Rtsp) => return Ok(Transport::Rtsps),
        _ => {}
    }
    let key = model_key(model);
    if JPEG_MODELS.contains(&key.as_str()) {
        Ok(Transport::Jpeg)
    } else if RTSPS_MODELS.contains(&key.as_str()) {
        Ok(Transport::Rtsps)
    } else if ipcam.is_none() {
        Err(OffReason::Waiting)
    } else {
        Err(OffReason::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::state::{IpCam, LanLiveview};

    fn cam(present: Option<bool>, liveview: LanLiveview) -> IpCam {
        IpCam { present, liveview }
    }

    #[test]
    fn the_transport_follows_the_printer_and_what_it_reports() {
        use LanLiveview::*;
        let on = cam(Some(true), Rtsp);
        let off = cam(Some(true), Off);
        let unset = cam(Some(true), Unset);
        let none = cam(Some(false), Unset);
        let cases: &[(&str, Option<&IpCam>, Result<Transport, OffReason>)] = &[
            ("H2D", Some(&on), Ok(Transport::Rtsps)),
            ("H2D", Some(&off), Err(OffReason::LiveviewOff)),
            ("H2D", Some(&unset), Ok(Transport::Rtsps)),
            ("H2D", None, Ok(Transport::Rtsps)),
            ("X1 Carbon", Some(&unset), Ok(Transport::Rtsps)),
            ("Bambu Lab P2S", Some(&unset), Ok(Transport::Rtsps)),
            ("H2D Pro", Some(&on), Ok(Transport::Rtsps)),
            ("P1S", Some(&unset), Ok(Transport::Jpeg)),
            ("A1 mini", None, Ok(Transport::Jpeg)),
            ("A1", Some(&off), Err(OffReason::LiveviewOff)),
            ("P1P", Some(&none), Err(OffReason::NoCamera)),
            ("H2S", Some(&none), Err(OffReason::NoCamera)),
            ("Mystery 9", Some(&on), Ok(Transport::Rtsps)),
            ("Mystery 9", Some(&unset), Err(OffReason::Unsupported)),
            ("Mystery 9", None, Err(OffReason::Waiting)),
            ("", None, Err(OffReason::Waiting)),
        ];
        for (model, ipcam, want) in cases {
            assert_eq!(choose(model, *ipcam), *want, "{model} {ipcam:?}");
        }
    }

    #[test]
    fn each_transport_has_its_port() {
        assert_eq!(Transport::Rtsps.port(), 322);
        assert_eq!(Transport::Jpeg.port(), 6000);
    }

    #[test]
    fn every_off_reason_has_its_sentence() {
        assert_eq!(OffReason::NoCamera.message(), "This printer has no camera.");
        assert_eq!(
            OffReason::NotConnected.message(),
            "The printer isn't connected."
        );
        assert_eq!(OffReason::NotConfigured.message(), "No printer is set up.");
        assert_eq!(
            OffReason::Waiting.message(),
            "Waiting for the printer's camera details…"
        );
        assert_eq!(
            OffReason::Unsupported.message(),
            "BambuMate can't show this printer's camera yet."
        );
        assert_eq!(
            OffReason::Disabled.message(),
            "Live view is off in Settings."
        );
        assert_eq!(
            OffReason::LiveviewOff.message(),
            "Turn on LAN Mode Liveview"
        );
    }
}
