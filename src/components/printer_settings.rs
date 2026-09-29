//! Settings → Printer: discovery, manual entry, the access code, Test
//! connection and Trust this printer. The access code is sent to the
//! backend once and never shown again.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::printer::bridge;
use crate::printer::types::{
    connection_message, ConnectionState, DiscoveredPrinter, PrinterConfigView, PrinterView,
    TestOutcome,
};
use crate::printer::PrinterShared;

const SERIAL_HINT: &str = "Check this serial matches the label on your printer.";
const CONFLICT_WARNING: &str = "Two devices on your network claim this serial — check the IP on the printer screen before connecting.";

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
        (state, _) => connection_message(state, ip).unwrap_or_default(),
    }
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
pub fn live_message(saved: Option<&PrinterConfigView>, view: &PrinterView) -> Option<String> {
    let saved = saved?;
    if !view.configured || !view.connection.needs_user() {
        return None;
    }
    connection_message(&view.connection, &saved.ip)
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

#[component]
pub fn PrinterSettings() -> impl IntoView {
    let shared = use_context::<PrinterShared>();
    let ip = RwSignal::new(String::new());
    let serial = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let model = RwSignal::new(String::new());
    let code = RwSignal::new(String::new());
    let pin = RwSignal::new(String::new());
    let saved = RwSignal::new(Option::<PrinterConfigView>::None);
    let found = RwSignal::new(Option::<Vec<DiscoveredPrinter>>::None);
    let scanning = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let message = RwSignal::new(Option::<(String, bool)>::None);
    // From Test connection; the live connection's offer is derived below.
    let untrusted = RwSignal::new(Option::<TrustOffer>::None);

    let load = move || {
        spawn_local(async move {
            if let Ok(Some(c)) = bridge::get_config().await {
                ip.set(c.ip.clone());
                serial.set(c.serial.clone());
                name.set(c.name.clone());
                model.set(c.model.clone());
                pin.set(c.pinned_fingerprint.clone().unwrap_or_default());
                saved.set(Some(c));
            }
        });
    };
    load();

    let live_view = move || shared.map(|s| s.view.get()).unwrap_or_default();
    let offer = move || {
        untrusted
            .get()
            .or_else(|| live_trust_offer(saved.get().as_ref(), &live_view().connection))
    };

    let scan = move |_| {
        scanning.set(true);
        found.set(None);
        spawn_local(async move {
            let list = bridge::discover().await.unwrap_or_default();
            found.set(Some(list));
            scanning.set(false);
        });
    };

    let pick = move |p: DiscoveredPrinter| {
        // The saved pin belongs to the saved printer only.
        let same = saved
            .get_untracked()
            .is_some_and(|c| c.ip == p.ip && c.serial == p.serial);
        if !same {
            pin.set(String::new());
        }
        ip.set(p.ip);
        serial.set(p.serial);
        name.set(p.name);
        model.set(p.model);
        message.set(None);
        untrusted.set(None);
    };

    // Tests with the form's values; the stored code is used when the field
    // is blank and the backend allows it.
    let run_test = move || {
        busy.set(true);
        message.set(None);
        untrusted.set(None);
        spawn_local(async move {
            let (i, s) = (ip.get_untracked(), serial.get_untracked());
            match bridge::test_connection(&i, &s, &code.get_untracked(), &pin.get_untracked()).await
            {
                Ok(outcome) => {
                    if let Some(m) = outcome.model.clone() {
                        model.set(m);
                    }
                    if let ConnectionState::CertUntrusted { fingerprint } = &outcome.connection {
                        untrusted.set(Some(TrustOffer {
                            ip: i.trim().to_string(),
                            serial: s.trim().to_ascii_uppercase(),
                            fingerprint: fingerprint.clone(),
                        }));
                    }
                    let ok = outcome.connection == ConnectionState::Connected;
                    message.set(Some((outcome_message(&outcome, &i), ok)));
                }
                Err(e) => message.set(Some((e, false))),
            }
            busy.set(false);
        });
    };

    // Saves the form (and a pin, when given), then refreshes the shared view.
    let run_save = move |then_test: bool| {
        busy.set(true);
        spawn_local(async move {
            let result = bridge::save(
                &ip.get_untracked(),
                &serial.get_untracked(),
                &name.get_untracked(),
                &model.get_untracked(),
                &code.get_untracked(),
                &pin.get_untracked(),
            )
            .await;
            busy.set(false);
            match result {
                Ok(c) => {
                    code.set(String::new());
                    saved.set(Some(c));
                    if let Some(s) = shared {
                        s.refresh();
                    }
                    if then_test {
                        run_test();
                    } else {
                        message.set(Some(("Saved.".into(), true)));
                    }
                }
                Err(e) => message.set(Some((e, false))),
            }
        });
    };

    // Pins the offered certificate for the printer that presented it.
    let trust = move |_| {
        let current = untrusted.get_untracked().or_else(|| {
            live_trust_offer(
                saved.get_untracked().as_ref(),
                &shared
                    .map(|s| s.view.get_untracked().connection)
                    .unwrap_or_default(),
            )
        });
        if let Some(o) = current {
            ip.set(o.ip);
            serial.set(o.serial);
            pin.set(o.fingerprint);
            run_save(true);
        }
    };

    let remove = move |_| {
        spawn_local(async move {
            match bridge::remove().await {
                Ok(()) => {
                    for s in [ip, serial, name, model, code, pin] {
                        s.set(String::new());
                    }
                    saved.set(None);
                    untrusted.set(None);
                    message.set(Some(("Printer removed.".into(), true)));
                    if let Some(s) = shared {
                        s.refresh();
                    }
                }
                Err(e) => message.set(Some((e, false))),
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

            {move || live_message(saved.get().as_ref(), &live_view()).map(|m| view! {
                <p class="status-text status-warning printer-live-status">{m}</p>
            })}

            <div class="form-group">
                <button class="btn btn-secondary btn-sm printer-scan" on:click=scan disabled=move || scanning.get()>
                    {move || if scanning.get() { "Searching…" } else { "Find printers" }}
                </button>
                {move || found.get().map(|list| {
                    if list.is_empty() {
                        view! {
                            <p class="section-description printer-none">
                                "No printers found. Enter the IP address and serial number below."
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
                    on:input=move |ev| ip.set(event_target_value(&ev)) />
            </div>
            <div class="form-group">
                <label for="printer-serial">"Serial number"</label>
                <input id="printer-serial" class="input printer-serial-input" type="text"
                    autocomplete="off" spellcheck="false"
                    prop:value=move || serial.get()
                    on:input=move |ev| serial.set(event_target_value(&ev)) />
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
                <button class="btn btn-save printer-save" on:click=move |_| run_save(false) disabled=move || busy.get()>
                    "Save"
                </button>
                <Show when=move || saved.get().is_some()>
                    <button class="btn btn-danger btn-sm printer-remove" on:click=remove>"Remove printer"</button>
                </Show>
            </div>

            <Show when=move || busy.get()>
                <span class="status-text">"Connecting…"</span>
            </Show>
            {move || message.get().map(|(text, ok)| view! {
                <p class={if ok { "status-text status-success printer-result" } else { "status-text status-warning printer-result" }}>
                    {text}
                </p>
            })}

            {move || offer().map(|o| view! {
                <div class="printer-trust">
                    <p class="section-description">
                        "This printer's certificate isn't signed by a Bambu CA that BambuMate knows. If this fingerprint matches your printer, trust it. BambuMate will accept only this certificate from now on."
                    </p>
                    <dl class="printer-trust-details">
                        <dt>"Serial number"</dt>
                        <dd><code class="printer-trust-serial">{o.serial}</code></dd>
                        <dt>"SHA-256 fingerprint"</dt>
                        <dd><code class="printer-fingerprint">{o.fingerprint}</code></dd>
                    </dl>
                    <p class="section-description printer-serial-hint">
                        {SERIAL_HINT}
                        " A trusted certificate is accepted without Bambu's CA check."
                    </p>
                    <button class="btn btn-primary btn-sm printer-trust-btn" on:click=trust disabled=move || busy.get()>
                        "Trust this printer"
                    </button>
                </div>
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
        let view = |connection| PrinterView {
            configured: true,
            connection,
            ..Default::default()
        };
        assert_eq!(
            live_message(Some(&saved()), &view(ConnectionState::AuthFailed)).as_deref(),
            Some("The access code was rejected. Check it on the printer screen (Settings → LAN).")
        );
        let wrong = view(ConnectionState::WrongSerial {
            presented: "EVIL".into(),
        });
        assert!(live_message(Some(&saved()), &wrong)
            .unwrap()
            .contains("192.168.1.20 reports serial EVIL"));
        // Retried by the backend, or nothing saved: nothing to ask.
        assert_eq!(
            live_message(Some(&saved()), &view(ConnectionState::Unreachable)),
            None
        );
        assert_eq!(live_message(None, &view(ConnectionState::AuthFailed)), None);
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
