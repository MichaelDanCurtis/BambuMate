//! Slot labels and the status of each slot: does the printer report the
//! preset the user assigned to it?

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::debug;

use super::state::{PrinterState, Tray};
use crate::history::SlotAssignment;
use crate::profile::{reader, FilamentProfile, ProfileRegistry};

/// `ams_id` for external spools in `slot_assignments`.
pub const EXTERNAL_AMS_ID: u32 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotStatus {
    /// The printer reports the assigned preset's filament id. "✓ Set".
    Matches,
    /// A Bambu RFID spool. "✓ Bambu spool".
    Rfid,
    /// The printer reports a different filament. "Set on printer".
    Different,
    /// No filament in the slot. "Empty".
    Empty,
    /// No assignment and no RFID. "Not set".
    Unassigned,
}

/// One slot as the Printer page and `bm_ams_slots` show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotView {
    pub ams_id: u32,
    pub tray_id: u32,
    /// `A1`…`D4`, `HT1`, `Ext-L`, `Ext-R` or `Ext`.
    pub label: String,
    /// What the printer reports for the slot.
    pub tray: Tray,
    pub rfid: bool,
    pub assigned_preset: Option<String>,
    pub assigned_filament_id: Option<String>,
    pub status: SlotStatus,
    /// The assigned preset is a user preset Bambu Cloud doesn't have yet,
    /// so the printer can't select it. Only computed while `status` is
    /// `Different` (the only status whose steps use it); false otherwise.
    pub needs_cloud_sync: bool,
    /// The slot has an assignment whose preset has no resolvable
    /// `filament_id`, so it can never report `Matches`. `status` stays
    /// `Different`; the UI says: "BambuMate can't check this slot: *{preset}*
    /// has no filament id. Set it on the printer and confirm by eye."
    #[serde(default)]
    pub preset_has_no_id: bool,
}

/// `A1`…`D4` for AMS units (lettered by id), `HT1`… for AMS HT units
/// (ids 128+), and `Ext-L`/`Ext-R` for the two H2 external spools (`Ext`
/// when there is only one). On the H2 series 254 is the left holder.
pub fn slot_label(ams_id: u32, tray_id: u32, external_count: usize) -> String {
    if ams_id == EXTERNAL_AMS_ID {
        return match (external_count >= 2, tray_id) {
            (true, 254) => "Ext-L".into(),
            (true, 255) => "Ext-R".into(),
            _ => "Ext".into(),
        };
    }
    if ams_id >= 128 {
        return format!("HT{}", ams_id - 127);
    }
    let letter = char::from_u32('A' as u32 + ams_id).unwrap_or('?');
    format!("{letter}{}", tray_id + 1)
}

/// The status rules, in order: an empty slot is Empty; a reported filament
/// id equal to the assigned preset's is Matches; an RFID spool is Rfid;
/// any other assignment is Different; otherwise Unassigned.
///
/// What the printer reports beats what the user assigned: an RFID spool
/// whose reported id matches the assignment is Matches, and one that
/// doesn't is Rfid, even when the assignment is stale.
pub fn slot_status(tray: &Tray, assigned: Option<&SlotAssignment>) -> SlotStatus {
    if tray.empty {
        return SlotStatus::Empty;
    }
    let reported = tray.tray_info_idx.trim();
    let assigned_id = assigned
        .and_then(|a| a.filament_id.as_deref())
        .map(str::trim)
        .filter(|id| !id.is_empty());
    if assigned_id.is_some_and(|id| !reported.is_empty() && id.eq_ignore_ascii_case(reported)) {
        return SlotStatus::Matches;
    }
    if tray.has_rfid() {
        return SlotStatus::Rfid;
    }
    if assigned.is_some() {
        return SlotStatus::Different;
    }
    SlotStatus::Unassigned
}

