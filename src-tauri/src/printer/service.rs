//! Owns the printer connection. Keeps the latest state and connection
//! state, and emits `printer://state` (at most twice a second) and
//! `printer://connection`.
//!
//! # Restarts
//!
//! Callers (`start`, `stop`, Save in Settings) are synchronous and may run on
//! any thread. They update the live state at once, under `live`'s lock, and
//! then hand the connection change to one spawned **sequencer** task
//! ([`sequence`]) over a channel. Only the sequencer starts and ends client
//! tasks, one command at a time: before it starts a new client it waits for
//! the old one to send its MQTT DISCONNECT (at most `STOP_GRACE`), so two
//! sessions to the printer never overlap. No lock is held across an await:
//! the sequencer owns the running client outright, and `live` is only
//! locked for short, synchronous updates.
//!
//! A restart for the same printer (same serial) keeps the merged report and
//! the derived state, so a printer that only sends deltas doesn't go blank
//! while the per-printer `pushall` limit is closed. A save that changes
//! nothing the connection uses (IP, serial, pin, access code) doesn't
//! restart it at all. If a client ends without a state that needs the user
//! (for example it panicked), the sequencer starts it again with backoff.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::async_runtime::JoinHandle;
use tokio::sync::{mpsc, oneshot, watch};

use super::client::{
    self, Backoff, ClientEvent, ClientParams, ConnectionState, TestOutcome, Timing,
};
use super::hms::{ErrorView, HmsCatalog};
use super::settings::{valid_serial, PrinterConfig};
use super::slots::{compute_slots, preset_needs_cloud_sync, SlotView, EXTERNAL_AMS_ID};
use super::state::{parse_report, PrinterState, Report, ReportMerger};
use super::tls::{client_config, normalize_fingerprint, PrinterCertVerifier, RejectionSlot};
use crate::history::{RefinementHistory, SlotAssignment};

pub const STATE_EVENT: &str = "printer://state";
pub const CONNECTION_EVENT: &str = "printer://connection";
/// `printer://state` is emitted at most once per this interval.
const EMIT_INTERVAL: Duration = Duration::from_millis(500);
/// How long a replaced or stopped client gets to send its MQTT DISCONNECT
/// before it is aborted.
const STOP_GRACE: Duration = Duration::from_secs(1);
/// The longest wait before restarting a client that ended on its own.
const RESTART_MAX: Duration = Duration::from_secs(60);
/// How often a Test connection against the running printer re-reads the
/// live connection while it waits for a result.
const LIVE_TEST_POLL: Duration = Duration::from_millis(100);

/// Where the service's events go: the webview in the app, a recorder in tests.
pub trait PrinterEvents: Send + Sync + 'static {
    fn state(&self, view: &PrinterView);
    fn connection(&self, state: &ConnectionState);
    /// The printer `serial` verified against a Bambu CA for the first time.
    fn ca_verified(&self, _serial: &str) {}
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
    fn ca_verified(&self, serial: &str) {
        let (app, serial) = (self.0.clone(), serial.to_string());
        // The settings store writes a file: off the event path.
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = super::settings::mark_ca_verified(&app, &serial) {
                tracing::warn!(%serial, "could not record the CA-verified printer: {e}");
            }
        });
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
    /// This printer has verified against a Bambu CA before.
    #[serde(default)]
    pub ca_verified: bool,
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

/// A pin in comparable form: normalized hex, `None` when blank.
fn pin_key(pin: Option<&str>) -> Option<String> {
    pin.map(normalize_fingerprint).filter(|p| !p.is_empty())
}

fn code_digest(code: &str) -> [u8; 32] {
    Sha256::digest(code.trim().as_bytes()).into()
}

/// What the running client connects with. Two saves with the same key need
/// no restart. The access code is kept only as a digest.
#[derive(Clone, PartialEq, Eq)]
struct ConnKey {
    host: String,
    port: u16,
    serial: String,
    pin: Option<String>,
    code: [u8; 32],
}

