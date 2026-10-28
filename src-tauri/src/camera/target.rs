//! Where the camera is and how to log in: built from the saved printer,
//! its live state and the keychain.

use std::sync::Arc;

use rustls::ClientConfig;
use sha2::{Digest, Sha256};

use super::transport::{choose, OffReason, Transport};
use crate::printer::service::{client_params, CameraFacts, PrinterService};
use crate::printer::settings::get_access_code;

/// Where the hub gets its camera from: the saved printer, its live state
/// and the keychain in the app, a fixed answer in tests.
pub trait TargetSource: Send + Sync + 'static {
    /// Where the camera is now, or why there is none. May block (keychain).
    fn resolve(&self) -> Result<CameraTarget, OffReason>;
    /// Whether a running session for `target` still fits the printer: same
    /// printer and code, connected, and the same transport chosen. Cheap,
    /// no keychain.
    fn still_valid(&self, target: &CameraTarget) -> Result<(), OffReason>;
}

/// One printer's camera. Written by hand so the code never reaches a log.
#[derive(Clone)]
pub struct CameraTarget {
    pub host: String,
    /// `transport.port()` for a real printer; a fake server's in tests.
    pub port: u16,
    pub serial: String,
    pub access_code: String,
    pub tls: Arc<ClientConfig>,
    pub transport: Transport,
    pub model: String,
}

impl std::fmt::Debug for CameraTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraTarget")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("serial", &self.serial)
            .field("transport", &self.transport)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

/// What a running session connects with: a target with another key needs
/// a new session. The code is kept only as a digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetKey {
    host: String,
    port: u16,
    serial: String,
    transport: Transport,
    code: [u8; 32],
}

impl CameraTarget {
    pub fn key(&self) -> TargetKey {
        TargetKey {
            host: self.host.clone(),
            port: self.port,
            serial: self.serial.clone(),
            transport: self.transport,
            code: Sha256::digest(self.access_code.as_bytes()).into(),
        }
    }
}

/// The camera for `facts` with the keychain's `code`, or why there is none.
pub fn resolve_from(facts: CameraFacts, code: Option<String>) -> Result<CameraTarget, OffReason> {
    let config = facts.config.ok_or(OffReason::NotConfigured)?;
    if !facts.connected {
        return Err(OffReason::NotConnected);
    }
    if !facts.has_state {
        return Err(OffReason::Waiting);
    }
    let transport = choose(&facts.model, facts.ipcam.as_ref())?;
    let code = code.ok_or(OffReason::NotConfigured)?;
    let params = client_params(&config, code).map_err(|_| OffReason::NotConfigured)?;
    Ok(CameraTarget {
        host: params.host,
        port: transport.port(),
        serial: params.serial,
        access_code: params.access_code,
        tls: params.tls,
        transport,
        model: facts.model,
    })
}

/// The app's printer: the service's live state and the keychain's code.
pub struct ServiceTargets {
    service: PrinterService,
}

impl ServiceTargets {
    pub fn new(service: PrinterService) -> Self {
        Self { service }
    }
}

impl TargetSource for ServiceTargets {
    fn resolve(&self) -> Result<CameraTarget, OffReason> {
        let facts = self.service.camera_facts();
        let code = match facts.config.as_ref() {
            Some(c) => get_access_code(&c.serial).ok().flatten(),
            None => None,
        };
        resolve_from(facts, code)
    }

    fn still_valid(&self, target: &CameraTarget) -> Result<(), OffReason> {
        let facts = self.service.camera_facts();
        let config = facts.config.as_ref().ok_or(OffReason::NotConfigured)?;
        if config.ip != target.host || !config.serial.eq_ignore_ascii_case(&target.serial) {
            return Err(OffReason::NotConfigured);
        }
        if !facts.connected {
            return Err(OffReason::NotConnected);
        }
        match choose(&facts.model, facts.ipcam.as_ref()) {
            Ok(t) if t == target.transport => Ok(()),
            Ok(_) => Err(OffReason::Waiting),
            Err(reason) => Err(reason),
        }
    }
}

/// A camera preference: on unless stored as `"false"`.
pub fn camera_pref(app: &tauri::AppHandle, key: &str) -> bool {
    use tauri_plugin_store::StoreExt;
    app.store("preferences.json")
        .ok()
        .and_then(|s| s.get(key))
        .and_then(|v| v.as_str().map(|s| s != "false"))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::testcam::{test_target, FakeCamera, Mode};
    use crate::printer::service::CameraFacts;
    use crate::printer::settings::PrinterConfig;
    use crate::printer::state::{IpCam, LanLiveview};

    fn config() -> PrinterConfig {
        PrinterConfig {
            ip: "192.168.1.50".into(),
            serial: "0938AC123456789".into(),
            name: "Shop".into(),
            model: "H2D".into(),
            pinned_fingerprint: None,
            ca_verified: false,
        }
    }

    fn facts() -> CameraFacts {
        CameraFacts {
            config: Some(config()),
            connected: true,
            has_state: true,
            model: "H2D".into(),
            ipcam: Some(IpCam {
                present: Some(true),
                liveview: LanLiveview::Rtsp,
            }),
        }
    }

    #[test]
    fn a_connected_printer_with_a_code_gives_its_camera() {
        let t = resolve_from(facts(), Some("12345678".into())).unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.transport),
            ("192.168.1.50", 322, Transport::Rtsps)
        );
        assert_eq!(t.serial, "0938AC123456789");
    }

    #[test]
    fn each_missing_piece_says_why() {
        let none = CameraFacts {
            config: None,
            ..facts()
        };
        assert_eq!(
            resolve_from(none, Some("1".into())).err(),
            Some(OffReason::NotConfigured)
        );
        let offline = CameraFacts {
            connected: false,
            ..facts()
        };
        assert_eq!(
            resolve_from(offline, Some("1".into())).err(),
            Some(OffReason::NotConnected)
        );
        let fresh = CameraFacts {
            has_state: false,
            ..facts()
        };
        assert_eq!(
            resolve_from(fresh, Some("1".into())).err(),
            Some(OffReason::Waiting)
        );
        assert_eq!(
            resolve_from(facts(), None).err(),
            Some(OffReason::NotConfigured)
        );
        let off = CameraFacts {
            ipcam: Some(IpCam {
                present: Some(true),
                liveview: LanLiveview::Off,
            }),
            ..facts()
        };
        assert_eq!(
            resolve_from(off, Some("1".into())).err(),
            Some(OffReason::LiveviewOff)
        );
    }

    #[tokio::test]
    async fn the_debug_form_never_shows_the_code() {
        let cam = FakeCamera::start(Mode::Silent).await;
        let mut t = test_target(&cam, Transport::Jpeg);
        // Not the shared test code: that one also occurs in the test serial.
        t.access_code = "s3cr3t-code".into();
        assert!(!format!("{t:?}").contains("s3cr3t-code"));
    }
}
