//! Owns the printer connection. Keeps the latest state and connection
//! state, and emits `printer://state` (at most twice a second) and
//! `printer://connection`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::async_runtime::JoinHandle;
use tokio::sync::{mpsc, watch};

use super::client::{self, ClientEvent, ClientParams, ConnectionState, Timing};
use super::hms::{ErrorView, HmsCatalog};
use super::settings::{valid_serial, PrinterConfig};
use super::slots::{compute_slots, preset_needs_cloud_sync, SlotView, EXTERNAL_AMS_ID};
use super::state::{parse_report, PrinterState, Report, ReportMerger};
use super::tls::{client_config, PrinterCertVerifier, RejectionSlot};
use crate::history::{RefinementHistory, SlotAssignment};

pub const STATE_EVENT: &str = "printer://state";
pub const CONNECTION_EVENT: &str = "printer://connection";
/// `printer://state` is emitted at most once per this interval.
const EMIT_INTERVAL: Duration = Duration::from_millis(500);
/// How long a replaced or stopped client gets to send its MQTT DISCONNECT
/// before it is aborted.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// Where the service's events go: the webview in the app, a recorder in tests.
pub trait PrinterEvents: Send + Sync + 'static {
    fn state(&self, view: &PrinterView);
    fn connection(&self, state: &ConnectionState);
}

pub struct TauriEvents(pub tauri::AppHandle);

impl PrinterEvents for TauriEvents {
    fn state(&self, view: &PrinterView) {
        use tauri::Emitter;
        let _ = self.0.emit(STATE_EVENT, view);
    }
    fn connection(&self, state: &ConnectionState) {
        use tauri::Emitter;
        let _ = self.0.emit(CONNECTION_EVENT, state);
    }
}

/// The printer as the Printer page shows it. Payload of `printer://state`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterView {
    pub configured: bool,
    pub printer: Option<PrinterSummary>,
    pub connection: ConnectionState,
    /// The last known state; kept while disconnected.
    pub state: Option<PrinterState>,
    pub slots: Vec<SlotView>,
    pub errors: Vec<ErrorView>,
}

