//! The H2, X1 and P2S camera: H.264 over RTSPS on port 322. `retina` only
//! speaks plain RTSP, so a one-connection relay on 127.0.0.1 carries its
//! bytes through TLS (checked like MQTT) to the printer. The TLS handshake
//! happens before retina connects, so a certificate or refusal is
//! reported as such rather than as an RTSP error.
//!
//! Known risk: live555 servers may stop a stream after about 10 s without
//! RTCP receiver reports, which retina doesn't send. A stall is reported
//! and the hub reconnects; to be checked on a real H2.

use std::net::SocketAddr;
use std::time::Duration;

use futures::StreamExt;
use retina::client::{
    Credentials, PlayOptions, SessionOptions, SetupOptions, TcpTransportOptions, Transport,
};
use retina::codec::{CodecItem, ParametersRef};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use url::Url;

use super::jpeg6000::connect_tls;
use super::target::CameraTarget;
use super::transport::RTSPS_PATH;
use super::{CameraError, Frame, MAX_MESSAGE};

/// How long the stream may go without a video frame.
pub const STALL: Duration = Duration::from_secs(10);
const USER: &str = "bblp";

/// A running relay. Dropping it ends the relay task.
pub struct Relay {
    pub addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Opens TLS to the printer, then waits for one local connection on
/// 127.0.0.1 and copies bytes both ways until either side closes.
pub async fn relay(target: &CameraTarget) -> Result<Relay, CameraError> {
    let mut printer = connect_tls(target).await?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| CameraError::Other {
            message: "BambuMate couldn't open a local port for the camera.".into(),
        })?;
    let addr = listener.local_addr().map_err(|_| CameraError::Other {
        message: "BambuMate couldn't open a local port for the camera.".into(),
    })?;
    let task = tokio::spawn(async move {
        let Ok((mut local, peer)) = listener.accept().await else {
            return;
        };
        drop(listener);
        if !peer.ip().is_loopback() {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut local, &mut printer).await;
    });
    Ok(Relay { addr, task })
}

/// The URL retina dials: the relay, with the printer's path. Credentials go
/// in `SessionOptions`, never in the URL.
fn local_url(port: u16) -> Url {
    Url::parse(&format!("rtsp://127.0.0.1:{port}{RTSPS_PATH}")).expect("a valid URL")
}

/// A retina error as a camera error. Its text may echo the URL, never the
/// code, since the code isn't in the URL.
fn map_error(text: &str) -> CameraError {
    if text.contains("401") {
        CameraError::Auth
    } else {
        CameraError::Other {
            message: "The camera stream failed.".into(),
        }
    }
}

