pub mod types;
pub mod paths;
pub mod reader;
pub mod writer;
pub mod inheritance;
pub mod registry;
pub mod generator;

pub use types::{FilamentProfile, ProfileMetadata};
pub use paths::BambuPaths;
pub use writer::write_profile_atomic;
pub use registry::ProfileRegistry;
pub use generator::{generate_profile, is_bambu_studio_running};
