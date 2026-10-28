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

use super::jpeg6000::{connect_tls, FIRST_FRAME_WAIT};
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
        // Defence in depth: the bind is 127.0.0.1, so this can't fail.
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

/// An RTSP status as a camera error: a 401 is a wrong access code. Retina's
/// error text carries addresses and ports, so it is never matched on.
fn map_status(code: Option<u16>) -> CameraError {
    if code == Some(401) {
        CameraError::Auth
    } else {
        CameraError::Other {
            message: "The camera stream failed.".into(),
        }
    }
}

fn map_error(e: &retina::Error) -> CameraError {
    map_status(e.status_code())
}

/// DESCRIBE, SETUP and PLAY over the relay; the demuxed stream and the
/// index of the video stream.
async fn handshake(
    target: &CameraTarget,
    relay: &Relay,
) -> Result<(retina::client::Demuxed, usize), CameraError> {
    let creds = Credentials {
        username: USER.into(),
        password: target.access_code.clone(),
    };
    let options = SessionOptions::default()
        .creds(Some(creds))
        .user_agent("BambuMate".into());
    let mut session = retina::client::Session::describe(local_url(relay.addr.port()), options)
        .await
        .map_err(|e| map_error(&e))?;
    let Some(video) = session
        .streams()
        .iter()
        .position(|s| s.media() == "video" && s.encoding_name().eq_ignore_ascii_case("h264"))
    else {
        return Err(CameraError::Other {
            message: "The printer's camera doesn't offer H.264 video.".into(),
        });
    };
    session
        .setup(
            video,
            SetupOptions::default().transport(Transport::Tcp(TcpTransportOptions::default())),
        )
        .await
        .map_err(|e| map_error(&e))?;
    let playing = session
        .play(PlayOptions::default())
        .await
        .map_err(|e| map_error(&e))?;
    let demuxed = playing.demuxed().map_err(|e| map_error(&e))?;
    Ok((demuxed, video))
}

/// Streams the printer's H.264 to `tx`: `Frame::Config` first and whenever
/// the parameters change, then every access unit. Frames before the first
/// keyframe are dropped. Returns why it ended.
pub async fn run(target: &CameraTarget, tx: mpsc::Sender<Frame>) -> CameraError {
    run_with(target, tx, FIRST_FRAME_WAIT).await
}

/// `run` with the wait for the first video frame (RTSP handshake included)
/// given. Past it, the printer being silent is `Busy`: connected, but no
/// picture in time.
async fn run_with(
    target: &CameraTarget,
    tx: mpsc::Sender<Frame>,
    first_wait: Duration,
) -> CameraError {
    let deadline = tokio::time::Instant::now() + first_wait;
    let relay = match relay(target).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (mut demuxed, video) =
        match tokio::time::timeout_at(deadline, handshake(target, &relay)).await {
            Err(_) => return CameraError::Busy,
            Ok(Err(e)) => return e,
            Ok(Ok(started)) => started,
        };
    let mut sent_config = false;
    let mut seen_key = false;
    let mut got_frame = false;
    loop {
        let wait = if got_frame {
            tokio::time::Instant::now() + STALL
        } else {
            deadline
        };
        let item = match tokio::time::timeout_at(wait, demuxed.next()).await {
            Err(_) if !got_frame => return CameraError::Busy,
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
            Ok(Some(Err(e))) => return map_error(&e),
            Ok(Some(Ok(item))) => item,
        };
        let CodecItem::VideoFrame(frame) = item else {
            continue;
        };
        got_frame = true;
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
    fn only_a_401_is_a_wrong_access_code() {
        assert_eq!(map_status(Some(401)), CameraError::Auth);
        assert!(matches!(map_status(Some(404)), CameraError::Other { .. }));
        assert!(matches!(map_status(None), CameraError::Other { .. }));
    }

    #[tokio::test]
    async fn a_camera_that_never_answers_rtsp_is_busy() {
        let cam = FakeCamera::start(Mode::Silent).await;
        let (tx, _rx) = mpsc::channel(4);
        let err = run_with(
            &test_target(&cam, Transport::Rtsps),
            tx,
            std::time::Duration::from_millis(200),
        )
        .await;
        assert_eq!(err, CameraError::Busy);
    }
}
