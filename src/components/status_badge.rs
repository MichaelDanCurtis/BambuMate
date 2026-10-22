use leptos::prelude::*;

/// Status for a health check item
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Fail,
    Unknown,
    /// An optional item that is known not to be set. Not a failure and not
    /// unknown: shown as a neutral dash.
    Optional,
}

impl CheckStatus {
    /// The badge's glyph and CSS classes.
    pub fn badge(self) -> (&'static str, &'static str) {
        match self {
            CheckStatus::Pass => ("\u{2713}", "status-badge status-pass"),
            CheckStatus::Fail => ("\u{2717}", "status-badge status-fail"),
            CheckStatus::Unknown => ("?", "status-badge status-unknown"),
            CheckStatus::Optional => ("\u{2013}", "status-badge status-optional"),
        }
    }
}

#[component]
pub fn StatusBadge(
    /// The label text, e.g. "Bambu Studio"
    #[prop(into)]
    label: String,
    /// The status of this check
    status: CheckStatus,
    /// Optional detail text, e.g. the path found
    #[prop(optional, into)]
    detail: Option<String>,
) -> impl IntoView {
    let (icon, class) = status.badge();

    view! {
        <div class="health-item">
            <span class=class>{icon}</span>
            <span class="health-name">{label}</span>
            <span class="health-detail">{detail.unwrap_or_default()}</span>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_optional_item_is_a_neutral_dash() {
        assert_eq!(
            CheckStatus::Optional.badge(),
            ("\u{2013}", "status-badge status-optional")
        );
    }

    #[test]
    fn only_a_failed_check_reads_as_a_failure() {
        for s in [
            CheckStatus::Pass,
            CheckStatus::Unknown,
            CheckStatus::Optional,
        ] {
            assert!(!s.badge().1.contains("status-fail"), "{s:?}");
        }
        assert!(CheckStatus::Fail.badge().1.contains("status-fail"));
    }
}
