//! Mirrors of the backend's printer types (`src-tauri/src/printer/`) and
//! the copy the printer UI shows. Fields default when missing, so a newer
//! backend field never breaks deserialization.

use serde::{Deserialize, Serialize};

/// `ams_id` for external spools (tray ids 254 and 255).
pub const EXTERNAL_AMS_ID: u32 = 255;

/// Most characters shown of a serial a printer's certificate presented.
pub const PRESENTED_MAX_CHARS: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    AuthFailed,
    CertUntrusted {
        fingerprint: String,
    },
    /// `presented` comes from the certificate the host sent, so it is
    /// untrusted text: show it only through [`untrusted_text`].
    WrongSerial {
        presented: String,
    },
    Unreachable,
}

impl ConnectionState {
    /// `data-state` for the rail dot: connected, connecting or error.
    pub fn dot(&self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Connecting | Self::Disconnected => "connecting",
            _ => "error",
        }
    }

    /// A state the backend won't retry: the user has to change something.
    pub fn needs_user(&self) -> bool {
        matches!(
            self,
            Self::AuthFailed | Self::CertUntrusted { .. } | Self::WrongSerial { .. }
        )
    }
}

/// Text from the network, made safe to show: control and bidi-override
/// characters become `�` and it is cut to `max` characters. Render it as
/// plain text only.
pub fn untrusted_text(s: &str, max: usize) -> String {
    let unsafe_char = |c: char| {
        c.is_control()
            || matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
    };
    s.chars()
        .map(|c| if unsafe_char(c) { '\u{FFFD}' } else { c })
        .take(max)
        .collect()
}

