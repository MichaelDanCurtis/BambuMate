use std::collections::HashMap;

use super::types::FilamentProfile;

/// Registry of discovered filament profiles, keyed by profile name.
///
/// Full implementation in Task 2.
pub struct ProfileRegistry {
    pub(crate) profiles: HashMap<String, FilamentProfile>,
}
