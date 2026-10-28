//! A fake printer camera over TLS for tests, using the test PKI. It speaks
//! the port-6000 protocol, or echoes bytes (for the RTSPS relay test).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::target::CameraTarget;
use super::transport::Transport;
use crate::printer::tls::testpki::{TestCa, TestLeaf};
use crate::printer::tls::{client_config, PrinterCertVerifier, RejectionSlot};

pub const SERIAL: &str = "01P00A123456789";
pub const CODE: &str = "12345678";

#[derive(Clone)]
pub enum Mode {
    /// Checks the login, then sends each JPEG `every` apart and stays open.
    Frames {
        jpegs: Vec<Vec<u8>>,
        every: Duration,
    },
    /// Reads the login and closes without a byte (a wrong code).
    CloseAfterLogin,
    /// Reads the login and never sends anything (busy).
    Silent,
    /// Echoes whatever it reads (relay tests).
    Echo,
}

pub struct FakeCamera {
    pub addr: SocketAddr,
    pub ca: TestCa,
    /// Every login packet received.
    pub logins: Arc<Mutex<Vec<Vec<u8>>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeCamera {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeCamera {
    pub async fn start(mode: Mode) -> Self {
        let ca = TestCa::new("Fake Camera CA");
        let leaf: TestLeaf = ca.leaf(SERIAL);
        let certs = vec![CertificateDer::from(leaf.cert_der.clone())];
        let key = PrivateKeyDer::from_pem_slice(leaf.key_pem.as_bytes()).unwrap();
        let config =
            rustls::ServerConfig::builder_with_provider(crate::printer::tls::crypto_provider())
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let logins = Arc::new(Mutex::new(Vec::new()));
        let seen = logins.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let (acceptor, mode, seen) = (acceptor.clone(), mode.clone(), seen.clone());
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    if let Mode::Echo = mode {
                        let mut buf = [0u8; 4096];
                        while let Ok(n) = tls.read(&mut buf).await {
                            if n == 0 || tls.write_all(&buf[..n]).await.is_err() {
                                return;
                            }
                        }
                        return;
                    }
                    let mut login = [0u8; 80];
                    if tls.read_exact(&mut login).await.is_err() {
                        return;
                    }
                    seen.lock().unwrap().push(login.to_vec());
                    match mode {
                        Mode::CloseAfterLogin => {
                            let _ = tls.shutdown().await;
                        }
                        Mode::Silent => tokio::time::sleep(Duration::from_secs(3600)).await,
                        Mode::Frames { jpegs, every } => {
                            for jpeg in jpegs.iter().cycle() {
                                let mut out = Vec::with_capacity(16 + jpeg.len());
                                out.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
                                out.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
                                out.extend_from_slice(jpeg);
                                if tls.write_all(&out).await.is_err() {
                                    return;
                                }
                                tokio::time::sleep(every).await;
                            }
                        }
                        Mode::Echo => unreachable!(),
                    }
                });
            }
        });
        FakeCamera {
            addr,
            ca,
            logins,
            task,
        }
    }
}

/// A target for `cam` that trusts its CA.
pub fn test_target(cam: &FakeCamera, transport: Transport) -> CameraTarget {
    let verifier =
        PrinterCertVerifier::with_trust(&cam.ca.pem, &[], SERIAL, None, RejectionSlot::default())
            .unwrap();
    CameraTarget {
        host: "127.0.0.1".into(),
        port: cam.addr.port(),
        serial: SERIAL.into(),
        access_code: CODE.into(),
        tls: client_config(Arc::new(verifier)).unwrap(),
        transport,
        model: "P1S".into(),
    }
}

/// A target nothing listens at: for tests whose connector is fake.
pub fn offline_target(transport: Transport) -> CameraTarget {
    let ca = TestCa::new("Offline CA");
    let verifier =
        PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, RejectionSlot::default())
            .unwrap();
    CameraTarget {
        host: "127.0.0.1".into(),
        port: 9,
        serial: SERIAL.into(),
        access_code: CODE.into(),
        tls: client_config(Arc::new(verifier)).unwrap(),
        transport,
        model: "P1S".into(),
    }
}

/// A tiny real JPEG for tests (8×8 grey).
pub fn tiny_jpeg() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb([128, 128, 128]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}
