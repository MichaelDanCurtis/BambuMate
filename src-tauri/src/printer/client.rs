//! The MQTT connection to the printer: TLS, subscribe, the two read
//! requests, and reconnect with backoff.
//!
//! Its only output is `ClientEvent`s: raw report payloads and connection
//! state changes. It publishes nothing but `pushall` and `get_version`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rumqttc::{
    AsyncClient, ConnectReturnCode, ConnectionError, Event, MqttOptions, Packet, QoS,
    TlsConfiguration, Transport,
};
use rustls::ClientConfig;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;

use super::tls::{Rejection, RejectionSlot};

/// The printer's MQTT-over-TLS port.
pub const MQTT_PORT: u16 = 8883;
/// The LAN-mode MQTT user.
pub const MQTT_USER: &str = "bblp";
/// The only commands BambuMate ever publishes.
pub const READ_ONLY_COMMANDS: &[&str] = &["pushall", "get_version"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    AuthFailed,
    CertUntrusted { fingerprint: String },
    WrongSerial { presented: String },
    Unreachable,
}

impl ConnectionState {
    /// A state that retrying won't fix without the user changing something.
    pub fn needs_user(&self) -> bool {
        matches!(
            self,
            Self::AuthFailed | Self::CertUntrusted { .. } | Self::WrongSerial { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    Connection(ConnectionState),
    /// A raw payload from `device/<serial>/report`.
    Report(Vec<u8>),
}

/// What the client needs to reach one printer.
#[derive(Clone)]
pub struct ClientParams {
    pub host: String,
    pub port: u16,
    pub serial: String,
    pub access_code: String,
    pub tls: Arc<ClientConfig>,
    pub rejection: RejectionSlot,
}

// Written by hand so the access code can never reach a log line.
impl std::fmt::Debug for ClientParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("serial", &self.serial)
            .field("access_code", &"<redacted>")
            .finish()
    }
}

/// Reconnect and request pacing. `Timing::default()` is the production value.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    pub pushall_interval: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            backoff_min: Duration::from_secs(2),
            backoff_max: Duration::from_secs(60),
            pushall_interval: Duration::from_secs(300),
        }
    }
}

pub fn report_topic(serial: &str) -> String {
    format!("device/{serial}/report")
}

pub fn request_topic(serial: &str) -> String {
    format!("device/{serial}/request")
}

pub fn pushall_request(seq: u64) -> String {
    json!({"pushing": {"sequence_id": seq.to_string(), "command": "pushall", "version": 1, "push_target": 1}})
        .to_string()
}

pub fn get_version_request(seq: u64) -> String {
    json!({"info": {"sequence_id": seq.to_string(), "command": "get_version"}}).to_string()
}

/// Exponential backoff: `min`, `2·min`, … up to `max`.
#[derive(Debug, Clone)]
pub struct Backoff {
    next: Duration,
    min: Duration,
    max: Duration,
}

impl Backoff {
    pub fn new(min: Duration, max: Duration) -> Self {
        Self {
            next: min,
            min,
            max,
        }
    }
    pub fn next_delay(&mut self) -> Duration {
        let d = self.next;
        self.next = (self.next * 2).min(self.max);
        d
    }
    pub fn reset(&mut self) {
        self.next = self.min;
    }
}

/// Allows one `pushall` per interval, across reconnects.
#[derive(Debug, Clone)]
pub struct PushallGate {
    last: Option<Instant>,
    interval: Duration,
}