impl PrinterView {
    /// The view when no printer is set up.
    pub fn unconfigured() -> Self {
        Self {
            configured: false,
            printer: None,
            connection: ConnectionState::Disconnected,
            state: None,
            slots: Vec::new(),
            errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterSummary {
    pub ip: String,
    pub serial: String,
    pub name: String,
    /// From `get_version` when connected, else from settings.
    pub model: String,
    pub firmware: Option<String>,
}

/// The client parameters for a real printer: MQTT port 8883 and Bambu's
/// bundled CAs (or the pinned fingerprint). Used by `start` and by Test
/// connection. A serial the verifier can't use is a user-facing error.
pub fn client_params(config: &PrinterConfig, access_code: String) -> Result<ClientParams, String> {
    let serial = valid_serial(&config.serial)
        .ok_or("Enter the printer's serial number (letters and digits).")?;
    let rejection = RejectionSlot::default();
    let verifier = PrinterCertVerifier::bambu(
        &serial,
        config.pinned_fingerprint.as_deref(),
        rejection.clone(),
    )
    .map_err(|e| format!("Could not set up the printer's certificate check: {e}"))?;
    Ok(ClientParams {
        host: config.ip.clone(),
        port: client::MQTT_PORT,
        serial,
        access_code,
        tls: client_config(Arc::new(verifier))?,
        rejection,
    })
}

struct Live {
    config: Option<PrinterConfig>,
    connection: ConnectionState,
    merger: ReportMerger,
    state: Option<PrinterState>,
    model: Option<String>,
    firmware: Option<String>,
    assignments: Vec<SlotAssignment>,
    /// preset_path → needs cloud sync, refreshed when assignments load.
    cloud_sync: HashMap<String, bool>,
    hms: Option<Arc<HmsCatalog>>,
    hms_loading: bool,
    /// Bumped on every start and stop, so a stopped client's late events
    /// are ignored.
    generation: u64,
}

impl Live {
    fn empty(generation: u64) -> Self {
        Self {
            config: None,
            connection: ConnectionState::Disconnected,
            merger: ReportMerger::default(),
            state: None,
            model: None,
            firmware: None,
            assignments: Vec::new(),
            cloud_sync: HashMap::new(),
            hms: None,
            hms_loading: false,
            generation,
        }
    }
}

struct Inner {
    events: Arc<dyn PrinterEvents>,
    history_db: Option<PathBuf>,
    data_dir: PathBuf,
    hms_url: String,
    timing: Timing,
    live: Mutex<Live>,
    dirty: watch::Sender<u64>,
    /// The running client, if any. Held across the whole replace or stop,
    /// so overlapping calls can't leave a second client running. Nothing
    /// awaits while holding it. Lock order: `running`, then `live`.
    running: Mutex<Option<Running>>,
}

/// One client connection's tasks.
struct Running {
    client: JoinHandle<()>,
    /// Owns the event receiver: ending it makes `client::run` send an MQTT
    /// DISCONNECT and return.
    consumer: JoinHandle<()>,
}

impl Running {
    /// Ends the consumer, which drops the receiver so the client takes its
    /// graceful DISCONNECT path. The returned future aborts the client if it
    /// hasn't ended within `STOP_GRACE`.
    fn retire(self) -> impl std::future::Future<Output = ()> + Send + 'static {
        self.consumer.abort();
        let mut client = self.client;
        async move {
            if tokio::time::timeout(STOP_GRACE, &mut client).await.is_err() {
                client.abort();
            }
        }
    }
}

#[derive(Clone)]
pub struct PrinterService {
    inner: Arc<Inner>,
}

impl PrinterService {
    /// `history_db` holds slot assignments; `data_dir` holds the HMS cache;
    /// `hms_url` is `hms::HMS_URL` in the app.
    pub fn new(
        events: Arc<dyn PrinterEvents>,
        history_db: Option<PathBuf>,
        data_dir: PathBuf,
        hms_url: &str,
        timing: Timing,
    ) -> Self {
        let (dirty, dirty_rx) = watch::channel(0u64);
        let inner = Arc::new(Inner {
            events,
            history_db,
            data_dir,
            hms_url: hms_url.to_string(),
            timing,
            live: Mutex::new(Live::empty(0)),
            dirty,
            running: Mutex::new(None),
        });
        tauri::async_runtime::spawn(emit_loop(Arc::downgrade(&inner), dirty_rx));
        Self { inner }
    }

    /// Connects to the configured printer, replacing any running connection.
    pub fn start(&self, config: PrinterConfig, access_code: String) -> Result<(), String> {
        let params = client_params(&config, access_code)?;
        self.start_with(config, params);
        Ok(())
    }

    /// `start` with explicit client parameters. Tests point it at a local broker.
    pub fn start_with(&self, config: PrinterConfig, params: ClientParams) {
        let mut running = self.inner.running.lock().unwrap();
        if let Some(old) = running.take() {
            tauri::async_runtime::spawn(old.retire());
        }
        let generation = self.configure(config);
        let (tx, rx) = mpsc::channel(256);
        *running = Some(Running {
            client: tauri::async_runtime::spawn(client::run(params, tx, self.inner.timing)),
            consumer: tauri::async_runtime::spawn(consume(self.clone(), generation, rx)),
        });
    }

    /// Resets the live state for `config` and returns the new generation.
    fn configure(&self, config: PrinterConfig) -> u64 {
        let assignments = self.load_assignments(&config.serial);
        let cloud_sync = cloud_sync_map(&assignments);
        let generation = {
            let mut live = self.inner.live.lock().unwrap();
            let generation = live.generation + 1;
            *live = Live::empty(generation);
            live.hms = Some(Arc::new(HmsCatalog::with_base_url(
                self.inner.data_dir.clone(),
                Some(&config.serial),
                &self.inner.hms_url,
            )));
            live.config = Some(config);
            live.connection = ConnectionState::Connecting;
            live.assignments = assignments;
            live.cloud_sync = cloud_sync;
            generation
        };
        self.inner.events.connection(&ConnectionState::Connecting);
        self.mark_dirty();
        generation
    }

    /// Disconnects and forgets the printer's live state. The client gets
    /// `STOP_GRACE` in the background to send its MQTT DISCONNECT.
    pub fn stop(&self) {
        if let Some(retiring) = self.stop_inner() {
            tauri::async_runtime::spawn(retiring);
        }
    }

    /// `stop` for app exit: waits up to `STOP_GRACE` for the DISCONNECT, so
    /// the printer frees the connection slot before the process ends.
    pub fn shutdown(&self) {
        if let Some(retiring) = self.stop_inner() {
            tauri::async_runtime::block_on(retiring);
        }
    }

    fn stop_inner(&self) -> Option<impl std::future::Future<Output = ()> + Send + 'static> {
        let mut running = self.inner.running.lock().unwrap();
        let retiring = running.take().map(Running::retire);
        {
            let mut live = self.inner.live.lock().unwrap();
            let generation = live.generation + 1;
            *live = Live::empty(generation);
        }
        drop(running);
        self.inner.events.connection(&ConnectionState::Disconnected);
        self.mark_dirty();
        retiring
    }

    pub fn connection(&self) -> ConnectionState {
        self.inner.live.lock().unwrap().connection.clone()
    }

    pub fn config(&self) -> Option<PrinterConfig> {
        self.inner.live.lock().unwrap().config.clone()
    }

    /// The running printer's config with its connection state, read together.
    pub fn live_connection(&self) -> Option<(PrinterConfig, ConnectionState)> {
        let live = self.inner.live.lock().unwrap();
        live.config
            .clone()
            .map(|config| (config, live.connection.clone()))
    }

    pub fn view(&self) -> PrinterView {
        let live = self.inner.live.lock().unwrap();
        let model = live
            .model
            .clone()
            .or_else(|| live.config.as_ref().map(|c| c.model.clone()))
            .unwrap_or_default();
        let slots = live
            .state
            .as_ref()
            .map(|s| {
                compute_slots(s, &live.assignments, &|path| {
                    live.cloud_sync.get(path).copied().unwrap_or(false)
                })
            })
            .unwrap_or_default();
        let errors = match (&live.state, &live.hms) {
            (Some(s), Some(hms)) => hms.describe(s, Some(model.as_str())),
            _ => Vec::new(),
        };
        PrinterView {
            configured: live.config.is_some(),
            printer: live.config.as_ref().map(|c| PrinterSummary {
                ip: c.ip.clone(),
                serial: c.serial.clone(),
                name: c.name.clone(),
                model: model.clone(),
                firmware: live.firmware.clone(),
            }),
            connection: live.connection.clone(),
            state: live.state.clone(),
            slots,
            errors,
        }
    }

    /// Records which preset is loaded in a slot. `filament_id` is already
    /// resolved through `inherits`.
    pub fn assign_slot(
        &self,
        ams_id: u32,
        tray_id: u32,
        preset_name: &str,
        filament_id: Option<&str>,
        preset_path: Option<&str>,
    ) -> Result<PrinterView, String> {
        check_slot(ams_id, tray_id)?;
        let (serial, generation) = self.current()?;
        self.history()?.assign_slot(
            &serial,
            ams_id,
            tray_id,
            preset_name,
            filament_id,
            preset_path,
        )?;
        self.reload_assignments(&serial, generation);
        Ok(self.view())
    }

    pub fn clear_slot(&self, ams_id: u32, tray_id: u32) -> Result<PrinterView, String> {
        check_slot(ams_id, tray_id)?;
        let (serial, generation) = self.current()?;
        self.history()?.clear_slot(&serial, ams_id, tray_id)?;
        self.reload_assignments(&serial, generation);
        Ok(self.view())
    }

    /// Re-reads assignments and their presets' cloud-sync state, e.g. when
    /// the Printer page opens after the user synced in Bambu Studio.
    pub fn refresh_assignments(&self) {
        if let Ok((serial, generation)) = self.current() {
            self.reload_assignments(&serial, generation);
        }
    }

    /// The configured printer's serial and the connection generation.
    fn current(&self) -> Result<(String, u64), String> {
        let live = self.inner.live.lock().unwrap();
        let serial = live_serial(&live).ok_or("No printer configured")?;
        Ok((serial, live.generation))
    }

    fn history(&self) -> Result<RefinementHistory, String> {
        let path = self
            .inner
            .history_db
            .as_ref()
            .ok_or("The app data folder is unavailable")?;
        RefinementHistory::new(path)
    }

    fn load_assignments(&self, serial: &str) -> Vec<SlotAssignment> {
        match self.history().and_then(|h| h.list_slot_assignments(serial)) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("could not load slot assignments: {e}");
                Vec::new()
            }
        }
    }

    /// Re-reads `serial`'s assignments. They are dropped if the printer was
    /// restarted or replaced (`generation` changed) while they were read.
    fn reload_assignments(&self, serial: &str, generation: u64) {
        let assignments = self.load_assignments(serial);
        let cloud_sync = cloud_sync_map(&assignments);
        {
            let mut live = self.inner.live.lock().unwrap();
            if live.generation != generation {
                return;
            }
            live.assignments = assignments;
            live.cloud_sync = cloud_sync;
        }
        self.mark_dirty();
    }

    fn mark_dirty(&self) {
        self.inner.dirty.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Applies one client event if it belongs to the current connection.
    pub(crate) fn handle_event(&self, generation: u64, event: ClientEvent) {
        let mut load_hms: Option<Arc<HmsCatalog>> = None;
        {
            let mut live = self.inner.live.lock().unwrap();
            if live.generation != generation {
                return;
            }
            match event {
                ClientEvent::Connection(state) => {
                    live.connection = state.clone();
                    drop(live);
                    self.inner.events.connection(&state);
                    self.mark_dirty();
                    return;
                }
                ClientEvent::Report(bytes) => match parse_report(&bytes) {
                    Err(e) => {
                        tracing::debug!(serial = ?live_serial(&live), "skipping malformed printer report: {e}");
                        return;
                    }
                    Ok(Report::Status { print, full }) => {
                        let state = live.merger.apply(&print, full);
                        let has_errors = !state.hms.is_empty() || state.print_error.is_some();
                        live.state = Some(state);
                        if has_errors && !live.hms_loading {
                            if let Some(hms) = live.hms.clone().filter(|h| !h.is_loaded()) {
                                live.hms_loading = true;
                                load_hms = Some(hms);
                            }
                        }
                    }
                    Ok(Report::Version { model, firmware }) => {
                        if model.is_some() {
                            live.model = model;
                        }
                        live.firmware = firmware;
                    }
                    Ok(Report::Other) => return,
                },
            }
        }
        self.mark_dirty();
        if let Some(hms) = load_hms {
            let service = self.clone();
            tauri::async_runtime::spawn(async move {
                hms.ensure_loaded(chrono::Utc::now().timestamp()).await;
                service.inner.live.lock().unwrap().hms_loading = false;
                service.mark_dirty();
            });
        }
    }
    /// The client stopped sending events. Unless it stopped on a state that
    /// needs the user, the printer is now disconnected.
    fn client_ended(&self, generation: u64) {
        {
            let mut live = self.inner.live.lock().unwrap();
            if live.generation != generation
                || live.connection.needs_user()
                || live.connection == ConnectionState::Disconnected
            {
                return;
            }
            live.connection = ConnectionState::Disconnected;
        }
        self.inner.events.connection(&ConnectionState::Disconnected);
        self.mark_dirty();
    }

    #[cfg(test)]
    pub(crate) fn configure_for_test(&self, config: PrinterConfig) -> u64 {
        self.configure(config)
    }
}