impl ConnKey {
    fn new(config: &PrinterConfig, params: &ClientParams) -> Self {
        Self {
            host: params.host.clone(),
            port: params.port,
            serial: params.serial.clone(),
            pin: pin_key(config.pinned_fingerprint.as_deref()),
            code: code_digest(&params.access_code),
        }
    }
}

struct Live {
    config: Option<PrinterConfig>,
    /// The running client's parameters; `None` once stopped.
    key: Option<ConnKey>,
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
            key: None,
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

/// Runs one client connection: `client::run` in the app, a stand-in in tests.
type ClientFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type Runner =
    Arc<dyn Fn(ClientParams, mpsc::Sender<ClientEvent>, Timing) -> ClientFuture + Send + Sync>;

fn real_client() -> Runner {
    Arc::new(|params, events, timing| Box::pin(client::run(params, events, timing)))
}

/// A connection change for the sequencer. `generation` is the live
/// generation the caller set; only the newest command matters.
struct Command {
    generation: u64,
    start: Option<ClientParams>,
    /// Answered once the command is carried out (`shutdown` waits on it).
    done: Option<oneshot::Sender<()>>,
}

struct Inner {
    events: Arc<dyn PrinterEvents>,
    history_db: Option<PathBuf>,
    data_dir: PathBuf,
    hms_url: String,
    timing: Timing,
    live: Mutex<Live>,
    dirty: watch::Sender<u64>,
    /// To the sequencer, the only task that starts and ends clients.
    commands: mpsc::UnboundedSender<Command>,
}

/// One client connection's tasks.
struct Running {
    client: JoinHandle<()>,
    /// Owns the event receiver: ending it makes `client::run` send an MQTT
    /// DISCONNECT and return.
    consumer: JoinHandle<()>,
    generation: u64,
    params: ClientParams,
    started: tokio::time::Instant,
}

impl Running {
    /// Ends the consumer, which drops the receiver so the client takes its
    /// graceful DISCONNECT path, and waits up to `STOP_GRACE` for it before
    /// aborting it.
    async fn retire(self) {
        self.consumer.abort();
        let mut client = self.client;
        if tokio::time::timeout(STOP_GRACE, &mut client).await.is_err() {
            client.abort();
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
        Self::with_runner(events, history_db, data_dir, hms_url, timing, real_client())
    }

    fn with_runner(
        events: Arc<dyn PrinterEvents>,
        history_db: Option<PathBuf>,
        data_dir: PathBuf,
        hms_url: &str,
        timing: Timing,
        runner: Runner,
    ) -> Self {
        let (dirty, dirty_rx) = watch::channel(0u64);
        let (commands, commands_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Inner {
            events,
            history_db,
            data_dir,
            hms_url: hms_url.to_string(),
            timing,
            live: Mutex::new(Live::empty(0)),
            dirty,
            commands,
        });
        tauri::async_runtime::spawn(emit_loop(Arc::downgrade(&inner), dirty_rx));
        tauri::async_runtime::spawn(sequence(Arc::downgrade(&inner), commands_rx, runner));
        Self { inner }
    }

    /// Connects to the configured printer, replacing any running connection.
    pub fn start(&self, config: PrinterConfig, access_code: String) -> Result<(), String> {
        let params = client_params(&config, access_code)?;
        self.start_with(config, params);
        Ok(())
    }

    /// `start` with explicit client parameters. Tests point it at a local
    /// broker. When the running client already connects with the same IP,
    /// serial, pin and access code, only the config's other fields (name,
    /// model) are updated and the connection is left alone.
    pub fn start_with(&self, config: PrinterConfig, params: ClientParams) {
        let key = ConnKey::new(&config, &params);
        if self.update_in_place(&config, &key) {
            return;
        }
        let generation = self.configure(config, Some(key));
        self.send(Command {
            generation,
            start: Some(params),
            done: None,
        });
    }

    /// Takes `config` without a restart if the running client already uses
    /// `key` and isn't stopped on a state that needs the user.
    fn update_in_place(&self, config: &PrinterConfig, key: &ConnKey) -> bool {
        {
            let mut live = self.inner.live.lock().unwrap();
            let unchanged = live.key.as_ref() == Some(key)
                && live.config.is_some()
                && !live.connection.needs_user();
            if !unchanged {
                return false;
            }
            let ca_verified = live.config.as_ref().is_some_and(|c| c.ca_verified);
            let mut config = config.clone();
            config.ca_verified |= ca_verified;
            live.config = Some(config);
        }
        self.mark_dirty();
        true
    }

    fn send(&self, command: Command) {
        // Fails only once the sequencer is gone, i.e. the runtime is ending.
        let _ = self.inner.commands.send(command);
    }

    /// Points the live state at `config` and returns the new generation.
    /// The merged report and derived state are kept when the serial is
    /// unchanged; a different printer starts from nothing.
    fn configure(&self, mut config: PrinterConfig, key: Option<ConnKey>) -> u64 {
        let assignments = self.load_assignments(&config.serial);
        let cloud_sync = cloud_sync_map(&assignments);
        let generation = {
            let mut live = self.inner.live.lock().unwrap();
            let generation = live.generation + 1;
            let same_printer = live
                .config
                .as_ref()
                .is_some_and(|c| c.serial == config.serial);
            if same_printer {
                live.generation = generation;
                config.ca_verified |= live.config.as_ref().is_some_and(|c| c.ca_verified);
            } else {
                *live = Live::empty(generation);
                live.hms = Some(Arc::new(HmsCatalog::with_base_url(
                    self.inner.data_dir.clone(),
                    Some(&config.serial),
                    &self.inner.hms_url,
                )));
            }
            live.config = Some(config);
            live.key = key;
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
        self.stop_inner(None);
    }

    /// `stop` for app exit: waits for the DISCONNECT (at most `STOP_GRACE`,
    /// plus a margin), so the printer frees the connection slot before the
    /// process ends. Call it outside the async runtime.
    pub fn shutdown(&self) {
        let (done, finished) = oneshot::channel();
        self.stop_inner(Some(done));
        tauri::async_runtime::block_on(async {
            let _ = tokio::time::timeout(STOP_GRACE * 2, finished).await;
        });
    }

    fn stop_inner(&self, done: Option<oneshot::Sender<()>>) {
        let generation = {
            let mut live = self.inner.live.lock().unwrap();
            let generation = live.generation + 1;
            *live = Live::empty(generation);
            generation
        };
        self.send(Command {
            generation,
            start: None,
            done,
        });
        self.inner.events.connection(&ConnectionState::Disconnected);
        self.mark_dirty();
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

    /// Whether a Test connection for `request` would test exactly what the
    /// running client connects with: the same IP, serial and pin, and no
    /// typed access code or the same one.
    fn runs(&self, request: &PrinterConfig, typed_code: Option<&str>) -> bool {
        let live = self.inner.live.lock().unwrap();
        let (Some(config), Some(key)) = (&live.config, &live.key) else {
            return false;
        };
        let same_target = config.ip == request.ip
            && config.serial == request.serial
            && pin_key(config.pinned_fingerprint.as_deref())
                == pin_key(request.pinned_fingerprint.as_deref());
        let same_code = match typed_code.map(str::trim).filter(|c| !c.is_empty()) {
            None => true,
            Some(code) => code_digest(code) == key.code,
        };
        same_target && same_code
    }

    /// Test connection for the printer the service is already connected to
    /// (see [`runs`](Self::runs)): answers from the live connection instead
    /// of opening a second MQTT session to the printer. Waits up to `wait`
    /// for the live connection to settle: connected with a report, or a
    /// failure. `None` when `request` isn't the running printer.
    pub async fn live_test(
        &self,
        request: &PrinterConfig,
        typed_code: Option<&str>,
        wait: Duration,
    ) -> Option<TestOutcome> {
        if !self.runs(request, typed_code) {
            return None;
        }
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let outcome = {
                let live = self.inner.live.lock().unwrap();
                TestOutcome {
                    connection: live.connection.clone(),
                    got_report: live.state.is_some(),
                    model: live.model.clone(),
                }
            };
            let settled = match outcome.connection {
                ConnectionState::Connected => outcome.got_report,
                ConnectionState::Connecting | ConnectionState::Disconnected => false,
                _ => true,
            };
            if settled || tokio::time::Instant::now() >= deadline {
                return Some(outcome);
            }
            tokio::time::sleep(LIVE_TEST_POLL).await;
        }
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
                ca_verified: c.ca_verified,
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
        self.refresh_assignments_with(&mut |_| None);
    }

    /// `refresh_assignments`, and each assignment stored without a
    /// `filament_id` gets one from `resolve` if it now has one (a user preset
    /// gains its id once it syncs). A resolved id is saved, so it is looked
    /// up only once. Assignments that have an id are never re-resolved.
    pub fn refresh_assignments_with(
        &self,
        resolve: &mut dyn FnMut(&SlotAssignment) -> Option<String>,
    ) {
        let Ok((serial, generation)) = self.current() else {
            return;
        };
        let mut assignments = self.load_assignments(&serial);
        let missing = |a: &SlotAssignment| {
            a.filament_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty())
        };
        for a in assignments.iter_mut().filter(|a| missing(a)) {
            let Some(id) = resolve(a).filter(|id| !id.trim().is_empty()) else {
                continue;
            };
            let saved = self.history().and_then(|h| {
                h.set_slot_filament_id(&a.serial, a.ams_id, a.tray_id, &a.preset_name, &id)
            });
            if let Err(e) = saved {
                tracing::warn!("could not save a resolved filament id: {e}");
            }
            a.filament_id = Some(id);
        }
        self.apply_assignments(generation, assignments);
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
        self.apply_assignments(generation, assignments);
    }

    fn apply_assignments(&self, generation: u64, assignments: Vec<SlotAssignment>) {
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
                ClientEvent::CaVerified => {
                    let Some(config) = live.config.as_mut().filter(|c| !c.ca_verified) else {
                        return;
                    };
                    config.ca_verified = true;
                    let serial = config.serial.clone();
                    drop(live);
                    self.inner.events.ca_verified(&serial);
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

    /// Whether a client of `generation` that ended on its own should be
    /// started again: the printer is still configured with that generation
    /// and the client didn't stop on a state that needs the user.
    fn wants_restart(&self, generation: u64) -> bool {
        let live = self.inner.live.lock().unwrap();
        live.generation == generation && live.config.is_some() && !live.connection.needs_user()
    }

    fn is_current(&self, generation: u64) -> bool {
        self.inner.live.lock().unwrap().generation == generation
    }

    #[cfg(test)]
    pub(crate) fn configure_for_test(&self, config: PrinterConfig) -> u64 {
        self.configure(config, None)
    }
}

/// Spawns one client and the task that applies its events.
fn launch(
    service: &PrinterService,
    runner: &Runner,
    generation: u64,
    params: ClientParams,
) -> Running {
    let (tx, rx) = mpsc::channel(256);
    Running {
        client: tauri::async_runtime::spawn(runner(params.clone(), tx, service.inner.timing)),
        consumer: tauri::async_runtime::spawn(consume(service.clone(), generation, rx)),
        generation,
        params,
        started: tokio::time::Instant::now(),
    }
}

/// Completes when `running`'s consumer has applied the client's last event
/// (the client ended, or was aborted); never completes when nothing runs.
async fn ended(running: &mut Option<Running>) {
    match running {
        Some(r) => {
            let _ = (&mut r.consumer).await;
        }
        None => std::future::pending().await,
    }
}

/// The sequencer: the only task that starts and ends clients. See the
/// module docs. Ends when the service is dropped.
async fn sequence(
    inner: Weak<Inner>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    runner: Runner,
) {
    let Some(timing) = inner.upgrade().map(|i| i.timing) else {
        return;
    };
    let restart_max = timing.backoff_max.min(RESTART_MAX);
    let mut backoff = Backoff::new(timing.backoff_min, restart_max);
    let mut running: Option<Running> = None;
    // Set while a client that ended on its own waits to be started again.
    let mut restart_at: Option<tokio::time::Instant> = None;
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(first) = command else { return };
                // Only the newest queued change matters; everyone waiting on
                // an older one is answered once it is carried out.
                let mut command = first;
                let mut waiting = Vec::new();
                while let Ok(next) = commands.try_recv() {
                    if next.generation >= command.generation {
                        waiting.extend(std::mem::replace(&mut command, next).done);
                    } else {
                        waiting.extend(next.done);
                    }
                }
                waiting.extend(command.done.take());
                restart_at = None;
                if let Some(old) = running.take() {
                    old.retire().await;
                }
                if let (Some(params), Some(inner)) = (command.start, inner.upgrade()) {
                    let service = PrinterService { inner };
                    // A start overtaken by a newer change is dropped.
                    if service.is_current(command.generation) {
                        backoff.reset();
                        running = Some(launch(&service, &runner, command.generation, params));
                    }
                }
                for done in waiting {
                    let _ = done.send(());
                }
            }
            _ = ended(&mut running), if restart_at.is_none() => {
                let Some(inner) = inner.upgrade() else { return };
                let service = PrinterService { inner };
                let r = running.as_ref().expect("ended only completes while running");
                if service.wants_restart(r.generation) {
                    // A client that ran well for a while starts the delay over.
                    if r.started.elapsed() >= restart_max {
                        backoff.reset();
                    }
                    let delay = backoff.next_delay();
                    tracing::warn!(delay_ms = delay.as_millis() as u64, "printer client ended unexpectedly; restarting");
                    restart_at = Some(tokio::time::Instant::now() + delay);
                } else {
                    running = None;
                }
            }
            _ = sleep_until_opt(restart_at) => {
                restart_at = None;
                let Some(old) = running.take() else { continue };
                let Some(inner) = inner.upgrade() else { return };
                let service = PrinterService { inner };
                old.client.abort();
                if service.wants_restart(old.generation) {
                    running = Some(launch(&service, &runner, old.generation, old.params));
                }
            }
        }
    }
}

async fn sleep_until_opt(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
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
        ca_verified: Mutex<Vec<String>>,
    }