/// Every AMS slot, then the external spools, with its status.
/// `needs_cloud_sync` answers for an assigned preset's JSON path and is only
/// called for `Different` slots, so other statuses cost no file IO.
pub fn compute_slots(
    state: &PrinterState,
    assignments: &[SlotAssignment],
    needs_cloud_sync: &dyn Fn(&str) -> bool,
) -> Vec<SlotView> {
    let find = |ams_id: u32, tray_id: u32| {
        assignments
            .iter()
            .find(|a| a.ams_id == ams_id && a.tray_id == tray_id)
    };
    let external_count = state.external_spools.len();
    let mut out = Vec::new();
    let slots = state
        .ams_units
        .iter()
        .flat_map(|u| u.trays.iter().map(move |t| (u.id, t)))
        .chain(state.external_spools.iter().map(|t| (EXTERNAL_AMS_ID, t)));
    for (ams_id, tray) in slots {
        let assigned = find(ams_id, tray.id);
        let status = slot_status(tray, assigned);
        let is_different = status == SlotStatus::Different;
        out.push(SlotView {
            ams_id,
            tray_id: tray.id,
            label: slot_label(ams_id, tray.id, external_count),
            tray: tray.clone(),
            rfid: tray.has_rfid(),
            assigned_preset: assigned.map(|a| a.preset_name.clone()),
            assigned_filament_id: assigned.and_then(|a| a.filament_id.clone()),
            status,
            needs_cloud_sync: is_different
                && assigned
                    .and_then(|a| a.preset_path.as_deref())
                    .is_some_and(needs_cloud_sync),
            preset_has_no_id: is_different
                && assigned.is_some_and(|a| {
                    a.filament_id
                        .as_deref()
                        .map(str::trim)
                        .is_none_or(str::is_empty)
                }),
        });
    }
    out
}

/// A user preset (it has a `.info`) whose `setting_id` is still empty has
/// never reached Bambu Cloud, so the printer can't offer it yet.
pub fn preset_needs_cloud_sync(json_path: &Path) -> bool {
    match reader::read_profile_metadata(json_path) {
        Ok(Some(meta)) => meta.setting_id.trim().is_empty(),
        Ok(None) => false,
        Err(e) => {
            debug!("Can't read the .info for {json_path:?}: {e}");
            false
        }
    }
}

/// The preset's `filament_id`, following `inherits` through `registry`
/// when the preset doesn't set one itself.
pub fn resolve_filament_id(
    profile: &FilamentProfile,
    registry: &ProfileRegistry,
) -> Option<String> {
    let mut current = profile;
    for _ in 0..10 {
        if let Some(id) = current
            .filament_id()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(id.to_string());
        }
        let parent = current.inherits().filter(|p| !p.is_empty())?;
        current = registry.get_by_name(parent)?;
    }
    None
}