/// What to tell the user about a connection state; `None` when connected.
pub fn connection_message(state: &ConnectionState, ip: &str) -> Option<String> {
    match state {
        ConnectionState::Connected => None,
        ConnectionState::Connecting => Some("Connecting to the printer…".into()),
        ConnectionState::Disconnected => Some("Not connected.".into()),
        ConnectionState::AuthFailed => Some(
            "The access code was rejected. Check it on the printer screen (Settings → LAN)."
                .into(),
        ),
        ConnectionState::Unreachable => Some(format!("Can't reach the printer at {ip}.")),
        ConnectionState::CertUntrusted { .. } => Some(
            "The printer's certificate isn't from a Bambu CA that BambuMate knows. Trust it in Settings → Printer."
                .into(),
        ),
        ConnectionState::WrongSerial { presented } => Some(format!(
            "The printer at {ip} reports serial {}. Check the serial in Settings → Printer.",
            untrusted_text(presented, PRESENTED_MAX_CHARS)
        )),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PrinterView {
    pub configured: bool,
    pub printer: Option<PrinterSummary>,
    pub connection: ConnectionState,
    pub state: Option<PrinterState>,
    pub slots: Vec<SlotView>,
    pub errors: Vec<ErrorView>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PrinterSummary {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
    pub firmware: Option<String>,
    /// The printer has verified against a Bambu CA before.
    pub ca_verified: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PrinterState {
    pub gcode_state: Option<String>,
    pub subtask_name: Option<String>,
    pub mc_percent: Option<u32>,
    pub mc_remaining_time: Option<u32>,
    pub layer_num: Option<u32>,
    pub total_layer_num: Option<u32>,
    pub bed_temp: Option<f64>,
    pub bed_target_temp: Option<f64>,
    pub nozzles: Vec<Nozzle>,
    pub active_nozzle: Option<u32>,
    pub ams_units: Vec<AmsUnit>,
    pub external_spools: Vec<Tray>,
    pub tray_now: Option<u32>,
    pub hms: Vec<HmsCode>,
    pub print_error: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Nozzle {
    pub id: u32,
    pub temp: Option<f64>,
    pub target_temp: Option<f64>,
    pub diameter: Option<f64>,
    pub nozzle_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AmsUnit {
    pub id: u32,
    pub humidity_level: Option<u32>,
    pub humidity_pct: Option<u32>,
    pub temp: Option<f64>,
    pub trays: Vec<Tray>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Tray {
    pub id: u32,
    pub empty: bool,
    pub tray_type: String,
    pub tray_color: String,
    pub tray_info_idx: String,
    pub tray_sub_brands: String,
    pub nozzle_temp_min: Option<u32>,
    pub nozzle_temp_max: Option<u32>,
    pub remain: Option<u32>,
    pub tag_uid: String,
    pub tray_uuid: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct HmsCode {
    pub attr: u32,
    pub code: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotStatus {
    Matches,
    Rfid,
    Different,
    Empty,
    #[default]
    Unassigned,
}

impl SlotStatus {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Matches => "✓ Set",
            Self::Rfid => "✓ Bambu spool",
            Self::Different => "Set on printer",
            Self::Empty => "Empty",
            Self::Unassigned => "Not set",
        }
    }
    /// `data-status` on the slot card.
    pub fn key(self) -> &'static str {
        match self {
            Self::Matches => "matches",
            Self::Rfid => "rfid",
            Self::Different => "different",
            Self::Empty => "empty",
            Self::Unassigned => "unassigned",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct SlotView {
    pub ams_id: u32,
    pub tray_id: u32,
    pub label: String,
    pub tray: Tray,
    pub rfid: bool,
    pub assigned_preset: Option<String>,
    pub assigned_filament_id: Option<String>,
    pub status: SlotStatus,
    pub needs_cloud_sync: bool,
    /// The assigned preset has no filament id, so the slot can't be checked.
    pub preset_has_no_id: bool,
}

impl SlotView {
    /// The preset name a card shows: the assigned one, else what the
    /// printer reports.
    pub fn preset_name(&self) -> String {
        if let Some(p) = &self.assigned_preset {
            return p.clone();
        }
        if !self.tray.tray_sub_brands.is_empty() {
            return self.tray.tray_sub_brands.clone();
        }
        self.tray.tray_info_idx.clone()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ErrorView {
    pub kind: String,
    pub code: String,
    pub text: Option<String>,
    pub wiki_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PrinterConfigView {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
    pub pinned_fingerprint: Option<String>,
    pub has_access_code: bool,
    /// The printer has verified against a Bambu CA before.
    pub ca_verified: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct DiscoveredPrinter {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
    /// Another address announced the same serial during the scan.
    pub conflict: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TestOutcome {
    pub connection: ConnectionState,
    pub got_report: bool,
    pub model: Option<String>,
}

/// `AMS A`, `AMS HT1` or `External`, for a row of slot cards.
pub fn row_title(ams_id: u32) -> String {
    match ams_id {
        EXTERNAL_AMS_ID => "External".into(),
        id if id >= 128 => format!("AMS HT{}", id - 127),
        id => format!("AMS {}", char::from_u32('A' as u32 + id).unwrap_or('?')),
    }
}

/// `1 h 05 m` or `9 m`.
pub fn format_remaining(minutes: u32) -> String {
    if minutes >= 60 {
        format!("{} h {:02} m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} m")
    }
}

/// `245 / 250 °C`, with `—` for a missing value.
pub fn format_temp(current: Option<f64>, target: Option<f64>) -> String {
    let t = |v: Option<f64>| v.map(|x| format!("{x:.0}")).unwrap_or_else(|| "—".into());
    format!("{} / {} °C", t(current), t(target))
}

/// On the H2 series extruder 0 is the right nozzle and 1 the left.
pub fn nozzle_label(id: u32, count: usize) -> &'static str {
    match (count >= 2, id) {
        (true, 0) => "Right nozzle",
        (true, 1) => "Left nozzle",
        _ => "Nozzle",
    }
}

/// A CSS colour for `RRGGBBAA`; transparent when the value is unusable.
pub fn swatch_color(tray_color: &str) -> String {
    let hex = tray_color.trim();
    if (hex.len() == 8 || hex.len() == 6) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("#{}", hex.to_ascii_lowercase())
    } else {
        "transparent".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_states_deserialize_from_the_backend_shape() {
        let s: ConnectionState =
            serde_json::from_str(r#"{"state":"cert_untrusted","fingerprint":"AB:CD"}"#).unwrap();
        assert_eq!(
            s,
            ConnectionState::CertUntrusted {
                fingerprint: "AB:CD".into()
            }
        );
        let s: ConnectionState = serde_json::from_str(r#"{"state":"auth_failed"}"#).unwrap();
        assert_eq!(s.dot(), "error");
        assert_eq!(ConnectionState::Connected.dot(), "connected");
        assert_eq!(ConnectionState::Connecting.dot(), "connecting");
    }

    #[test]
    fn messages_use_the_spec_copy() {
        assert_eq!(
            connection_message(&ConnectionState::AuthFailed, "10.0.0.2").unwrap(),
            "The access code was rejected. Check it on the printer screen (Settings → LAN)."
        );
        assert_eq!(
            connection_message(&ConnectionState::Unreachable, "10.0.0.2").unwrap(),
            "Can't reach the printer at 10.0.0.2."
        );
        assert_eq!(connection_message(&ConnectionState::Connected, "x"), None);
    }

    #[test]
    fn badges_use_the_spec_copy() {
        assert_eq!(SlotStatus::Matches.badge(), "✓ Set");
        assert_eq!(SlotStatus::Rfid.badge(), "✓ Bambu spool");
        assert_eq!(SlotStatus::Different.badge(), "Set on printer");
        assert_eq!(SlotStatus::Empty.badge(), "Empty");
        assert_eq!(SlotStatus::Unassigned.badge(), "Not set");
    }

    #[test]
    fn a_view_with_missing_fields_still_deserializes() {
        let v: PrinterView = serde_json::from_str(
            r#"{"configured":true,"connection":{"state":"connected"},
                "slots":[{"label":"A1","status":"rfid","tray":{"tray_type":"PLA"},"future":1}]}"#,
        )
        .unwrap();
        assert!(v.configured);
        assert_eq!(v.slots[0].status, SlotStatus::Rfid);
        assert_eq!(v.slots[0].tray.tray_type, "PLA");
        assert!(!v.slots[0].preset_has_no_id);
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(row_title(0), "AMS A");
        assert_eq!(row_title(3), "AMS D");
        assert_eq!(row_title(128), "AMS HT1");
        assert_eq!(row_title(255), "External");
        assert_eq!(format_remaining(549), "9 h 09 m");
        assert_eq!(format_remaining(9), "9 m");
        assert_eq!(format_temp(Some(245.4), Some(250.0)), "245 / 250 °C");
        assert_eq!(format_temp(None, Some(0.0)), "— / 0 °C");
        assert_eq!(nozzle_label(0, 2), "Right nozzle");
        assert_eq!(nozzle_label(1, 2), "Left nozzle");
        assert_eq!(nozzle_label(0, 1), "Nozzle");
        assert_eq!(swatch_color("F95959FF"), "#f95959ff");
        assert_eq!(swatch_color("nope"), "transparent");
    }

    #[test]
    fn a_card_prefers_the_assigned_preset_name() {
        let mut s = SlotView {
            tray: Tray {
                tray_sub_brands: "PLA Basic".into(),
                tray_info_idx: "GFA00".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(s.preset_name(), "PLA Basic");
        s.assigned_preset = Some("Acme PLA".into());
        assert_eq!(s.preset_name(), "Acme PLA");
    }

    #[test]
    fn only_auth_certificate_and_serial_failures_need_the_user() {
        assert!(ConnectionState::AuthFailed.needs_user());
        assert!(ConnectionState::CertUntrusted {
            fingerprint: "AB".into()
        }
        .needs_user());
        assert!(ConnectionState::WrongSerial {
            presented: "X".into()
        }
        .needs_user());
        assert!(!ConnectionState::Unreachable.needs_user());
        assert!(!ConnectionState::Disconnected.needs_user());
        assert!(!ConnectionState::Connecting.needs_user());
        assert!(!ConnectionState::Connected.needs_user());
    }

    #[test]
    fn a_presented_serial_is_shortened_and_stripped_of_control_characters() {
        let long = "A".repeat(200);
        let msg = connection_message(
            &ConnectionState::WrongSerial {
                presented: long.clone(),
            },
            "10.0.0.2",
        )
        .unwrap();
        assert!(msg.contains(&"A".repeat(PRESENTED_MAX_CHARS)));
        assert!(!msg.contains(&"A".repeat(PRESENTED_MAX_CHARS + 1)));
        assert_eq!(
            untrusted_text("ok\nno\u{202E}x", 64),
            "ok\u{FFFD}no\u{FFFD}x"
        );
        assert_eq!(untrusted_text("ééé", 2), "éé");
    }

    #[test]
    fn a_discovered_printer_carries_the_conflict_flag() {
        let p: DiscoveredPrinter = serde_json::from_str(
            r#"{"ip":"1.2.3.4","serial":"A","name":"","model":"","conflict":true}"#,
        )
        .unwrap();
        assert!(p.conflict);
        let p: DiscoveredPrinter =
            serde_json::from_str(r#"{"ip":"1.2.3.4","serial":"A","name":"","model":""}"#).unwrap();
        assert!(!p.conflict);
    }
}
