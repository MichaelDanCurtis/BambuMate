//! The seven hubs' tabs and where the routes from before the hubs went
//! (spec 2026-10-17 §1).

/// A tab inside a hub: its label and its route.
pub type Tab = (&'static str, &'static str);

pub const FILAMENT_TABS: &[Tab] = &[
    ("Advise", "/filament/advise"),
    ("Search", "/filament"),
    ("Batch", "/filament/batch"),
];

pub const PROFILES_TABS: &[Tab] = &[("Library", "/profiles"), ("Compare", "/profiles/compare")];

pub const SETTINGS_TABS: &[Tab] = &[
    ("General", "/settings"),
    ("Health", "/settings/health"),
    ("About", "/settings/about"),
];

/// Routes from before the hubs, and the tab each one moved to. The router's
/// fallback redirects them (see `app.rs`), so links and the agent's saved
/// routes keep working.
pub const LEGACY_REDIRECTS: &[(&str, &str)] = &[
    ("/batch", "/filament/batch"),
    ("/compare", "/profiles/compare"),
    ("/health", "/settings/health"),
    ("/about", "/settings/about"),
];

/// Where an old route moved to; `None` for any other path.
pub fn legacy_target(path: &str) -> Option<&'static str> {
    LEGACY_REDIRECTS
        .iter()
        .find(|(old, _)| *old == path)
        .map(|(_, new)| *new)
}

/// Every route a page is served at. Mirrors the `<Route>` list in
/// `app.rs`; the tests below and the rail's tests check against it.
#[cfg(test)]
pub const PAGE_ROUTES: &[&str] = &[
    "/",
    "/printer",
    "/filament",
    "/filament/advise",
    "/filament/batch",
    "/profiles",
    "/profiles/compare",
    "/slice",
    "/analysis",
    "/settings",
    "/settings/health",
    "/settings/about",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_old_route_moves_to_a_served_tab() {
        for (old, new) in LEGACY_REDIRECTS {
            assert!(
                PAGE_ROUTES.contains(new),
                "{old} -> {new}, which is not served"
            );
            assert!(!PAGE_ROUTES.contains(old), "{old} is still served directly");
        }
    }

    #[test]
    fn legacy_target_maps_old_routes_only() {
        assert_eq!(legacy_target("/batch"), Some("/filament/batch"));
        assert_eq!(legacy_target("/compare"), Some("/profiles/compare"));
        assert_eq!(legacy_target("/health"), Some("/settings/health"));
        assert_eq!(legacy_target("/about"), Some("/settings/about"));
        assert_eq!(legacy_target("/settings"), None);
        assert_eq!(legacy_target("/nope"), None);
    }

    #[test]
    fn every_tab_is_served_and_the_hub_itself_is_a_tab() {
        for (hub, tabs) in [
            ("/filament", FILAMENT_TABS),
            ("/profiles", PROFILES_TABS),
            ("/settings", SETTINGS_TABS),
        ] {
            assert!(
                tabs.iter().any(|(_, href)| *href == hub),
                "{hub} has no tab"
            );
            for (_, href) in tabs {
                assert!(PAGE_ROUTES.contains(href), "{href} is not served");
                assert!(href.starts_with(hub), "{href} is not under {hub}");
            }
        }
    }

    #[test]
    fn filament_tabs_read_advise_search_batch() {
        let labels: Vec<&str> = FILAMENT_TABS.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, ["Advise", "Search", "Batch"]);
    }
}
