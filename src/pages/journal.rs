//! Printer → Journal (spec 2026-10-24 §2): every print the backend
//! recorded, newest first, under this month's tally (the page's one hero).
//! The list reloads on `journal://changed`; a generation counter drops a
//! load that a newer one overtook. Plate thumbnails load only as their rows
//! come near the visible part of the list.

use std::collections::HashMap;

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::components::patterns::{Hero, StateKind, StateText};
use crate::journal::bridge;
use crate::journal::format::{
    fact_rows, filter_key, hero_detail, hero_unit, outcome_key, outcome_text, tray_chip,
    verdict_text, when_line, EMPTY_JOURNAL, FILTERS, NAME_MAX, NO_MATCH, NO_PRINTER,
    PHOTO_UNAVAILABLE, PRINTING_NOW,
};
use crate::journal::types::{FilterKind, JournalFilter, MonthSummary, Outcome, PrintEntry};
use crate::journal::JournalShared;
use crate::printer::types::{swatch_color, untrusted_text};
use crate::printer::PrinterShared;

/// The loaded list; `None` until the first answer.
pub(crate) type Entries = RwSignal<Option<Result<Vec<PrintEntry>, String>>>;

/// Entry `id` from the loaded list. Tracked: call it in a `Memo`.
pub(crate) fn find_entry(entries: Entries, id: i64) -> Option<PrintEntry> {
    entries.with(|r| match r {
        Some(Ok(list)) => list.iter().find(|e| e.id == id).cloned(),
        _ => None,
    })
}

/// The prints still running, shown as "Printing now" above the list.
fn printing_now(list: &[PrintEntry]) -> Vec<PrintEntry> {
    list.iter()
        .filter(|e| e.outcome == Outcome::Printing)
        .cloned()
        .collect()
}

/// The rows of the list: every print that has ended, in the backend's order.
fn row_ids(list: &[PrintEntry]) -> Vec<i64> {
    list.iter()
        .filter(|e| e.outcome != Outcome::Printing)
        .map(|e| e.id)
        .collect()
}

/// Whether a pill other than All is pressed or there is search text, so an
/// empty answer means "nothing matches" rather than "nothing recorded".
fn is_filtered(kind: FilterKind, query: &str) -> bool {
    kind != FilterKind::All || !query.trim().is_empty()
}

/// The state line under the controls, if the list needs one.
fn list_state(
    entries: &Option<Result<Vec<PrintEntry>, String>>,
    filtered: bool,
) -> Option<(StateKind, Option<String>)> {
    match entries {
        None => Some((StateKind::Loading, None)),
        Some(Err(e)) => Some((StateKind::Error, Some(e.clone()))),
        Some(Ok(list)) if list.is_empty() => {
            let text = if filtered { NO_MATCH } else { EMPTY_JOURNAL };
            Some((StateKind::Empty, Some(text.to_string())))
        }
        Some(Ok(_)) => None,
    }
}

/// The hero's number: this month's prints, or a dash until it is counted.
fn hero_value(summary: Option<MonthSummary>) -> String {
    summary.map_or_else(|| "—".into(), |s| s.prints.to_string())
}

/// Whether the entry has a plate image to show: only a linked slice brings
/// one. Without it the row has no thumbnail and asks for none.
fn wants_thumbnail(e: &PrintEntry) -> bool {
    e.slice.as_ref().is_some_and(|s| s.has_thumbnail)
}

/// The "no printer" line shows only once the printer config is known, so a
/// cold load doesn't flash it before the first view arrives.
fn shows_no_printer(known: bool, configured: bool) -> bool {
    known && !configured
}

/// How far beyond the visible list a row starts loading its thumbnail.
const THUMB_MARGIN: &str = "300px 0px";

/// Loads plate thumbnails only once their row comes near the visible part of
/// the list: one `IntersectionObserver` for the page, made when the first
/// thumbnail mounts (the page is in the document by then) and disconnected
/// when the page unmounts. Loaded images are kept by entry id, so a row that
/// a filter hid and brings back doesn't ask again.
#[derive(Clone, Copy)]
struct Thumbs {
    observer: StoredValue<Option<web_sys::IntersectionObserver>, LocalStorage>,
    /// Rows waiting to come into view: entry id → their "near" flag.
    waiting: StoredValue<HashMap<i64, RwSignal<bool>>>,
    loaded: StoredValue<HashMap<i64, String>>,
}

