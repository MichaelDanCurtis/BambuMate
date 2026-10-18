//! Shared page patterns (spec 2026-10-17 §2). Pages build from these and
//! do not restyle them locally; their CSS is the "Page patterns" section
//! of style/main.css.

use leptos::prelude::*;
use leptos_router::hooks::use_location;

use crate::routes::Tab;

/// The colour a hero's value takes. Only the value is coloured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Neutral,
    Signal,
    Warning,
    Accent,
}

impl Tone {
    /// `data-tone` on the hero.
    pub fn key(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Signal => "signal",
            Self::Warning => "warning",
            Self::Accent => "accent",
        }
    }
}

/// What a `StateText` line reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    Loading,
    Empty,
    Error,
}

/// `[LOADING…]`, `[EMPTY]`, `[EMPTY: …]`, `[ERROR]` or `[ERROR: …]`.
pub fn state_text(kind: StateKind, message: Option<&str>) -> String {
    match (kind, message) {
        (StateKind::Loading, _) => "[LOADING…]".into(),
        (StateKind::Empty, None) => "[EMPTY]".into(),
        (StateKind::Empty, Some(m)) => format!("[EMPTY: {m}]"),
        (StateKind::Error, None) => "[ERROR]".into(),
        (StateKind::Error, Some(m)) => format!("[ERROR: {m}]"),
    }
}

/// How many of `total` segments to fill for `percent`. Rounds to the
/// nearest segment, but only 0% shows none and only 100% shows all.
pub fn segments_filled(percent: u32, total: u32) -> u32 {
    let p = percent.min(100);
    match total {
        0 => 0,
        1 => u32::from(p == 100),
        _ => match p {
            0 => 0,
            100 => total,
            _ => ((p * total + 50) / 100).clamp(1, total - 1),
        },
    }
}

/// The page's single `<h1>`, an optional row of actions, and an optional
/// row of tab pills that link to routes.
#[component]
pub fn PageHeader(
    title: &'static str,
    #[prop(optional)] tabs: &'static [Tab],
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let pathname = use_location().pathname;
    let tab_row = (!tabs.is_empty()).then(|| {
        let links = tabs
            .iter()
            .map(|&(label, href)| {
                let current = move || (pathname.get() == href).then_some("page");
                view! { <a href=href class="nd-tab" aria-current=current>{label}</a> }
            })
            .collect_view();
        view! { <nav class="nd-tabs" aria-label=format!("{title} sections")>{links}</nav> }
    });
    view! {
        <header class="nd-page-header">
            <div class="nd-page-header-row">
                <h1 class="nd-page-title">{title}</h1>
                {children.map(|c| view! { <div class="nd-page-actions">{c()}</div> })}
            </div>
            {tab_row}
        </header>
    }
}

/// The one big thing on a page: a mono label, a Doto value with an
/// optional unit, and an optional detail line. One per page.
#[component]
pub fn Hero(
    #[prop(into)] label: Signal<String>,
    #[prop(into)] value: Signal<String>,
    #[prop(into, optional)] unit: MaybeProp<String>,
    #[prop(into, optional)] detail: MaybeProp<String>,
    #[prop(into, optional)] tone: MaybeProp<Tone>,
) -> impl IntoView {
    view! {
        <section class="nd-hero" data-tone=move || tone.get().unwrap_or_default().key()>
            <p class="nd-label nd-hero-label">{move || label.get()}</p>
            <p class="nd-hero-value">
                <span class="nd-display nd-hero-number">{move || value.get()}</span>
                {move || unit.get().map(|u| view! { <span class="nd-hero-unit">{u}</span> })}
            </p>
            {move || {
                detail
                    .get()
                    .filter(|d| !d.is_empty())
                    .map(|d| view! { <p class="nd-hero-detail">{d}</p> })
            }}
        </section>
    }
}

/// Discrete progress: `total` segments, filled per [`segments_filled`].
/// `label` names the bar for assistive tech.
#[component]
pub fn SegmentedBar(
    label: &'static str,
    #[prop(into)] percent: Signal<u32>,
    total: u32,
) -> impl IntoView {
    let filled = move || segments_filled(percent.get(), total);
    let segments = (0..total)
        .map(|i| view! { <span class="nd-seg" class:nd-seg-on=move || i < filled()></span> })
        .collect_view();
    view! {
        <div class="nd-segbar" role="progressbar" aria-label=label aria-valuemin="0" aria-valuemax="100"
            aria-valuenow=move || percent.get().min(100).to_string()>
            {segments}
        </div>
    }
}

/// A label on the left, a mono value right-aligned.
#[component]
pub fn StatRow(label: &'static str, #[prop(into)] value: Signal<String>) -> impl IntoView {
    view! {
        <div class="nd-stat-row">
            <span class="nd-stat-row-label">{label}</span>
            <span class="nd-stat-row-value">{move || value.get()}</span>
        </div>
    }
}

/// An inline `[LOADING…]`, `[EMPTY…]` or `[ERROR…]` line, in place of
/// spinners, toasts and empty-state paragraphs.
#[component]
pub fn StateText(
    kind: StateKind,
    #[prop(into, optional)] message: MaybeProp<String>,
) -> impl IntoView {
    let key = match kind {
        StateKind::Loading => "loading",
        StateKind::Empty => "empty",
        StateKind::Error => "error",
    };
    let role = if kind == StateKind::Error {
        "alert"
    } else {
        "status"
    };
    view! {
        <p class="nd-state" data-kind=key role=role>
            {move || state_text(kind, message.get().as_deref())}
        </p>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_fill_only_at_the_ends_when_the_percent_is_at_the_ends() {
        assert_eq!(segments_filled(0, 12), 0);
        assert_eq!(segments_filled(100, 12), 12);
        // 1% shows something; 99% never looks finished.
        assert_eq!(segments_filled(1, 12), 1);
        assert_eq!(segments_filled(99, 12), 11);
        assert_eq!(segments_filled(6, 12), 1);
        assert_eq!(segments_filled(42, 12), 5);
        assert_eq!(segments_filled(50, 12), 6);
        assert_eq!(segments_filled(250, 12), 12);
    }

    #[test]
    fn tiny_bars_do_not_panic() {
        assert_eq!(segments_filled(50, 0), 0);
        assert_eq!(segments_filled(99, 1), 0);
        assert_eq!(segments_filled(100, 1), 1);
    }

    #[test]
    fn state_text_is_bracketed() {
        assert_eq!(state_text(StateKind::Loading, None), "[LOADING…]");
        assert_eq!(state_text(StateKind::Loading, Some("x")), "[LOADING…]");
        assert_eq!(state_text(StateKind::Empty, None), "[EMPTY]");
        assert_eq!(
            state_text(StateKind::Empty, Some("Page not found")),
            "[EMPTY: Page not found]"
        );
        assert_eq!(state_text(StateKind::Error, None), "[ERROR]");
        assert_eq!(
            state_text(StateKind::Error, Some("scan failed")),
            "[ERROR: scan failed]"
        );
    }

    #[test]
    fn tones_have_distinct_keys() {
        let keys = [Tone::Neutral, Tone::Signal, Tone::Warning, Tone::Accent].map(Tone::key);
        assert_eq!(keys, ["neutral", "signal", "warning", "accent"]);
        assert_eq!(Tone::default(), Tone::Neutral);
    }
}
