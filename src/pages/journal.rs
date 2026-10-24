//! Printer → Journal (spec 2026-10-24 §2): every print the backend
//! recorded, newest first, under this month's tally (the page's one hero).
//! The list reloads on `journal://changed`; a generation counter drops a
//! load that a newer one overtook.

use leptos::prelude::*;
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

/// The open row after `id` is clicked: one row is open at a time.
fn toggled(open: Option<i64>, id: i64) -> Option<i64> {
    (open != Some(id)).then_some(id)
}

#[component]
pub fn JournalPage() -> impl IntoView {
    let printer = expect_context::<PrinterShared>();
    let changes = expect_context::<JournalShared>().changes;
    let configured = Memo::new(move |_| printer.view.with(|v| v.configured));
    let kind = RwSignal::new(FilterKind::All);
    let search = RwSignal::new(String::new());
    let entries: Entries = RwSignal::new(None);
    let summary = RwSignal::new(None::<MonthSummary>);
    let now = RwSignal::new(js_sys::Date::now());
    let open = RwSignal::new(None::<i64>);
    // Each load takes the next number; only the newest load's answer lands.
    let load_gen = StoredValue::new(0u64);

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
            let month = bridge::month_summary().await;
            if load_gen.try_get_value() != Some(generation) {
                return;
            }
            entries.try_set(Some(list));
            if let Ok(m) = month {
                summary.try_set(Some(m));
            }
            now.try_set(js_sys::Date::now());
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
            <Show when=move || !configured.get()>
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
                    children=move |id| view! { <JournalRow id=id entries=entries now=now open=open /> }
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
) -> impl IntoView {
    let entry = Memo::new(move |_| find_entry(entries, id));
    let expanded = Memo::new(move |_| open.get() == Some(id));
    let detail_id = format!("jr-detail-{id}");
    let controls = detail_id.clone();
    view! {
        <li class="jr-row" data-entry=id.to_string()>
            <button type="button" class="jr-row-main"
                aria-expanded=move || expanded.get().to_string()
                aria-controls=move || expanded.get().then(|| controls.clone())
                on:click=move |_| open.update(|o| *o = toggled(*o, id))>
                <RowSummary id=id entry=entry now=now />
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
fn RowSummary(id: i64, entry: Memo<Option<PrintEntry>>, now: RwSignal<f64>) -> impl IntoView {
    let has_thumbnail = Memo::new(move |_| {
        entry.with(|e| {
            e.as_ref()
                .and_then(|e| e.slice.as_ref())
                .is_some_and(|s| s.has_thumbnail)
        })
    });
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
    let verdict = move || {
        entry.with(|e| {
            e.as_ref()
                .and_then(|e| e.verdict)
                .map(verdict_text)
                .unwrap_or_default()
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
            <RowThumb id=id />
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

/// The plate thumbnail of the linked slice. It loads on its own once the row
/// is shown, so the list never waits for it; a missing or unreadable file
/// reads "Photo unavailable".
#[component]
fn RowThumb(id: i64) -> impl IntoView {
    let src = RwSignal::new(None::<Result<String, ()>>);
    spawn_local(async move {
        let url = bridge::image(id, None).await.map_err(|_| ());
        src.try_set(Some(url));
    });
    view! {
        <span class="jr-thumb">
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
