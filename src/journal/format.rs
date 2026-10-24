//! Pure formatting for Printer → Journal and Home's last print (spec
//! 2026-10-24 §2). No browser APIs, so it is tested on the host.

use crate::journal::types::{
    FilterKind, MonthSummary, Outcome, PrintEntry, SliceSnapshot, TrayUse, Verdict, VerdictChoice,
};
use crate::pages::home_state::format_ago;
use crate::printer::types::{format_remaining, untrusted_text};

pub const PRINTING_NOW: &str = "Printing now";
pub const UNWATCHED: &str = "Ended while BambuMate wasn't watching";
pub const APPROXIMATE: &str = " (approximate)";
pub const SLICE_ESTIMATE: &str = "Slice estimate";
pub const EMPTY_JOURNAL: &str = "Prints appear here as your printer runs them";
pub const NO_MATCH: &str = "No prints match";
pub const NO_PRINTER: &str = "The journal fills in by itself once a printer is set up.";
pub const RATE_LAST: &str = "Rate your last print";
pub const DELETE_CONFIRM: &str = "Delete this entry and its photos?";
/// The backend's refusals (`journal::photos`), for the checks made before
/// a file is sent.
pub const TOO_LARGE: &str = "That photo is over 25 MB. Choose a smaller one.";
pub const NOT_AN_IMAGE: &str =
    "That file isn't a photo BambuMate can read. Choose a JPEG, PNG or WebP image.";
pub const MAX_PHOTO_BYTES: f64 = 25.0 * 1024.0 * 1024.0;
/// Most characters shown of a print name the printer reports.
pub const NAME_MAX: usize = 120;
/// Most characters shown of other printer-reported text.
const TEXT_MAX: usize = 64;

/// Analyze's defect ids, in the order the chips show them.
pub const ISSUE_TAGS: &[&str] = &[
    "stringing",
    "warping",
    "layer_adhesion",
    "elephants_foot",
    "under_extrusion",
    "over_extrusion",
    "z_banding",
];

pub const FILTERS: &[(FilterKind, &str)] = &[
    (FilterKind::All, "All"),
    (FilterKind::NeedsVerdict, "Needs a verdict"),
    (FilterKind::Failed, "Failed"),
];

pub const VERDICTS: &[Verdict] = &[Verdict::Good, Verdict::Usable, Verdict::Failed];

/// `data-kind` on a filter pill.
pub fn filter_key(k: FilterKind) -> &'static str {
    match k {
        FilterKind::All => "all",
        FilterKind::NeedsVerdict => "needs_verdict",
        FilterKind::Failed => "failed",
    }
}

/// `data-outcome` on a row.
pub fn outcome_key(o: Outcome) -> &'static str {
    match o {
        Outcome::Printing => "printing",
        Outcome::Finished => "finished",
        Outcome::Failed => "failed",
        Outcome::Unknown => "unknown",
    }
}

pub fn outcome_text(o: Outcome) -> &'static str {
    match o {
        Outcome::Printing => PRINTING_NOW,
        Outcome::Finished => "Finished",
        Outcome::Failed => "Failed",
        Outcome::Unknown => UNWATCHED,
    }
}

pub fn verdict_key(v: Verdict) -> &'static str {
    match v {
        Verdict::Good => "good",
        Verdict::Usable => "usable",
        Verdict::Failed => "failed",
    }
}

pub fn verdict_text(v: Verdict) -> &'static str {
    match v {
        Verdict::Good => "Good",
        Verdict::Usable => "Usable",
        Verdict::Failed => "Failed",
    }
}

/// Clicking the pressed verdict clears it; any other sets that one.
pub fn verdict_after_click(current: Option<Verdict>, clicked: Verdict) -> VerdictChoice {
    if current == Some(clicked) {
        VerdictChoice::Clear
    } else {
        clicked.into()
    }
}

/// `tags` with `tag` removed if present, else added at the end.
pub fn toggle_tag(tags: &[String], tag: &str) -> Vec<String> {
    if tags.iter().any(|t| t == tag) {
        tags.iter().filter(|t| *t != tag).cloned().collect()
    } else {
        let mut out = tags.to_vec();
        out.push(tag.to_string());
        out
    }
}

