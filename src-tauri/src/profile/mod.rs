pub mod generator;
pub mod inheritance;
pub mod nozzle;
pub mod paths;
pub mod reader;
pub mod registry;
pub mod sync;
pub mod types;
pub mod writer;

pub use generator::{find_existing_filament_id, generate_profile, is_bambu_studio_running};
pub use nozzle::{
    apply_nozzle_limits, clamp_volumetric_speed, max_volumetric_speed_cap, parse_nozzle_diameter,
    NozzleAdjustment,
};
pub use paths::{config_root_override, set_config_root_override, BambuPaths};
pub use registry::ProfileRegistry;
pub use types::{FilamentProfile, ProfileMetadata};
pub use writer::write_profile_atomic;

use std::sync::atomic::{AtomicU64, Ordering};

static PRESETS_CHANGED: AtomicU64 = AtomicU64::new(0);

/// Records that BambuMate installed or repaired a preset, so the printer's
/// filament ID index rebuilds at once instead of within the minute.
pub fn note_presets_changed() {
    PRESETS_CHANGED.fetch_add(1, Ordering::Relaxed);
    #[cfg(test)]
    CHANGED_ON_THIS_THREAD.with(|n| n.set(n.get() + 1));
}

#[cfg(test)]
thread_local! {
    static CHANGED_ON_THIS_THREAD: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times this thread called [`note_presets_changed`]. Tests run
/// in parallel and bump the process-wide count too, so a test asserts an
/// exact change through this one.
#[cfg(test)]
pub(crate) fn presets_changed_on_this_thread() -> u64 {
    CHANGED_ON_THIS_THREAD.with(|n| n.get())
}

/// How many preset changes [`note_presets_changed`] has seen.
pub fn presets_generation() -> u64 {
    PRESETS_CHANGED.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    #[test]
    fn noting_a_preset_change_moves_the_generation_on() {
        let before = super::presets_generation();
        let mine = super::presets_changed_on_this_thread();
        super::note_presets_changed();
        assert!(super::presets_generation() > before);
        assert_eq!(super::presets_changed_on_this_thread(), mine + 1);
    }
}