impl Thumbs {
    fn new() -> Self {
        let observer = StoredValue::new_local(None::<web_sys::IntersectionObserver>);
        on_cleanup(move || {
            if let Some(Some(o)) = observer.try_with_value(|o| o.clone()) {
                o.disconnect();
            }
        });
        Self {
            observer,
            waiting: StoredValue::new(HashMap::new()),
            loaded: StoredValue::new(HashMap::new()),
        }
    }

    /// The page's observer, made on first use.
    fn observer(self) -> Option<web_sys::IntersectionObserver> {
        if let Some(o) = self.observer.try_with_value(|o| o.clone()).flatten() {
            return Some(o);
        }
        let waiting = self.waiting;
        let on_seen = Closure::<dyn FnMut(js_sys::Array, web_sys::IntersectionObserver)>::new(
            move |seen: js_sys::Array, observer: web_sys::IntersectionObserver| {
                for item in seen.iter() {
                    let item: web_sys::IntersectionObserverEntry = item.unchecked_into();
                    if !item.is_intersecting() {
                        continue;
                    }
                    let target = item.target();
                    observer.unobserve(&target);
                    let near = target
                        .get_attribute("data-thumb-for")
                        .and_then(|id| id.parse::<i64>().ok())
                        .and_then(|id| waiting.try_update_value(|w| w.remove(&id)).flatten());
                    if let Some(near) = near {
                        near.try_set(true);
                    }
                }
            },
        );
        let init = web_sys::IntersectionObserverInit::new();
        init.set_root_margin(THUMB_MARGIN);
        // The app scrolls inside `.content`, not the window; with it as the
        // root, the margin reaches below the fold.
        let content = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector(".content").ok().flatten());
        init.set_root(content.as_ref());
        let made = web_sys::IntersectionObserver::new_with_options(
            on_seen.as_ref().unchecked_ref(),
            &init,
        )
        .ok();
        // A callback already queued when the page unmounts may still run, so
        // the closure is leaked rather than dropped; it touches only `try_*`.
        on_seen.forget();
        self.observer.try_set_value(made.clone());
        made
    }

    /// Sets `near` once `el` comes near the visible list; at once if no
    /// observer could be made.
    fn watch(self, id: i64, el: &web_sys::Element, near: RwSignal<bool>) {
        match self.observer() {
            Some(o) => {
                self.waiting.try_update_value(|w| {
                    w.insert(id, near);
                });
                o.observe(el);
            }
            None => {
                near.try_set(true);
            }
        }
    }

    fn unwatch(self, id: i64, el: &web_sys::Element) {
        self.waiting.try_update_value(|w| {
            w.remove(&id);
        });
        if let Some(Some(o)) = self.observer.try_with_value(|o| o.clone()) {
            o.unobserve(el);
        }
    }
}

/// The open row after `id` is clicked: one row is open at a time.
fn toggled(open: Option<i64>, id: i64) -> Option<i64> {
    (open != Some(id)).then_some(id)
}