/// `print_error` as Bambu shows it: `0300-400C`.
pub fn error_code_text(code: u32) -> String {
    let hex = format!("{code:08X}");
    format!("{}-{}", &hex[..4], &hex[4..])
}

/// `1 h 58 m`, marked approximate when the start was; `None` until it ends.
pub fn duration_text(e: &PrintEntry) -> Option<String> {
    let end = e.ended_at?;
    let minutes = ((end - e.started_at).max(0) + 30) / 60;
    Some(format!(
        "{}{}",
        format_remaining(minutes as u32),
        if e.start_approx { APPROXIMATE } else { "" }
    ))
}

/// `4h ago · 1 h 58 m` on a row; `Started 30m ago` while printing.
pub fn when_line(e: &PrintEntry, now_ms: f64) -> String {
    let ago = format_ago(now_ms, e.started_at as f64 * 1000.0);
    if e.outcome == Outcome::Printing {
        return format!("Started {ago}");
    }
    match duration_text(e) {
        Some(d) => format!("{ago} · {d}"),
        None => ago,
    }
}

/// `PLA · A2`.
pub fn tray_chip(t: &TrayUse) -> String {
    let kind = untrusted_text(t.tray_type.trim(), TEXT_MAX);
    let label = untrusted_text(t.label.trim(), TEXT_MAX);
    match (kind.is_empty(), label.is_empty()) {
        (false, false) => format!("{kind} · {label}"),
        (false, true) => kind,
        _ => label,
    }
}

/// `14 m · 3.7 g`, from the linked slice.
pub fn slice_estimate(s: &SliceSnapshot) -> String {
    format!(
        "{} · {:.1} g",
        format_remaining(((s.time_seconds + 30) / 60) as u32),
        s.weight_g
    )
}

/// The hero's unit: `14 prints`, `1 print`.
pub fn hero_unit(n: u32) -> &'static str {
    if n == 1 {
        " print"
    } else {
        " prints"
    }
}

/// `38 h`, or `45 m` under an hour. Rounds the minutes first, so 59.6
/// minutes reads `1 h`, never `60 m`.
pub fn printing_time(secs: i64) -> String {
    let minutes = (secs.max(0) + 30) / 60;
    if minutes < 60 {
        format!("{minutes} m")
    } else {
        format!("{} h", (minutes + 30) / 60)
    }
}

