//! Is the job's filament loaded? Worked out from the job's filament and the
//! live slots, so the Slice result and Home follow every `printer://state`.
//! Slots count only while the printer is connected. Pure and host-tested.

use super::types::{untrusted_text, ConnectionState, PrinterView, SlotStatus, SlotView};
use crate::slicer::types::SliceResult;

/// Most characters shown of a material the printer reports.
const MATERIAL_MAX: usize = 16;

/// The filament a job was sliced with, as the AMS check needs it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobFilament {
    pub preset: String,
    pub filament_id: Option<String>,
    pub filament_type: String,
    /// The advisor's material id (`pla`, `pet_cf`), from the backend.
    pub material: Option<String>,
    pub color: String,
}

impl JobFilament {
    /// `slot`'s filament in `result`. `job_preset` stands in when the result
    /// doesn't name a preset, and the plate's use fills in a missing type or
    /// colour.
    pub fn of(result: &SliceResult, slot: u32, job_preset: &str) -> Self {
        let info = result.filaments.iter().find(|f| f.slot == slot);
        let used = result
            .plates
            .iter()
            .flat_map(|p| &p.filaments)
            .find(|f| f.slot == slot);
        let pick = |from_info: Option<&String>, from_use: Option<&String>| {
            from_info
                .filter(|s| !s.trim().is_empty())
                .or(from_use)
                .cloned()
                .unwrap_or_default()
        };
        Self {
            preset: info
                .map(|f| f.preset.clone())
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| job_preset.to_string()),
            filament_id: info.and_then(|f| f.filament_id.clone()),
            filament_type: pick(
                info.map(|f| &f.filament_type),
                used.map(|u| &u.filament_type),
            ),
            material: info.and_then(|f| f.material.clone()),
            color: pick(info.map(|f| &f.color), used.map(|u| &u.color)),
        }
    }

    /// The job's filament. BambuMate slices one filament per job.
    pub fn main(result: &SliceResult, job_preset: &str) -> Self {
        let slot = result
            .filaments
            .first()
            .map(|f| f.slot)
            .or_else(|| {
                result
                    .plates
                    .iter()
                    .flat_map(|p| &p.filaments)
                    .map(|f| f.slot)
                    .next()
            })
            .unwrap_or(1);
        Self::of(result, slot, job_preset)
    }

    /// What to load, as the lines name it: the type, else the preset.
    pub fn needs(&self) -> String {
        let t = self.filament_type.trim();
        if t.is_empty() {
            self.preset.clone()
        } else {
            t.to_string()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AmsMatch {
    /// A slot reports the job's filament ID, or is assigned the job's preset.
    Loaded {
        slot: String,
        other_colour: bool,
    },
    /// A slot holds the same material with another profile.
    SameMaterial {
        slot: String,
        material: String,
        other_colour: bool,
    },
    NotLoaded {
        material: String,
    },
    /// The printer isn't connected, or hasn't reported yet.
    NoPrinter,
}

impl AmsMatch {
    pub fn line(&self) -> String {
        let colour = |other: bool| if other { " · different colour" } else { "" };
        match self {
            Self::Loaded { slot, other_colour } => {
                format!("✓ Loaded in {slot}{}", colour(*other_colour))
            }
            Self::SameMaterial {
                slot,
                material,
                other_colour,
            } => format!(
                "~ {material} in {slot}, different profile: check temperatures{}",
                colour(*other_colour)
            ),
            Self::NotLoaded { material } => {
                format!("✗ Not loaded: load {material} before printing")
            }
            Self::NoPrinter => "Can't check the AMS: printer not connected".to_string(),
        }
    }

    /// `data-match`. The Slice result colours `loaded` with the signal tone
    /// and `same` with the warning tone; the others stay plain text.
    pub fn tone(&self) -> &'static str {
        match self {
            Self::Loaded { .. } => "loaded",
            Self::SameMaterial { .. } => "same",
            Self::NotLoaded { .. } => "missing",
            Self::NoPrinter => "unknown",
        }
    }
}

/// A filament ID, or `None` for the "no ID" values: empty and `"0"`.
fn real_id(id: &str) -> Option<&str> {
    let id = id.trim();
    (!id.is_empty() && id != "0").then_some(id)
}

/// `RRGGBB`, upper-cased, from `#00ae42`, `00AE42FF` and the like.
fn rgb(c: &str) -> Option<String> {
    let hex = c.trim().trim_start_matches('#');
    (matches!(hex.len(), 6 | 8) && hex.chars().all(|ch| ch.is_ascii_hexdigit()))
        .then(|| hex[..6].to_ascii_uppercase())
}

/// Among equal matches, a slot of the job's colour first, else the first.
fn pick<'a>(
    matches: Vec<&'a SlotView>,
    other_colour: impl Fn(&SlotView) -> bool,
) -> Option<&'a SlotView> {
    matches
        .iter()
        .copied()
        .find(|s| !other_colour(*s))
        .or_else(|| matches.first().copied())
}