/// Streams the printer's H.264 to `tx`: `Frame::Config` first and whenever
/// the parameters change, then every access unit. Frames before the first
/// keyframe are dropped. Returns why it ended.
pub async fn run(target: &CameraTarget, tx: mpsc::Sender<Frame>) -> CameraError {
    let relay = match relay(target).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let creds = Credentials {
        username: USER.into(),
        password: target.access_code.clone(),
    };
    let options = SessionOptions::default()
        .creds(Some(creds))
        .user_agent("BambuMate".into());
    let mut session =
        match retina::client::Session::describe(local_url(relay.addr.port()), options).await {
            Ok(s) => s,
            Err(e) => return map_error(&e.to_string()),
        };
    let Some(video) = session
        .streams()
        .iter()
        .position(|s| s.media() == "video" && s.encoding_name().eq_ignore_ascii_case("h264"))
    else {
        return CameraError::Other {
            message: "The printer's camera doesn't offer H.264 video.".into(),
        };
    };
    if let Err(e) = session
        .setup(
            video,
            SetupOptions::default().transport(Transport::Tcp(TcpTransportOptions::default())),
        )
        .await
    {
        return map_error(&e.to_string());
    }
    let playing = match session.play(PlayOptions::default()).await {
        Ok(p) => p,
        Err(e) => return map_error(&e.to_string()),
    };
    let mut demuxed = match playing.demuxed() {
        Ok(d) => d,
        Err(e) => return map_error(&e.to_string()),
    };
    let mut sent_config = false;
    let mut seen_key = false;
    loop {
        let item = match tokio::time::timeout(STALL, demuxed.next()).await {
            Err(_) => {
                return CameraError::Other {
                    message: "The camera stopped sending video.".into(),
                }
            }
            Ok(None) => {
                return CameraError::Other {
                    message: "The camera stream ended.".into(),
                }
            }
            Ok(Some(Err(e))) => return map_error(&e.to_string()),
            Ok(Some(Ok(item))) => item,
        };
        let CodecItem::VideoFrame(frame) = item else {
            continue;
        };
        if !sent_config || frame.has_new_parameters() {
            let params = demuxed.streams()[video].parameters();
            let Some(ParametersRef::Video(v)) = params else {
                continue;
            };
            let config = Frame::Config {
                codec: v.rfc6381_codec().to_string(),
                avcc: v.extra_data().to_vec().into(),
            };
            if tx.send(config).await.is_err() {
                return CameraError::Other {
                    message: "stopped".into(),
                };
            }
            sent_config = true;
        }
        let key = frame.is_random_access_point();
        seen_key |= key;
        if !seen_key {
            continue;
        }
        let data = frame.into_data();
        if data.len() > MAX_MESSAGE {
            tracing::warn!("camera: dropped a {}-byte video frame", data.len());
            continue;
        }
        if tx
            .send(Frame::Video {
                key,
                data: data.into(),
            })
            .await
            .is_err()
        {
            return CameraError::Other {
                message: "stopped".into(),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::testcam::{test_target, FakeCamera, Mode};
    use crate::camera::transport::Transport;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn the_relay_carries_bytes_both_ways_over_tls() {
        let cam = FakeCamera::start(Mode::Echo).await;
        let relay = relay(&test_target(&cam, Transport::Rtsps)).await.unwrap();
        let mut local = tokio::net::TcpStream::connect(relay.addr).await.unwrap();
        local
            .write_all(b"OPTIONS * RTSP/1.0\r\n\r\n")
            .await
            .unwrap();
        let mut got = vec![0u8; 22];
        local.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"OPTIONS * RTSP/1.0\r\n\r\n");
    }

    #[tokio::test]
    async fn the_relay_listens_on_loopback_only_and_takes_one_connection() {
        let cam = FakeCamera::start(Mode::Echo).await;
        let relay = relay(&test_target(&cam, Transport::Rtsps)).await.unwrap();
        assert!(relay.addr.ip().is_loopback());
        let _first = tokio::net::TcpStream::connect(relay.addr).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let second = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            tokio::net::TcpStream::connect(relay.addr),
        )
        .await;
        assert!(
            !matches!(second, Ok(Ok(_))),
            "a second connection must not be accepted"
        );
    }

    #[tokio::test]
    async fn an_untrusted_camera_fails_before_any_rtsp() {
        let cam = FakeCamera::start(Mode::Echo).await;
        let other = FakeCamera::start(Mode::Echo).await;
        let mut target = test_target(&other, Transport::Rtsps);
        target.port = cam.addr.port();
        assert_eq!(relay(&target).await.err(), Some(CameraError::Tls));
    }

    #[test]
    fn the_rtsp_url_points_at_the_relay_without_credentials() {
        let u = local_url(4321);
        assert_eq!(u.as_str(), "rtsp://127.0.0.1:4321/streaming/live/1");
        assert!(u.password().is_none());
    }

    #[test]
    fn retina_errors_map_to_camera_errors() {
        assert_eq!(map_error("401 Unauthorized"), CameraError::Auth);
        assert_eq!(map_error("RTSP/1.0 401"), CameraError::Auth);
        assert!(matches!(
            map_error("connection reset"),
            CameraError::Other { .. }
        ));
    }
}