impl PushallGate {
    pub fn new(interval: Duration) -> Self {
        Self {
            last: None,
            interval,
        }
    }
    pub fn try_take(&mut self, now: Instant) -> bool {
        match self.last {
            Some(t) if now.duration_since(t) < self.interval => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

/// Connects, and reconnects with backoff, until `events` is closed or the
/// task is aborted.
pub async fn run(params: ClientParams, events: mpsc::Sender<ClientEvent>, timing: Timing) {
    let mut backoff = Backoff::new(timing.backoff_min, timing.backoff_max);
    let mut gate = PushallGate::new(timing.pushall_interval);
    let mut seq: u64 = 0;
    loop {
        if events
            .send(ClientEvent::Connection(ConnectionState::Connecting))
            .await
            .is_err()
        {
            return;
        }
        let ended = connect_once(&params, &events, &mut gate, &mut seq, &mut backoff).await;
        let Some(state) = ended else { return };
        tracing::debug!(serial = %params.serial, ?state, "printer connection ended");
        if events.send(ClientEvent::Connection(state)).await.is_err() {
            return;
        }
        tokio::time::sleep(backoff.next_delay()).await;
    }
}

/// One connection attempt. `None` means the receiver is gone.
async fn connect_once(
    params: &ClientParams,
    events: &mpsc::Sender<ClientEvent>,
    gate: &mut PushallGate,
    seq: &mut u64,
    backoff: &mut Backoff,
) -> Option<ConnectionState> {
    let client_id = format!(
        "bambumate-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let mut opts = MqttOptions::new(client_id, params.host.clone(), params.port);
    opts.set_credentials(MQTT_USER, params.access_code.clone());
    opts.set_keep_alive(Duration::from_secs(30));
    // A full H2 push is ~30 KB; rumqttc's default limit is 10 KB.
    opts.set_max_packet_size(1 << 20, 1 << 16);
    opts.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(
        params.tls.clone(),
    )));
    let (client, mut eventloop) = AsyncClient::new(opts, 10);
    let report = report_topic(&params.serial);
    let request = request_topic(&params.serial);
    let mut connected = false;
    params.rejection.take();
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                connected = true;
                backoff.reset();
                events
                    .send(ClientEvent::Connection(ConnectionState::Connected))
                    .await
                    .ok()?;
                let _ = client.try_subscribe(report.clone(), QoS::AtMostOnce);
                *seq += 1;
                let _ = client.try_publish(
                    request.clone(),
                    QoS::AtMostOnce,
                    false,
                    get_version_request(*seq),
                );
                if gate.try_take(Instant::now()) {
                    *seq += 1;
                    let _ = client.try_publish(
                        request.clone(),
                        QoS::AtMostOnce,
                        false,
                        pushall_request(*seq),
                    );
                }
            }
            Ok(Event::Incoming(Packet::Publish(p))) if p.topic == report => {
                events
                    .send(ClientEvent::Report(p.payload.to_vec()))
                    .await
                    .ok()?;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(serial = %params.serial, "printer MQTT error: {e}");
                return Some(classify(&e, params.rejection.take(), connected));
            }
        }
    }
}

fn classify(
    err: &ConnectionError,
    rejection: Option<Rejection>,
    was_connected: bool,
) -> ConnectionState {
    match (err, rejection) {
        (
            ConnectionError::ConnectionRefused(
                ConnectReturnCode::BadUserNamePassword | ConnectReturnCode::NotAuthorized,
            ),
            _,
        ) => ConnectionState::AuthFailed,
        (_, Some(Rejection::Untrusted { fingerprint })) => {
            ConnectionState::CertUntrusted { fingerprint }
        }
        (_, Some(Rejection::WrongSerial { presented })) => {
            ConnectionState::WrongSerial { presented }
        }
        _ if was_connected => ConnectionState::Disconnected,
        _ => ConnectionState::Unreachable,
    }
}

/// The result of Settings → Printer → Test connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestOutcome {
    pub connection: ConnectionState,
    /// True once a full status report arrived.
    pub got_report: bool,
    /// The model from `get_version`, e.g. `H2D`.
    pub model: Option<String>,
}

