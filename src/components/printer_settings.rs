//! Settings → Printer: discovery, manual entry, the access code, Test
//! connection and Trust this printer. The access code is sent to the
//! backend once and never shown again.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::printer::bridge;
use crate::printer::types::{
    connection_message, untrusted_text, ConnectionState, DiscoveredPrinter, PrinterConfigView,
    TestOutcome, PRESENTED_MAX_CHARS,
};
use crate::printer::PrinterShared;

const SERIAL_HINT: &str = "Check this serial matches the label on your printer.";
const CONFLICT_WARNING: &str = "Two devices on your network claim this serial — check the IP on the printer screen before connecting.";
pub const TRUSTED_CONNECTING: &str = "Trusted — connecting…";
pub const DOWNGRADE_WARNING: &str = "This printer previously proved it's a genuine Bambu printer. A new, unrecognised certificate could mean another device is impersonating it. Only trust it if you've just replaced or reset the printer.";
pub const TRUST_LABEL: &str = "Trust this printer";
pub const CONFIRM_TRUST_LABEL: &str = "Yes, trust this certificate";

/// `connection_message` as Settings → Printer words it: without pointing
/// the user to Settings → Printer, where they already are.
pub fn settings_message(state: &ConnectionState, ip: &str) -> Option<String> {
    match state {
        ConnectionState::CertUntrusted { .. } => {
            Some("The printer's certificate isn't from a Bambu CA that BambuMate knows.".into())
        }
        ConnectionState::WrongSerial { presented } => Some(format!(
            "The printer at {ip} reports serial {}. Check the serial number.",
            untrusted_text(presented, PRESENTED_MAX_CHARS)
        )),
        other => connection_message(other, ip),
    }
}

/// The line shown after Test connection.
pub fn outcome_message(outcome: &TestOutcome, ip: &str) -> String {
    match (&outcome.connection, outcome.got_report) {
        (ConnectionState::Connected, true) => format!(
            "Connected to {}. Live status is on the Printer page.",
            outcome
                .model
                .clone()
                .unwrap_or_else(|| "the printer".into())
        ),
        (ConnectionState::Connected, false) => {
            "Connected, but the printer hasn't sent its status yet.".into()
        }
        (state, _) => settings_message(state, ip).unwrap_or_default(),
    }
}

/// Fingerprints compared the way the backend compares them: hex digits
/// only, any case.
pub fn same_fingerprint(a: &str, b: &str) -> bool {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(char::is_ascii_hexdigit)
            .map(|c| c.to_ascii_uppercase())
            .collect()
    };
    let a = norm(a);
    !a.is_empty() && a == norm(b)
}

/// What the saved printer's live connection looks like to Settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveInfo {
    pub configured: bool,
    pub connection: ConnectionState,
    /// A status report has arrived.
    pub has_state: bool,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub ca_verified: bool,
}

/// The line shown after Trust this printer, from the live connection the
/// save restarted (Trust runs no separate test). `pinned` is the
/// fingerprint just trusted: until the restarted connection reports, the
/// live state may still be the old rejection of that same certificate.
pub fn trust_progress(pinned: &str, live: &LiveInfo, ip: &str) -> (String, bool) {
    match &live.connection {
        ConnectionState::Connected => (
            outcome_message(
                &TestOutcome {
                    connection: ConnectionState::Connected,
                    got_report: live.has_state,
                    model: live.model.clone(),
                },
                ip,
            ),
            true,
        ),
        ConnectionState::CertUntrusted { fingerprint } if same_fingerprint(fingerprint, pinned) => {
            (TRUSTED_CONNECTING.into(), true)
        }
        ConnectionState::Connecting | ConnectionState::Disconnected => {
            (TRUSTED_CONNECTING.into(), true)
        }
        other => (settings_message(other, ip).unwrap_or_default(), false),
    }
}