#[component]
pub fn JournalPage() -> impl IntoView {
    let printer = expect_context::<PrinterShared>();
    let changes = expect_context::<JournalShared>().changes;
    let no_printer = Memo::new(move |_| {
        shows_no_printer(printer.known.get(), printer.view.with(|v| v.configured))
    });
    let kind = RwSignal::new(FilterKind::All);
    let search = RwSignal::new(String::new());
    let entries: Entries = RwSignal::new(None);
    let summary = RwSignal::new(None::<MonthSummary>);
    let now = RwSignal::new(js_sys::Date::now());
    let open = RwSignal::new(None::<i64>);
    let thumbs = Thumbs::new();
    // Each load takes the next number; only the newest load's answer lands.
    let load_gen = StoredValue::new(0u64);
    let month_gen = StoredValue::new(0u64);

    Effect::new(move |_| {
        changes.track();
        let filter = JournalFilter {
            kind: kind.get(),
            query: search.get(),
        };
        let generation = load_gen.get_value().wrapping_add(1);
        load_gen.set_value(generation);
        spawn_local(async move {
            let list = bridge::list(&filter).await;
            if load_gen.try_get_value() != Some(generation) {
                return;
            }
            entries.try_set(Some(list));
            now.try_set(js_sys::Date::now());
        });
    });

    // The month doesn't depend on the filter: it is counted when the page
    // opens and when the journal changes, never on a search keystroke.
    Effect::new(move |_| {
        changes.track();
        let generation = month_gen.get_value().wrapping_add(1);
        month_gen.set_value(generation);
        spawn_local(async move {
            let month = bridge::month_summary().await;
            if month_gen.try_get_value() != Some(generation) {
                return;
            }
            if let Ok(m) = month {
                summary.try_set(Some(m));
            }
        });
    });

    let printing = Memo::new(move |_| {
        entries.with(|r| match r {
            Some(Ok(list)) => printing_now(list),
            _ => Vec::new(),
        })
    });
    let ids = Memo::new(move |_| {
        entries.with(|r| match r {
            Some(Ok(list)) => row_ids(list),
            _ => Vec::new(),
        })
    });
    let status = move || {
        let filtered = search.with(|q| is_filtered(kind.get(), q));
        let (state, message) = entries.with(|r| list_state(r, filtered))?;
        Some(match message {
            Some(m) => view! { <StateText kind=state message=m /> }.into_any(),
            None => view! { <StateText kind=state /> }.into_any(),
        })
    };

    view! {
        <div class="page journal-page nd">
            <Hero
                label="This month"
                value=Signal::derive(move || hero_value(summary.get()))
                unit=Signal::derive(move || hero_unit(summary.get().map_or(0, |s| s.prints)).to_string())
                detail=Signal::derive(move || summary.get().map(|s| hero_detail(&s)).unwrap_or_default())
            />
            <Show when=move || no_printer.get()>
                <p class="jr-no-printer">
                    {NO_PRINTER}
                    " "
                    <a href="/settings#printer" class="jr-setup-link">"Set one up in Settings → Printer"</a>
                </p>
            </Show>
            <div class="jr-controls">
                <div class="jr-filters" role="group" aria-label="Show">
                    {FILTERS
                        .iter()
                        .map(|&(k, label)| view! {
                            <button type="button" class="nd-tab jr-filter" data-kind=filter_key(k)
                                aria-pressed=move || (kind.get() == k).to_string()
                                on:click=move |_| kind.set(k)>
                                {label}
                            </button>
                        })
                        .collect_view()}
                </div>
                <input type="search" class="jr-search" placeholder="Search name, filament or notes"
                    aria-label="Search the journal"
                    prop:value=move || search.get()
                    on:input=move |ev| search.set(event_target_value(&ev)) />
            </div>
            <ul class="jr-now-list">
                <For
                    each=move || printing.get()
                    key=|e| e.id
                    children=move |e| view! {
                        <li class="jr-now">
                            <a href="/printer" class="jr-now-link">
                                <span class="nd-label">{PRINTING_NOW}</span>
                                <span class="jr-name">{untrusted_text(&e.name, NAME_MAX)}</span>
                            </a>
                        </li>
                    }
                />
            </ul>
            {status}
            <ul class="jr-list">
                <For
                    each=move || ids.get()
                    key=|id| *id
                    children=move |id| view! { <JournalRow id=id entries=entries now=now open=open thumbs=thumbs /> }
                />
            </ul>
        </div>
    }
}

/// One print in the list. The row is a button that opens its detail in
/// place, under it; opening another row closes this one.
#[component]
fn JournalRow(
    id: i64,
    entries: Entries,
    now: RwSignal<f64>,
    open: RwSignal<Option<i64>>,
    thumbs: Thumbs,
) -> impl IntoView {
    let entry = Memo::new(move |_| find_entry(entries, id));
    let has_thumbnail = Memo::new(move |_| entry.with(|e| e.as_ref().is_some_and(wants_thumbnail)));
    let expanded = Memo::new(move |_| open.get() == Some(id));
    let detail_id = format!("jr-detail-{id}");
    let controls = detail_id.clone();
    view! {
        <li class="jr-row" data-entry=id.to_string()>
            <button type="button" class="jr-row-main" class:jr-has-thumb=move || has_thumbnail.get()
                aria-expanded=move || expanded.get().to_string()
                aria-controls=move || expanded.get().then(|| controls.clone())
                on:click=move |_| open.update(|o| *o = toggled(*o, id))>
                <RowSummary id=id entry=entry now=now has_thumbnail=has_thumbnail thumbs=thumbs />
            </button>
            <Show when=move || expanded.get()>
                <RowDetail dom_id=detail_id.clone() entry=entry />
            </Show>
        </li>
    }
}