/// Connects once, waits for the first full report or a failure, then
/// disconnects.
pub async fn test_connection(params: ClientParams, timing: Timing, wait: Duration) -> TestOutcome {
    let (tx, mut rx) = mpsc::channel(64);
    let task = tokio::spawn(run(params, tx, timing));
    let mut outcome = TestOutcome {
        connection: ConnectionState::Unreachable,
        got_report: false,
        model: None,
    };
    let deadline = tokio::time::Instant::now() + wait;
    while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        match ev {
            ClientEvent::Connection(ConnectionState::Connecting) => {}
            ClientEvent::Connection(state) => {
                let done = state != ConnectionState::Connected;
                outcome.connection = state;
                if done {
                    break;
                }
            }
            ClientEvent::Report(bytes) => match super::state::parse_report(&bytes) {
                Ok(super::state::Report::Version { model, .. }) => outcome.model = model,
                Ok(super::state::Report::Status { full: true, .. }) => {
                    outcome.got_report = true;
                    // get_version is sent first, so its answer is usually in.
                    let grace = tokio::time::Instant::now() + Duration::from_millis(500);
                    while let Ok(Some(ClientEvent::Report(b))) =
                        tokio::time::timeout_at(grace, rx.recv()).await
                    {
                        if let Ok(super::state::Report::Version { model, .. }) =
                            super::state::parse_report(&b)
                        {
                            outcome.model = model;
                        }
                    }
                    break;
                }
                _ => {}
            },
        }
    }
    task.abort();
    outcome
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::state::fixtures::{GET_VERSION_H2D, H2D_FULL};
    use crate::printer::testbroker::FakeBroker;
    use crate::printer::tls::testpki::TestCa;
    use crate::printer::tls::{client_config, fingerprint, PrinterCertVerifier};

    const SERIAL: &str = "0948AB000000001";
    const CODE: &str = "12345678";

    fn fast() -> Timing {
        Timing {
            backoff_min: Duration::from_millis(50),
            backoff_max: Duration::from_millis(200),
            pushall_interval: Duration::from_secs(300),
        }
    }

    fn params(ca: &TestCa, broker: &FakeBroker, code: &str, pin: Option<&str>) -> ClientParams {
        let rejection = RejectionSlot::default();
        let verifier =
            PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, pin, rejection.clone()).unwrap();
        ClientParams {
            host: "127.0.0.1".into(),
            port: broker.addr.port(),
            serial: SERIAL.into(),
            access_code: code.into(),
            tls: client_config(Arc::new(verifier)).unwrap(),
            rejection,
        }
    }

    async fn next_state(rx: &mut mpsc::Receiver<ClientEvent>) -> ConnectionState {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(ClientEvent::Connection(ConnectionState::Connecting))) => {}
                Ok(Some(ClientEvent::Connection(s))) => return s,
                Ok(Some(ClientEvent::Report(_))) => {}
                other => panic!("no state change: {other:?}"),
            }
        }
    }

    async fn next_report(rx: &mut mpsc::Receiver<ClientEvent>) -> Vec<u8> {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(ClientEvent::Report(b))) => return b,
                Ok(Some(_)) => {}
                other => panic!("no report: {other:?}"),
            }
        }
    }

    #[test]
    fn requests_are_the_documented_read_only_json() {
        let v: serde_json::Value = serde_json::from_str(&pushall_request(7)).unwrap();
        assert_eq!(
            v,
            json!({"pushing":{"sequence_id":"7","command":"pushall","version":1,"push_target":1}})
        );
        let v: serde_json::Value = serde_json::from_str(&get_version_request(8)).unwrap();
        assert_eq!(
            v,
            json!({"info":{"sequence_id":"8","command":"get_version"}})
        );
    }

    #[test]
    fn backoff_doubles_from_two_seconds_to_a_minute() {
        let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
        let seen: Vec<u64> = (0..7).map(|_| b.next_delay().as_secs()).collect();
        assert_eq!(seen, vec![2, 4, 8, 16, 32, 60, 60]);
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(2));
    }

    #[test]
    fn pushall_is_allowed_once_per_interval() {
        let mut g = PushallGate::new(Duration::from_secs(300));
        let t0 = Instant::now();
        assert!(g.try_take(t0));
        assert!(!g.try_take(t0 + Duration::from_secs(299)));
        assert!(g.try_take(t0 + Duration::from_secs(300)));
    }

    #[test]
    fn debug_output_never_contains_the_access_code() {
        let ca = TestCa::new("CA");
        let v =
            PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, RejectionSlot::default())
                .unwrap();
        let p = ClientParams {
            host: "10.0.0.2".into(),
            port: MQTT_PORT,
            serial: SERIAL.into(),
            access_code: "SECRET99".into(),
            tls: client_config(Arc::new(v)).unwrap(),
            rejection: RejectionSlot::default(),
        };
        assert!(!format!("{p:?}").contains("SECRET99"));
    }

    #[tokio::test]
    async fn connects_subscribes_requests_and_forwards_reports() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, CODE, None), tx, fast()));

        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        let report = next_report(&mut rx).await;
        assert!(matches!(
            crate::printer::state::parse_report(&report).unwrap(),
            crate::printer::state::Report::Status { full: true, .. }
        ));
        assert_eq!(broker.subscriptions(), vec![report_topic(SERIAL)]);
        let published = broker.published();
        assert!(published
            .iter()
            .all(|(topic, _)| *topic == request_topic(SERIAL)));
        let commands: Vec<String> = published
            .iter()
            .map(|(_, body)| {
                let v: serde_json::Value = serde_json::from_slice(body).unwrap();
                let inner = v.as_object().unwrap().values().next().unwrap();
                inner["command"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(commands, vec!["get_version", "pushall"]);
        assert!(commands
            .iter()
            .all(|c| READ_ONLY_COMMANDS.contains(&c.as_str())));
        task.abort();
    }

    #[tokio::test]
    async fn reconnects_after_the_broker_drops_without_a_second_pushall() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, CODE, None), tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        next_report(&mut rx).await;

        broker.drop_connections();
        assert_eq!(next_state(&mut rx).await, ConnectionState::Disconnected);
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        assert_eq!(broker.connection_count(), 2);
        // Wait until the second connection's get_version has arrived.
        for _ in 0..50 {
            if broker.published().len() >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let pushalls = broker
            .published()
            .iter()
            .filter(|(_, b)| String::from_utf8_lossy(b).contains("pushall"))
            .count();
        assert_eq!(pushalls, 1, "pushall is limited to once per 5 minutes");
        task.abort();
    }

    #[tokio::test]
    async fn a_wrong_access_code_is_auth_failed() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, "00000000", None), tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::AuthFailed);
        task.abort();
    }

    #[tokio::test]
    async fn an_unknown_ca_is_cert_untrusted_with_the_fingerprint() {
        let trusted = TestCa::new("Test Printer CA");
        let other = TestCa::new("Unknown CA");
        let leaf = other.leaf(SERIAL);
        let broker = FakeBroker::start(&leaf, SERIAL, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&trusted, &broker, CODE, None), tx, fast()));
        assert_eq!(
            next_state(&mut rx).await,
            ConnectionState::CertUntrusted {
                fingerprint: fingerprint(&leaf.cert_der)
            }
        );
        task.abort();
    }

    #[tokio::test]
    async fn nothing_listening_is_unreachable() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL).await;
        let mut p = params(&ca, &broker, CODE, None);
        drop(broker);
        // A port nothing listens on.
        let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        p.port = spare.local_addr().unwrap().port();
        drop(spare);
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(p, tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::Unreachable);
        task.abort();
    }

    #[tokio::test]
    async fn test_connection_reports_the_model_and_a_full_report() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL)
            .await
            .with_version_reply(GET_VERSION_H2D);
        let out = test_connection(
            params(&ca, &broker, CODE, None),
            fast(),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(out.connection, ConnectionState::Connected);
        assert!(out.got_report);
        assert_eq!(out.model.as_deref(), Some("H2D"));
    }

    #[tokio::test]
    async fn test_connection_stops_at_a_rejected_code() {
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, CODE, H2D_FULL).await;
        let out = test_connection(
            params(&ca, &broker, "wrong", None),
            fast(),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(out.connection, ConnectionState::AuthFailed);
        assert!(!out.got_report);
    }
}
