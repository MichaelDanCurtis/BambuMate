//! What Home shows, worked out from the printer view and plain data so it
//! can be tested without a browser (spec 2026-10-17 §3).

use crate::commands::GeneratedPreset;
use crate::journal::format::{duration_text, UNWATCHED};
use crate::journal::types::{Outcome, PrintEntry};
use crate::printer::types::{
    format_remaining, untrusted_text, ConnectionState, ErrorView, PrinterView,
};

/// Segments in Home's progress bar.
pub const HOME_SEGMENTS: u32 = 12;

/// Most characters shown of a printer name the printer reports.
const NAME_MAX: usize = 64;

/// Most characters shown of a preset name.
const PRESET_MAX: usize = 64;

/// Most characters shown of an error code the printer reports.
const CODE_MAX: usize = 32;

/// Most characters shown of a file name the printer reports.
const FILE_MAX: usize = 120;

/// `gcode_state`s that mean a print is under way.
const PRINTING: &[&str] = &["PREPARE", "SLICING", "RUNNING"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeMode {
    /// Paused or failed, a print error, or an HMS code while not printing.
    Attention,
    Printing,
    Idle,
    /// No printer set up (`disconnected: false`), or set up but not connected.
    NoPrinter {
        disconnected: bool,
    },
}

/// Which hero Home leads with. Precedence: attention, printing, idle, no
/// printer. A printer that isn't connected never shows its last state.
///
/// An HMS code alone doesn't pull Home away from a print: the backend can't
/// tell an informational code (AMS humidity) from a fatal one, so during a
/// print it only shows in the label (see `printing_label`). A print error
/// always needs attention, and so does any code when nothing is printing.
pub fn home_mode(v: &PrinterView) -> HomeMode {
    if !v.configured {
        return HomeMode::NoPrinter {
            disconnected: false,
        };
    }
    if v.connection != ConnectionState::Connected {
        return HomeMode::NoPrinter { disconnected: true };
    }
    let gcode = v.state.as_ref().and_then(|s| s.gcode_state.as_deref());
    let printing = gcode.is_some_and(|g| PRINTING.contains(&g));
    let print_error = v.errors.iter().any(|e| e.kind == "print_error");
    if matches!(gcode, Some("PAUSE" | "FAILED"))
        || print_error
        || (!v.errors.is_empty() && !printing)
    {
        return HomeMode::Attention;
    }
    if printing {
        return HomeMode::Printing;
    }
    HomeMode::Idle
}

/// `data-mode` on the Home page.
pub fn mode_key(mode: HomeMode) -> &'static str {
    match mode {
        HomeMode::Attention => "attention",
        HomeMode::Printing => "printing",
        HomeMode::Idle => "idle",
        HomeMode::NoPrinter { .. } => "no-printer",
    }
}

/// The printer's name, else its model; cleaned for display.
fn printer_name(v: &PrinterView) -> String {
    v.printer
        .as_ref()
        .map(|p| {
            let name = if p.name.trim().is_empty() {
                &p.model
            } else {
                &p.name
            };
            untrusted_text(name, NAME_MAX)
        })
        .unwrap_or_default()
}

/// The file being printed; cleaned for display.
fn file_name(v: &PrinterView) -> String {
    v.state
        .as_ref()
        .and_then(|s| s.subtask_name.as_deref())
        .filter(|f| !f.is_empty())
        .map(|f| untrusted_text(f, FILE_MAX))
        .unwrap_or_default()
}

