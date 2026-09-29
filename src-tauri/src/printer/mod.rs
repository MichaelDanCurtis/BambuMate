//! Live, read-only connection to one Bambu printer on the local network.
//!
//! BambuMate only ever publishes the `pushall` and `get_version` read
//! requests. It never sends a control command and never talks to Bambu Cloud.

pub mod state;
pub mod tls;