/// `11 good · 2 usable · 1 failed · 38 h printing`, leaving out zeros.
pub fn hero_detail(s: &MonthSummary) -> String {
    let mut parts: Vec<String> = [
        (s.good, "good"),
        (s.usable, "usable"),
        (s.finished, "finished"),
        (s.failed, "failed"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, word)| format!("{n} {word}"))
    .collect();
    // Under half a minute would read "0 m printing".
    if s.printing_seconds >= 30 {
        parts.push(format!("{} printing", printing_time(s.printing_seconds)));
    }
    parts.join(" · ")
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `24 Oct 2026 14:05` at `offset_minutes` east of UTC.
pub fn local_time_text(unix_secs: i64, offset_minutes: i32) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let local = unix_secs + i64::from(offset_minutes) * 60;
    let (year, month, day) = civil_from_days(local.div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    format!(
        "{day} {} {year} {:02}:{:02}",
        MONTHS[(month - 1) as usize],
        secs / 3_600,
        secs % 3_600 / 60
    )
}

pub fn start_text(e: &PrintEntry, offset_minutes: i32) -> String {
    let at = local_time_text(e.started_at, offset_minutes);
    if e.start_approx {
        format!("{at}{APPROXIMATE}")
    } else {
        at
    }
}

pub fn end_text(e: &PrintEntry, offset_minutes: i32) -> String {
    match (e.ended_at, e.outcome) {
        (Some(at), _) => local_time_text(at, offset_minutes),
        (None, Outcome::Printing) => "Still printing".into(),
        (None, _) => UNWATCHED.into(),
    }
}

/// The detail's facts, label then value. Error and HMS codes only on a
/// failure; the slice estimate only with a linked slice.
pub fn fact_rows(e: &PrintEntry, offset_minutes: i32) -> Vec<(&'static str, String)> {
    let mut rows = vec![
        ("Started", start_text(e, offset_minutes)),
        ("Ended", end_text(e, offset_minutes)),
    ];
    if let Some(d) = duration_text(e) {
        rows.push(("Duration", d));
    }
    if let Some(p) = e.printer_model.as_deref().filter(|p| !p.trim().is_empty()) {
        rows.push(("Printer", untrusted_text(p, TEXT_MAX)));
    }
    if let Some(n) = e.plate {
        rows.push(("Plate", n.to_string()));
    }
    if e.outcome == Outcome::Failed {
        if let Some(code) = e.error_code {
            rows.push(("Error", error_code_text(code)));
        }
        if !e.hms.is_empty() {
            let codes: Vec<String> = e.hms.iter().map(|h| untrusted_text(h, TEXT_MAX)).collect();
            rows.push(("HMS", codes.join(", ")));
        }
    }
    if let Some(s) = &e.slice {
        rows.push((SLICE_ESTIMATE, slice_estimate(s)));
        let presets: Vec<&str> = [s.printer.as_str(), s.process.as_str(), s.filament.as_str()]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        rows.push(("Presets", presets.join(" · ")));
    }
    if !e.analyses.is_empty() {
        rows.push(("Analyses", e.analyses.len().to_string()));
    }
    rows
}

pub fn entry_href(id: i64) -> String {
    format!("/printer/journal?entry={id}")
}

pub fn analyze_href(id: i64) -> String {
    format!("/analysis?journal={id}")
}

/// Home's "Rate your last print": a finished print with no verdict.
pub fn rate_href(e: &PrintEntry) -> Option<String> {
    (e.outcome == Outcome::Finished && e.verdict.is_none()).then(|| entry_href(e.id))
}

/// The Analyze material option a tray type falls under, if any.
pub fn material_option(tray_type: &str) -> Option<&'static str> {
    let t = tray_type.trim().to_ascii_uppercase();
    ["PETG", "PLA", "ABS", "ASA", "TPU", "PA", "PC"]
        .into_iter()
        .find(|m| t.starts_with(m))
}

/// What Analyze fills in when opened from an entry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnalyzePrefill {
    pub model_path: Option<String>,
    /// The slice's filament preset name, matched to a profile by name.
    pub filament_preset: Option<String>,
    pub material: Option<&'static str>,
    /// The newest photo.
    pub photo_id: Option<i64>,
}

pub fn analyze_prefill(e: &PrintEntry) -> AnalyzePrefill {
    let slice = e.slice.as_ref();
    AnalyzePrefill {
        model_path: slice
            .map(|s| s.model_path.clone())
            .filter(|p| !p.is_empty()),
        filament_preset: slice.map(|s| s.filament.clone()).filter(|p| !p.is_empty()),
        material: e
            .trays
            .iter()
            .find_map(|t| material_option(&t.tray_type))
            .or_else(|| {
                slice.and_then(|s| {
                    s.filaments
                        .iter()
                        .find_map(|f| material_option(&f.filament_type))
                })
            }),
        photo_id: e.photos.last().map(|p| p.id),
    }
}