/// Joins the non-empty parts with ` · `.
fn join(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `Workshop H2D · Printing · Benchy`, plus `· HMS <code>` when the printer
/// reports a code during the print.
pub fn printing_label(v: &PrinterView) -> String {
    let hms = v
        .errors
        .first()
        .map(|e| format!("HMS {}", untrusted_text(&e.code, CODE_MAX)))
        .unwrap_or_default();
    join(&[printer_name(v), "Printing".into(), file_name(v), hms])
}

pub fn idle_label(v: &PrinterView) -> String {
    join(&[printer_name(v), "Idle".into()])
}

/// The error Home names in Attention: a print error if there is one (it is
/// what puts Home into Attention mid-print, and the backend lists it after
/// the HMS codes), else the first HMS code.
fn attention_error(v: &PrinterView) -> Option<&ErrorView> {
    v.errors
        .iter()
        .find(|e| e.kind == "print_error")
        .or_else(|| v.errors.first())
}

pub fn attention_label(v: &PrinterView) -> String {
    let code = attention_error(v)
        .map(|e| e.code.clone())
        .unwrap_or_default();
    join(&[printer_name(v), "Needs attention".into(), code])
}

/// The hero label with no live printer.
pub fn no_printer_label(v: &PrinterView) -> String {
    if !v.configured {
        "BambuMate".into()
    } else if v.connection == ConnectionState::Connecting {
        "[CONNECTING…]".into()
    } else {
        "[DISCONNECTED]".into()
    }
}

pub fn attention_word(v: &PrinterView) -> &'static str {
    match v.state.as_ref().and_then(|s| s.gcode_state.as_deref()) {
        Some("FAILED") => "FAILED",
        Some("PAUSE") => "PAUSED",
        _ => "ERROR",
    }
}

fn is_failed(v: &PrinterView) -> bool {
    v.state.as_ref().and_then(|s| s.gcode_state.as_deref()) == Some("FAILED")
}

pub fn attention_detail(v: &PrinterView) -> String {
    match attention_error(v) {
        Some(ErrorView { text: Some(t), .. }) => t.clone(),
        Some(e) => format!("Look up {} on the Bambu wiki.", e.code),
        None if is_failed(v) => "The print failed. Check the printer.".into(),
        None => "Paused. Check the printer, then resume the print there.".into(),
    }
}

/// The percentage, capped at 100; `—` with no report.
pub fn percent_text(v: &PrinterView) -> String {
    v.state
        .as_ref()
        .and_then(|s| s.mc_percent)
        .map(|p| p.min(100).to_string())
        .unwrap_or_else(|| "—".into())
}

/// `1 h 12 m left · Layer 118 / 284 · Nozzle 220 °C · Bed 65 °C`, leaving
/// out what the printer didn't report.
pub fn printing_detail(v: &PrinterView) -> String {
    let Some(s) = &v.state else {
        return String::new();
    };
    let nozzle = s
        .nozzles
        .iter()
        .find(|n| Some(n.id) == s.active_nozzle)
        .or_else(|| s.nozzles.first())
        .and_then(|n| n.temp);
    join(&[
        s.mc_remaining_time
            .map(|m| format!("{} left", format_remaining(m)))
            .unwrap_or_default(),
        match (s.layer_num, s.total_layer_num) {
            (Some(l), Some(t)) if t > 0 => format!("Layer {l} / {t}"),
            _ => String::new(),
        },
        nozzle
            .map(|t| format!("Nozzle {t:.0} °C"))
            .unwrap_or_default(),
        s.bed_temp
            .map(|t| format!("Bed {t:.0} °C"))
            .unwrap_or_default(),
    ])
}

/// `READY`, or `WAITING` until the first status report arrives.
pub fn idle_word(v: &PrinterView) -> &'static str {
    if v.state.is_some() {
        "READY"
    } else {
        "WAITING"
    }
}

/// The `(ams_id, tray_id)` feeding the print. `tray_now` counts trays across
/// the four-slot AMS units (0-15); higher values mean an external spool or
/// none, which Home doesn't outline.
pub fn active_slot(v: &PrinterView) -> Option<(u32, u32)> {
    match v.state.as_ref()?.tray_now? {
        n @ 0..=15 => Some((n / 4, n % 4)),
        _ => None,
    }
}

