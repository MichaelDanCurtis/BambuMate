//! The P1 and A1 camera: JPEG frames over TLS on port 6000. After the TLS
//! handshake the client sends one 80-byte login; the printer then sends
//! frames, each a 16-byte header (u32 LE size, 0, 1, 0) and the JPEG. A
//! wrong access code makes the printer close the connection unanswered.

use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use super::target::CameraTarget;
use super::{CameraError, Frame, MAX_MESSAGE};

/// How long the first frame may take before the printer counts as busy.
pub const FIRST_FRAME_WAIT: Duration = Duration::from_secs(10);
/// How long a running stream may go without a frame.
pub const STALL: Duration = Duration::from_secs(10);
const CONNECT_WAIT: Duration = Duration::from_secs(5);
const USER: &str = "bblp";

/// The login: `0x40`, `0x3000`, `0`, `0` (u32 LE), then the user and the
/// access code, each NUL-padded (or cut) to 32 bytes.
pub fn auth_packet(user: &str, code: &str) -> [u8; 80] {
    let mut p = [0u8; 80];
    p[0..4].copy_from_slice(&0x40u32.to_le_bytes());
    p[4..8].copy_from_slice(&0x3000u32.to_le_bytes());
    for (dst, src) in [(16usize, user.as_bytes()), (48usize, code.as_bytes())] {
        let n = src.len().min(32);
        p[dst..dst + n].copy_from_slice(&src[..n]);
    }
    p
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The header's constant words aren't 0, 1, 0.
    BadHeader,
    TooLarge(usize),
}

/// Cuts the byte stream into frames. Refuses a size over [`MAX_MESSAGE`]
/// before reading it.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame's payload, `Ok(None)` when more bytes are needed.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        if self.buf.len() < 16 {
            return Ok(None);
        }
        let word = |i: usize| u32::from_le_bytes(self.buf[i..i + 4].try_into().unwrap());
        let size = word(0) as usize;
        if (word(4), word(8), word(12)) != (0, 1, 0) {
            return Err(FrameError::BadHeader);
        }
        if size > MAX_MESSAGE {
            return Err(FrameError::TooLarge(size));
        }
        if self.buf.len() < 16 + size {
            return Ok(None);
        }
        let payload = self.buf[16..16 + size].to_vec();
        self.buf.drain(..16 + size);
        Ok(Some(payload))
    }
}

/// A whole JPEG: starts with SOI (`FF D8`) and ends with EOI (`FF D9`).
pub fn is_jpeg(b: &[u8]) -> bool {
    b.len() >= 4 && b[..2] == [0xFF, 0xD8] && b[b.len() - 2..] == [0xFF, 0xD9]
}

/// TCP then TLS to the target, mapping failures: nothing listening (or
/// a reset/close during the handshake) is `Refused`, a certificate the
/// verifier rejects is `Tls`.
pub async fn connect_tls(target: &CameraTarget) -> Result<TlsStream<TcpStream>, CameraError> {
    let tcp = tokio::time::timeout(
        CONNECT_WAIT,
        TcpStream::connect((target.host.as_str(), target.port)),
    )
    .await
    .map_err(|_| CameraError::Other {
        message: "The printer's camera didn't answer.".into(),
    })?
    .map_err(|_| CameraError::Refused)?;
    let name = ServerName::try_from(target.serial.clone()).map_err(|_| CameraError::Tls)?;
    let connector = TlsConnector::from(Arc::clone(&target.tls));
    match tokio::time::timeout(CONNECT_WAIT, connector.connect(name, tcp)).await {
        Err(_) => Err(CameraError::Other {
            message: "The printer's camera didn't answer.".into(),
        }),
        Ok(Ok(tls)) => Ok(tls),
        Ok(Err(e)) => Err(if is_certificate_error(&e) {
            CameraError::Tls
        } else {
            CameraError::Refused
        }),
    }
}

/// A handshake that failed on the certificate, not on the connection.
fn is_certificate_error(e: &std::io::Error) -> bool {
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .is_some_and(|r| {
            matches!(
                r,
                rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented
            )
        })
}