    impl PrinterEvents for Recorder {
        fn state(&self, view: &PrinterView) {
            self.states.lock().unwrap().push(view.clone());
        }
        fn connection(&self, state: &ConnectionState) {
            self.connections.lock().unwrap().push(state.clone());
        }
        fn ca_verified(&self, serial: &str) {
            self.ca_verified.lock().unwrap().push(serial.to_string());
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
            ca_verified: false,
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
        // Two restarts at once, from two threads. Each pins a different
        // certificate, so neither is a no-op.
        let threads: Vec<_> = (0..2)
            .map(|i| {
                let (svc, params) = (svc.clone(), broker_params(&ca, &broker, serial, "12345678"));
                let config = pinned(serial, &format!("AA:0{i}"));
                std::thread::spawn(move || svc.start_with(config, params))
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

    /// `config_for(serial)` with a pin: a different connection key, so
    /// `start_with` restarts. The broker's CA still vouches for it.
    fn pinned(serial: &str, pin: &str) -> PrinterConfig {
        PrinterConfig {
            pinned_fingerprint: Some(pin.into()),
            ..config_for(serial)
        }
    }

    fn pushall_count(broker: &FakeBroker) -> usize {
        broker
            .published()
            .iter()
            .filter(|(_, b)| String::from_utf8_lossy(b).contains("pushall"))
            .count()
    }

    fn ams_units(svc: &PrinterService) -> usize {
        svc.view().state.map_or(0, |s| s.ams_units.len())
    }

    #[tokio::test]
    async fn a_restart_for_the_same_printer_keeps_its_state() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let serial = "0948SV000000010";
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(serial), serial, "12345678", H2D_FULL).await;
        start_connected(&svc, &ca, &broker, serial).await;
        wait_for(|| ams_units(&svc) > 0).await;
        let before = svc.view().state;

        svc.start_with(
            pinned(serial, "AA:01"),
            broker_params(&ca, &broker, serial, "12345678"),
        );
        // Kept through the restart itself...
        assert_eq!(svc.view().state, before);
        wait_for(|| {
            svc.connection() == ConnectionState::Connected && broker.connection_count() == 2
        })
        .await;
        // ...and after it, although the printer's pushall limit kept the new
        // connection from asking for a full report.
        assert_eq!(pushall_count(&broker), 1);
        assert_eq!(svc.view().state, before);
        assert!(ams_units(&svc) > 0);
        svc.stop();
        assert!(svc.view().state.is_none(), "stop forgets the state");
    }

    #[tokio::test]
    async fn a_different_printer_starts_without_the_old_state() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        svc.handle_event(
            generation,
            ClientEvent::Report(GET_VERSION_H2D.as_bytes().to_vec()),
        );
        svc.configure_for_test(config());
        assert!(ams_units(&svc) > 0, "same serial: kept");
        svc.configure_for_test(config_for("0948SV000000098"));
        let view = svc.view();
        assert!(view.state.is_none());
        assert_eq!(view.printer.unwrap().firmware, None);
    }

