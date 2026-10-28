//! Where the camera is and how to log in: built from the saved printer,
//! its live state and the keychain.

use std::sync::Arc;

use rustls::ClientConfig;
use sha2::{Digest, Sha256};

use super::transport::Transport;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::testcam::{test_target, FakeCamera, Mode};

    #[tokio::test]
    async fn the_debug_form_never_shows_the_code() {
        let cam = FakeCamera::start(Mode::Silent).await;
        let mut t = test_target(&cam, Transport::Jpeg);
        // Not the shared test code: that one also occurs in the test serial.
        t.access_code = "s3cr3t-code".into();
        assert!(!format!("{t:?}").contains("s3cr3t-code"));
    }
}
