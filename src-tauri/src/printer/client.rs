//! The MQTT connection to the printer: TLS, subscribe, the two read
//! requests, and reconnect with backoff.
//!
//! Its only output is `ClientEvent`s: raw report payloads and connection
//! state changes. It publishes nothing but `pushall` and `get_version`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
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
    /// `min` is clamped to at least 1 ms (a zero `min` would double to zero
    /// forever and spin), and `max` to at least `min`.
    pub fn new(min: Duration, max: Duration) -> Self {
        let min = min.max(Duration::from_millis(1));
        let max = max.max(min);
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

/// When each printer last got a `pushall`, process-wide. The limit belongs
/// to the printer, not to one connection: it must hold across reconnects,
/// restarts of the client, and Test connection.
static LAST_PUSHALL: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Allows one `pushall` per interval for one printer, shared by every
/// `run` and `test_connection` on that serial.
#[derive(Debug, Clone)]
pub struct PushallGate {
    serial: String,
    interval: Duration,
}

impl PushallGate {
    pub fn new(serial: &str, interval: Duration) -> Self {
        Self {
            serial: serial.to_string(),
            interval,
        }
    }

    fn last(&self) -> std::sync::MutexGuard<'static, HashMap<String, Instant>> {
        LAST_PUSHALL.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records a `pushall` at `now` and returns true, or returns false when
    /// one was sent less than an interval ago.
    pub fn try_take(&self, now: Instant) -> bool {
        let mut last = self.last();
        match last.get(&self.serial) {
            Some(t) if now.saturating_duration_since(*t) < self.interval => false,
            _ => {
                last.insert(self.serial.clone(), now);
                true
            }
        }
    }

    /// True if `try_take(now)` would succeed. Takes nothing.
    pub fn is_open(&self, now: Instant) -> bool {
        self.reopens_at(now).is_none()
    }

    /// When the gate next opens, or `None` if it is open at `now`.
    pub fn reopens_at(&self, now: Instant) -> Option<Instant> {
        let at = *self.last().get(&self.serial)? + self.interval;
        (at > now).then_some(at)
    }
}

/// Connects, and reconnects with backoff, until `events` is closed or the
/// task is aborted. A state that needs the user (`needs_user()`: wrong
/// access code, untrusted certificate, wrong serial) is sent once and then
/// `run` returns: retrying can't fix it, so the caller restarts the client
/// after the user saves a new code or trusts the printer.
pub async fn run(params: ClientParams, events: mpsc::Sender<ClientEvent>, timing: Timing) {
    let mut backoff = Backoff::new(timing.backoff_min, timing.backoff_max);
    let gate = PushallGate::new(&params.serial, timing.pushall_interval);
    let mut seq: u64 = 0;
    // One id for every reconnect, so the printer replaces a stale session
    // instead of holding a second slot.
    let client_id = format!(
        "bambumate-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    loop {
        if events
            .send(ClientEvent::Connection(ConnectionState::Connecting))
            .await
            .is_err()
        {
            return;
        }
        let ended = connect_once(&params, &client_id, &events, &gate, &mut seq, &mut backoff).await;
        let Some(state) = ended else { return };
        tracing::debug!(serial = %params.serial, ?state, "printer connection ended");
        let stop = state.needs_user();
        if events.send(ClientEvent::Connection(state)).await.is_err() || stop {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(backoff.next_delay()) => {}
            _ = events.closed() => return,
        }
    }
}

/// Sleeps until `at`; never completes when `at` is `None`.
async fn sleep_until_opt(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
        None => std::future::pending().await,
    }
}

/// Sends `pushall` if the printer's gate allows it. Returns whether it did.
fn try_pushall(client: &AsyncClient, request: &str, gate: &PushallGate, seq: &mut u64) -> bool {
    if !gate.try_take(Instant::now()) {
        return false;
    }
    *seq += 1;
    let _ = client.try_publish(
        request.to_string(),
        QoS::AtMostOnce,
        false,
        pushall_request(*seq),
    );
    true
}