/// AMS units 0–3 and AMS HT units 128–135 (trays 0–3), and the external
/// spools (`ams_id` 255, trays 254 and 255).
fn check_slot(ams_id: u32, tray_id: u32) -> Result<(), String> {
    let ok = match ams_id {
        0..=3 | 128..=135 => tray_id <= 3,
        EXTERNAL_AMS_ID => matches!(tray_id, 254 | 255),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err("That isn't a slot on this printer.".into())
    }
}

fn cloud_sync_map(assignments: &[SlotAssignment]) -> HashMap<String, bool> {
    assignments
        .iter()
        .filter_map(|a| a.preset_path.clone())
        .map(|p| {
            let needs = preset_needs_cloud_sync(std::path::Path::new(&p));
            (p, needs)
        })
        .collect()
}

/// Applies the client's events until `client::run` returns. After a state
/// that needs the user (wrong code, untrusted certificate, wrong serial)
/// `run` returns on its own; that last state is kept as the connection
/// state, and the client only runs again through `start`/`start_with`
/// once the user saves a new code or trusts the printer.
async fn consume(service: PrinterService, generation: u64, mut rx: mpsc::Receiver<ClientEvent>) {
    while let Some(event) = rx.recv().await {
        if let ClientEvent::Report(bytes) = &event {
            // Debug level at most; capture with
            // RUST_LOG=info,bambumate_tauri::printer::payload=debug
            tracing::debug!(
                target: "bambumate_tauri::printer::payload",
                serial = ?service.config().map(|c| c.serial),
                "{}",
                String::from_utf8_lossy(bytes)
            );
        }
        service.handle_event(generation, event);
    }
    service.client_ended(generation);
}

