pub mod generator;
pub mod inheritance;
pub mod paths;
pub mod reader;
pub mod registry;
pub mod types;
pub mod writer;

pub use generator::{find_existing_filament_id, generate_profile, is_bambu_studio_running};
pub use paths::{config_root_override, set_config_root_override, BambuPaths};
pub use registry::ProfileRegistry;
pub use types::{FilamentProfile, ProfileMetadata};
pub use writer::write_profile_atomic;
