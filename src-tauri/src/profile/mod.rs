pub mod generator;
pub mod inheritance;
pub mod nozzle;
pub mod paths;
pub mod reader;
pub mod registry;
pub mod types;
pub mod writer;

pub use generator::{find_existing_filament_id, generate_profile, is_bambu_studio_running};
pub use nozzle::{
    apply_nozzle_limits, clamp_volumetric_speed, max_volumetric_speed_cap, parse_nozzle_diameter,
    NozzleAdjustment,
};
pub use paths::BambuPaths;
pub use registry::ProfileRegistry;
pub use types::{FilamentProfile, ProfileMetadata};
pub use writer::write_profile_atomic;
