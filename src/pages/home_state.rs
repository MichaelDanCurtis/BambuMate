//! What Home shows, worked out from the printer view and plain data so it
//! can be tested without a browser (spec 2026-10-17 §3).

use crate::commands::GeneratedPreset;
use crate::printer::last_print::LastPrint;
use crate::printer::types::{
    format_remaining, untrusted_text, ConnectionState, ErrorView, PrinterView,
};

/// Segments in Home's progress bar.
pub const HOME_SEGMENTS: u32 = 12;

/// Most characters shown of a printer name the printer reports.
const NAME_MAX: usize = 64;

/// Most characters shown of a file name the printer reports.
const FILE_MAX: usize = 120;

/// `gcode_state`s that mean a print is under way.
const PRINTING: &[&str] = &["PREPARE", "SLICING", "RUNNING"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeMode {
    /// Paused, or the printer reports an error.
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
    if gcode == Some("PAUSE") || !v.errors.is_empty() {
        return HomeMode::Attention;
    }
    if gcode.is_some_and(|g| PRINTING.contains(&g)) {
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

pub fn printing_label(v: &PrinterView) -> String {
    join(&[printer_name(v), "Printing".into(), file_name(v)])
}

pub fn idle_label(v: &PrinterView) -> String {
    join(&[printer_name(v), "Idle".into()])
}

pub fn attention_label(v: &PrinterView) -> String {
    let code = v.errors.first().map(|e| e.code.clone()).unwrap_or_default();
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
        Some("PAUSE") => "PAUSED",
        _ => "ERROR",
    }
}

pub fn attention_detail(v: &PrinterView) -> String {
    match v.errors.first() {
        Some(ErrorView { text: Some(t), .. }) => t.clone(),
        Some(e) => format!("Look up {} on the Bambu wiki.", e.code),
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

/// `Last print finished 3h ago · Benchy · 1 h 58 m`; empty with no print.
pub fn last_print_detail(p: Option<&LastPrint>, now_ms: f64) -> String {
    let Some(p) = p else {
        return String::new();
    };
    join(&[
        format!(
            "Last print finished {}",
            format_ago(now_ms, p.finished_at_ms)
        ),
        untrusted_text(&p.name, FILE_MAX),
        p.duration_ms
            .map(|d| format_remaining((d / 60_000.0).round() as u32))
            .unwrap_or_default(),
    ])
}

/// `Recent · Polymaker PETG Red · 2d ago`.
pub fn recent_line(p: &GeneratedPreset, now_ms: f64) -> String {
    format!(
        "Recent · {} · {}",
        p.profile_name,
        format_ago(now_ms, p.created_at as f64 * 1000.0)
    )
}

pub fn sync_line(unsynced: usize) -> String {
    match unsynced {
        0 => "All presets synced".into(),
        1 => "1 preset unsynced".into(),
        n => format!("{n} presets unsynced"),
    }
}

pub fn profiles_detail(total: usize, unsynced: usize) -> String {
    format!(
        "{} synced to your Bambu account",
        total.saturating_sub(unsynced)
    )
}

/// The question "What does this mean?" puts in the agent's composer.
pub fn ask_about(v: &PrinterView) -> String {
    match v.errors.first() {
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
        None => {
            "My printer paused mid-print. What usually causes that, and what should I check?".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::types::{Nozzle, PrinterState, PrinterSummary};

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

    #[test]
    fn attention_beats_printing_beats_idle_beats_no_printer() {
        let mut v = connected("RUNNING");
        assert_eq!(home_mode(&v), HomeMode::Printing);
        v.errors.push(error("0300_0100", None));
        assert_eq!(home_mode(&v), HomeMode::Attention);
        assert_eq!(home_mode(&connected("PAUSE")), HomeMode::Attention);
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
        let p = LastPrint {
            name: "Benchy".into(),
            finished_at_ms: now - 3.0 * 3_600_000.0,
            duration_ms: Some(118.0 * 60_000.0),
        };
        assert_eq!(
            last_print_detail(Some(&p), now),
            "Last print finished 3h ago · Benchy · 1 h 58 m"
        );
        let unknown = LastPrint {
            duration_ms: None,
            ..p
        };
        assert_eq!(
            last_print_detail(Some(&unknown), now),
            "Last print finished 3h ago · Benchy"
        );
        assert_eq!(last_print_detail(None, now), "");
    }

    #[test]
    fn footer_and_profile_lines() {
        let p = GeneratedPreset {
            profile_name: "Polymaker PETG Red".into(),
            created_at: 1_000_000,
        };
        assert_eq!(
            recent_line(&p, (1_000_000.0 + 2.0 * 86_400.0) * 1000.0),
            "Recent · Polymaker PETG Red · 2d ago"
        );
        assert_eq!(sync_line(0), "All presets synced");
        assert_eq!(sync_line(1), "1 preset unsynced");
        assert_eq!(sync_line(3), "3 presets unsynced");
        assert_eq!(profiles_detail(12, 3), "9 synced to your Bambu account");
        assert_eq!(profiles_detail(2, 5), "0 synced to your Bambu account");
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
