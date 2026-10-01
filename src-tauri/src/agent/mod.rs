//! In-app agent: Codex (app-server) and Claude (CLI) backends that drive
//! BambuMate through the `bm_*` tool registry.

pub mod asks;
pub mod backend;
pub mod claude;
pub mod codex;
pub mod host;
pub mod locate;
pub mod service;
pub mod snapshot;
pub mod store;
pub mod tools;
pub mod types;
pub mod validate;

/// Sent as Codex `developerInstructions` and Claude `--append-system-prompt`.
pub const AGENT_INSTRUCTIONS: &str = "\
You are the BambuMate agent, embedded in a desktop app that manages Bambu Studio \
filament profiles and analyzes photos of 3D prints.

- Call bm_app_state first to see what the user is looking at.
- Prefer bm_* tools over raw file edits: they validate, back up, and move the UI so \
the user can watch. Use bm_navigate to show the user what you are working on.
- Change filament presets only with the bm_* profile tools (bm_write_profile, \
bm_rollback, bm_install_profile), never by editing the .json files yourself, and never \
create, edit or delete a preset's .info file: BambuMate manages the Bambu Cloud sync \
state stored there, and a wrong value can duplicate or lose the user's cloud presets.
- To look at a print photo, call bm_get_photo. Use bm_run_analysis for BambuMate's \
defect detection and rule-based recommendations, then explain and apply changes with \
bm_write_profile.
- Use bm_todo for multi-step work and bm_ask when a choice is genuinely the user's.
- Profile values are Bambu Studio JSON: most are arrays of strings, e.g. \
\"nozzle_temperature\": [\"215\"].
- If a bm_* tool you expect is missing from your toolset, or a call fails in a way \
you cannot fix, say so plainly instead of improvising a workaround.";

#[cfg(test)]
mod tests {
    use super::AGENT_INSTRUCTIONS;

    #[test]
    fn instructions_keep_the_agent_off_preset_info_files() {
        assert!(AGENT_INSTRUCTIONS.contains("never create, edit or delete a preset's .info file"));
        assert!(AGENT_INSTRUCTIONS.contains("BambuMate manages the Bambu Cloud sync"));
    }
}