/// Whether `serial` has verified against a Bambu CA before, per the saved
/// settings or the live connection. A new untrusted certificate for such a
/// printer gets the downgrade warning and a second confirmation.
pub fn previously_verified(
    serial: &str,
    saved: Option<&PrinterConfigView>,
    live: &LiveInfo,
) -> bool {
    let serial = serial.trim().to_ascii_uppercase();
    saved.is_some_and(|c| c.ca_verified && c.serial == serial)
        || (live.ca_verified && live.serial.as_deref() == Some(serial.as_str()))
}

/// A certificate the user may trust, and the printer that presented it.
/// Trusting pins `fingerprint` for exactly this IP and serial.
#[derive(Debug, Clone, PartialEq)]
pub struct TrustOffer {
    pub ip: String,
    pub serial: String,
    pub fingerprint: String,
}

/// The saved printer's live connection rejected its certificate. Trusting
/// it needs no retyped code: the backend accepts the stored code for the
/// saved IP and serial with the fingerprint the live connection reported.
pub fn live_trust_offer(
    saved: Option<&PrinterConfigView>,
    connection: &ConnectionState,
) -> Option<TrustOffer> {
    let saved = saved?;
    match connection {
        ConnectionState::CertUntrusted { fingerprint } => Some(TrustOffer {
            ip: saved.ip.clone(),
            serial: saved.serial.clone(),
            fingerprint: fingerprint.clone(),
        }),
        _ => None,
    }
}

/// What the saved printer's live connection needs from the user. The
/// backend doesn't retry these states, so Settings says so; states it
/// retries show nothing here.
pub fn live_message(
    saved: Option<&PrinterConfigView>,
    configured: bool,
    connection: &ConnectionState,
) -> Option<String> {
    let saved = saved?;
    if !configured || !connection.needs_user() {
        return None;
    }
    settings_message(connection, &saved.ip)
}

/// Whether a pin set for `target` (IP, serial) still applies to the form.
pub fn pin_applies(target: Option<&(String, String)>, ip: &str, serial: &str) -> bool {
    target.is_some_and(|(t_ip, t_serial)| {
        t_ip == ip.trim() && *t_serial == serial.trim().to_ascii_uppercase()
    })
}

/// `Workshop H2D · H2D` for a discovery row.
pub fn found_title(p: &DiscoveredPrinter) -> String {
    let parts: Vec<&str> = [p.name.as_str(), p.model.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        "Bambu printer".into()
    } else {
        parts.join(" · ")
    }
}

/// The form values a request sends, read without tracking. `None` once the
/// section is gone, so work finishing after the user left does nothing.
struct Form {
    ip: String,
    serial: String,
    name: String,
    model: String,
    code: String,
    pin: String,
}

