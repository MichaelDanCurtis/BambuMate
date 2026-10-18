use leptos::prelude::*;
use leptos_router::hooks::use_location;

use crate::app::{FeatureFlagsContext, UpdateContext};
use crate::components::icons::{Icon, IconKind};
use crate::components::stl_indicator::StlIndicator;
use crate::printer::PrinterShared;

/// Which list an item sits in: the main rail, or pinned below the divider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavSection {
    Main,
    End,
}

pub struct NavItem {
    pub href: &'static str,
    pub label: &'static str,
    pub icon: IconKind,
    pub section: NavSection,
}

/// The seven hubs (spec 2026-10-17 §1). Tabbed hubs keep their tabs in
/// `crate::routes`.
pub const NAV_ITEMS: &[NavItem] = &[
    NavItem {
        href: "/",
        label: "Home",
        icon: IconKind::Home,
        section: NavSection::Main,
    },
    NavItem {
        href: "/printer",
        label: "Printer",
        icon: IconKind::Printer,
        section: NavSection::Main,
    },
    NavItem {
        href: "/filament",
        label: "Filament",
        icon: IconKind::Create,
        section: NavSection::Main,
    },
    NavItem {
        href: "/profiles",
        label: "Profiles",
        icon: IconKind::Profiles,
        section: NavSection::Main,
    },
    NavItem {
        href: "/slice",
        label: "Slice",
        icon: IconKind::Slice,
        section: NavSection::Main,
    },
    NavItem {
        href: "/analysis",
        label: "Analyze",
        icon: IconKind::Analyze,
        section: NavSection::Main,
    },
    NavItem {
        href: "/settings",
        label: "Settings",
        icon: IconKind::Settings,
        section: NavSection::End,
    },
];

/// Home matches only "/"; every other item also matches its sub-paths.
pub fn is_active(path: &str, href: &str) -> bool {
    if href == "/" {
        path == "/"
    } else {
        path == href || path.starts_with(&format!("{href}/"))
    }
}

#[component]
pub fn Sidebar() -> impl IntoView {
    let flags = use_context::<FeatureFlagsContext>()
        .expect("FeatureFlagsContext not provided")
        .flags;
    let update_info = use_context::<UpdateContext>()
        .expect("UpdateContext not provided")
        .update_info;
    let printer = use_context::<PrinterShared>().expect("PrinterShared not provided");
    // No dot until a printer is set up.
    let printer_dot = move || {
        printer
            .view
            .with(|v| v.configured.then(|| v.connection.dot()))
    };
    let pathname = use_location().pathname;
    // A navigation click completes the hover interaction. Collapse even if
    // the pointer stays on the rail, so it cannot cover the new page's controls.
    // Leaving the rail arms hover again; keyboard focus still expands it.
    let hover_dismissed = RwSignal::new(false);

    let render = move |item: &'static NavItem| {
        let (href, label, icon) = (item.href, item.label, item.icon);
        let active = move || is_active(&pathname.get(), href);
        let locked = move || href == "/analysis" && !flags.get().analysis_enabled;
        // About is a Settings tab now, so the update dot sits on Settings.
        let has_update =
            move || href == "/settings" && update_info.get().map(|i| i.has_update).unwrap_or(false);
        view! {
            <li class="nav-item" class:nav-item-locked=locked>
                <a href=href class="nav-link" class:active=active class:nav-link-locked=locked
                    on:click=move |_| hover_dismissed.set(true)
                    aria-current=move || if active() { Some("page") } else { None }
                    aria-label=move || has_update().then(|| format!("{label}, update available"))
                    title=move || locked().then(|| "Requires AI — enable in Settings".to_string())>
                    <span class="nav-icon"><Icon kind=icon /></span>
                    <span class="nav-label">{label}</span>
                    <Show when=locked>
                        <span class="nav-lock"><Icon kind=IconKind::Lock /></span>
                    </Show>
                    <Show when=has_update>
                        <span class="nav-update-dot" aria-hidden="true" title="Update available"></span>
                    </Show>
                    <Show when=move || href == "/printer">
                        {move || printer_dot().map(|state| view! {
                            <span class="printer-dot" data-state=state role="img"
                                aria-label=format!("Printer: {state}") title="Printer connection"></span>
                        })}
                    </Show>
                </a>
            </li>
        }
    };
    let main_items = NAV_ITEMS
        .iter()
        .filter(|i| i.section == NavSection::Main)
        .map(render)
        .collect_view();
    let end_items = NAV_ITEMS
        .iter()
        .filter(|i| i.section == NavSection::End)
        .map(render)
        .collect_view();

    view! {
        <nav class="sidebar" class:hover-dismissed=move || hover_dismissed.get()
            on:mouseleave=move |_| hover_dismissed.set(false) aria-label="Main">
            <div class="sidebar-header">
                <span class="rail-mark" aria-hidden="true"></span>
                <span class="sidebar-wordmark">"BAMBUMATE"</span>
            </div>
            <ul class="nav-list">{main_items}</ul>
            <ul class="nav-list nav-list-end">{end_items}</ul>
            <div class="sidebar-footer">
                <StlIndicator />
            </div>
        </nav>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::PAGE_ROUTES;

    #[test]
    fn the_rail_has_the_seven_hubs_in_order() {
        let hrefs: Vec<&str> = NAV_ITEMS.iter().map(|i| i.href).collect();
        assert_eq!(
            hrefs,
            vec![
                "/",
                "/printer",
                "/filament",
                "/profiles",
                "/slice",
                "/analysis",
                "/settings"
            ]
        );
        let labels: Vec<&str> = NAV_ITEMS.iter().map(|i| i.label).collect();
        assert_eq!(
            labels,
            vec!["Home", "Printer", "Filament", "Profiles", "Slice", "Analyze", "Settings"]
        );
    }

    #[test]
    fn only_settings_sits_below_the_divider() {
        let end: Vec<&str> = NAV_ITEMS
            .iter()
            .filter(|i| i.section == NavSection::End)
            .map(|i| i.href)
            .collect();
        assert_eq!(end, vec!["/settings"]);
    }

    #[test]
    fn every_rail_item_is_a_served_route() {
        for item in NAV_ITEMS {
            assert!(
                PAGE_ROUTES.contains(&item.href),
                "{} is not served",
                item.href
            );
        }
    }

    #[test]
    fn active_matching_is_exact_for_home_and_prefix_for_others() {
        assert!(is_active("/", "/"));
        assert!(!is_active("/profiles", "/"));
        assert!(is_active("/profiles", "/profiles"));
        assert!(is_active("/profiles/compare", "/profiles"));
        assert!(!is_active("/profilesx", "/profiles"));
        assert!(is_active("/filament/batch", "/filament"));
        assert!(is_active("/settings/about", "/settings"));
    }
}
