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

/// Where a step sits relative to the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Done,
    Current,
    Upcoming,
}

impl StepState {
    /// `data-state` on the step.
    pub fn key(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Current => "current",
            Self::Upcoming => "upcoming",
        }
    }
}

pub fn step_state(index: usize, current: usize) -> StepState {
    match index.cmp(&current) {
        std::cmp::Ordering::Less => StepState::Done,
        std::cmp::Ordering::Equal => StepState::Current,
        std::cmp::Ordering::Greater => StepState::Upcoming,
    }
}

/// A hero with several Doto values side by side, each under its own mono
/// label. Still the page's one hero.
#[component]
pub fn HeroStats(
    #[prop(into)] label: Signal<String>,
    #[prop(into)] stats: Signal<Vec<(String, String)>>,
    #[prop(into, optional)] detail: MaybeProp<String>,
) -> impl IntoView {
    view! {
        <section class="nd-hero">
            <p class="nd-label nd-hero-label">{move || label.get()}</p>
            <div class="nd-hero-stats">
                {move || {
                    stats
                        .get()
                        .into_iter()
                        .map(|(name, value)| view! {
                            <div class="nd-hero-stat">
                                <span class="nd-label">{name}</span>
                                <span class="nd-display nd-hero-stat-value">{value}</span>
                            </div>
                        })
                        .collect_view()
                }}
            </div>
            {move || {
                detail
                    .get()
                    .filter(|d| !d.is_empty())
                    .map(|d| view! { <p class="nd-hero-detail">{d}</p> })
            }}
        </section>
    }
}

/// Numbered steps of a flow. Steps before the current one are buttons back
/// to that step while `can_go_back` holds; the rest are plain text.
#[component]
pub fn StepStrip(
    steps: &'static [&'static str],
    #[prop(into)] current: Signal<usize>,
    #[prop(into)] can_go_back: Signal<bool>,
    #[prop(into)] on_back: Callback<usize>,
) -> impl IntoView {
    let items = steps
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let state = move || step_state(i, current.get());
            let number = format!("{:02}", i + 1);
            view! {
                <li class="nd-step" data-state=move || state().key()>
                    {move || {
                        let inner = view! {
                            <span class="nd-step-num">{number.clone()}</span>
                            <span class="nd-step-name">{name}</span>
                            {(state() == StepState::Done)
                                .then(|| view! { <span class="nd-visually-hidden">" (done)"</span> })}
                        };
                        if state() == StepState::Done && can_go_back.get() {
                            view! {
                                <button type="button" class="nd-step-btn"
                                    title=format!("Back to {name}")
                                    on:click=move |_| on_back.run(i)>
                                    {inner}
                                </button>
                            }
                            .into_any()
                        } else {
                            view! {
                                <span class="nd-step-btn"
                                    aria-current=move || (state() == StepState::Current).then_some("step")>
                                    {inner}
                                </span>
                            }
                            .into_any()
                        }
                    }}
                </li>
            }
        })
        .collect_view();
    view! { <ol class="nd-steps" role="list" aria-label="Steps">{items}</ol> }
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

    #[test]
    fn steps_before_the_current_one_are_done() {
        assert_eq!(step_state(0, 1), StepState::Done);
        assert_eq!(step_state(1, 1), StepState::Current);
        assert_eq!(step_state(2, 1), StepState::Upcoming);
        assert_eq!(step_state(0, 0), StepState::Current);
    }

    #[test]
    fn step_states_have_distinct_keys() {
        let keys = [StepState::Done, StepState::Current, StepState::Upcoming].map(StepState::key);
        assert_eq!(keys, ["done", "current", "upcoming"]);
    }
}