/// The base64 part of a `data:…;base64,…` URL.
pub fn data_url_base64(url: &str) -> Option<String> {
    url.split_once(";base64,")
        .map(|(_, data)| data.to_string())
        .filter(|data| !data.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::types::{Photo, SliceFilament};

    const END: i64 = 1_792_850_700; // 2026-10-24 14:05 UTC

    fn entry() -> PrintEntry {
        PrintEntry {
            id: 2,
            name: "Benchy".into(),
            started_at: END - 7_080,
            ended_at: Some(END),
            outcome: Outcome::Finished,
            printer_model: Some("H2D".into()),
            plate: Some(1),
            ..Default::default()
        }
    }

    fn slice() -> SliceSnapshot {
        SliceSnapshot {
            model_name: "cube.stl".into(),
            model_path: "/m/cube.stl".into(),
            printer: "Bambu Lab H2D 0.4 nozzle".into(),
            process: "0.20mm Standard @BBL H2D".into(),
            filament: "Bambu PLA Basic @BBL H2D".into(),
            plate: 1,
            time_seconds: 843,
            weight_g: 3.69,
            filaments: vec![SliceFilament {
                slot: 1,
                filament_type: "PLA".into(),
                color: "#00AE42".into(),
                used_g: 3.69,
            }],
            has_thumbnail: true,
        }
    }

    fn tray(label: &str, tray_type: &str) -> TrayUse {
        TrayUse {
            label: label.into(),
            tray_type: tray_type.into(),
            ..Default::default()
        }
    }

    #[test]
    fn outcomes_and_verdicts_read_as_the_spec_says() {
        assert_eq!(
            outcome_text(Outcome::Unknown),
            "Ended while BambuMate wasn't watching"
        );
        assert_eq!(outcome_text(Outcome::Printing), "Printing now");
        assert_eq!(outcome_text(Outcome::Finished), "Finished");
        assert_eq!(outcome_text(Outcome::Failed), "Failed");
        assert_eq!(
            VERDICTS
                .iter()
                .map(|v| verdict_text(*v))
                .collect::<Vec<_>>(),
            ["Good", "Usable", "Failed"]
        );
        assert_eq!(
            VERDICTS.iter().map(|v| verdict_key(*v)).collect::<Vec<_>>(),
            ["good", "usable", "failed"]
        );
        assert_eq!(
            FILTERS
                .iter()
                .map(|(k, l)| (filter_key(*k), *l))
                .collect::<Vec<_>>(),
            [
                ("all", "All"),
                ("needs_verdict", "Needs a verdict"),
                ("failed", "Failed")
            ]
        );
        assert_eq!(ISSUE_TAGS.len(), 7);
    }

    #[test]
    fn print_errors_show_as_two_groups_of_four_hex_digits() {
        assert_eq!(error_code_text(0x0300_400C), "0300-400C");
        assert_eq!(error_code_text(0), "0000-0000");
    }

    #[test]
    fn durations_are_marked_approximate_with_an_approximate_start() {
        assert_eq!(duration_text(&entry()).as_deref(), Some("1 h 58 m"));
        let approx = PrintEntry {
            start_approx: true,
            ..entry()
        };
        assert_eq!(
            duration_text(&approx).as_deref(),
            Some("1 h 58 m (approximate)")
        );
        let open = PrintEntry {
            ended_at: None,
            outcome: Outcome::Printing,
            ..entry()
        };
        assert_eq!(duration_text(&open), None);
    }

    #[test]
    fn the_row_says_when_and_how_long() {
        let now_ms = (END + 3 * 3_600) as f64 * 1000.0;
        assert_eq!(when_line(&entry(), now_ms), "4h ago · 1 h 58 m");
        let open = PrintEntry {
            started_at: END - 1_800,
            ended_at: None,
            outcome: Outcome::Printing,
            ..entry()
        };
        assert_eq!(when_line(&open, END as f64 * 1000.0), "Started 30m ago");
        assert_eq!(tray_chip(&tray("A2", "PLA")), "PLA · A2");
        assert_eq!(tray_chip(&tray("B4", "")), "B4");
    }

    #[test]
    fn the_slice_estimate_is_time_and_weight() {
        assert_eq!(slice_estimate(&slice()), "14 m · 3.7 g");
    }

    #[test]
    fn the_hero_reads_like_the_spec_example() {
        let s = MonthSummary {
            prints: 14,
            good: 11,
            usable: 2,
            finished: 0,
            failed: 1,
            printing_seconds: 38 * 3_600,
        };
        assert_eq!(
            hero_detail(&s),
            "11 good · 2 usable · 1 failed · 38 h printing"
        );
        let unrated = MonthSummary {
            prints: 3,
            finished: 2,
            printing_seconds: 45 * 60,
            ..Default::default()
        };
        assert_eq!(hero_detail(&unrated), "2 finished · 45 m printing");
        assert_eq!(hero_detail(&MonthSummary::default()), "");
        assert_eq!(
            (hero_unit(1), hero_unit(0), hero_unit(14)),
            (" print", " prints", " prints")
        );
        assert_eq!(printing_time(9_480), "3 h");
    }

    #[test]
    fn local_times_follow_the_given_offset() {
        assert_eq!(local_time_text(END, 0), "24 Oct 2026 14:05");
        assert_eq!(local_time_text(END, -420), "24 Oct 2026 07:05");
        assert_eq!(local_time_text(0, 0), "1 Jan 1970 00:00");
        assert_eq!(local_time_text(1_709_251_140, 0), "29 Feb 2024 23:59");
        assert_eq!(local_time_text(1_793_489_400, 60), "1 Nov 2026 00:30");
    }

    #[test]
    fn starts_and_ends_mark_what_was_not_seen() {
        let approx = PrintEntry {
            start_approx: true,
            ..entry()
        };
        assert_eq!(start_text(&approx, 0), "24 Oct 2026 12:07 (approximate)");
        assert_eq!(end_text(&entry(), 0), "24 Oct 2026 14:05");
        let lost = PrintEntry {
            ended_at: None,
            outcome: Outcome::Unknown,
            ..entry()
        };
        assert_eq!(end_text(&lost, 0), UNWATCHED);
        let open = PrintEntry {
            ended_at: None,
            outcome: Outcome::Printing,
            ..entry()
        };
        assert_eq!(end_text(&open, 0), "Still printing");
    }

    #[test]
    fn facts_show_errors_only_on_failure_and_estimates_only_with_a_slice() {
        let labels = |e: &PrintEntry| {
            fact_rows(e, 0)
                .into_iter()
                .map(|(l, _)| l)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            labels(&entry()),
            ["Started", "Ended", "Duration", "Printer", "Plate"]
        );
        let failed = PrintEntry {
            outcome: Outcome::Failed,
            error_code: Some(0x0300_400C),
            hms: vec!["0300_0100_0001_0007".into()],
            slice: Some(slice()),
            analyses: vec![41],
            ..entry()
        };
        let rows = fact_rows(&failed, 0);
        assert!(rows.contains(&("Error", "0300-400C".to_string())));
        assert!(rows.contains(&("HMS", "0300_0100_0001_0007".to_string())));
        assert!(rows.contains(&(SLICE_ESTIMATE, "14 m · 3.7 g".to_string())));
        assert!(rows.contains(&("Analyses", "1".to_string())));
        let finished_with_code = PrintEntry {
            error_code: Some(1),
            ..entry()
        };
        assert!(!labels(&finished_with_code).contains(&"Error"));
    }

    #[test]
    fn verdict_pills_toggle_and_tags_add_or_remove() {
        assert_eq!(
            verdict_after_click(None, Verdict::Good),
            VerdictChoice::Good
        );
        assert_eq!(
            verdict_after_click(Some(Verdict::Good), Verdict::Good),
            VerdictChoice::Clear
        );
        assert_eq!(
            verdict_after_click(Some(Verdict::Good), Verdict::Failed),
            VerdictChoice::Failed
        );
        let tags = vec!["warping".to_string()];
        assert_eq!(toggle_tag(&tags, "stringing"), ["warping", "stringing"]);
        assert!(toggle_tag(&tags, "warping").is_empty());
    }

    #[test]
    fn only_a_finished_print_without_a_verdict_asks_to_be_rated() {
        assert_eq!(
            rate_href(&entry()).as_deref(),
            Some("/printer/journal?entry=2")
        );
        let rated = PrintEntry {
            verdict: Some(Verdict::Good),
            ..entry()
        };
        assert_eq!(rate_href(&rated), None);
        let failed = PrintEntry {
            outcome: Outcome::Failed,
            ..entry()
        };
        assert_eq!(rate_href(&failed), None);
        assert_eq!(analyze_href(2), "/analysis?journal=2");
    }

    #[test]
    fn analyze_is_prefilled_from_the_entry() {
        assert_eq!(material_option("PLA-S"), Some("PLA"));
        assert_eq!(material_option("petg-cf"), Some("PETG"));
        assert_eq!(material_option("PAHT-CF"), Some("PA"));
        assert_eq!(material_option("PET-CF"), None);
        let e = PrintEntry {
            trays: vec![tray("A1", "PET-CF"), tray("A2", "PLA Matte")],
            slice: Some(slice()),
            photos: vec![
                Photo {
                    id: 7,
                    ..Default::default()
                },
                Photo {
                    id: 9,
                    ..Default::default()
                },
            ],
            ..entry()
        };
        assert_eq!(
            analyze_prefill(&e),
            AnalyzePrefill {
                model_path: Some("/m/cube.stl".into()),
                filament_preset: Some("Bambu PLA Basic @BBL H2D".into()),
                material: Some("PLA"),
                photo_id: Some(9),
            }
        );
        assert_eq!(analyze_prefill(&entry()), AnalyzePrefill::default());
        assert_eq!(
            data_url_base64("data:image/jpeg;base64,QUJD").as_deref(),
            Some("QUJD")
        );
        assert_eq!(data_url_base64("not a data url"), None);
    }

    #[test]
    fn an_unrated_finished_print_reads_finished_never_good() {
        // 6 prints: 3 finished without a verdict, 1 failed, and 2 that are
        // still printing or ended unwatched, which count only in the total.
        let s = MonthSummary {
            prints: 6,
            finished: 3,
            failed: 1,
            ..Default::default()
        };
        assert_eq!(hero_detail(&s), "3 finished · 1 failed");
        assert!(!hero_detail(&s).contains("good"));
        let verdicts_first = MonthSummary {
            prints: 4,
            good: 1,
            usable: 1,
            finished: 1,
            failed: 1,
            printing_seconds: 3_599,
        };
        assert_eq!(
            hero_detail(&verdicts_first),
            "1 good · 1 usable · 1 finished · 1 failed · 1 h printing"
        );
    }

    #[test]
    fn printing_time_rounds_minutes_before_choosing_the_unit() {
        assert_eq!(printing_time(0), "0 m");
        assert_eq!(printing_time(29), "0 m");
        assert_eq!(printing_time(3_570), "1 h");
        assert_eq!(printing_time(3_540), "59 m");
        assert_eq!(printing_time(38 * 3_600), "38 h");
        assert_eq!(printing_time(-5), "0 m");
        let blip = MonthSummary {
            prints: 1,
            good: 1,
            printing_seconds: 20,
            ..Default::default()
        };
        assert_eq!(hero_detail(&blip), "1 good");
    }

    #[test]
    fn a_tray_without_a_label_or_type_still_reads_sensibly() {
        assert_eq!(tray_chip(&tray("", "PETG")), "PETG");
        assert_eq!(tray_chip(&tray("", "")), "");
        assert_eq!(tray_chip(&tray(" A2 ", " PLA ")), "PLA · A2");
    }

    #[test]
    fn printer_reported_text_is_scrubbed_of_control_characters() {
        assert_eq!(tray_chip(&tray("A\u{202e}2", "PLA")), "PLA · A\u{fffd}2");
        let e = PrintEntry {
            printer_model: Some("H2D\n".into()),
            ..entry()
        };
        let rows = fact_rows(&e, 0);
        assert!(rows.contains(&("Printer", "H2D\u{fffd}".to_string())));
    }

    #[test]
    fn an_approximate_start_shows_in_the_row_the_detail_and_the_facts() {
        let approx = PrintEntry {
            start_approx: true,
            ..entry()
        };
        let now_ms = (END + 3 * 3_600) as f64 * 1000.0;
        assert_eq!(
            when_line(&approx, now_ms),
            "4h ago · 1 h 58 m (approximate)"
        );
        let rows = fact_rows(&approx, 0);
        assert!(rows.contains(&("Started", "24 Oct 2026 12:07 (approximate)".to_string())));
        assert!(rows.contains(&("Duration", "1 h 58 m (approximate)".to_string())));
        // No end was seen, so there is nothing to subtract the start from.
        let lost = PrintEntry {
            start_approx: true,
            ended_at: None,
            outcome: Outcome::Unknown,
            ..entry()
        };
        assert_eq!(duration_text(&lost), None);
        assert_eq!(when_line(&lost, now_ms), "4h ago");
    }

    #[test]
    fn an_end_before_the_start_never_goes_negative() {
        let odd = PrintEntry {
            ended_at: Some(END - 10_000),
            ..entry()
        };
        assert_eq!(duration_text(&odd).as_deref(), Some("0 m"));
    }
}