    #[tokio::test]
    async fn saving_unchanged_connection_settings_does_not_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, rec) = service(&dir);
        let serial = "0948SV000000011";
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(serial), serial, "12345678", H2D_FULL).await;
        start_connected(&svc, &ca, &broker, serial).await;
        let events = rec.connections.lock().unwrap().len();

        let renamed = PrinterConfig {
            name: "Garage".into(),
            ..config_for(serial)
        };
        svc.start_with(renamed, broker_params(&ca, &broker, serial, "12345678"));
        assert_eq!(svc.view().printer.unwrap().name, "Garage");
        tokio::time::sleep(STOP_GRACE + Duration::from_millis(300)).await;
        assert_eq!(broker.connection_count(), 1);
        assert_eq!(broker.open_connections(), 1);
        assert_eq!(broker.disconnect_count(), 0);
        assert_eq!(svc.connection(), ConnectionState::Connected);
        assert_eq!(rec.connections.lock().unwrap().len(), events);
        svc.stop();
    }

    #[tokio::test]
    async fn a_restart_never_has_two_open_broker_connections() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let serial = "0948SV000000012";
        let ca = TestCa::new("Test Printer CA");
        let broker =
            Arc::new(FakeBroker::start(&ca.leaf(serial), serial, "12345678", H2D_FULL).await);
        start_connected(&svc, &ca, &broker, serial).await;

        let most = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let watching = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let poller = {
            let (broker, most, watching) = (broker.clone(), most.clone(), watching.clone());
            tokio::spawn(async move {
                while watching.load(std::sync::atomic::Ordering::SeqCst) {
                    most.fetch_max(
                        broker.open_connections(),
                        std::sync::atomic::Ordering::SeqCst,
                    );
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
        };
        for (i, pin) in ["AA:01", "AA:02", "AA:03"].into_iter().enumerate() {
            svc.start_with(
                pinned(serial, pin),
                broker_params(&ca, &broker, serial, "12345678"),
            );
            wait_for(|| {
                svc.connection() == ConnectionState::Connected && broker.connection_count() == i + 2
            })
            .await;
        }
        watching.store(false, std::sync::atomic::Ordering::SeqCst);
        poller.await.unwrap();
        assert_eq!(most.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            broker.disconnect_count(),
            3,
            "each replaced client said goodbye"
        );
        svc.stop();
    }

    #[tokio::test]
    async fn a_test_of_the_running_printer_uses_the_live_connection() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let serial = "0948SV000000013";
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(serial), serial, "12345678", H2D_FULL)
            .await
            .with_version_reply(GET_VERSION_H2D);
        start_connected(&svc, &ca, &broker, serial).await;
        let wait = Duration::from_secs(5);
        for typed in [None, Some("12345678"), Some(" 12345678 ")] {
            let out = svc.live_test(&config_for(serial), typed, wait).await;
            assert_eq!(
                out,
                Some(TestOutcome {
                    connection: ConnectionState::Connected,
                    got_report: true,
                    model: Some("H2D".into()),
                }),
                "{typed:?}"
            );
        }
        assert_eq!(broker.connection_count(), 1, "no second session");
        // Anything the running client doesn't use needs a real test.
        assert_eq!(
            svc.live_test(&config_for(serial), Some("87654321"), wait)
                .await,
            None
        );
        assert_eq!(
            svc.live_test(&pinned(serial, "AA:01"), None, wait).await,
            None
        );
        let elsewhere = PrinterConfig {
            ip: "10.0.0.66".into(),
            ..config_for(serial)
        };
        assert_eq!(svc.live_test(&elsewhere, None, wait).await, None);
        assert_eq!(
            svc.live_test(&config_for("0948SV000000097"), None, wait)
                .await,
            None
        );
        svc.stop();
        assert_eq!(svc.live_test(&config_for(serial), None, wait).await, None);
    }

    #[tokio::test]
    async fn the_first_ca_verified_connection_is_recorded_once() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, rec) = service(&dir);
        let serial = "0948SV000000014";
        let ca = TestCa::new("Test Printer CA");
        let broker = FakeBroker::start(&ca.leaf(serial), serial, "12345678", H2D_FULL).await;
        start_connected(&svc, &ca, &broker, serial).await;
        wait_for(|| svc.config().is_some_and(|c| c.ca_verified)).await;
        assert!(svc.view().printer.unwrap().ca_verified);
        // A restart for the same printer keeps it, and doesn't record it again.
        svc.start_with(
            pinned(serial, "AA:01"),
            broker_params(&ca, &broker, serial, "12345678"),
        );
        assert!(svc.config().unwrap().ca_verified);
        wait_for(|| {
            broker.connection_count() == 2 && svc.connection() == ConnectionState::Connected
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(*rec.ca_verified.lock().unwrap(), [serial.to_string()]);
        svc.stop();
    }

    #[tokio::test]
    async fn a_pinned_only_connection_is_not_ca_verified() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, rec) = service(&dir);
        let serial = "0948SV000000015";
        let trusted = TestCa::new("Test Printer CA");
        let other = TestCa::new("Unknown CA");
        let leaf = other.leaf(serial);
        let pin = crate::printer::tls::fingerprint(&leaf.cert_der);
        let broker = FakeBroker::start(&leaf, serial, "12345678", H2D_FULL).await;
        let rejection = RejectionSlot::default();
        let verifier = PrinterCertVerifier::with_trust(
            &trusted.pem,
            &[],
            serial,
            Some(&pin),
            rejection.clone(),
        )
        .unwrap();
        svc.start_with(
            pinned(serial, &pin),
            ClientParams {
                host: "127.0.0.1".into(),
                port: broker.addr.port(),
                serial: serial.into(),
                access_code: "12345678".into(),
                tls: client_config(Arc::new(verifier)).unwrap(),
                rejection,
            },
        );
        wait_for(|| svc.view().state.is_some()).await;
        assert!(!svc.config().unwrap().ca_verified);
        assert!(rec.ca_verified.lock().unwrap().is_empty());
        svc.stop();
    }

    #[tokio::test]
    async fn a_client_that_ends_on_its_own_is_restarted_with_backoff() {
        let dir = tempfile::tempdir().unwrap();
        let rec = Arc::new(Recorder::default());
        let runs = Arc::new(Mutex::new(Vec::<tokio::time::Instant>::new()));
        let runner: Runner = {
            let runs = runs.clone();
            Arc::new(move |_params, events, _timing| {
                let runs = runs.clone();
                Box::pin(async move {
                    let n = {
                        let mut runs = runs.lock().unwrap();
                        runs.push(tokio::time::Instant::now());
                        runs.len()
                    };
                    let _ = events
                        .send(ClientEvent::Connection(ConnectionState::Connected))
                        .await;
                    // The first two runs end at once, as a panic would; the
                    // third keeps going.
                    if n >= 3 {
                        events.closed().await;
                    }
                })
            })
        };
        let timing = Timing {
            backoff_min: Duration::from_millis(100),
            backoff_max: Duration::from_secs(60),
            pushall_interval: Duration::from_secs(300),
        };
        let svc = PrinterService::with_runner(
            rec.clone(),
            Some(dir.path().join("history.db")),
            dir.path().to_path_buf(),
            "http://127.0.0.1:9/query.php",
            timing,
            runner,
        );
        let ca = TestCa::new("Test Printer CA");
        let serial = "0948SV000000016";
        let rejection = RejectionSlot::default();
        let verifier =
            PrinterCertVerifier::with_trust(&ca.pem, &[], serial, None, rejection.clone()).unwrap();
        svc.start_with(
            config_for(serial),
            ClientParams {
                host: "127.0.0.1".into(),
                port: 1,
                serial: serial.into(),
                access_code: "12345678".into(),
                tls: client_config(Arc::new(verifier)).unwrap(),
                rejection,
            },
        );
        wait_for(|| runs.lock().unwrap().len() == 3).await;
        wait_for(|| svc.connection() == ConnectionState::Connected).await;
        let at = runs.lock().unwrap().clone();
        // 100 ms, then 200 ms: the delay doubles.
        assert!(at[1] - at[0] >= Duration::from_millis(100));
        assert!(at[2] - at[1] >= Duration::from_millis(200));
        assert!(rec
            .connections
            .lock()
            .unwrap()
            .contains(&ConnectionState::Disconnected));
        // A stop ends it for good.
        svc.stop();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(runs.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_client_stopped_on_a_state_that_needs_the_user_is_not_restarted() {
        let dir = tempfile::tempdir().unwrap();
        let rec = Arc::new(Recorder::default());
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runner: Runner = {
            let runs = runs.clone();
            Arc::new(move |_params, events, _timing| {
                runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async move {
                    let _ = events
                        .send(ClientEvent::Connection(ConnectionState::AuthFailed))
                        .await;
                })
            })
        };
        let timing = Timing {
            backoff_min: Duration::from_millis(20),
            backoff_max: Duration::from_millis(40),
            pushall_interval: Duration::from_secs(300),
        };
        let svc = PrinterService::with_runner(
            rec,
            Some(dir.path().join("history.db")),
            dir.path().to_path_buf(),
            "http://127.0.0.1:9/query.php",
            timing,
            runner,
        );
        let ca = TestCa::new("Test Printer CA");
        let serial = "0948SV000000017";
        let rejection = RejectionSlot::default();
        let verifier =
            PrinterCertVerifier::with_trust(&ca.pem, &[], serial, None, rejection.clone()).unwrap();
        svc.start_with(
            config_for(serial),
            ClientParams {
                host: "127.0.0.1".into(),
                port: 1,
                serial: serial.into(),
                access_code: "12345678".into(),
                tls: client_config(Arc::new(verifier)).unwrap(),
                rejection,
            },
        );
        wait_for(|| svc.connection() == ConnectionState::AuthFailed).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(svc.connection(), ConnectionState::AuthFailed);
        svc.stop();
    }

    #[tokio::test]
    async fn a_resolved_filament_id_is_saved_and_only_missing_ids_are_looked_up() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        // B2 holds P4d6ae04. Assigned before the preset had an id: can't check.
        svc.assign_slot(1, 1, "Mine", None, Some("/u/Mine.json"))
            .unwrap();
        svc.assign_slot(0, 2, "Acme PETG", Some("P0000001"), None)
            .unwrap();
        let b2 = |v: &PrinterView| v.slots.iter().find(|s| s.label == "B2").unwrap().status;
        assert_ne!(b2(&svc.view()), SlotStatus::Matches);

        let mut asked = Vec::new();
        svc.refresh_assignments_with(&mut |a| {
            asked.push(a.preset_name.clone());
            Some("P4d6ae04".into())
        });
        assert_eq!(asked, ["Mine"], "only the assignment without an id");
        assert_eq!(b2(&svc.view()), SlotStatus::Matches);
        // Saved: a plain refresh keeps it.
        svc.refresh_assignments();
        assert_eq!(b2(&svc.view()), SlotStatus::Matches);
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