#[component]
pub fn PrinterSettings() -> impl IntoView {
    let shared = use_context::<PrinterShared>();
    let ip = RwSignal::new(String::new());
    let serial = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let model = RwSignal::new(String::new());
    let code = RwSignal::new(String::new());
    let pin = RwSignal::new(String::new());
    // The (IP, serial) `pin` was set for; editing either away from it clears the pin.
    let pin_target = RwSignal::new(Option::<(String, String)>::None);
    let saved = RwSignal::new(Option::<PrinterConfigView>::None);
    let found = RwSignal::new(Option::<Vec<DiscoveredPrinter>>::None);
    let scanning = RwSignal::new(false);
    let scan_elapsed = RwSignal::new(0u64);
    let discovery_error = RwSignal::new(Option::<String>::None);
    let discovery_warnings = RwSignal::new(Vec::<String>::new());
    // Keep progress alive only while this section exists. After 15 seconds
    // say we're finishing, rather than claiming to listen indefinitely.
    if let Ok(handle) = set_interval_with_handle(
        move || {
            if scanning.try_get_untracked() == Some(true) {
                scan_elapsed.try_update(|n| *n = (*n + 1).min(15));
            }
        },
        std::time::Duration::from_secs(1),
    ) {
        on_cleanup(move || handle.clear());
    }
    let busy = RwSignal::new(false);
    let message = RwSignal::new(Option::<(String, bool)>::None);
    // From Test connection; the live connection's offer is derived below.
    let untrusted = RwSignal::new(Option::<TrustOffer>::None);
    // The fingerprint just trusted: the result line then follows the live
    // connection instead of a Test connection.
    let trusted = RwSignal::new(Option::<String>::None);
    // The fingerprint whose downgrade warning got its first click.
    let confirming = RwSignal::new(Option::<String>::None);

    // All reads after an await go through this: the user may have left
    // Settings, which disposes these signals.
    let form = move || {
        Some(Form {
            ip: ip.try_get_untracked()?,
            serial: serial.try_get_untracked()?,
            name: name.try_get_untracked()?,
            model: model.try_get_untracked()?,
            code: code.try_get_untracked()?,
            pin: pin.try_get_untracked()?,
        })
    };

    let set_pin = move |value: String, target: Option<(String, String)>| {
        pin.try_set(value);
        pin_target.try_set(target);
    };
    // Drops the pin once the form no longer names the printer it was set for.
    let check_pin = move || {
        let (Some(i), Some(s), Some(t)) = (
            ip.try_get_untracked(),
            serial.try_get_untracked(),
            pin_target.try_get_untracked(),
        ) else {
            return;
        };
        if !pin_applies(t.as_ref(), &i, &s) {
            set_pin(String::new(), None);
        }
    };

    // Fills the form from the saved printer, or empties it when there is
    // none (after Reset for clean install).
    let load_config = move || {
        spawn_local(async move {
            let Ok(config) = bridge::get_config().await else {
                return;
            };
            match config {
                Some(c) => {
                    ip.try_set(c.ip.clone());
                    serial.try_set(c.serial.clone());
                    name.try_set(c.name.clone());
                    model.try_set(c.model.clone());
                    set_pin(
                        c.pinned_fingerprint.clone().unwrap_or_default(),
                        Some((c.ip.clone(), c.serial.clone())),
                    );
                    saved.try_set(Some(c));
                }
                None => {
                    if saved.try_get_untracked().flatten().is_none() {
                        return; // Nothing was shown; keep what the user typed.
                    }
                    for s in [ip, serial, name, model, code] {
                        s.try_set(String::new());
                    }
                    set_pin(String::new(), None);
                    saved.try_set(None);
                    untrusted.try_set(None);
                    trusted.try_set(None);
                    message.try_set(None);
                }
            }
        });
    };
    load_config();
    if let Some(s) = shared {
        Effect::new(move |seen: Option<u64>| {
            let changes = s.config_changes();
            if seen.is_some_and(|n| n != changes) {
                load_config();
            }
            changes
        });
    }

    // Only these fields matter here, so a state event that changes
    // temperatures doesn't re-render the section.
    let live = Memo::new(move |_| {
        shared
            .map(|s| {
                s.view.with(|v| LiveInfo {
                    configured: v.configured,
                    connection: v.connection.clone(),
                    has_state: v.state.is_some(),
                    model: v.printer.as_ref().map(|p| p.model.clone()),
                    serial: v.printer.as_ref().map(|p| p.serial.clone()),
                    ca_verified: v.printer.as_ref().is_some_and(|p| p.ca_verified),
                })
            })
            .unwrap_or_default()
    });
    // A live offer for the certificate just trusted is the connection's
    // stale state from before the restart, not a new problem.
    let offer = move || {
        untrusted
            .get()
            .or_else(|| live.with(|l| live_trust_offer(saved.get().as_ref(), &l.connection)))
            .filter(|o| {
                trusted.with(|t| {
                    t.as_deref()
                        .is_none_or(|t| !same_fingerprint(t, &o.fingerprint))
                })
            })
    };
    // A new problem after Trust (anything that needs the user other than
    // the stale rejection of the certificate just trusted) ends the trust
    // result line; the live status line then says what is wrong.
    Effect::new(move |_| {
        let Some(fp) = trusted.get() else {
            return;
        };
        let new_problem = live.with(|l| match &l.connection {
            ConnectionState::CertUntrusted { fingerprint } => !same_fingerprint(fingerprint, &fp),
            other => other.needs_user(),
        });
        if new_problem {
            trusted.set(None);
        }
    });
    let warn_downgrade = move |o: &TrustOffer| {
        live.with(|l| previously_verified(&o.serial, saved.get().as_ref(), l))
    };
    let shown_message = move || match trusted.get() {
        Some(fp) => {
            let ip_now = saved
                .with(|c| c.as_ref().map(|c| c.ip.clone()))
                .unwrap_or_default();
            Some(live.with(|l| trust_progress(&fp, l, &ip_now)))
        }
        None => message.get(),
    };

    let scan = move |_| {
        if scanning.get_untracked() {
            return;
        }
        scanning.set(true);
        scan_elapsed.set(0);
        found.set(None);
        discovery_error.set(None);
        discovery_warnings.set(Vec::new());
        spawn_local(async move {
            match bridge::discover().await {
                Ok(report) => {
                    found.try_set(Some(report.printers));
                    discovery_warnings.try_set(report.warnings);
                }
                Err(e) => {
                    discovery_error.try_set(Some(e));
                }
            }
            scanning.try_set(false);
        });
    };

    let pick = move |p: DiscoveredPrinter| {
        ip.set(p.ip);
        serial.set(p.serial);
        name.set(p.name);
        model.set(p.model);
        message.set(None);
        untrusted.set(None);
        trusted.set(None);
        check_pin();
    };

    // Tests with the form's values; the stored code is used when the field
    // is blank and the backend allows it.
    let run_test = move || {
        let Some(f) = form() else {
            return;
        };
        busy.try_set(true);
        message.try_set(None);
        untrusted.try_set(None);
        trusted.try_set(None);
        spawn_local(async move {
            let result = bridge::test_connection(&f.ip, &f.serial, &f.code, &f.pin).await;
            match result {
                Ok(outcome) => {
                    if let Some(m) = outcome.model.clone() {
                        model.try_set(m);
                    }
                    if let ConnectionState::CertUntrusted { fingerprint } = &outcome.connection {
                        untrusted.try_set(Some(TrustOffer {
                            ip: f.ip.trim().to_string(),
                            serial: f.serial.trim().to_ascii_uppercase(),
                            fingerprint: fingerprint.clone(),
                        }));
                    }
                    let ok = outcome.connection == ConnectionState::Connected;
                    message.try_set(Some((outcome_message(&outcome, &f.ip), ok)));
                }
                Err(e) => {
                    message.try_set(Some((e, false)));
                }
            }
            busy.try_set(false);
        });
    };

    // Saves the form (and a pin, when given), then refreshes the shared view.
    // `trusting` is the fingerprint Trust this printer pins: the save
    // restarts the live connection, which then shows the result (no second
    // session to the printer for a test).
    let run_save = move |trusting: Option<String>| {
        let Some(f) = form() else {
            return;
        };
        busy.try_set(true);
        trusted.try_set(None);
        spawn_local(async move {
            let result = bridge::save(&f.ip, &f.serial, &f.name, &f.model, &f.code, &f.pin).await;
            if let Some(s) = shared {
                if result.is_ok() {
                    s.refresh();
                }
            }
            if busy.try_set(false).is_some() {
                return; // The section is gone.
            }
            match result {
                Ok(c) => {
                    code.try_set(String::new());
                    saved.try_set(Some(c));
                    if trusting.is_some() {
                        untrusted.try_set(None);
                        message.try_set(None);
                        trusted.try_set(trusting);
                    } else {
                        message.try_set(Some(("Saved.".into(), true)));
                    }
                }
                Err(e) => {
                    message.try_set(Some((e, false)));
                }
            }
        });
    };

    // Pins the offered certificate for the printer that presented it. For
    // a printer that once verified against a Bambu CA, the first click only
    // arms the confirmation.
    let trust = move |_| {
        let Some(o) = untrack(offer) else {
            return;
        };
        let armed = confirming.with_untracked(|c| c.as_deref() == Some(o.fingerprint.as_str()));
        if untrack(|| warn_downgrade(&o)) && !armed {
            confirming.set(Some(o.fingerprint));
            return;
        }
        confirming.set(None);
        ip.set(o.ip.clone());
        serial.set(o.serial.clone());
        set_pin(o.fingerprint.clone(), Some((o.ip, o.serial)));
        run_save(Some(o.fingerprint));
    };

    let remove = move |_| {
        busy.set(true);
        spawn_local(async move {
            let result = bridge::remove().await;
            if result.is_ok() {
                if let Some(s) = shared {
                    s.refresh();
                }
            }
            if busy.try_set(false).is_some() {
                return; // The section is gone.
            }
            match result {
                Ok(()) => {
                    for s in [ip, serial, name, model, code] {
                        s.try_set(String::new());
                    }
                    set_pin(String::new(), None);
                    saved.try_set(None);
                    untrusted.try_set(None);
                    trusted.try_set(None);
                    message.try_set(Some(("Printer removed.".into(), true)));
                }
                Err(e) => {
                    message.try_set(Some((e, false)));
                }
            }
        });
    };

    let code_placeholder = move || {
        if saved.get().is_some_and(|c| c.has_access_code) {
            "Saved in the system keychain"
        } else {
            "Shown on the printer screen (Settings → LAN)"
        }
    };

    view! {
        <section class="settings-section printer-settings" id="printer">
            <h3>"Printer"</h3>
            <p class="section-description">
                "Connect to your Bambu printer on the local network to see live status and what is loaded in each AMS slot. BambuMate only reads from the printer."
            </p>

            // After Trust the result line follows the live connection instead.
            {move || trusted.with(Option::is_none).then(|| live.with(|l| live_message(saved.get().as_ref(), l.configured, &l.connection))).flatten().map(|m| view! {
                <p class="status-text status-warning printer-live-status">{m}</p>
            })}

            <div class="form-group">
                <button class="btn btn-secondary btn-sm printer-scan" on:click=scan disabled=move || scanning.get()>
                    {move || if scanning.get() { "Searching…" } else { "Find printers" }}
                </button>
                <Show when=move || scanning.get()>
                    <p class="section-description printer-scan-progress" role="status">
                        {move || {
                            let elapsed = scan_elapsed.get();
                            if elapsed < 15 {
                                format!("Listening for printer announcements… {} seconds remaining. Keep the printer awake and on the same local network.", 15 - elapsed)
                            } else {
                                "Finishing the discovery scan…".to_string()
                            }
                        }}
                    </p>
                </Show>
                {move || discovery_error.get().map(|e| view! {
                    <p class="status-text status-warning printer-discovery-error" role="alert">{e}</p>
                })}
                {move || (!discovery_warnings.get().is_empty()).then(|| view! {
                    <p class="status-text status-warning printer-discovery-warning" role="status">
                        "Some discovery listeners were unavailable: " {move || discovery_warnings.get().join("; ")}
                    </p>
                })}
                {move || found.get().map(|list| {
                    if list.is_empty() {
                        view! {
                            <p class="section-description printer-none">
                                "No printer announcements received in 15 seconds. Keep the printer awake and on the same local network, then retry, or enter its IP address and serial number below."
                            </p>
                        }
                        .into_any()
                    } else {
                        view! {
                            <ul class="printer-found">
                                {list.into_iter().map(|p| {
                                    let title = found_title(&p);
                                    let (ip_text, serial_text, conflict) = (p.ip.clone(), p.serial.clone(), p.conflict);
                                    view! {
                                        <li class="printer-found-row" class:printer-found-conflict=conflict>
                                            <button class="printer-found-item" on:click=move |_| pick(p.clone())>
                                                <span class="printer-found-title">{title}</span>
                                                <span class="printer-found-serial">
                                                    "Serial " <code>{serial_text}</code>
                                                </span>
                                                <span class="printer-found-ip">{ip_text}</span>
                                            </button>
                                            {conflict.then(|| view! {
                                                <p class="status-text status-warning printer-conflict">{CONFLICT_WARNING}</p>
                                            })}
                                        </li>
                                    }
                                }).collect::<Vec<_>>()}
                            </ul>
                        }
                        .into_any()
                    }
                })}
            </div>

            <div class="form-group">
                <label for="printer-ip">"IP address"</label>
                <input id="printer-ip" class="input" type="text" placeholder="192.168.1.20"
                    prop:value=move || ip.get()
                    on:input=move |ev| {
                        ip.set(event_target_value(&ev));
                        check_pin();
                    } />
            </div>
            <div class="form-group">
                <label for="printer-serial">"Serial number"</label>
                <input id="printer-serial" class="input printer-serial-input" type="text"
                    autocomplete="off" spellcheck="false"
                    prop:value=move || serial.get()
                    on:input=move |ev| {
                        serial.set(event_target_value(&ev));
                        check_pin();
                    } />
                <p class="section-description printer-serial-hint">{SERIAL_HINT}</p>
            </div>
            <div class="form-group">
                <label for="printer-code">"Access code"</label>
                <input id="printer-code" class="input" type="password" autocomplete="off"
                    placeholder=code_placeholder
                    prop:value=move || code.get()
                    on:input=move |ev| code.set(event_target_value(&ev)) />
            </div>

            <div class="input-row">
                <button class="btn btn-secondary printer-test" on:click=move |_| run_test() disabled=move || busy.get()>
                    "Test connection"
                </button>
                <button class="btn btn-save printer-save" on:click=move |_| run_save(None) disabled=move || busy.get()>
                    "Save"
                </button>
                <Show when=move || saved.get().is_some()>
                    <button class="btn btn-danger btn-sm printer-remove" on:click=remove disabled=move || busy.get()>
                        "Remove printer"
                    </button>
                </Show>
            </div>

            <Show when=move || busy.get()>
                <span class="status-text">"Connecting…"</span>
            </Show>
            {move || shown_message().map(|(text, ok)| view! {
                <p class={if ok { "status-text status-success printer-result" } else { "status-text status-warning printer-result" }}>
                    {text}
                </p>
            })}

            {move || offer().map(|o| {
                let warn = warn_downgrade(&o);
                let fp = o.fingerprint.clone();
                let armed = move || confirming.with(|c| c.as_deref() == Some(fp.as_str()));
                view! {
                <div class="printer-trust">
                    {warn.then(|| view! {
                        <p class="status-text status-warning printer-trust-warning" role="alert">{DOWNGRADE_WARNING}</p>
                    })}
                    <p class="section-description">
                        "This printer's certificate isn't signed by a Bambu CA that BambuMate knows. If this fingerprint matches your printer, trust it. From now on BambuMate accepts this certificate, or one signed by a Bambu CA, for this serial."
                    </p>
                    <dl class="printer-trust-details">
                        <dt>"Serial number"</dt>
                        <dd><code class="printer-trust-serial">{o.serial}</code></dd>
                        <dt>"SHA-256 fingerprint"</dt>
                        <dd><code class="printer-fingerprint">{o.fingerprint}</code></dd>
                    </dl>
                    <p class="section-description printer-serial-hint">{SERIAL_HINT}</p>
                    <button class="btn btn-sm printer-trust-btn"
                        class:btn-primary=move || !warn
                        class:btn-danger=warn
                        on:click=trust disabled=move || busy.get()>
                        {move || if warn && armed() { CONFIRM_TRUST_LABEL } else { TRUST_LABEL }}
                    </button>
                </div>
                }
            })}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_messages() {
        let ok = TestOutcome {
            connection: ConnectionState::Connected,
            got_report: true,
            model: Some("H2D".into()),
        };
        assert_eq!(
            outcome_message(&ok, "10.0.0.2"),
            "Connected to H2D. Live status is on the Printer page."
        );
        let refused = TestOutcome {
            connection: ConnectionState::AuthFailed,
            ..Default::default()
        };
        assert_eq!(
            outcome_message(&refused, "10.0.0.2"),
            "The access code was rejected. Check it on the printer screen (Settings → LAN)."
        );
    }

    fn saved() -> PrinterConfigView {
        PrinterConfigView {
            ip: "192.168.1.20".into(),
            serial: "0948AB000000001".into(),
            has_access_code: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_live_untrusted_certificate_is_offered_for_the_saved_printer() {
        let untrusted = ConnectionState::CertUntrusted {
            fingerprint: "AB:CD".into(),
        };
        assert_eq!(
            live_trust_offer(Some(&saved()), &untrusted),
            Some(TrustOffer {
                ip: "192.168.1.20".into(),
                serial: "0948AB000000001".into(),
                fingerprint: "AB:CD".into(),
            })
        );
        assert_eq!(live_trust_offer(None, &untrusted), None);
        assert_eq!(
            live_trust_offer(Some(&saved()), &ConnectionState::Connected),
            None
        );
    }

    #[test]
    fn a_live_state_that_needs_the_user_is_shown_in_settings() {
        assert_eq!(
            live_message(Some(&saved()), true, &ConnectionState::AuthFailed).as_deref(),
            Some("The access code was rejected. Check it on the printer screen (Settings → LAN).")
        );
        let wrong = ConnectionState::WrongSerial {
            presented: "EVIL".into(),
        };
        assert!(live_message(Some(&saved()), true, &wrong)
            .unwrap()
            .contains("192.168.1.20 reports serial EVIL"));
        // Retried by the backend, not configured, or nothing saved: nothing to ask.
        assert_eq!(
            live_message(Some(&saved()), true, &ConnectionState::Unreachable),
            None
        );
        assert_eq!(
            live_message(Some(&saved()), false, &ConnectionState::AuthFailed),
            None
        );
        assert_eq!(live_message(None, true, &ConnectionState::AuthFailed), None);
    }

    #[test]
    fn settings_messages_do_not_send_the_user_to_settings() {
        let untrusted = ConnectionState::CertUntrusted {
            fingerprint: "AB:CD".into(),
        };
        let wrong = ConnectionState::WrongSerial {
            presented: "EVIL".into(),
        };
        assert_eq!(
            settings_message(&untrusted, "10.0.0.2").as_deref(),
            Some("The printer's certificate isn't from a Bambu CA that BambuMate knows.")
        );
        assert_eq!(
            settings_message(&wrong, "10.0.0.2").as_deref(),
            Some("The printer at 10.0.0.2 reports serial EVIL. Check the serial number.")
        );
        for state in [untrusted.clone(), wrong.clone()] {
            let on_page = settings_message(&state, "10.0.0.2").unwrap();
            assert!(!on_page.contains("Settings → Printer"), "{on_page}");
            let live = live_message(Some(&saved()), true, &state).unwrap();
            assert!(!live.contains("Settings → Printer"), "{live}");
            let outcome = TestOutcome {
                connection: state,
                ..Default::default()
            };
            assert!(!outcome_message(&outcome, "10.0.0.2").contains("Settings → Printer"));
        }
        // Other states keep the shared copy.
        assert_eq!(
            settings_message(&ConnectionState::Unreachable, "10.0.0.2"),
            connection_message(&ConnectionState::Unreachable, "10.0.0.2")
        );
    }

    fn live(connection: ConnectionState) -> LiveInfo {
        LiveInfo {
            configured: true,
            connection,
            has_state: true,
            model: Some("H2D".into()),
            serial: Some("0948AB000000001".into()),
            ca_verified: false,
        }
    }

    #[test]
    fn after_trust_the_result_follows_the_live_connection() {
        let pinned = "AB:CD:EF";
        let at = |c| trust_progress(pinned, &live(c), "192.168.1.20");
        assert_eq!(
            at(ConnectionState::Connecting),
            (TRUSTED_CONNECTING.to_string(), true)
        );
        assert_eq!(
            at(ConnectionState::Disconnected),
            (TRUSTED_CONNECTING.to_string(), true)
        );
        // The old rejection of the certificate just trusted is stale.
        let stale = ConnectionState::CertUntrusted {
            fingerprint: "abcdef".into(),
        };
        assert_eq!(at(stale), (TRUSTED_CONNECTING.to_string(), true));
        assert_eq!(
            at(ConnectionState::Connected),
            (
                "Connected to H2D. Live status is on the Printer page.".to_string(),
                true
            )
        );
        let (text, ok) = at(ConnectionState::CertUntrusted {
            fingerprint: "11:22".into(),
        });
        assert!(!ok);
        assert_eq!(
            text,
            "The printer's certificate isn't from a Bambu CA that BambuMate knows."
        );
        assert_eq!(
            at(ConnectionState::Unreachable),
            (
                "Can't reach the printer at 192.168.1.20.".to_string(),
                false
            )
        );
    }

    #[test]
    fn fingerprints_match_ignoring_separators_and_case() {
        assert!(same_fingerprint("AB:CD:01", "abcd01"));
        assert!(!same_fingerprint("AB:CD:01", "AB:CD:02"));
        assert!(!same_fingerprint("zz", "zz"), "no hex digits never matches");
    }

    #[test]
    fn a_printer_that_verified_before_gets_the_downgrade_warning() {
        let verified = PrinterConfigView {
            ca_verified: true,
            ..saved()
        };
        let none = LiveInfo::default();
        assert!(previously_verified(
            "0948ab000000001",
            Some(&verified),
            &none
        ));
        assert!(!previously_verified(
            "0948AB000000002",
            Some(&verified),
            &none
        ));
        assert!(!previously_verified(
            "0948AB000000001",
            Some(&saved()),
            &none
        ));
        let live_verified = LiveInfo {
            ca_verified: true,
            ..live(ConnectionState::Connected)
        };
        assert!(previously_verified(
            "0948AB000000001",
            Some(&saved()),
            &live_verified
        ));
        assert!(previously_verified("0948AB000000001", None, &live_verified));
        assert!(!previously_verified(
            "0948AB000000002",
            None,
            &live_verified
        ));
    }

    #[test]
    fn a_pin_applies_only_to_the_printer_it_was_set_for() {
        let target = ("192.168.1.20".to_string(), "0948AB000000001".to_string());
        assert!(pin_applies(
            Some(&target),
            "192.168.1.20",
            "0948AB000000001"
        ));
        assert!(pin_applies(
            Some(&target),
            " 192.168.1.20 ",
            "0948ab000000001"
        ));
        assert!(!pin_applies(
            Some(&target),
            "192.168.1.21",
            "0948AB000000001"
        ));
        assert!(!pin_applies(
            Some(&target),
            "192.168.1.20",
            "0948AB000000002"
        ));
        assert!(!pin_applies(None, "192.168.1.20", "0948AB000000001"));
    }

    #[test]
    fn a_found_printer_is_named_by_name_and_model() {
        let p = |name: &str, model: &str| DiscoveredPrinter {
            name: name.into(),
            model: model.into(),
            ..Default::default()
        };
        assert_eq!(found_title(&p("Workshop H2D", "H2D")), "Workshop H2D · H2D");
        assert_eq!(found_title(&p("", "H2D")), "H2D");
        assert_eq!(found_title(&p("", "")), "Bambu printer");
    }
}