fn live_serial(live: &Live) -> Option<String> {
    live.config.as_ref().map(|c| c.serial.clone())
}

/// Emits the view after each change, then waits, so bursts of reports
/// collapse into at most two events a second.
async fn emit_loop(inner: Weak<Inner>, mut dirty: watch::Receiver<u64>) {
    while dirty.changed().await.is_ok() {
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let view = PrinterService {
            inner: inner.clone(),
        }
        .view();
        inner.events.state(&view);
        drop(inner);
        tokio::time::sleep(EMIT_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::client::ConnectionState;
    use crate::printer::slots::SlotStatus;
    use crate::printer::state::fixtures::{GET_VERSION_H2D, H2D_DELTA_PROGRESS, H2D_FULL};
    use crate::printer::testbroker::FakeBroker;
    use crate::printer::tls::testpki::TestCa;

    // The pushall limit is process-wide per serial, and the client tests
    // number their serials `0948AB…`; a separate prefix keeps these tests
    // from finding the limit already taken.
    const SERIAL: &str = "0948SV000000001";

    #[derive(Default)]
    struct Recorder {
        states: Mutex<Vec<PrinterView>>,
        connections: Mutex<Vec<ConnectionState>>,
    }

    impl PrinterEvents for Recorder {
        fn state(&self, view: &PrinterView) {
            self.states.lock().unwrap().push(view.clone());
        }
        fn connection(&self, state: &ConnectionState) {
            self.connections.lock().unwrap().push(state.clone());
        }
    }

    fn config() -> PrinterConfig {
        config_for(SERIAL)
    }

    fn config_for(serial: &str) -> PrinterConfig {
        PrinterConfig {
            ip: "127.0.0.1".into(),
            serial: serial.into(),
            name: "Workshop".into(),
            model: "H2D".into(),
            pinned_fingerprint: None,
        }
    }

    fn service(dir: &tempfile::TempDir) -> (PrinterService, Arc<Recorder>) {
        service_with(dir, Timing::default())
    }

    fn service_with(dir: &tempfile::TempDir, timing: Timing) -> (PrinterService, Arc<Recorder>) {
        let rec = Arc::new(Recorder::default());
        // Port 9 (discard) on loopback: the HMS fetch fails fast, offline.
        let svc = PrinterService::new(
            rec.clone(),
            Some(dir.path().join("history.db")),
            dir.path().to_path_buf(),
            "http://127.0.0.1:9/query.php",
            timing,
        );
        (svc, rec)
    }

    async fn wait_for(mut check: impl FnMut() -> bool) {
        for _ in 0..100 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("condition not met in 5 s");
    }

    #[tokio::test]
    async fn connects_and_emits_the_state_with_slots() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, rec) = service(&dir);
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(SERIAL), SERIAL, "12345678", H2D_FULL)
            .await
            .with_version_reply(GET_VERSION_H2D);
        let rejection = RejectionSlot::default();
        let verifier =
            PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, rejection.clone()).unwrap();
        svc.start_with(
            config(),
            ClientParams {
                host: "127.0.0.1".into(),
                port: broker.addr.port(),
                serial: SERIAL.into(),
                access_code: "12345678".into(),
                tls: client_config(Arc::new(verifier)).unwrap(),
                rejection,
            },
        );
        wait_for(|| {
            rec.connections
                .lock()
                .unwrap()
                .contains(&ConnectionState::Connected)
        })
        .await;
        wait_for(|| svc.view().slots.len() == 10).await;
        let view = svc.view();
        assert!(view.configured);
        assert_eq!(view.connection, ConnectionState::Connected);
        assert_eq!(view.printer.as_ref().unwrap().serial, SERIAL);
        assert_eq!(view.state.as_ref().unwrap().mc_percent, Some(6));
        assert_eq!(view.errors.len(), 1, "the fixture has one HMS code");
        wait_for(|| {
            rec.states
                .lock()
                .unwrap()
                .iter()
                .any(|v| v.slots.len() == 10)
        })
        .await;
        svc.stop();
    }

    fn broker_params(ca: &TestCa, broker: &FakeBroker, serial: &str, code: &str) -> ClientParams {
        let rejection = RejectionSlot::default();
        let verifier =
            PrinterCertVerifier::with_trust(&ca.pem, &[], serial, None, rejection.clone()).unwrap();
        ClientParams {
            host: "127.0.0.1".into(),
            port: broker.addr.port(),
            serial: serial.into(),
            access_code: code.into(),
            tls: client_config(Arc::new(verifier)).unwrap(),
            rejection,
        }
    }

    #[tokio::test]
    async fn a_state_that_needs_the_user_is_kept_until_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let fast = Timing {
            backoff_min: Duration::from_millis(50),
            backoff_max: Duration::from_millis(200),
            pushall_interval: Duration::from_secs(300),
        };
        let (svc, rec) = service_with(&dir, fast);
        let serial = "0948SV000000002";
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(serial), serial, "12345678", H2D_FULL).await;
        svc.start_with(
            config_for(serial),
            broker_params(&ca, &broker, serial, "87654321"),
        );
        wait_for(|| svc.connection() == ConnectionState::AuthFailed).await;
        // Many backoff periods: the client has returned, and nothing retries
        // or turns the state into Disconnected.
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(svc.connection(), ConnectionState::AuthFailed);
        assert!(svc.view().configured);
        assert_eq!(
            *rec.connections.lock().unwrap(),
            [
                ConnectionState::Connecting,
                ConnectionState::Connecting,
                ConnectionState::AuthFailed
            ]
        );
        // A new code restarts the client.
        svc.start_with(
            config_for(serial),
            broker_params(&ca, &broker, serial, "12345678"),
        );
        wait_for(|| svc.connection() == ConnectionState::Connected).await;
        svc.stop();
    }

    async fn start_connected(svc: &PrinterService, ca: &TestCa, broker: &FakeBroker, serial: &str) {
        svc.start_with(
            config_for(serial),
            broker_params(ca, broker, serial, "12345678"),
        );
        wait_for(|| {
            svc.connection() == ConnectionState::Connected && broker.open_connections() == 1
        })
        .await;
    }

    #[tokio::test]
    async fn overlapping_restarts_leave_one_broker_connection() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let serial = "0948SV000000003";
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(serial), serial, "12345678", H2D_FULL).await;
        start_connected(&svc, &ca, &broker, serial).await;
        // Two restarts at once, from two threads.
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let (svc, params) = (svc.clone(), broker_params(&ca, &broker, serial, "12345678"));
                std::thread::spawn(move || svc.start_with(config_for(serial), params))
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        wait_for(|| svc.connection() == ConnectionState::Connected).await;
        // Past the grace period: any replaced client has ended by now.
        tokio::time::sleep(STOP_GRACE + Duration::from_millis(500)).await;
        assert_eq!(broker.open_connections(), 1);
        assert!(
            broker.disconnect_count() >= 1,
            "the replaced connection closes with an MQTT DISCONNECT"
        );
        svc.stop();
    }

    #[tokio::test]
    async fn stop_disconnects_from_the_broker() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let serial = "0948SV000000004";
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(serial), serial, "12345678", H2D_FULL).await;
        start_connected(&svc, &ca, &broker, serial).await;
        svc.stop();
        wait_for(|| broker.open_connections() == 0).await;
        assert_eq!(broker.disconnect_count(), 1);
        assert_eq!(svc.connection(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn shutdown_waits_for_the_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let serial = "0948SV000000005";
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(serial), serial, "12345678", H2D_FULL).await;
        start_connected(&svc, &ca, &broker, serial).await;
        // As at app exit: a plain thread, outside any async runtime.
        let exiting = svc.clone();
        std::thread::spawn(move || exiting.shutdown())
            .join()
            .unwrap();
        wait_for(|| broker.disconnect_count() == 1 && broker.open_connections() == 0).await;
        assert!(!svc.view().configured);
    }

    #[tokio::test]
    async fn a_client_that_ends_without_needing_the_user_is_disconnected() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, rec) = service(&dir);
        let generation = svc.configure_for_test(config());
        let (tx, rx) = mpsc::channel(4);
        tx.send(ClientEvent::Connection(ConnectionState::Connected))
            .await
            .unwrap();
        drop(tx);
        consume(svc.clone(), generation, rx).await;
        assert_eq!(svc.connection(), ConnectionState::Disconnected);
        assert_eq!(
            rec.connections.lock().unwrap().last(),
            Some(&ConnectionState::Disconnected)
        );
    }

    #[tokio::test]
    async fn assignments_read_for_a_replaced_printer_are_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let old = svc.configure_for_test(config());
        svc.assign_slot(0, 0, "Acme PLA", Some("P4d6ae04"), None)
            .unwrap();
        svc.configure_for_test(config_for("0948SV000000099"));
        // A reload that started before the switch finishes after it.
        svc.reload_assignments(SERIAL, old);
        assert!(svc.inner.live.lock().unwrap().assignments.is_empty());
    }

    #[tokio::test]
    async fn slot_ids_out_of_range_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        svc.configure_for_test(config());
        for (ams, tray) in [
            (4, 0),
            (0, 4),
            (127, 0),
            (136, 0),
            (255, 0),
            (254, 254),
            (255, 256),
        ] {
            assert_eq!(
                svc.assign_slot(ams, tray, "X", None, None).unwrap_err(),
                "That isn't a slot on this printer.",
                "{ams}/{tray}"
            );
            assert!(svc.clear_slot(ams, tray).is_err(), "{ams}/{tray}");
        }
        for (ams, tray) in [(0, 0), (3, 3), (128, 0), (135, 3), (255, 254), (255, 255)] {
            assert!(
                svc.assign_slot(ams, tray, "X", None, None).is_ok(),
                "{ams}/{tray}"
            );
            assert!(svc.clear_slot(ams, tray).is_ok(), "{ams}/{tray}");
        }
    }

    #[test]
    fn client_params_reject_a_blank_serial_with_a_user_facing_error() {
        let mut c = config();
        c.serial = "  ".into();
        assert_eq!(
            client_params(&c, "12345678".into()).unwrap_err(),
            "Enter the printer's serial number (letters and digits)."
        );
        let p = client_params(&config(), "12345678".into()).unwrap();
        assert_eq!((p.host.as_str(), p.port), ("127.0.0.1", client::MQTT_PORT));
        assert_eq!(p.serial, SERIAL);
    }

    #[tokio::test]
    async fn a_burst_of_reports_is_emitted_at_most_twice_a_second() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, rec) = service(&dir);
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        for _ in 0..30 {
            svc.handle_event(
                generation,
                ClientEvent::Report(H2D_DELTA_PROGRESS.as_bytes().to_vec()),
            );
        }
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let n = rec.states.lock().unwrap().len();
        assert!((1..=3).contains(&n), "{n} state events in ~1 s");
        let last = rec.states.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.state.unwrap().mc_percent, Some(7));
    }

    #[tokio::test]
    async fn malformed_reports_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let generation = svc.configure_for_test(config());
        svc.handle_event(generation, ClientEvent::Report(b"{not json".to_vec()));
        assert!(svc.view().state.is_none());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        assert!(svc.view().state.is_some());
    }

    #[tokio::test]
    async fn events_from_a_stopped_connection_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let old = svc.configure_for_test(config());
        let new = svc.configure_for_test(config());
        svc.handle_event(old, ClientEvent::Report(H2D_FULL.as_bytes().to_vec()));
        assert!(svc.view().state.is_none());
        svc.handle_event(new, ClientEvent::Connection(ConnectionState::Connected));
        assert_eq!(svc.connection(), ConnectionState::Connected);
    }

    #[tokio::test]
    async fn assigning_a_slot_persists_and_shows_its_status() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );

        let view = svc
            .assign_slot(1, 1, "Acme PLA", Some("P4d6ae04"), None)
            .unwrap();
        let b2 = view.slots.iter().find(|s| s.label == "B2").unwrap();
        assert_eq!(b2.status, SlotStatus::Matches);
        let view = svc
            .assign_slot(0, 2, "Acme PETG", Some("P0000001"), None)
            .unwrap();
        let a3 = view.slots.iter().find(|s| s.label == "A3").unwrap();
        assert_eq!(a3.status, SlotStatus::Different);

        // A restart reloads assignments from the database.
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        assert_eq!(
            svc.view()
                .slots
                .iter()
                .filter(|s| s.assigned_preset.is_some())
                .count(),
            2
        );
        let view = svc.clear_slot(0, 2).unwrap();
        let a3 = view.slots.iter().find(|s| s.label == "A3").unwrap();
        assert_eq!(a3.status, SlotStatus::Unassigned);
    }

    #[tokio::test]
    async fn without_a_printer_assignments_are_refused_and_stop_clears_the_view() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, rec) = service(&dir);
        assert_eq!(
            svc.assign_slot(0, 0, "X", None, None).unwrap_err(),
            "No printer configured"
        );
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        svc.stop();
        let view = svc.view();
        assert!(!view.configured);
        assert!(view.state.is_none());
        assert_eq!(
            rec.connections.lock().unwrap().last(),
            Some(&ConnectionState::Disconnected)
        );
    }
}
