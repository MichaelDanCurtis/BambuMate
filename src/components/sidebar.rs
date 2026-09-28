use leptos::prelude::*;
use leptos_router::hooks::use_location;

use crate::app::{FeatureFlagsContext, UpdateContext};
use crate::components::icons::{Icon, IconKind};
use crate::components::stl_indicator::StlIndicator;

pub struct NavItem {
    pub href: &'static str,
    pub label: &'static str,
    pub icon: IconKind,
}

pub const NAV_ITEMS: &[NavItem] = &[
    NavItem {
        href: "/",
        label: "Home",
        icon: IconKind::Home,
    },
    NavItem {
        href: "/filament",
        label: "Create Profile",
        icon: IconKind::Create,
    },
    NavItem {
        href: "/analysis",
        label: "Print Analysis",
        icon: IconKind::Analyze,
    },
    NavItem {
        href: "/profiles",
        label: "Profiles",
        icon: IconKind::Profiles,
    },
    NavItem {
        href: "/batch",
        label: "Batch Generate",
        icon: IconKind::Batch,
    },
    NavItem {
        href: "/compare",
        label: "Compare Profiles",
        icon: IconKind::Compare,
    },
    NavItem {
        href: "/settings",
        label: "Settings",
        icon: IconKind::Settings,
    },
    NavItem {
        href: "/health",
        label: "Health Check",
        icon: IconKind::Health,
    },
    NavItem {
        href: "/about",
        label: "About",
        icon: IconKind::About,
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
    let ff_ctx = use_context::<FeatureFlagsContext>().expect("FeatureFlagsContext not provided");
    let update_ctx = use_context::<UpdateContext>().expect("UpdateContext not provided");
    let pathname = use_location().pathname;

    let items = NAV_ITEMS
        .iter()
        .map(|item| {
            let (href, label, icon) = (item.href, item.label, item.icon);
            let active = move || is_active(&pathname.get(), href);
            let locked = move || href == "/analysis" && !ff_ctx.flags.get().analysis_enabled;
            let has_update =
                move || href == "/about" && update_ctx.update_info.get().map(|i| i.has_update).unwrap_or(false);
            view! {
                <li class="nav-item" class:nav-item-locked=locked>
                    <a href=href class="nav-link" class:active=active class:nav-link-locked=locked
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
                    </a>
                </li>
            }
        })
        .collect_view();

    view! {
        <nav class="sidebar" aria-label="Main">
            <div class="sidebar-header">
                <span class="rail-mark" aria-hidden="true"></span>
                <span class="sidebar-wordmark">"BAMBUMATE"</span>
            </div>
            <ul class="nav-list">{items}</ul>
            <div class="sidebar-footer">
                <StlIndicator />
            </div>
        </nav>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_has_exactly_one_nav_item() {
        let hrefs: Vec<&str> = NAV_ITEMS.iter().map(|i| i.href).collect();
        assert_eq!(
            hrefs,
            vec![
                "/",
                "/filament",
                "/analysis",
                "/profiles",
                "/batch",
                "/compare",
                "/settings",
                "/health",
                "/about"
            ]
        );
    }

    #[test]
    fn active_matching_is_exact_for_home_and_prefix_for_others() {
        assert!(is_active("/", "/"));
        assert!(!is_active("/profiles", "/"));
        assert!(is_active("/profiles", "/profiles"));
        assert!(is_active("/profiles/abc", "/profiles"));
        assert!(!is_active("/profilesx", "/profiles"));
    }
}