/// `just now`, `5m ago`, `3h ago` or `2d ago`.
pub fn format_ago(now_ms: f64, then_ms: f64) -> String {
    let secs = ((now_ms - then_ms) / 1000.0).max(0.0) as u64;
    match secs {
        0..=59 => "just now".into(),
        60..=3_599 => format!("{}m ago", secs / 60),
        3_600..=86_399 => format!("{}h ago", secs / 3_600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

/// `Last print finished 3h ago · Benchy · 1 h 58 m`, `Last print failed 3h
/// ago · Benchy`, or, when the end wasn't seen, `Last print · Benchy ·
/// Ended while BambuMate wasn't watching`. Empty with no print.
pub fn last_print_detail(p: Option<&PrintEntry>, now_ms: f64) -> String {
    let Some(p) = p else {
        return String::new();
    };
    let name = untrusted_text(&p.name, FILE_MAX);
    match (p.outcome, p.ended_at) {
        (Outcome::Finished, Some(end)) => join(&[
            format!(
                "Last print finished {}",
                format_ago(now_ms, end as f64 * 1000.0)
            ),
            name,
            duration_text(p).unwrap_or_default(),
        ]),
        (Outcome::Failed, Some(end)) => join(&[
            format!(
                "Last print failed {}",
                format_ago(now_ms, end as f64 * 1000.0)
            ),
            name,
        ]),
        _ => join(&["Last print".into(), name, UNWATCHED.into()]),
    }
}

/// `Recent · Polymaker PETG Red · 2d ago`; `None` without a name or a date,
/// which would read as `Recent ·  · 104d ago`.
pub fn recent_line(p: &GeneratedPreset, now_ms: f64) -> Option<String> {
    let name = untrusted_text(p.profile_name.trim(), PRESET_MAX);
    if name.is_empty() || p.created_at <= 0 {
        return None;
    }
    Some(format!(
        "Recent · {name} · {}",
        format_ago(now_ms, p.created_at as f64 * 1000.0)
    ))
}

/// What BambuMate can say about sync. It sees only presets stuck out of
/// sync, so it can't confirm that every other preset reached the cloud.
pub fn sync_line(not_syncing: usize) -> String {
    match not_syncing {
        0 => "No sync problems found".into(),
        1 => "1 preset not syncing".into(),
        n => format!("{n} presets not syncing"),
    }
}

/// The hero's unit for a count of presets.
pub fn presets_unit(n: usize) -> &'static str {
    if n == 1 {
        " preset"
    } else {
        " presets"
    }
}

/// The question "What does this mean?" puts in the agent's composer.
pub fn ask_about(v: &PrinterView) -> String {
    match attention_error(v) {
        Some(ErrorView {
            code,
            text: Some(t),
            ..
        }) => format!(
            "My printer shows error {code}: \"{t}\". What does it mean, and how do I fix it?"
        ),
        Some(e) => format!(
            "My printer shows error {}. What does it mean, and how do I fix it?",
            e.code
        ),
        None if is_failed(v) => {
            "My printer reports that a print failed. What usually causes that, and what should I check?"
                .into()
        }
        None => {
            "My printer paused mid-print. What usually causes that, and what should I check?".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::types::{Nozzle, PrinterState, PrinterSummary};

    #[test]
    fn the_preset_unit_is_singular_for_one() {
        assert_eq!(presets_unit(1), " preset");
        assert_eq!(presets_unit(0), " presets");
        assert_eq!(presets_unit(2), " presets");
    }

    fn connected(gcode: &str) -> PrinterView {
        PrinterView {
            configured: true,
            connection: ConnectionState::Connected,
            printer: Some(PrinterSummary {
                name: "Workshop H2D".into(),
                model: "H2D".into(),
                ..Default::default()
            }),
            state: Some(PrinterState {
                gcode_state: Some(gcode.into()),
                subtask_name: Some("Benchy".into()),
                mc_percent: Some(42),
                mc_remaining_time: Some(72),
                layer_num: Some(118),
                total_layer_num: Some(284),
                bed_temp: Some(65.0),
                nozzles: vec![Nozzle {
                    id: 0,
                    temp: Some(220.0),
                    ..Default::default()
                }],
                active_nozzle: Some(0),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn error(code: &str, text: Option<&str>) -> ErrorView {
        ErrorView {
            kind: "hms".into(),
            code: code.into(),
            text: text.map(Into::into),
            wiki_url: String::new(),
        }
    }

    fn print_error(code: &str) -> ErrorView {
        ErrorView {
            kind: "print_error".into(),
            ..error(code, None)
        }
    }

    #[test]
    fn attention_beats_printing_beats_idle_beats_no_printer() {
        let mut v = connected("RUNNING");
        assert_eq!(home_mode(&v), HomeMode::Printing);
        v.errors.push(print_error("0500_4001"));
        assert_eq!(home_mode(&v), HomeMode::Attention);
        assert_eq!(home_mode(&connected("PAUSE")), HomeMode::Attention);
        assert_eq!(home_mode(&connected("FAILED")), HomeMode::Attention);
        assert_eq!(home_mode(&connected("PREPARE")), HomeMode::Printing);
        assert_eq!(home_mode(&connected("IDLE")), HomeMode::Idle);
        assert_eq!(home_mode(&connected("FINISH")), HomeMode::Idle);
        let waiting = PrinterView {
            state: None,
            ..connected("IDLE")
        };
        assert_eq!(home_mode(&waiting), HomeMode::Idle);
        assert_eq!(
            home_mode(&PrinterView::default()),
            HomeMode::NoPrinter {
                disconnected: false
            }
        );
    }

    #[test]
    fn attention_names_the_print_error_that_caused_it() {
        // The backend lists HMS entries first and the print error last; the
        // print error is what put Home into Attention mid-print.
        let mut v = connected("RUNNING");
        v.errors
            .push(error("0300_0100", Some("AMS humidity is high.")));
        v.errors.push(ErrorView {
            text: Some("The nozzle is clogged.".into()),
            ..print_error("0500_4001")
        });
        assert_eq!(
            attention_label(&v),
            "Workshop H2D · Needs attention · 0500_4001"
        );
        assert_eq!(attention_detail(&v), "The nozzle is clogged.");
        assert!(ask_about(&v).contains("0500_4001"));
        // With only HMS codes (not printing), the first one is shown.
        let mut idle = connected("IDLE");
        idle.errors
            .push(error("0300_0100", Some("AMS humidity is high.")));
        idle.errors.push(error("0700_2000", None));
        assert_eq!(attention_detail(&idle), "AMS humidity is high.");
    }

    #[test]
    fn an_hms_code_alone_does_not_hide_a_print() {
        // BambuMate can't tell an HMS code's severity, so one during a print
        // (AMS humidity, say) must not replace the progress with an error.
        let mut printing = connected("RUNNING");
        printing.errors.push(error("0300_0100", None));
        assert_eq!(home_mode(&printing), HomeMode::Printing);
        assert_eq!(
            printing_label(&printing),
            "Workshop H2D · Printing · Benchy · HMS 0300_0100"
        );
        // A print error during a print still needs attention.
        let mut both = connected("RUNNING");
        both.errors.push(error("0300_0100", None));
        both.errors.push(print_error("0500_4001"));
        assert_eq!(home_mode(&both), HomeMode::Attention);
        // Not printing, any code is attention.
        let mut idle = connected("IDLE");
        idle.errors.push(error("0300_0100", None));
        assert_eq!(home_mode(&idle), HomeMode::Attention);
        let mut finished = connected("FINISH");
        finished.errors.push(error("0300_0100", None));
        assert_eq!(home_mode(&finished), HomeMode::Attention);
        // A printer that reports no state yet counts as not printing.
        let mut waiting = PrinterView {
            state: None,
            ..connected("IDLE")
        };
        waiting.errors.push(error("0300_0100", None));
        assert_eq!(home_mode(&waiting), HomeMode::Attention);
    }

    #[test]
    fn a_disconnect_mid_print_never_shows_the_print() {
        let v = PrinterView {
            connection: ConnectionState::Unreachable,
            ..connected("RUNNING")
        };
        assert_eq!(home_mode(&v), HomeMode::NoPrinter { disconnected: true });
        assert_eq!(no_printer_label(&v), "[DISCONNECTED]");
        let connecting = PrinterView {
            connection: ConnectionState::Connecting,
            ..connected("RUNNING")
        };
        assert_eq!(no_printer_label(&connecting), "[CONNECTING…]");
        assert_eq!(no_printer_label(&PrinterView::default()), "BambuMate");
    }

    #[test]
    fn mode_keys() {
        assert_eq!(mode_key(HomeMode::Attention), "attention");
        assert_eq!(mode_key(HomeMode::Printing), "printing");
        assert_eq!(mode_key(HomeMode::Idle), "idle");
        assert_eq!(
            mode_key(HomeMode::NoPrinter { disconnected: true }),
            "no-printer"
        );
    }

    #[test]
    fn labels_name_the_printer_once_and_skip_blanks() {
        let v = connected("RUNNING");
        assert_eq!(printing_label(&v), "Workshop H2D · Printing · Benchy");
        assert_eq!(idle_label(&v), "Workshop H2D · Idle");
        let mut e = connected("RUNNING");
        e.errors.push(error("0300_0100", Some("Heatbed")));
        assert_eq!(
            attention_label(&e),
            "Workshop H2D · Needs attention · 0300_0100"
        );
        let mut unnamed = connected("IDLE");
        unnamed.printer.as_mut().unwrap().name = String::new();
        assert_eq!(idle_label(&unnamed), "H2D · Idle");
    }

    #[test]
    fn printing_hero_text() {
        let v = connected("RUNNING");
        assert_eq!(percent_text(&v), "42");
        assert_eq!(
            printing_detail(&v),
            "1 h 12 m left · Layer 118 / 284 · Nozzle 220 °C · Bed 65 °C"
        );
        let mut over = connected("RUNNING");
        over.state.as_mut().unwrap().mc_percent = Some(140);
        assert_eq!(percent_text(&over), "100");
        assert_eq!(percent_text(&PrinterView::default()), "—");
    }

    #[test]
    fn attention_hero_text() {
        let mut v = connected("RUNNING");
        v.errors
            .push(error("0300_0100", Some("The heatbed is too hot.")));
        assert_eq!(attention_word(&v), "ERROR");
        assert_eq!(attention_detail(&v), "The heatbed is too hot.");
        let mut no_text = connected("RUNNING");
        no_text.errors.push(error("0300_400C", None));
        assert_eq!(
            attention_detail(&no_text),
            "Look up 0300_400C on the Bambu wiki."
        );
        let paused = connected("PAUSE");
        assert_eq!(attention_word(&paused), "PAUSED");
        let failed = connected("FAILED");
        assert_eq!(attention_word(&failed), "FAILED");
        assert_eq!(
            attention_detail(&failed),
            "The print failed. Check the printer."
        );
        let mut idle_err = connected("IDLE");
        idle_err.errors.push(error("0300_0100", None));
        assert_eq!(attention_word(&idle_err), "ERROR");
        assert_eq!(
            attention_detail(&paused),
            "Paused. Check the printer, then resume the print there."
        );
    }

    #[test]
    fn idle_hero_word_waits_for_the_first_report() {
        assert_eq!(idle_word(&connected("IDLE")), "READY");
        assert_eq!(
            idle_word(&PrinterView {
                state: None,
                ..connected("IDLE")
            }),
            "WAITING"
        );
    }

    #[test]
    fn the_slot_in_use_comes_from_tray_now() {
        let mut v = connected("RUNNING");
        assert_eq!(active_slot(&v), None);
        v.state.as_mut().unwrap().tray_now = Some(6);
        assert_eq!(active_slot(&v), Some((1, 2)));
        v.state.as_mut().unwrap().tray_now = Some(255);
        assert_eq!(active_slot(&v), None);
    }

    #[test]
    fn ago_text() {
        let now = 10_000_000_000.0;
        assert_eq!(format_ago(now, now - 30_000.0), "just now");
        assert_eq!(format_ago(now, now - 5.0 * 60_000.0), "5m ago");
        assert_eq!(format_ago(now, now - 3.0 * 3_600_000.0), "3h ago");
        assert_eq!(format_ago(now, now - 2.0 * 86_400_000.0), "2d ago");
        assert_eq!(format_ago(now, now + 60_000.0), "just now");
    }

    #[test]
    fn last_print_line() {
        let now = 10_000_000_000.0;
        let end = (now / 1000.0) as i64 - 3 * 3_600;
        let p = PrintEntry {
            id: 2,
            name: "Benchy".into(),
            started_at: end - 118 * 60,
            ended_at: Some(end),
            outcome: Outcome::Finished,
            ..Default::default()
        };
        assert_eq!(
            last_print_detail(Some(&p), now),
            "Last print finished 3h ago · Benchy · 1 h 58 m"
        );
        let approx = PrintEntry {
            start_approx: true,
            ..p.clone()
        };
        assert_eq!(
            last_print_detail(Some(&approx), now),
            "Last print finished 3h ago · Benchy · 1 h 58 m (approximate)"
        );
        let failed = PrintEntry {
            outcome: Outcome::Failed,
            ..p.clone()
        };
        assert_eq!(
            last_print_detail(Some(&failed), now),
            "Last print failed 3h ago · Benchy"
        );
        let unwatched = PrintEntry {
            outcome: Outcome::Unknown,
            ended_at: None,
            ..p
        };
        assert_eq!(
            last_print_detail(Some(&unwatched), now),
            "Last print · Benchy · Ended while BambuMate wasn't watching"
        );
        assert_eq!(last_print_detail(None, now), "");
    }

    fn preset(name: &str, created_at: i64) -> GeneratedPreset {
        GeneratedPreset {
            profile_name: name.into(),
            created_at,
        }
    }

    #[test]
    fn recent_line_text() {
        let now = (1_000_000.0 + 2.0 * 86_400.0) * 1000.0;
        assert_eq!(
            recent_line(&preset("Polymaker PETG Red", 1_000_000), now).as_deref(),
            Some("Recent · Polymaker PETG Red · 2d ago")
        );
    }

    #[test]
    fn recent_line_is_skipped_without_a_name_or_a_date() {
        let now = 10_000_000_000.0;
        assert_eq!(recent_line(&preset("", 1_000_000), now), None);
        assert_eq!(recent_line(&preset("   ", 1_000_000), now), None);
        assert_eq!(recent_line(&preset("PETG", 0), now), None);
        assert_eq!(recent_line(&preset("PETG", -5), now), None);
    }

    #[test]
    fn recent_line_cleans_the_profile_name() {
        let now = (1_000_000.0 + 60.0) * 1000.0;
        let long = "x".repeat(200);
        let line = recent_line(&preset(&format!("A\u{202e}B\n{long}"), 1_000_000), now).unwrap();
        assert!(!line.contains('\u{202e}'));
        assert!(!line.contains('\n'));
        assert!(line.chars().count() < 100, "{line}");
    }

    #[test]
    fn sync_line_only_claims_what_the_data_shows() {
        assert_eq!(sync_line(0), "No sync problems found");
        assert_eq!(sync_line(1), "1 preset not syncing");
        assert_eq!(sync_line(3), "3 presets not syncing");
    }

    #[test]
    fn the_agent_question_names_the_error() {
        let mut v = connected("RUNNING");
        v.errors.push(error("0300_0100", Some("Heatbed abnormal.")));
        assert_eq!(
            ask_about(&v),
            "My printer shows error 0300_0100: \"Heatbed abnormal.\". What does it mean, and how do I fix it?"
        );
        let mut bare = connected("RUNNING");
        bare.errors.push(error("0300_400C", None));
        assert_eq!(
            ask_about(&bare),
            "My printer shows error 0300_400C. What does it mean, and how do I fix it?"
        );
        assert_eq!(
            ask_about(&connected("PAUSE")),
            "My printer paused mid-print. What usually causes that, and what should I check?"
        );
    }
}