/// Exact, then the same material, then not loaded. Empty slots never
/// count; the external spools do. Stale slots are never used: anything but
/// a connected printer with a report is `NoPrinter`.
pub fn ams_match(job: &JobFilament, view: &PrinterView) -> AmsMatch {
    if !view.configured || view.connection != ConnectionState::Connected || view.state.is_none() {
        return AmsMatch::NoPrinter;
    }
    let loaded: Vec<&SlotView> = view.slots.iter().filter(|s| !s.tray.empty).collect();
    let want = rgb(&job.color);
    let other_colour = |s: &SlotView| match (&want, rgb(&s.tray.tray_color)) {
        (Some(a), Some(b)) => *a != b,
        _ => false,
    };
    let id = job.filament_id.as_deref().and_then(real_id);
    let exact = |s: &&SlotView| {
        let by_id = id.is_some_and(|id| {
            real_id(&s.tray.tray_info_idx).is_some_and(|r| id.eq_ignore_ascii_case(r))
        });
        // The printer's own report beats a stale assignment on an RFID spool.
        let by_preset = !job.preset.is_empty()
            && s.status != SlotStatus::Rfid
            && s.assigned_preset.as_deref() == Some(job.preset.as_str());
        by_id || by_preset
    };
    let exact_slots: Vec<&SlotView> = loaded.iter().copied().filter(exact).collect();
    if let Some(s) = pick(exact_slots, &other_colour) {
        return AmsMatch::Loaded {
            slot: s.label.clone(),
            other_colour: other_colour(s),
        };
    }
    let same = |s: &&SlotView| match (&job.material, &s.material) {
        (Some(a), Some(b)) => a == b,
        _ => {
            let (a, b) = (job.filament_type.trim(), s.tray.tray_type.trim());
            !a.is_empty() && a.eq_ignore_ascii_case(b)
        }
    };
    let same_slots: Vec<&SlotView> = loaded.iter().copied().filter(same).collect();
    if let Some(s) = pick(same_slots, &other_colour) {
        return AmsMatch::SameMaterial {
            slot: s.label.clone(),
            material: untrusted_text(&s.tray.tray_type, MATERIAL_MAX),
            other_colour: other_colour(s),
        };
    }
    AmsMatch::NotLoaded {
        material: job.needs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::types::{PrinterState, Tray};
    use crate::slicer::types::{FilamentInfo, FilamentUse, PlateResult};

    fn slot(
        label: &str,
        tray_type: &str,
        id: &str,
        color: &str,
        material: Option<&str>,
    ) -> SlotView {
        SlotView {
            label: label.into(),
            tray: Tray {
                empty: tray_type.is_empty() && id.is_empty(),
                tray_type: tray_type.into(),
                tray_info_idx: id.into(),
                tray_color: color.into(),
                ..Default::default()
            },
            material: material.map(str::to_string),
            ..Default::default()
        }
    }

    fn connected(slots: Vec<SlotView>) -> PrinterView {
        PrinterView {
            configured: true,
            connection: ConnectionState::Connected,
            state: Some(PrinterState::default()),
            slots,
            ..Default::default()
        }
    }

    fn petg_cf() -> JobFilament {
        JobFilament {
            preset: "Bambu PETG-CF @BBL H2D".into(),
            filament_id: Some("GFG50".into()),
            filament_type: "PETG-CF".into(),
            material: Some("pet_cf".into()),
            color: "#000000".into(),
        }
    }

    fn pla_a1() -> SlotView {
        slot("A1", "PLA", "GFA00", "FFFFFFFF", Some("pla"))
    }

    #[test]
    fn a_slot_reporting_the_jobs_filament_id_is_loaded() {
        let v = connected(vec![
            pla_a1(),
            slot("B3", "PETG-CF", "gfg50", "000000FF", Some("pet_cf")),
        ]);
        let m = ams_match(&petg_cf(), &v);
        assert_eq!(
            m,
            AmsMatch::Loaded {
                slot: "B3".into(),
                other_colour: false
            }
        );
        assert_eq!(m.line(), "✓ Loaded in B3");
        assert_eq!(m.tone(), "loaded");
    }

    #[test]
    fn a_slot_assigned_the_jobs_preset_is_loaded_unless_an_rfid_report_beats_it() {
        let mut b1 = slot("B1", "PETG", "", "000000FF", Some("petg"));
        b1.assigned_preset = Some("Bambu PETG-CF @BBL H2D".into());
        b1.status = SlotStatus::Different;
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![b1.clone()])),
            AmsMatch::Loaded {
                slot: "B1".into(),
                other_colour: false
            }
        );
        b1.status = SlotStatus::Rfid;
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![b1])),
            AmsMatch::NotLoaded {
                material: "PETG-CF".into()
            }
        );
    }

    #[test]
    fn a_different_colour_only_adds_a_note_and_a_slot_of_the_jobs_colour_comes_first() {
        let white = slot("A2", "PETG-CF", "GFG50", "FFFFFFFF", Some("pet_cf"));
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![white.clone()])).line(),
            "✓ Loaded in A2 · different colour"
        );
        let black = slot("B3", "PETG-CF", "GFG50", "000000FF", Some("pet_cf"));
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![white, black])),
            AmsMatch::Loaded {
                slot: "B3".into(),
                other_colour: false
            }
        );
    }

    #[test]
    fn the_same_material_with_another_profile_says_to_check_temperatures() {
        let v = connected(vec![
            pla_a1(),
            slot("B1", "PETG-CF", "GFL99", "000000FF", Some("pet_cf")),
        ]);
        let m = ams_match(&petg_cf(), &v);
        assert_eq!(
            m,
            AmsMatch::SameMaterial {
                slot: "B1".into(),
                material: "PETG-CF".into(),
                other_colour: false
            }
        );
        assert_eq!(
            m.line(),
            "~ PETG-CF in B1, different profile: check temperatures"
        );
        assert_eq!(m.tone(), "same");
    }

    #[test]
    fn unclassified_materials_compare_by_their_type() {
        let pva = JobFilament {
            preset: "Bambu PVA @BBL H2D".into(),
            filament_type: "PVA".into(),
            ..Default::default()
        };
        assert_eq!(
            ams_match(&pva, &connected(vec![slot("A4", "PVA", "GFS99", "", None)])).tone(),
            "same"
        );
        let pla = JobFilament {
            filament_type: "PLA".into(),
            material: Some("pla".into()),
            ..pva
        };
        assert_eq!(
            ams_match(
                &pla,
                &connected(vec![slot("A1", "PLA-S", "GFS02", "", None)])
            )
            .tone(),
            "missing",
            "a support material isn't the PLA it supports"
        );
    }

    #[test]
    fn nothing_matching_is_not_loaded() {
        let m = ams_match(&petg_cf(), &connected(vec![pla_a1()]));
        assert_eq!(m.line(), "✗ Not loaded: load PETG-CF before printing");
        assert_eq!(m.tone(), "missing");
        let untyped = JobFilament {
            filament_type: String::new(),
            filament_id: None,
            material: None,
            ..petg_cf()
        };
        assert_eq!(
            ams_match(&untyped, &connected(vec![pla_a1()])).line(),
            "✗ Not loaded: load Bambu PETG-CF @BBL H2D before printing"
        );
    }

    #[test]
    fn empty_slots_are_ignored_and_the_external_spool_counts() {
        let mut stale = slot("B4", "", "", "", None);
        stale.tray.tray_info_idx = "GFG50".into();
        stale.assigned_preset = Some("Bambu PETG-CF @BBL H2D".into());
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![stale])).tone(),
            "missing"
        );
        let mut ext = slot("Ext-L", "PETG-CF", "GFG50", "000000FF", Some("pet_cf"));
        ext.ams_id = 255;
        ext.tray_id = 254;
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![pla_a1(), ext])).line(),
            "✓ Loaded in Ext-L"
        );
    }

    #[test]
    fn stale_slots_are_never_used_while_the_printer_is_not_connected() {
        let mut v = connected(vec![slot(
            "B3",
            "PETG-CF",
            "GFG50",
            "000000FF",
            Some("pet_cf"),
        )]);
        v.connection = ConnectionState::Unreachable;
        let m = ams_match(&petg_cf(), &v);
        assert_eq!(m, AmsMatch::NoPrinter);
        assert_eq!(m.line(), "Can't check the AMS: printer not connected");
        assert_eq!(m.tone(), "unknown");
        v.connection = ConnectionState::Connected;
        v.state = None;
        assert_eq!(
            ams_match(&petg_cf(), &v),
            AmsMatch::NoPrinter,
            "no report yet"
        );
        assert_eq!(
            ams_match(&petg_cf(), &PrinterView::default()),
            AmsMatch::NoPrinter
        );
    }

    #[test]
    fn the_jobs_filament_comes_from_the_result() {
        let result = SliceResult {
            plates: vec![PlateResult {
                index: 1,
                time_seconds: 1,
                weight_g: 1.0,
                cost: None,
                filaments: vec![FilamentUse {
                    slot: 1,
                    filament_type: "PLA".into(),
                    color: "#00AE42".into(),
                    used_g: 1.0,
                    used_m: 1.0,
                    cost: None,
                }],
                warnings: vec![],
                objects: vec![],
                thumbnail: None,
            }],
            printer: "P".into(),
            printer_model: "M".into(),
            process: "Q".into(),
            filaments: vec![FilamentInfo {
                slot: 1,
                preset: "Bambu PLA Basic @BBL H2C".into(),
                filament_type: "PLA".into(),
                color: "#00AE42".into(),
                cost_per_kg: None,
                density: None,
                filament_id: Some("GFA00".into()),
                material: Some("pla".into()),
            }],
            bed_type: None,
            bambu_studio_version: "02.08.02.61".into(),
            output_path: String::new(),
        };
        assert_eq!(
            JobFilament::main(&result, "Job preset"),
            JobFilament {
                preset: "Bambu PLA Basic @BBL H2C".into(),
                filament_id: Some("GFA00".into()),
                filament_type: "PLA".into(),
                material: Some("pla".into()),
                color: "#00AE42".into(),
            }
        );
        let bare = SliceResult {
            filaments: vec![],
            ..result
        };
        let f = JobFilament::main(&bare, "Job preset");
        assert_eq!(
            (
                f.preset.as_str(),
                f.filament_type.as_str(),
                f.color.as_str(),
                f.filament_id
            ),
            ("Job preset", "PLA", "#00AE42", None)
        );
    }

    #[test]
    fn a_tray_id_of_zero_is_no_id() {
        // A raw "0" from the printer is no ID: it must not match a job whose
        // ID is "0" either, and the type decides instead.
        let zero = JobFilament {
            filament_id: Some("0".into()),
            ..petg_cf()
        };
        let v = connected(vec![slot("A1", "PLA", "0", "", Some("pla"))]);
        assert_eq!(ams_match(&zero, &v).tone(), "missing");
        let v = connected(vec![slot("A1", "PETG-CF", "0", "", Some("pet_cf"))]);
        assert_eq!(ams_match(&zero, &v).tone(), "same");
    }

    #[test]
    fn a_slot_with_no_preset_for_the_nozzle_matches_like_one_without_a_detection() {
        let mut a1 = slot("A1", "PETG-CF", "GFG50", "000000FF", Some("pet_cf"));
        a1.other_nozzle = Some(0.6);
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![a1])),
            AmsMatch::Loaded {
                slot: "A1".into(),
                other_colour: false
            },
            "the printer still reports the job's filament ID"
        );
    }
}