/// Streams JPEGs to `tx` until the stream fails or `tx` is closed; returns
/// why it ended.
pub async fn run(target: &CameraTarget, tx: mpsc::Sender<Frame>) -> CameraError {
    run_with(target, tx, FIRST_FRAME_WAIT, STALL).await
}

/// `run` with the waits passed in, so tests needn't wait ten seconds.
async fn run_with(
    target: &CameraTarget,
    tx: mpsc::Sender<Frame>,
    first_frame_wait: Duration,
    stall: Duration,
) -> CameraError {
    let mut tls = match connect_tls(target).await {
        Ok(tls) => tls,
        Err(e) => return e,
    };
    if tls
        .write_all(&auth_packet(USER, &target.access_code))
        .await
        .is_err()
    {
        return CameraError::Refused;
    }
    let mut reader = FrameReader::default();
    let mut buf = vec![0u8; 64 * 1024];
    let mut got_frame = false;
    loop {
        let wait = if got_frame { stall } else { first_frame_wait };
        let n = match tokio::time::timeout(wait, tls.read(&mut buf)).await {
            Err(_) if got_frame => {
                return CameraError::Other {
                    message: "The camera stopped sending pictures.".into(),
                }
            }
            Err(_) => return CameraError::Busy,
            Ok(Err(_)) | Ok(Ok(0)) if !got_frame => return CameraError::Auth,
            Ok(Err(_)) | Ok(Ok(0)) => {
                return CameraError::Other {
                    message: "The camera stream ended.".into(),
                }
            }
            Ok(Ok(n)) => n,
        };
        reader.push(&buf[..n]);
        loop {
            match reader.next_frame() {
                Ok(None) => break,
                Ok(Some(jpeg)) => {
                    got_frame = true;
                    if !is_jpeg(&jpeg) {
                        tracing::debug!("camera: dropped a frame that isn't a whole JPEG");
                        continue;
                    }
                    if tx.send(Frame::Jpeg(jpeg.into())).await.is_err() {
                        return CameraError::Other {
                            message: "stopped".into(),
                        };
                    }
                }
                Err(e) => {
                    tracing::warn!("camera: bad frame from the printer: {e:?}");
                    return CameraError::Other {
                        message: "The camera sent something BambuMate couldn't read.".into(),
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(jpeg: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(jpeg);
        v
    }

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 0xFF, 0xD9];

    #[test]
    fn the_auth_packet_is_80_bytes_little_endian_and_nul_padded() {
        let p = auth_packet("bblp", "12345678");
        assert_eq!(p.len(), 80);
        assert_eq!(&p[0..4], &0x40u32.to_le_bytes());
        assert_eq!(&p[4..8], &0x3000u32.to_le_bytes());
        assert_eq!(&p[8..16], &[0u8; 8]);
        assert_eq!(&p[16..20], b"bblp");
        assert!(p[20..48].iter().all(|b| *b == 0));
        assert_eq!(&p[48..56], b"12345678");
        assert!(p[56..80].iter().all(|b| *b == 0));
    }

    #[test]
    fn a_long_code_is_cut_to_32_bytes() {
        let p = auth_packet("bblp", &"9".repeat(40));
        assert_eq!(&p[48..80], "9".repeat(32).as_bytes());
    }

    #[test]
    fn frames_are_read_across_split_reads() {
        let mut bytes = frame(JPEG);
        bytes.extend(frame(JPEG));
        let mut r = FrameReader::default();
        let mut got = Vec::new();
        for chunk in bytes.chunks(5) {
            r.push(chunk);
            while let Some(f) = r.next_frame().unwrap() {
                got.push(f);
            }
        }
        assert_eq!(got, vec![JPEG.to_vec(), JPEG.to_vec()]);
    }

    #[test]
    fn a_frame_over_the_limit_is_refused_before_it_is_read() {
        let mut r = FrameReader::default();
        let mut header = Vec::new();
        header.extend_from_slice(&((MAX_MESSAGE as u32) + 1).to_le_bytes());
        header.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        r.push(&header);
        assert_eq!(r.next_frame(), Err(FrameError::TooLarge(MAX_MESSAGE + 1)));
    }

    #[test]
    fn a_header_with_the_wrong_constant_words_is_refused() {
        let mut bad = frame(JPEG);
        bad[8] = 2;
        let mut r = FrameReader::default();
        r.push(&bad);
        assert_eq!(r.next_frame(), Err(FrameError::BadHeader));
    }

    #[test]
    fn a_truncated_frame_waits_for_more() {
        let bytes = frame(JPEG);
        let mut r = FrameReader::default();
        r.push(&bytes[..bytes.len() - 1]);
        assert_eq!(r.next_frame(), Ok(None));
        r.push(&bytes[bytes.len() - 1..]);
        assert_eq!(r.next_frame(), Ok(Some(JPEG.to_vec())));
    }

    #[test]
    fn only_complete_jpegs_count() {
        assert!(is_jpeg(JPEG));
        assert!(!is_jpeg(&[0xFF, 0xD8, 0, 0]));
        assert!(!is_jpeg(&[0, 0, 0xFF, 0xD9]));
        assert!(!is_jpeg(&[]));
    }

    use crate::camera::testcam::{test_target, tiny_jpeg, FakeCamera, Mode, CODE};
    use crate::camera::transport::Transport;

    #[tokio::test]
    async fn it_logs_in_and_sends_each_jpeg() {
        let jpeg = tiny_jpeg();
        let cam = FakeCamera::start(Mode::Frames {
            jpegs: vec![jpeg.clone()],
            every: std::time::Duration::from_millis(20),
        })
        .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let target = test_target(&cam, Transport::Jpeg);
        let task = tokio::spawn(async move { run(&target, tx).await });
        for _ in 0..2 {
            let got = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(got, Frame::Jpeg(jpeg.clone().into()));
        }
        drop(rx);
        let ended = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(ended, CameraError::Other { .. }));
        let login = cam.logins.lock().unwrap()[0].clone();
        assert_eq!(login, auth_packet("bblp", CODE).to_vec());
    }

    #[tokio::test]
    async fn a_close_after_login_is_a_wrong_code() {
        let cam = FakeCamera::start(Mode::CloseAfterLogin).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        assert_eq!(
            run(&test_target(&cam, Transport::Jpeg), tx).await,
            CameraError::Auth
        );
    }

    #[tokio::test]
    async fn an_untrusted_certificate_is_a_tls_error() {
        let cam = FakeCamera::start(Mode::Silent).await;
        let other = FakeCamera::start(Mode::Silent).await;
        let mut target = test_target(&other, Transport::Jpeg);
        target.port = cam.addr.port();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        assert_eq!(run(&target, tx).await, CameraError::Tls);
    }

    #[tokio::test]
    async fn nothing_listening_is_refused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let cam = FakeCamera::start(Mode::Silent).await;
        let mut target = test_target(&cam, Transport::Jpeg);
        target.port = port;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        assert_eq!(run(&target, tx).await, CameraError::Refused);
    }

    #[tokio::test]
    async fn no_first_frame_in_time_is_busy() {
        let cam = FakeCamera::start(Mode::Silent).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let wait = std::time::Duration::from_millis(200);
        assert_eq!(
            run_with(&test_target(&cam, Transport::Jpeg), tx, wait, wait).await,
            CameraError::Busy
        );
    }

    #[tokio::test]
    async fn a_stream_that_goes_quiet_after_a_frame_stalls() {
        let cam = FakeCamera::start(Mode::Frames {
            jpegs: vec![tiny_jpeg()],
            every: std::time::Duration::from_secs(30),
        })
        .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let wait = std::time::Duration::from_millis(200);
        let ended = run_with(&test_target(&cam, Transport::Jpeg), tx, wait, wait).await;
        assert!(matches!(ended, CameraError::Other { .. }));
        assert!(
            rx.try_recv().is_ok(),
            "the first frame arrived before the stall"
        );
    }
}