/// One connection attempt. `None` means the receiver is gone.
async fn connect_once(
    params: &ClientParams,
    client_id: &str,
    events: &mpsc::Sender<ClientEvent>,
    gate: &PushallGate,
    seq: &mut u64,
    backoff: &mut Backoff,
) -> Option<ConnectionState> {
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
    let mut got_report = false;
    // When the gate refused a pushall at ConnAck: send one as soon as it
    // reopens, so a quick reconnect doesn't leave the state stale.
    let mut deferred: Option<Instant> = None;
    params.rejection.take();
    loop {
        tokio::select! {
            _ = events.closed() => {
                disconnect(&client, &mut eventloop).await;
                return None;
            }
            _ = sleep_until_opt(deferred), if deferred.is_some() => {
                deferred = None;
                if connected && !try_pushall(&client, &request, gate, seq) {
                    deferred = gate.reopens_at(Instant::now());
                }
            }
            polled = eventloop.poll() => match polled {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    connected = true;
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
                    if !try_pushall(&client, &request, gate, seq) {
                        deferred = gate.reopens_at(Instant::now());
                    }
                }
                Ok(Event::Incoming(Packet::Publish(p))) if p.topic == report => {
                    // A connection that delivers data is a healthy one, so only
                    // now does the reconnect delay start over.
                    if !got_report {
                        got_report = true;
                        backoff.reset();
                    }
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
}

/// Sends MQTT DISCONNECT and gives the event loop a moment to flush it, so
/// the printer frees the connection slot at once.
async fn disconnect(client: &AsyncClient, eventloop: &mut rumqttc::EventLoop) {
    if client.try_disconnect().is_err() {
        return;
    }
    let flush = async {
        while let Ok(event) = eventloop.poll().await {
            if matches!(event, Event::Outgoing(rumqttc::Outgoing::Disconnect)) {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(1), flush).await;
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

/// Connects once, waits for a report or a failure, then disconnects.
///
/// What counts as "a report" depends on the printer's `pushall` limit (one
/// per interval, shared with the live connection):
/// - If this call can send `pushall`, it waits for the first full status
///   report, and `got_report` means exactly that.
/// - If the limit blocks `pushall` (the live connection just sent one), no
///   full report is coming, so it waits for any parsed report, a status
///   push or the `get_version` answer, and `got_report` means the printer
///   answered us.
///
/// Either way `connection == Connected` is what proves the address, access
/// code and certificate; if no report arrives before `wait` it stays
/// `Connected` with `got_report == false`.
pub async fn test_connection(params: ClientParams, timing: Timing, wait: Duration) -> TestOutcome {
    let full_report_expected =
        PushallGate::new(&params.serial, timing.pushall_interval).is_open(Instant::now());
    let (tx, mut rx) = mpsc::channel(64);
    let mut task = tokio::spawn(run(params, tx, timing));
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
            ClientEvent::Report(bytes) => {
                let finished = match super::state::parse_report(&bytes) {
                    Ok(super::state::Report::Version { model, .. }) => {
                        outcome.model = model;
                        !full_report_expected
                    }
                    Ok(super::state::Report::Status { full, .. }) => full || !full_report_expected,
                    Ok(super::state::Report::Other) | Err(_) => false,
                };
                if finished {
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
            }
        }
    }
    // Dropping the receiver makes `run` send an MQTT DISCONNECT and return,
    // freeing the printer's connection slot. Abort only if that stalls.
    drop(rx);
    if tokio::time::timeout(Duration::from_secs(2), &mut task)
        .await
        .is_err()
    {
        tracing::debug!("printer test connection did not close in time");
        task.abort();
    }
    outcome
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::printer::state::fixtures::{GET_VERSION_H2D, H2D_FULL};
    use crate::printer::testbroker::FakeBroker;
    use crate::printer::tls::testpki::TestCa;
    use crate::printer::tls::{client_config, fingerprint, PrinterCertVerifier};

    const CODE: &str = "12345678";

    /// The pushall limit is process-wide and keyed by serial, so every test
    /// uses its own serial and tests can't starve each other of a pushall.
    fn unique_serial() -> String {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        format!("0948AB{:09}", NEXT.fetch_add(1, Ordering::SeqCst))
    }

    fn fast() -> Timing {
        Timing {
            backoff_min: Duration::from_millis(50),
            backoff_max: Duration::from_millis(200),
            pushall_interval: Duration::from_secs(300),
        }
    }

    fn params(
        ca: &TestCa,
        broker: &FakeBroker,
        serial: &str,
        code: &str,
        pin: Option<&str>,
    ) -> ClientParams {
        let rejection = RejectionSlot::default();
        let verifier =
            PrinterCertVerifier::with_trust(&ca.pem, &[], serial, pin, rejection.clone()).unwrap();
        ClientParams {
            host: "127.0.0.1".into(),
            port: broker.addr.port(),
            serial: serial.into(),
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

    /// Asserts `run` has returned: no event arrives and the channel closes.
    async fn assert_run_ended(rx: &mut mpsc::Receiver<ClientEvent>) {
        match tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(None) => {}
            other => panic!("expected the client to stop, got {other:?}"),
        }
    }

    fn pushall_count(broker: &FakeBroker) -> usize {
        broker
            .published()
            .iter()
            .filter(|(_, b)| String::from_utf8_lossy(b).contains("pushall"))
            .count()
    }

    /// Polls `cond` for up to 5 s; fails the test on timeout.
    async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
        for _ in 0..250 {
            if cond() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("timed out waiting for {what}");
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
    fn backoff_never_delays_less_than_a_millisecond() {
        let mut b = Backoff::new(Duration::ZERO, Duration::ZERO);
        assert_eq!(b.next_delay(), Duration::from_millis(1));
        assert_eq!(b.next_delay(), Duration::from_millis(1));
        let mut b = Backoff::new(Duration::ZERO, Duration::from_millis(4));
        let seen: Vec<u128> = (0..4).map(|_| b.next_delay().as_millis()).collect();
        assert_eq!(seen, vec![1, 2, 4, 4]);
    }

    #[test]
    fn pushall_is_allowed_once_per_interval() {
        let g = PushallGate::new(&unique_serial(), Duration::from_secs(300));
        let t0 = Instant::now();
        assert!(g.is_open(t0));
        assert!(g.try_take(t0));
        assert!(!g.is_open(t0));
        assert!(!g.try_take(t0 + Duration::from_secs(299)));
        assert_eq!(g.reopens_at(t0), Some(t0 + Duration::from_secs(300)));
        assert!(g.try_take(t0 + Duration::from_secs(300)));
    }

    #[test]
    fn the_pushall_limit_is_shared_per_serial_across_gates() {
        let serial = unique_serial();
        let a = PushallGate::new(&serial, Duration::from_secs(300));
        let b = PushallGate::new(&serial, Duration::from_secs(300));
        let other = PushallGate::new(&unique_serial(), Duration::from_secs(300));
        let t0 = Instant::now();
        assert!(a.try_take(t0));
        assert!(!b.try_take(t0), "a second handle on the same printer");
        assert!(other.try_take(t0), "a different printer is unaffected");
    }

    #[test]
    fn debug_output_never_contains_the_access_code() {
        let ca = TestCa::new("CA");
        let serial = unique_serial();
        let v =
            PrinterCertVerifier::with_trust(&ca.pem, &[], &serial, None, RejectionSlot::default())
                .unwrap();
        let p = ClientParams {
            host: "10.0.0.2".into(),
            port: MQTT_PORT,
            serial,
            access_code: "SECRET99".into(),
            tls: client_config(Arc::new(v)).unwrap(),
            rejection: RejectionSlot::default(),
        };
        assert!(!format!("{p:?}").contains("SECRET99"));
    }

    #[tokio::test]
    async fn connects_subscribes_requests_and_forwards_reports() {
        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, &serial, CODE, None), tx, fast()));

        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        let report = next_report(&mut rx).await;
        assert!(matches!(
            crate::printer::state::parse_report(&report).unwrap(),
            crate::printer::state::Report::Status { full: true, .. }
        ));
        assert_eq!(broker.subscriptions(), vec![report_topic(&serial)]);
        let published = broker.published();
        assert!(published
            .iter()
            .all(|(topic, _)| *topic == request_topic(&serial)));
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
    async fn a_full_size_report_over_the_default_packet_limit_is_received() {
        // rumqttc's default incoming limit is 10 KB; a real H2 push is ~30 KB.
        let mut report: serde_json::Value = serde_json::from_str(H2D_FULL).unwrap();
        report["print"]["padding"] = "x".repeat(40_000).into();
        let big = report.to_string();
        assert!(big.len() > 40_000);

        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, &big).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, &serial, CODE, None), tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        let received = next_report(&mut rx).await;
        assert_eq!(received.len(), big.len());
        assert!(matches!(
            crate::printer::state::parse_report(&received).unwrap(),
            crate::printer::state::Report::Status { full: true, .. }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn reconnects_after_the_broker_drops_without_a_second_pushall() {
        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, &serial, CODE, None), tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        next_report(&mut rx).await;

        broker.drop_connections();
        assert_eq!(next_state(&mut rx).await, ConnectionState::Disconnected);
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        assert_eq!(broker.connection_count(), 2);
        // The second connection's get_version must arrive: get_version, pushall,
        // get_version. Fail if it doesn't, so the pushall count below is final.
        eventually("the second get_version", || broker.published().len() >= 3).await;
        assert_eq!(
            pushall_count(&broker),
            1,
            "pushall is limited to once per 5 minutes"
        );
        task.abort();
    }

    #[tokio::test]
    async fn a_quick_reconnect_gets_its_pushall_once_the_interval_has_passed() {
        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, H2D_FULL).await;
        let timing = Timing {
            pushall_interval: Duration::from_millis(800),
            ..fast()
        };
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, &serial, CODE, None), tx, timing));
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        next_report(&mut rx).await;
        let first = Instant::now();

        broker.drop_connections();
        assert_eq!(next_state(&mut rx).await, ConnectionState::Disconnected);
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        eventually("the second get_version", || broker.published().len() >= 3).await;
        assert_eq!(pushall_count(&broker), 1, "refused at ConnAck, not yet due");

        eventually("the deferred pushall", || pushall_count(&broker) == 2).await;
        assert!(
            first.elapsed() >= Duration::from_millis(800),
            "the deferred pushall waited for the interval"
        );
        // Once, not repeatedly: nothing more is scheduled.
        tokio::time::sleep(Duration::from_millis(900)).await;
        assert_eq!(pushall_count(&broker), 2);
        task.abort();
    }

    #[tokio::test]
    async fn a_wrong_access_code_is_auth_failed_once_and_not_retried() {
        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(
            params(&ca, &broker, &serial, "00000000", None),
            tx,
            fast(),
        ));
        assert_eq!(next_state(&mut rx).await, ConnectionState::AuthFailed);
        // Several backoff periods pass with no new Connecting and no new event.
        assert_run_ended(&mut rx).await;
        task.await.unwrap();
    }

    #[tokio::test]
    async fn an_unknown_ca_is_cert_untrusted_with_the_fingerprint_and_not_retried() {
        let serial = unique_serial();
        let trusted = TestCa::new("Test Printer CA");
        let other = TestCa::new("Unknown CA");
        let leaf = other.leaf(&serial);
        let broker = FakeBroker::start(&leaf, &serial, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(
            params(&trusted, &broker, &serial, CODE, None),
            tx,
            fast(),
        ));
        assert_eq!(
            next_state(&mut rx).await,
            ConnectionState::CertUntrusted {
                fingerprint: fingerprint(&leaf.cert_der)
            }
        );
        assert_run_ended(&mut rx).await;
        task.await.unwrap();
    }

    #[tokio::test]
    async fn nothing_listening_is_unreachable_and_keeps_retrying() {
        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, H2D_FULL).await;
        let mut p = params(&ca, &broker, &serial, CODE, None);
        drop(broker);
        // A port nothing listens on.
        let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        p.port = spare.local_addr().unwrap().port();
        drop(spare);
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(p, tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::Unreachable);
        // Unreachable is not a user problem: it goes on trying.
        assert_eq!(next_state(&mut rx).await, ConnectionState::Unreachable);
        task.abort();
    }

    #[tokio::test]
    async fn reconnects_reuse_one_client_id() {
        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, H2D_FULL).await;
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run(params(&ca, &broker, &serial, CODE, None), tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        broker.drop_connections();
        assert_eq!(next_state(&mut rx).await, ConnectionState::Disconnected);
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        let ids = broker.client_ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], ids[1]);
        assert!(ids[0].starts_with("bambumate-"));
        task.abort();
    }

    #[tokio::test]
    async fn test_connection_reports_the_model_and_a_full_report() {
        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, H2D_FULL)
            .await
            .with_version_reply(GET_VERSION_H2D);
        let out = test_connection(
            params(&ca, &broker, &serial, CODE, None),
            fast(),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(out.connection, ConnectionState::Connected);
        assert!(out.got_report);
        assert_eq!(out.model.as_deref(), Some("H2D"));
        // It said goodbye instead of just dropping the socket.
        eventually("the MQTT disconnect", || broker.disconnect_count() == 1).await;
    }

    #[tokio::test]
    async fn test_connection_stops_at_a_rejected_code() {
        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, H2D_FULL).await;
        let out = test_connection(
            params(&ca, &broker, &serial, "wrong", None),
            fast(),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(out.connection, ConnectionState::AuthFailed);
        assert!(!out.got_report);
    }

    #[tokio::test]
    async fn test_connection_returns_promptly_when_unreachable() {
        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, H2D_FULL).await;
        let mut p = params(&ca, &broker, &serial, CODE, None);
        drop(broker);
        let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        p.port = spare.local_addr().unwrap().port();
        drop(spare);
        // The production backoff is 2 s; the call must not wait it out.
        let started = Instant::now();
        let out = test_connection(p, Timing::default(), Duration::from_secs(5)).await;
        assert_eq!(out.connection, ConnectionState::Unreachable);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn test_connection_after_a_live_pushall_sends_none_and_still_succeeds() {
        let serial = unique_serial();
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(&serial), &serial, CODE, H2D_FULL)
            .await
            .with_version_reply(GET_VERSION_H2D);
        // The live connection takes the printer's pushall.
        let (tx, mut rx) = mpsc::channel(64);
        let live = tokio::spawn(run(params(&ca, &broker, &serial, CODE, None), tx, fast()));
        assert_eq!(next_state(&mut rx).await, ConnectionState::Connected);
        next_report(&mut rx).await;
        assert_eq!(pushall_count(&broker), 1);
        live.abort();

        let out = test_connection(
            params(&ca, &broker, &serial, CODE, None),
            fast(),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(pushall_count(&broker), 1, "no second pushall within 5 min");
        // No full report was asked for, so the version answer is the proof
        // that the printer talks to us.
        assert_eq!(out.connection, ConnectionState::Connected);
        assert!(out.got_report);
        assert_eq!(out.model.as_deref(), Some("H2D"));
    }
}