/// The row's line: thumbnail, name, when and how long, trays, outcome and
/// verdict. Printer-reported text goes through `untrusted_text`.
#[component]
fn RowSummary(
    id: i64,
    entry: Memo<Option<PrintEntry>>,
    now: RwSignal<f64>,
    has_thumbnail: Memo<bool>,
    thumbs: Thumbs,
) -> impl IntoView {
    let outcome =
        Memo::new(move |_| entry.with(|e| e.as_ref().map(|e| e.outcome).unwrap_or_default()));
    let name = move || {
        entry.with(|e| {
            e.as_ref()
                .map(|e| untrusted_text(&e.name, NAME_MAX))
                .unwrap_or_default()
        })
    };
    let when = move || {
        let now = now.get();
        entry.with(|e| e.as_ref().map(|e| when_line(e, now)).unwrap_or_default())
    };
    // Labelled, so the user's rating isn't read as an echo of the outcome.
    let verdict = move || {
        entry.with(|e| e.as_ref().and_then(|e| e.verdict)).map(|v| {
            view! {
                <span class="nd-label jr-verdict-label">"Verdict"</span>
                " "
                <span class="jr-verdict-value">{verdict_text(v)}</span>
            }
        })
    };
    let chips = move || {
        entry
            .with(|e| e.as_ref().map(|e| e.trays.clone()).unwrap_or_default())
            .into_iter()
            .map(|t| view! {
                <span class="jr-chip">
                    // The colour is the printer's tray colour, checked to be
                    // hex by `swatch_color`; anything else is transparent.
                    <span class="jr-swatch" aria-hidden="true" style:background-color=swatch_color(&t.color)></span>
                    {tray_chip(&t)}
                </span>
            })
            .collect_view()
    };
    view! {
        <Show when=move || has_thumbnail.get()>
            <RowThumb id=id thumbs=thumbs />
        </Show>
        <span class="jr-name">{name}</span>
        <span class="jr-when">{when}</span>
        <span class="jr-chips">{chips}</span>
        <span class="jr-outcome nd-label" data-outcome=move || outcome_key(outcome.get())>
            {move || outcome_text(outcome.get())}
        </span>
        <span class="jr-verdict-tag">{verdict}</span>
    }
}

/// The plate thumbnail of the linked slice, in a fixed box. It is asked for
/// only once the row comes near the visible list (see [`Thumbs`]), so the
/// list never waits for it; a missing or unreadable file reads "Photo
/// unavailable".
#[component]
fn RowThumb(id: i64, thumbs: Thumbs) -> impl IntoView {
    let cached = thumbs.loaded.with_value(|l| l.get(&id).cloned());
    let near = RwSignal::new(cached.is_some());
    let src = RwSignal::new(cached.map(Ok::<String, ()>));
    let node = NodeRef::<leptos::html::Span>::new();
    let watched = StoredValue::new_local(None::<web_sys::Element>);

    Effect::new(move |_| {
        let Some(el) = node.get() else {
            return;
        };
        if near.get_untracked() {
            return;
        }
        let el: web_sys::Element = el.unchecked_into();
        thumbs.watch(id, &el, near);
        watched.set_value(Some(el));
    });
    on_cleanup(move || {
        if let Some(Some(el)) = watched.try_with_value(|e| e.clone()) {
            thumbs.unwatch(id, &el);
        }
    });
    Effect::new(move |_| {
        if !near.get() || src.with_untracked(Option::is_some) {
            return;
        }
        spawn_local(async move {
            let url = bridge::image(id, None).await.map_err(|_| ());
            if let Ok(u) = &url {
                thumbs.loaded.try_update_value(|l| {
                    l.insert(id, u.clone());
                });
            }
            src.try_set(Some(url));
        });
    });

    view! {
        <span class="jr-thumb" node_ref=node data-thumb-for=id.to_string()>
            {move || match src.get() {
                Some(Ok(s)) => view! { <img src=s alt="" /> }.into_any(),
                Some(Err(())) => view! { <span class="jr-thumb-missing">{PHOTO_UNAVAILABLE}</span> }.into_any(),
                None => ().into_any(),
            }}
        </span>
    }
}