/// Plain-text steps for a slot whose printer setting differs from the
/// assignment. The Printer page renders the same copy with the preset name
/// in italics.
pub fn set_on_printer_steps(label: &str, preset: &str, needs_cloud_sync: bool) -> Vec<String> {
    let mut steps = vec![
        format!("On the printer: Filament → {label} → choose {preset}."),
        format!("Or in Bambu Studio: Device → AMS → {label} → {preset}."),
    ];
    if needs_cloud_sync {
        steps.push("It must sync to Bambu Cloud first — open Bambu Studio while signed in.".into());
    }
    steps
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::state::fixtures::{state_after, H2D_DELTA_AMS, H2D_FULL};

    fn assignment(ams_id: u32, tray_id: u32, filament_id: Option<&str>) -> SlotAssignment {
        SlotAssignment {
            serial: "SN".into(),
            ams_id,
            tray_id,
            preset_name: "Acme PLA".into(),
            filament_id: filament_id.map(str::to_string),
            preset_path: Some("/u/Acme PLA.json".into()),
            assigned_at: "2026-09-28 10:00:00".into(),
        }
    }

    fn tray(idx: &str, rfid: bool) -> Tray {
        Tray {
            id: 0,
            empty: false,
            tray_type: "PLA".into(),
            tray_info_idx: idx.into(),
            tag_uid: if rfid {
                "9AD3FBAC00000100"
            } else {
                "0000000000000000"
            }
            .into(),
            tray_uuid: "00000000000000000000000000000000".into(),
            ..Default::default()
        }
    }

    #[test]
    fn labels_cover_ams_units_ams_ht_and_external_spools() {
        assert_eq!(slot_label(0, 0, 2), "A1");
        assert_eq!(slot_label(1, 3, 2), "B4");
        assert_eq!(slot_label(3, 3, 2), "D4");
        assert_eq!(slot_label(128, 0, 2), "HT1");
        assert_eq!(slot_label(EXTERNAL_AMS_ID, 254, 2), "Ext-L");
        assert_eq!(slot_label(EXTERNAL_AMS_ID, 255, 2), "Ext-R");
        assert_eq!(slot_label(EXTERNAL_AMS_ID, 254, 1), "Ext");
    }

    #[test]
    fn matching_filament_id_is_set() {
        let a = assignment(0, 0, Some("P1234567"));
        assert_eq!(
            slot_status(&tray("p1234567", false), Some(&a)),
            SlotStatus::Matches
        );
    }

    #[test]
    fn an_rfid_spool_needs_no_assignment() {
        assert_eq!(slot_status(&tray("GFA00", true), None), SlotStatus::Rfid);
        let a = assignment(0, 0, Some("P1234567"));
        assert_eq!(
            slot_status(&tray("GFA00", true), Some(&a)),
            SlotStatus::Rfid
        );
    }

    #[test]
    fn a_different_reported_filament_needs_setting_on_the_printer() {
        let a = assignment(0, 0, Some("P1234567"));
        assert_eq!(
            slot_status(&tray("GFL99", false), Some(&a)),
            SlotStatus::Different
        );
        let unresolved = assignment(0, 0, None);
        assert_eq!(
            slot_status(&tray("", false), Some(&unresolved)),
            SlotStatus::Different
        );
    }

    #[test]
    fn an_empty_slot_is_empty_even_with_an_assignment() {
        let mut t = tray("", false);
        t.empty = true;
        let a = assignment(0, 0, Some("P1234567"));
        assert_eq!(slot_status(&t, Some(&a)), SlotStatus::Empty);
    }

    #[test]
    fn no_assignment_and_no_rfid_is_not_set() {
        assert_eq!(
            slot_status(&tray("GFL99", false), None),
            SlotStatus::Unassigned
        );
    }

    #[test]
    fn compute_slots_walks_every_unit_then_the_external_spools() {
        let state = state_after(&[H2D_FULL]);
        let assignments = vec![assignment(1, 1, Some("P1234567"))];
        let slots = compute_slots(&state, &assignments, &|_| true);
        let labels: Vec<&str> = slots.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["A1", "A2", "A3", "A4", "B1", "B2", "B3", "B4", "Ext-L", "Ext-R"]
        );
        let b2 = &slots[5];
        assert_eq!(b2.status, SlotStatus::Different);
        assert_eq!(b2.assigned_preset.as_deref(), Some("Acme PLA"));
        assert!(b2.needs_cloud_sync);
        assert_eq!(slots[1].status, SlotStatus::Rfid);
        assert_eq!(slots[2].status, SlotStatus::Unassigned);
        assert_eq!(slots[7].status, SlotStatus::Empty);
        assert_eq!(slots[9].status, SlotStatus::Empty);
    }

    #[test]
    fn a_report_that_changes_to_match_flips_the_slot_to_set() {
        let assignments = vec![assignment(1, 1, Some("P1234567"))];
        let before = compute_slots(&state_after(&[H2D_FULL]), &assignments, &|_| false);
        let after = compute_slots(
            &state_after(&[H2D_FULL, H2D_DELTA_AMS]),
            &assignments,
            &|_| false,
        );
        assert_eq!(before[5].status, SlotStatus::Different);
        assert_eq!(after[5].status, SlotStatus::Matches);
    }

    #[test]
    fn cloud_sync_is_needed_only_for_a_user_preset_without_a_setting_id() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("Acme PLA.json");
        std::fs::write(&json, r#"{"name":"Acme PLA"}"#).unwrap();
        assert!(!preset_needs_cloud_sync(&json), "no .info: a system preset");
        std::fs::write(
            json.with_extension("info"),
            "sync_info =\nuser_id = 1\nsetting_id =\nbase_id =\nupdated_time = 1\n",
        )
        .unwrap();
        assert!(preset_needs_cloud_sync(&json));
        std::fs::write(
            json.with_extension("info"),
            "sync_info =\nuser_id = 1\nsetting_id = PFUS123\nbase_id =\nupdated_time = 1\n",
        )
        .unwrap();
        assert!(!preset_needs_cloud_sync(&json));
    }

    #[test]
    fn filament_id_is_inherited_from_the_parent_preset() {
        let mut registry = ProfileRegistry::new();
        registry.insert(
            FilamentProfile::from_json(r#"{"name":"Bambu PLA Basic @base","filament_id":"GFA00"}"#)
                .unwrap(),
        );
        registry.insert(
            FilamentProfile::from_json(
                r#"{"name":"Bambu PLA Basic @BBL H2D","inherits":"Bambu PLA Basic @base"}"#,
            )
            .unwrap(),
        );
        let leaf = FilamentProfile::from_json(
            r#"{"name":"My PLA","inherits":"Bambu PLA Basic @BBL H2D"}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_filament_id(&leaf, &registry).as_deref(),
            Some("GFA00")
        );
        let own =
            FilamentProfile::from_json(r#"{"name":"Mine","filament_id":"P1234567"}"#).unwrap();
        assert_eq!(
            resolve_filament_id(&own, &registry).as_deref(),
            Some("P1234567")
        );
        let orphan =
            FilamentProfile::from_json(r#"{"name":"Orphan","inherits":"Missing"}"#).unwrap();
        assert_eq!(resolve_filament_id(&orphan, &registry), None);
    }

    #[test]
    fn steps_add_the_cloud_sync_note_only_when_needed() {
        let steps = set_on_printer_steps("B2", "Acme PLA", false);
        assert_eq!(
            steps,
            vec![
                "On the printer: Filament → B2 → choose Acme PLA.",
                "Or in Bambu Studio: Device → AMS → B2 → Acme PLA.",
            ]
        );
        assert_eq!(set_on_printer_steps("B2", "Acme PLA", true).len(), 3);
    }

    #[test]
    fn a_matching_report_beats_the_rfid_shortcut() {
        let a = assignment(0, 0, Some("GFA00"));
        assert_eq!(
            slot_status(&tray("GFA00", true), Some(&a)),
            SlotStatus::Matches
        );
    }

    #[test]
    fn an_rfid_spool_beats_a_stale_mismatching_assignment() {
        let a = assignment(0, 0, Some("P1234567"));
        assert_eq!(
            slot_status(&tray("GFA00", true), Some(&a)),
            SlotStatus::Rfid
        );
    }

    #[test]
    fn an_assignment_without_a_filament_id_is_flagged_and_stays_different() {
        for id in [None, Some(""), Some("  ")] {
            let state = state_after(&[H2D_FULL]);
            let slots = compute_slots(&state, &[assignment(1, 1, id)], &|_| false);
            assert_eq!(slots[5].status, SlotStatus::Different);
            assert!(slots[5].preset_has_no_id, "{id:?}");
        }
        let state = state_after(&[H2D_FULL]);
        let with_id = compute_slots(&state, &[assignment(1, 1, Some("P1"))], &|_| false);
        assert!(!with_id[5].preset_has_no_id);
        // Not flagged where the assignment isn't what decides the status.
        let rfid = compute_slots(&state, &[assignment(0, 1, None)], &|_| false);
        assert_eq!(rfid[1].status, SlotStatus::Rfid);
        assert!(!rfid[1].preset_has_no_id);
    }

    #[test]
    fn cloud_sync_is_only_checked_for_different_slots() {
        let state = state_after(&[H2D_FULL]);
        // A2 is an RFID spool, B2 is Different, B4 is empty.
        let assignments = vec![
            assignment(0, 1, Some("P1")),
            assignment(1, 1, Some("P1")),
            assignment(1, 3, Some("P1")),
        ];
        let calls = std::cell::Cell::new(0);
        let slots = compute_slots(&state, &assignments, &|_| {
            calls.set(calls.get() + 1);
            true
        });
        assert_eq!(calls.get(), 1);
        assert!(slots[5].needs_cloud_sync);
        assert!(!slots[1].needs_cloud_sync);
        assert!(!slots[7].needs_cloud_sync);
    }

    #[test]
    fn a_missing_preset_file_does_not_need_cloud_sync() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!preset_needs_cloud_sync(&dir.path().join("Gone.json")));
    }

    #[test]
    fn an_unreadable_info_file_does_not_need_cloud_sync() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("Bad.json");
        // A directory where the .info file should be: reading it fails.
        std::fs::create_dir(json.with_extension("info")).unwrap();
        assert!(!preset_needs_cloud_sync(&json));
    }

    #[test]
    fn an_inherits_cycle_terminates_with_none() {
        let mut registry = ProfileRegistry::new();
        registry.insert(FilamentProfile::from_json(r#"{"name":"A","inherits":"B"}"#).unwrap());
        registry.insert(FilamentProfile::from_json(r#"{"name":"B","inherits":"A"}"#).unwrap());
        let a = FilamentProfile::from_json(r#"{"name":"A","inherits":"B"}"#).unwrap();
        assert_eq!(resolve_filament_id(&a, &registry), None);
    }

    #[test]
    fn an_empty_filament_id_falls_through_to_the_parent() {
        let mut registry = ProfileRegistry::new();
        registry.insert(
            FilamentProfile::from_json(r#"{"name":"Base","filament_id":"GFA00"}"#).unwrap(),
        );
        let leaf =
            FilamentProfile::from_json(r#"{"name":"Leaf","filament_id":"  ","inherits":"Base"}"#)
                .unwrap();
        assert_eq!(
            resolve_filament_id(&leaf, &registry).as_deref(),
            Some("GFA00")
        );
    }
}
