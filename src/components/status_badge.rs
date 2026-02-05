use leptos::prelude::*;

/// Status for a health check item
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Fail,
    Unknown,
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
    let (icon, class) = match status {
        CheckStatus::Pass => ("\u{2713}", "status-badge status-pass"),
        CheckStatus::Fail => ("\u{2717}", "status-badge status-fail"),
        CheckStatus::Unknown => ("?", "status-badge status-unknown"),
    };

    view! {
        <div class="health-item">
            <span class=class>{icon}</span>
            <span class="health-name">{label}</span>
            <span class="health-detail">{detail.unwrap_or_default()}</span>
        </div>
    }
}