/// What opens under a row. For now it lists the entry's facts; the verdict,
/// tags, notes and photos join it with the entry detail (plan Task 8).
#[component]
fn RowDetail(dom_id: String, entry: Memo<Option<PrintEntry>>) -> impl IntoView {
    // Minutes east of UTC, for the start and end times.
    let offset = -(js_sys::Date::new_0().get_timezone_offset() as i32);
    let facts = move || {
        entry
            .with(|e| e.as_ref().map(|e| fact_rows(e, offset)))
            .unwrap_or_default()
            .into_iter()
            .map(|(label, value)| {
                view! {
                    <div class="jr-fact">
                        <dt>{label}</dt>
                        <dd>{value}</dd>
                    </div>
                }
            })
            .collect_view()
    };
    view! {
        <div class="jr-detail" id=dom_id role="region" aria-label="Print details">
            <dl class="jr-facts">{facts}</dl>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::patterns::state_text;

    fn at(id: i64, outcome: Outcome) -> PrintEntry {
        PrintEntry {
            id,
            outcome,
            ..Default::default()
        }
    }

    #[test]
    fn a_running_print_goes_in_printing_now_not_the_list() {
        let list = [
            at(3, Outcome::Printing),
            at(2, Outcome::Finished),
            at(1, Outcome::Failed),
            at(4, Outcome::Unknown),
        ];
        let now: Vec<i64> = printing_now(&list).iter().map(|e| e.id).collect();
        assert_eq!(now, [3]);
        assert_eq!(row_ids(&list), [2, 1, 4]);
    }

    #[test]
    fn the_state_line_tells_an_empty_journal_from_no_match() {
        let text = |r: &Option<Result<Vec<PrintEntry>, String>>, filtered| {
            list_state(r, filtered).map(|(k, m)| state_text(k, m.as_deref()))
        };
        assert_eq!(text(&None, false).as_deref(), Some("[LOADING…]"));
        assert_eq!(
            text(&Some(Ok(vec![])), false).as_deref(),
            Some("[EMPTY: Prints appear here as your printer runs them]")
        );
        assert_eq!(
            text(&Some(Ok(vec![])), true).as_deref(),
            Some("[EMPTY: No prints match]")
        );
        assert_eq!(
            text(
                &Some(Err("The print journal couldn't be read.".into())),
                false
            )
            .as_deref(),
            Some("[ERROR: The print journal couldn't be read.]")
        );
        assert_eq!(text(&Some(Ok(vec![at(1, Outcome::Finished)])), false), None);
        // A print still running is listed above, so the journal isn't empty.
        assert_eq!(text(&Some(Ok(vec![at(3, Outcome::Printing)])), false), None);
    }

    #[test]
    fn a_pill_other_than_all_or_any_search_text_is_a_filter() {
        assert!(!is_filtered(FilterKind::All, ""));
        assert!(!is_filtered(FilterKind::All, "   "));
        assert!(is_filtered(FilterKind::All, "petg"));
        assert!(is_filtered(FilterKind::NeedsVerdict, ""));
        assert!(is_filtered(FilterKind::Failed, ""));
    }

    #[test]
    fn the_hero_shows_a_dash_until_the_month_is_counted() {
        assert_eq!(hero_value(None), "—");
        let s = MonthSummary {
            prints: 5,
            ..Default::default()
        };
        assert_eq!(hero_value(Some(s)), "5");
    }

    #[test]
    fn only_a_slice_with_a_plate_image_asks_for_a_thumbnail() {
        use crate::journal::types::SliceSnapshot;
        assert!(!wants_thumbnail(&at(1, Outcome::Finished)));
        let no_image = PrintEntry {
            slice: Some(SliceSnapshot::default()),
            ..at(2, Outcome::Finished)
        };
        assert!(!wants_thumbnail(&no_image));
        let linked = PrintEntry {
            slice: Some(SliceSnapshot {
                has_thumbnail: true,
                ..Default::default()
            }),
            ..at(2, Outcome::Finished)
        };
        assert!(wants_thumbnail(&linked));
    }

    #[test]
    fn the_no_printer_line_waits_until_the_printer_config_is_known() {
        assert!(!shows_no_printer(false, false));
        assert!(!shows_no_printer(false, true));
        assert!(shows_no_printer(true, false));
        assert!(!shows_no_printer(true, true));
    }

    #[test]
    fn opening_a_row_closes_the_one_that_was_open() {
        assert_eq!(toggled(None, 2), Some(2));
        assert_eq!(toggled(Some(1), 2), Some(2));
        assert_eq!(toggled(Some(2), 2), None);
    }

    #[test]
    fn find_entry_picks_the_entry_from_the_loaded_list() {
        let owner = Owner::new();
        owner.with(|| {
            let entries: Entries = RwSignal::new(None);
            assert_eq!(find_entry(entries, 2), None);
            entries.set(Some(Ok(vec![
                at(1, Outcome::Failed),
                at(2, Outcome::Finished),
            ])));
            assert_eq!(
                find_entry(entries, 2).map(|e| e.outcome),
                Some(Outcome::Finished)
            );
            assert_eq!(find_entry(entries, 9), None);
            entries.set(Some(Err("x".into())));
            assert_eq!(find_entry(entries, 2), None);
        });
    }
}
