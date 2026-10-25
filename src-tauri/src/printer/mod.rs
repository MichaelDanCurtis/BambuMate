//! Live, read-only connection to one Bambu printer on the local network.
//!
//! BambuMate only ever publishes the `pushall` and `get_version` read
//! requests. It never sends a control command and never talks to Bambu Cloud.

pub mod client;
pub mod discovery;
pub mod filament_index;
pub mod hms;
pub mod model;
pub mod service;
pub mod settings;
pub mod slots;
pub mod state;
#[cfg(test)]
mod testbroker;
pub mod tls;
