//! The seven hubs' tabs and where the routes from before the hubs went
//! (spec 2026-10-17 §1).

/// A tab inside a hub: its label and its route.
pub type Tab = (&'static str, &'static str);
