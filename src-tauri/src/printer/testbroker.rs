//! A minimal in-process MQTT 3.1.1 broker over TLS, standing in for a
//! printer in tests. It checks the `bblp` password, acknowledges
//! subscriptions, records every publish, answers `pushall` with a canned
//! report and `get_version` with a canned version reply, and can drop all
//! connections to exercise reconnect.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use rumqttc::{ConnAck, ConnectReturnCode, Packet, Publish, QoS, SubAck, SubscribeReasonCode};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;

use super::tls::testpki::TestLeaf;

const MAX_PACKET: usize = 1 << 20;

#[derive(Default)]
struct Shared {
    published: Mutex<Vec<(String, Vec<u8>)>>,
    subscriptions: Mutex<Vec<String>>,
    version_reply: Mutex<Option<Vec<u8>>>,
    connections: AtomicUsize,
    disconnects: AtomicUsize,
    /// Logged-in connections still open.
    open: AtomicUsize,
    client_ids: Mutex<Vec<String>>,
}

pub struct FakeBroker {
    pub addr: SocketAddr,
    shared: Arc<Shared>,
    kill: watch::Sender<u64>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeBroker {
    fn drop(&mut self) {
        self.accept_task.abort();
        self.kill.send_modify(|g| *g += 1);
    }
}

impl FakeBroker {
    /// Serves `leaf` on 127.0.0.1 with an ephemeral port.
    pub async fn start(leaf: &TestLeaf, serial: &str, password: &str, pushall_reply: &str) -> Self {
        let any = SocketAddr::from(([127, 0, 0, 1], 0));
        Self::start_at(any, leaf, serial, password, pushall_reply).await
    }

    /// `start` on a given address, e.g. a port a client is already retrying.
    pub async fn start_at(
        addr: SocketAddr,
        leaf: &TestLeaf,
        serial: &str,
        password: &str,
        pushall_reply: &str,
    ) -> Self {
        let certs = vec![CertificateDer::from(leaf.cert_der.clone())];
        let key = PrivateKeyDer::from_pem_slice(leaf.key_pem.as_bytes()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(super::tls::crypto_provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind(addr).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared::default());
        let (kill, _) = watch::channel(0u64);

        let report_topic = format!("device/{serial}/report");
        let password = password.to_string();
        let reply = pushall_reply.as_bytes().to_vec();
        let accept_shared = shared.clone();
        let accept_kill = kill.clone();
        let accept_task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let acceptor = acceptor.clone();
                let shared = accept_shared.clone();
                let mut kill_rx = accept_kill.subscribe();
                let (topic, password, reply) =
                    (report_topic.clone(), password.clone(), reply.clone());
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    tokio::select! {
                        _ = serve(tls, &shared, &topic, &password, &reply) => {}
                        _ = kill_rx.changed() => {}
                    }
                });
            }
        });
        Self {
            addr,
            shared,
            kill,
            accept_task,
        }
    }

    /// Also answer `get_version` with this payload.
    pub fn with_version_reply(self, payload: &str) -> Self {
        *self.shared.version_reply.lock().unwrap() = Some(payload.as_bytes().to_vec());
        self
    }

    pub fn published(&self) -> Vec<(String, Vec<u8>)> {
        self.shared.published.lock().unwrap().clone()
    }

    pub fn subscriptions(&self) -> Vec<String> {
        self.shared.subscriptions.lock().unwrap().clone()
    }

    /// Connections that passed the password check.
    pub fn connection_count(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }

    /// Logged-in connections that are still open.
    pub fn open_connections(&self) -> usize {
        self.shared.open.load(Ordering::SeqCst)
    }

    /// Clean MQTT DISCONNECT packets received.
    pub fn disconnect_count(&self) -> usize {
        self.shared.disconnects.load(Ordering::SeqCst)
    }

    /// The client id of every connection that passed the password check.
    pub fn client_ids(&self) -> Vec<String> {
        self.shared.client_ids.lock().unwrap().clone()
    }

    /// Closes every open connection, as a printer reboot would.
    pub fn drop_connections(&self) {
        self.kill.send_modify(|g| *g += 1);
    }
}

/// Counts a connection as open until `serve` returns or is dropped.
struct OpenGuard<'a>(&'a AtomicUsize);

impl Drop for OpenGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn serve<S>(mut stream: S, shared: &Shared, report_topic: &str, password: &str, reply: &[u8])
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut buf = BytesMut::new();
    let Some(Packet::Connect(connect)) = read_packet(&mut stream, &mut buf).await else {
        return;
    };
    let ok = connect
        .login
        .as_ref()
        .is_some_and(|l| l.username == "bblp" && l.password == password);
    let code = if ok {
        ConnectReturnCode::Success
    } else {
        ConnectReturnCode::NotAuthorized
    };
    if write_packet(&mut stream, Packet::ConnAck(ConnAck::new(code, false)))
        .await
        .is_err()
        || !ok
    {
        return;
    }
    shared.connections.fetch_add(1, Ordering::SeqCst);
    shared.open.fetch_add(1, Ordering::SeqCst);
    let _open = OpenGuard(&shared.open);
    shared
        .client_ids
        .lock()
        .unwrap()
        .push(connect.client_id.clone());
    while let Some(packet) = read_packet(&mut stream, &mut buf).await {
        let answer =
            match packet {
                Packet::Subscribe(s) => {
                    let mut subs = shared.subscriptions.lock().unwrap();
                    for f in &s.filters {
                        if !subs.contains(&f.path) {
                            subs.push(f.path.clone());
                        }
                    }
                    let codes = s
                        .filters
                        .iter()
                        .map(|_| SubscribeReasonCode::Success(QoS::AtMostOnce))
                        .collect();
                    Some(Packet::SubAck(SubAck::new(s.pkid, codes)))
                }
                Packet::Publish(p) => {
                    let body = p.payload.to_vec();
                    shared
                        .published
                        .lock()
                        .unwrap()
                        .push((p.topic.clone(), body.clone()));
                    let text = String::from_utf8_lossy(&body);
                    if text.contains("\"pushall\"") {
                        Some(Packet::Publish(Publish::new(
                            report_topic,
                            QoS::AtMostOnce,
                            reply.to_vec(),
                        )))
                    } else if text.contains("\"get_version\"") {
                        shared.version_reply.lock().unwrap().clone().map(|v| {
                            Packet::Publish(Publish::new(report_topic, QoS::AtMostOnce, v))
                        })
                    } else {
                        None
                    }
                }
                Packet::PingReq => Some(Packet::PingResp),
                Packet::Disconnect => {
                    shared.disconnects.fetch_add(1, Ordering::SeqCst);
                    return;
                }
                _ => None,
            };
        if let Some(answer) = answer {
            if write_packet(&mut stream, answer).await.is_err() {
                return;
            }
        }
    }
}

async fn read_packet<S: tokio::io::AsyncRead + Unpin>(
    stream: &mut S,
    buf: &mut BytesMut,
) -> Option<Packet> {
    loop {
        match Packet::read(buf, MAX_PACKET) {
            Ok(p) => return Some(p),
            Err(rumqttc::Error::InsufficientBytes(_)) => {
                let mut chunk = [0u8; 4096];
                let n = stream.read(&mut chunk).await.ok()?;
                if n == 0 {
                    return None;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(_) => return None,
        }
    }
}

async fn write_packet<S: tokio::io::AsyncWrite + Unpin>(
    stream: &mut S,
    packet: Packet,
) -> std::io::Result<()> {
    let mut out = BytesMut::new();
    packet
        .write(&mut out, MAX_PACKET)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    stream.write_all(&out).await?;
    stream.flush().await
}
