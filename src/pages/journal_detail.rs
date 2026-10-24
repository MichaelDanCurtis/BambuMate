//! One journal entry, expanded in place under its row (spec 2026-10-24 §2):
//! the facts, the verdict and issue tags, notes saved on blur, photos, and
//! Analyze / Delete. There is no modal and no new page.
//!
//! Every write after an await goes through `try_*`. Each part keeps its own
//! bookkeeping in a `StoredValue`, and an answer that lands once that is gone
//! (the detail closed, or another entry opened) changes nothing; the list
//! catches up through `journal://changed`.

use std::collections::VecDeque;

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::app::FeatureFlagsContext;
use crate::components::defect_report::defect_display_name;
use crate::journal::bridge;
use crate::journal::format::{
    analyze_href, fact_rows, toggle_tag, verdict_after_click, verdict_key, verdict_text,
    DELETE_CONFIRM, ISSUE_TAGS, MAX_PHOTO_BYTES, NOT_AN_IMAGE, PHOTO_UNAVAILABLE, TOO_LARGE,
    VERDICTS,
};
use crate::journal::types::{EntryUpdate, PrintEntry, Verdict, VerdictChoice};
use crate::pages::journal::{find_entry, remove_entry, replace_entry, row_ids, Entries};
use crate::pages::print_analysis::read_file_as_base64;

/// The backend's fixed sentences (`journal::store`). Anything else the
/// bridge reports (a malformed answer, say) is shown as the fallback.
const SAVE_FAILED: &str = "The print journal couldn't be saved.";
const READ_FAILED: &str = "The print journal couldn't be read.";
const NEWER_SCHEMA: &str =
    "This print journal was made by a newer BambuMate. Update BambuMate to open it.";
const PHOTO_FAILED: &str = "The photo couldn't be saved.";
const GONE: &str = "That print isn't in the journal any more.";
/// Shown while photos are read and sent.
const ADDING: &str = "Adding the photo";
const DELETING: &str = "Deleting the entry";
/// The backend keeps this many characters of notes (`NOTES_MAX`).
const NOTES_MAX: &str = "10000";

/// The error to show: one of the fixed sentences, else `fallback`. Raw text
/// never reaches the page.
fn fixed_error(e: &str, fallback: &'static str) -> &'static str {
    [
        SAVE_FAILED,
        READ_FAILED,
        NEWER_SCHEMA,
        PHOTO_FAILED,
        GONE,
        TOO_LARGE,
        NOT_AN_IMAGE,
    ]
    .into_iter()
    .find(|s| *s == e)
    .unwrap_or(fallback)
}

/// What a status line under a part of the detail says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Quiet,
    Busy(&'static str),
    Saved,
    Failed(&'static str),
}

fn status_kind(s: Status) -> &'static str {
    match s {
        Status::Quiet => "",
        Status::Busy(_) => "loading",
        Status::Saved => "saved",
        Status::Failed(_) => "error",
    }
}

fn status_text(s: Status) -> String {
    use crate::components::patterns::{state_text, StateKind};
    match s {
        Status::Quiet => String::new(),
        Status::Busy(m) => state_text(StateKind::Loading, Some(m)),
        Status::Saved => "[SAVED]".into(),
        Status::Failed(m) => state_text(StateKind::Error, Some(m)),
    }
}

/// A file the page refuses before reading it.
fn refusal(size_bytes: f64) -> Option<&'static str> {
    (size_bytes > MAX_PHOTO_BYTES).then_some(TOO_LARGE)
}

/// Whether a drag is over the photo strip (`dragenter`s not yet left).
fn dragging_over(depth: u32) -> bool {
    depth > 0
}

/// The row to focus once entry `id` is deleted: the next one, else the one
/// before it.
fn neighbour(ids: &[i64], id: i64) -> Option<i64> {
    let at = ids.iter().position(|i| *i == id)?;
    ids.get(at + 1)
        .or_else(|| at.checked_sub(1).and_then(|p| ids.get(p)))
        .copied()
}

fn chosen(choice: VerdictChoice) -> Option<Verdict> {
    match choice {
        VerdictChoice::Good => Some(Verdict::Good),
        VerdictChoice::Usable => Some(Verdict::Usable),
        VerdictChoice::Failed => Some(Verdict::Failed),
        VerdictChoice::Clear => None,
    }
}

/// What the verdict pills and the tag chips show.
#[derive(Debug, Clone, Default, PartialEq)]
struct Rating {
    verdict: Option<Verdict>,
    tags: Vec<String>,
}

impl Rating {
    fn of(e: &PrintEntry) -> Self {
        Self {
            verdict: e.verdict,
            tags: e.tags.clone(),
        }
    }

    /// The rating after verdict `v` is clicked, and the update that saves it.
    fn click_verdict(&self, v: Verdict) -> (Self, EntryUpdate) {
        let choice = verdict_after_click(self.verdict, v);
        let next = Self {
            verdict: chosen(choice),
            tags: self.tags.clone(),
        };
        let update = EntryUpdate {
            verdict: Some(choice),
            ..Default::default()
        };
        (next, update)
    }

    /// The rating after tag `tag` is clicked, and the update that saves it.
    fn click_tag(&self, tag: &str) -> (Self, EntryUpdate) {
        let tags = toggle_tag(&self.tags, tag);
        let next = Self {
            verdict: self.verdict,
            tags: tags.clone(),
        };
        let update = EntryUpdate {
            tags: Some(tags),
            ..Default::default()
        };
        (next, update)
    }
}

/// Save bookkeeping for controls that save on every click, as Settings'
/// preferred brands do. Each click is a request with a rising id;
/// `confirmed` is what the backend last said it stored, from the newest
/// request that succeeded. A failure puts that back, and once nothing is in
/// flight what is shown is what is stored.
#[derive(Debug)]
struct SaveSync<T> {
    confirmed: T,
    confirmed_id: u64,
    issued: u64,
    pending: u32,
    latest_error: Option<&'static str>,
}

/// What the controls do after an answer.
#[derive(Debug, PartialEq)]
struct Settled<T> {
    /// What to show now, if it changes.
    show: Option<T>,
    status: Option<Status>,
    /// This answer is now the confirmed state: the list takes it.
    adopted: bool,
}

impl<T: Clone> SaveSync<T> {
    fn new(stored: T) -> Self {
        Self {
            confirmed: stored,
            confirmed_id: 0,
            issued: 0,
            pending: 0,
            latest_error: None,
        }
    }

    fn idle(&self) -> bool {
        self.pending == 0
    }

    /// The journal changed while nothing was in flight.
    fn loaded(&mut self, stored: T) {
        self.confirmed = stored;
    }

    /// A click is about to be sent; returns its id.
    fn issue(&mut self) -> u64 {
        self.issued += 1;
        self.pending += 1;
        self.latest_error = None;
        self.issued
    }

    /// The answer to request `id`. A success advances `confirmed` unless a
    /// newer request already did, so a success that a newer failure follows
    /// is kept. The view moves when the newest request answers, and again
    /// when the last one in flight does.
    fn settle(&mut self, id: u64, result: Result<T, &'static str>) -> Settled<T> {
        self.pending = self.pending.saturating_sub(1);
        let mut adopted = false;
        let error = result.as_ref().err().copied();
        if let Ok(stored) = result {
            if id > self.confirmed_id {
                self.confirmed = stored;
                self.confirmed_id = id;
                adopted = true;
            }
        }
        if id == self.issued {
            self.latest_error = error;
        }
        if id == self.issued || self.pending == 0 {
            Settled {
                show: Some(self.confirmed.clone()),
                status: Some(self.latest_error.map_or(Status::Saved, Status::Failed)),
                adopted,
            }
        } else {
            Settled {
                show: None,
                status: None,
                adopted,
            }
        }
    }
}

/// Notes save on blur, and only when the text differs from what the journal
/// holds. Each save takes the next generation; only the newest one's answer
/// sets the status or the list.
#[derive(Debug)]
struct NotesSync {
    /// The text the journal holds, as far as the page knows; `None` after a
    /// failed save, so the next blur tries again.
    saved: Option<String>,
    generation: u64,
}

impl NotesSync {
    fn new(stored: String) -> Self {
        Self {
            saved: Some(stored),
            generation: 0,
        }
    }

    /// On blur: the generation to send `draft` under, or `None` if the
    /// journal already holds it.
    fn blur(&mut self, draft: &str) -> Option<u64> {
        if self.saved.as_deref() == Some(draft) {
            return None;
        }
        self.saved = Some(draft.to_string());
        self.generation += 1;
        Some(self.generation)
    }

    /// The answer to save `generation`; `None` when a newer save has gone
    /// out since, which owns the status.
    fn answer(&mut self, generation: u64, result: Result<(), &'static str>) -> Option<Status> {
        if generation != self.generation {
            return None;
        }
        Some(match result {
            Ok(()) => Status::Saved,
            Err(e) => {
                self.saved = None;
                Status::Failed(e)
            }
        })
    }
}

/// A status line, mounted once and empty while there is nothing to say, so
/// a screen reader announces what arrives in it.
#[component]
fn StatusLine(status: RwSignal<Status>) -> impl IntoView {
    view! {
        <p class="nd-state jr-status" role="status" data-kind=move || status_kind(status.get())>
            {move || status_text(status.get())}
        </p>
    }
}

#[component]
pub fn EntryDetail(id: i64, entries: Entries, open: RwSignal<Option<i64>>) -> impl IntoView {
    let flags = expect_context::<FeatureFlagsContext>().flags;
    let entry = Memo::new(move |_| find_entry(entries, id));
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
        <div class="jr-detail" id=format!("jr-detail-{id}") role="region" aria-label="Print details">
            <dl class="jr-facts">{facts}</dl>
            <RatingControls id=id entry=entry entries=entries />
            <Notes id=id entry=entry entries=entries />
            <Photos id=id entry=entry entries=entries />
            <Actions id=id entries=entries open=open
                analysis=Signal::derive(move || flags.get().analysis_enabled) />
        </div>
    }
}

/// The verdict pills and the issue-tag chips. Each click saves at once; the
/// pills show the click straight away, then settle on what was stored.
#[component]
fn RatingControls(id: i64, entry: Memo<Option<PrintEntry>>, entries: Entries) -> impl IntoView {
    let stored = Memo::new(move |_| entry.with(|e| e.as_ref().map(Rating::of).unwrap_or_default()));
    let shown = RwSignal::new(stored.get_untracked());
    let status = RwSignal::new(Status::Quiet);
    let sync = StoredValue::new(SaveSync::new(stored.get_untracked()));

    // The list reloaded while nothing was in flight: show what it holds.
    Effect::new(move |_| {
        let now = stored.get();
        if sync.try_with_value(SaveSync::idle) == Some(true) {
            sync.try_update_value(|s| s.loaded(now.clone()));
            shown.try_set(now);
        }
    });

    let save = move |(next, update): (Rating, EntryUpdate)| {
        shown.set(next);
        status.set(Status::Quiet);
        let Some(request) = sync.try_update_value(|s| s.issue()) else {
            return;
        };
        spawn_local(async move {
            let answer = bridge::update(id, &update)
                .await
                .map_err(|e| fixed_error(&e, SAVE_FAILED));
            let rating = answer.as_ref().map(Rating::of).map_err(|e| *e);
            let Some(settled) = sync.try_update_value(|s| s.settle(request, rating)) else {
                return;
            };
            if let (true, Ok(saved)) = (settled.adopted, answer) {
                replace_entry(entries, saved);
            }
            if let Some(show) = settled.show {
                shown.try_set(show);
            }
            if let Some(s) = settled.status {
                status.try_set(s);
            }
        });
    };

    let verdicts = VERDICTS
        .iter()
        .map(|&v| {
            view! {
                <button type="button" class="nd-tab jr-verdict" data-verdict=verdict_key(v)
                    aria-pressed=move || shown.with(|r| r.verdict == Some(v)).to_string()
                    on:click=move |_| save(shown.with_untracked(|r| r.click_verdict(v)))>
                    {verdict_text(v)}
                </button>
            }
        })
        .collect_view();
    let tags = ISSUE_TAGS
        .iter()
        .map(|&tag| {
            view! {
                <button type="button" class="nd-tab jr-tag" data-tag=tag
                    aria-pressed=move || shown.with(|r| r.tags.iter().any(|t| t == tag)).to_string()
                    on:click=move |_| save(shown.with_untracked(|r| r.click_tag(tag)))>
                    {defect_display_name(tag)}
                </button>
            }
        })
        .collect_view();
    let verdict_label = format!("jr-verdict-label-{id}");
    let issues_label = format!("jr-issues-label-{id}");

    view! {
        <section class="jr-section jr-rate">
            <h2 class="nd-label" id=verdict_label.clone()>"Verdict"</h2>
            <div class="jr-verdicts" role="group" aria-labelledby=verdict_label>{verdicts}</div>
            <h2 class="nd-label" id=issues_label.clone()>"Issues"</h2>
            <div class="jr-tags" role="group" aria-labelledby=issues_label>{tags}</div>
            <StatusLine status=status />
        </section>
    }
}

/// The notes box. It saves when it loses focus, and only if the text changed.
#[component]
fn Notes(id: i64, entry: Memo<Option<PrintEntry>>, entries: Entries) -> impl IntoView {
    let initial = entry.with_untracked(|e| e.as_ref().map(|e| e.notes.clone()).unwrap_or_default());
    let draft = RwSignal::new(initial.clone());
    let status = RwSignal::new(Status::Quiet);
    let sync = StoredValue::new(NotesSync::new(initial.clone()));
    let box_id = format!("jr-notes-{id}");

    let on_blur = move |_| {
        let text = draft.get_untracked();
        let Some(generation) = sync.try_update_value(|s| s.blur(&text)).flatten() else {
            return;
        };
        status.set(Status::Quiet);
        spawn_local(async move {
            let update = EntryUpdate {
                notes: Some(text),
                ..Default::default()
            };
            let answer = bridge::update(id, &update)
                .await
                .map_err(|e| fixed_error(&e, SAVE_FAILED));
            let result = answer.as_ref().map(|_| ()).map_err(|e| *e);
            let Some(Some(s)) = sync.try_update_value(|n| n.answer(generation, result)) else {
                return;
            };
            if let Ok(saved) = answer {
                replace_entry(entries, saved);
            }
            status.try_set(s);
        });
    };

    view! {
        <div class="jr-section jr-notes-field">
            <label class="nd-label" for=box_id.clone()>"Notes"</label>
            <textarea class="jr-notes" id=box_id rows="3" maxlength=NOTES_MAX
                prop:value=initial
                on:input=move |ev| draft.set(event_target_value(&ev))
                on:blur=on_blur></textarea>
            <StatusLine status=status />
        </div>
    }
}

/// The photo strip: add with the picker or by dropping files on it, and
/// remove each one. Files go to the backend one at a time; a file over
/// 25 MB is refused before it is read, and one that isn't an image before it
/// is sent.
#[component]
fn Photos(id: i64, entry: Memo<Option<PrintEntry>>, entries: Entries) -> impl IntoView {
    let status = RwSignal::new(Status::Quiet);
    // Drags over the strip, counted so that crossing a child doesn't clear it.
    let over = RwSignal::new(0u32);
    let queue = StoredValue::new_local(VecDeque::<web_sys::File>::new());
    let busy = StoredValue::new(false);
    // The last refusal of the batch in flight, shown when it ends.
    let refused = StoredValue::new(None::<&'static str>);
    let input = NodeRef::<leptos::html::Input>::new();
    let heading = format!("jr-photos-label-{id}");
    let labelled_by = heading.clone();

    let drain = move || {
        if busy.try_get_value() != Some(false) {
            return;
        }
        busy.set_value(true);
        spawn_local(async move {
            loop {
                let Some(next) = queue.try_update_value(VecDeque::pop_front) else {
                    return;
                };
                let Some(file) = next else {
                    break;
                };
                status.try_set(Status::Busy(ADDING));
                let answer = match read_file_as_base64(file).await {
                    Ok((_, data)) => bridge::add_photo(id, data).await,
                    Err(_) => Err(NOT_AN_IMAGE.to_string()),
                };
                if busy.try_get_value().is_none() {
                    return;
                }
                match answer {
                    Ok(saved) => replace_entry(entries, saved),
                    Err(e) => {
                        refused.try_set_value(Some(fixed_error(&e, PHOTO_FAILED)));
                    }
                }
            }
            busy.try_set_value(false);
            let last = refused.try_update_value(Option::take).flatten();
            status.try_set(last.map_or(Status::Quiet, Status::Failed));
        });
    };

    let add = move |files: Vec<web_sys::File>| {
        if files.is_empty() {
            return;
        }
        if busy.get_value() {
            // Joins the batch in flight; its refusals show when it ends.
        } else {
            refused.set_value(None);
        }
        for file in files {
            match refusal(file.size()) {
                Some(r) => refused.set_value(Some(r)),
                None => queue.update_value(|q| q.push_back(file)),
            }
        }
        if queue.with_value(VecDeque::is_empty) && !busy.get_value() {
            let last = refused.try_update_value(Option::take).flatten();
            status.set(last.map_or(Status::Quiet, Status::Failed));
            return;
        }
        drain();
    };
    let files_of = |list: Option<web_sys::FileList>| -> Vec<web_sys::File> {
        list.map(|l| (0..l.length()).filter_map(|i| l.get(i)).collect())
            .unwrap_or_default()
    };
    let on_change = move |ev: web_sys::Event| {
        let el: web_sys::HtmlInputElement = event_target(&ev);
        let files = files_of(el.files());
        // The same file can be picked again.
        el.set_value("");
        add(files);
    };
    let on_drop = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        over.set(0);
        add(files_of(ev.data_transfer().and_then(|d| d.files())));
    };
    let pick = move |_| {
        if let Some(el) = input.get_untracked() {
            el.click();
        }
    };
    let photo_ids = Memo::new(move |_| {
        entry.with(|e| {
            e.as_ref()
                .map(|e| e.photos.iter().map(|p| p.id).collect::<Vec<_>>())
                .unwrap_or_default()
        })
    });

    view! {
        <section class="jr-section jr-photos" aria-labelledby=labelled_by
            class:jr-drop-over=move || dragging_over(over.get())
            on:dragenter=move |ev: web_sys::DragEvent| {
                ev.prevent_default();
                over.update(|n| *n += 1);
            }
            on:dragover=move |ev: web_sys::DragEvent| {
                ev.prevent_default();
            }
            on:dragleave=move |_| {
                over.update(|n| *n = n.saturating_sub(1));
            }
            on:drop=on_drop>
            <h2 class="nd-label" id=heading>"Photos"</h2>
            <div class="jr-photo-strip">
                <For
                    each=move || photo_ids.get()
                    key=|photo_id| *photo_id
                    children=move |photo_id| view! {
                        <PhotoThumb id=id photo_id=photo_id entries=entries status=status />
                    }
                />
            </div>
            <div class="jr-photo-actions">
                <button type="button" class="btn btn-secondary jr-photo-add" on:click=pick>
                    "Add photo"
                </button>
                <input type="file" class="jr-photo-input" node_ref=input hidden tabindex="-1"
                    aria-hidden="true" multiple accept="image/jpeg,image/png,image/webp"
                    on:change=on_change />
                <span class="jr-photo-hint">"or drop one here"</span>
            </div>
            <StatusLine status=status />
        </section>
    }
}

/// One photo. A file the backend can't load, or the browser can't decode,
/// reads "Photo unavailable".
#[component]
fn PhotoThumb(id: i64, photo_id: i64, entries: Entries, status: RwSignal<Status>) -> impl IntoView {
    // `None` while loading.
    let src = RwSignal::new(None::<Result<String, ()>>);
    let broken = RwSignal::new(false);
    let removing = RwSignal::new(false);
    spawn_local(async move {
        let url = bridge::image(id, Some(photo_id)).await.map_err(|_| ());
        src.try_set(Some(url));
    });
    let remove = move |_| {
        if removing.get_untracked() {
            return;
        }
        removing.set(true);
        status.set(Status::Quiet);
        spawn_local(async move {
            let answer = bridge::remove_photo(id, photo_id).await;
            // Gone with the detail, or with a reload that dropped the photo.
            if removing.try_get_untracked().is_none() {
                return;
            }
            match answer {
                Ok(saved) => replace_entry(entries, saved),
                Err(e) => {
                    status.try_set(Status::Failed(fixed_error(&e, SAVE_FAILED)));
                    removing.try_set(false);
                }
            }
        });
    };
    view! {
        <figure class="jr-photo">
            {move || match (src.get(), broken.get()) {
                (Some(Ok(url)), false) => view! {
                    <img src=url alt="Photo of the print" on:error=move |_| broken.set(true) />
                }
                .into_any(),
                (None, _) => view! { <span class="jr-photo-missing" aria-hidden="true"></span> }.into_any(),
                _ => view! { <span class="jr-photo-missing">{PHOTO_UNAVAILABLE}</span> }.into_any(),
            }}
            <button type="button" class="btn btn-ghost jr-photo-remove" aria-label="Remove photo"
                disabled=move || removing.get() on:click=remove>
                "Remove"
            </button>
        </figure>
    }
}

/// "Analyze a photo" (while AI analysis is on) and "Delete entry", which
/// asks inline first. After a delete the detail closes and focus moves to
/// the next row.
#[component]
fn Actions(
    id: i64,
    entries: Entries,
    open: RwSignal<Option<i64>>,
    analysis: Signal<bool>,
) -> impl IntoView {
    let confirming = RwSignal::new(false);
    let deleting = RwSignal::new(false);
    let status = RwSignal::new(Status::Quiet);
    let delete_button = NodeRef::<leptos::html::Button>::new();
    let keep_button = NodeRef::<leptos::html::Button>::new();
    let question = format!("jr-confirm-{id}");

    // The button that had focus goes away with the swap, so focus follows.
    let ask = move |_| {
        status.set(Status::Quiet);
        confirming.set(true);
        request_animation_frame(move || {
            if let Some(Some(b)) = keep_button.try_get_untracked() {
                let _ = b.focus();
            }
        });
    };
    let keep = move |_| {
        confirming.set(false);
        request_animation_frame(move || {
            if let Some(Some(b)) = delete_button.try_get_untracked() {
                let _ = b.focus();
            }
        });
    };
    let delete = move |_| {
        if deleting.get_untracked() {
            return;
        }
        deleting.set(true);
        status.set(Status::Busy(DELETING));
        let next = entries.with_untracked(|r| match r {
            Some(Ok(list)) => neighbour(&row_ids(list), id),
            _ => None,
        });
        spawn_local(async move {
            match bridge::delete(id).await {
                Ok(()) => {
                    remove_entry(entries, id);
                    open.try_update(|o| {
                        if *o == Some(id) {
                            *o = None;
                        }
                    });
                    focus_after_delete(next);
                }
                Err(e) => {
                    status.try_set(Status::Failed(fixed_error(&e, SAVE_FAILED)));
                    deleting.try_set(false);
                    confirming.try_set(false);
                }
            }
        });
    };

    view! {
        <div class="jr-section">
            <div class="jr-actions">
                <Show when=move || analysis.get()>
                    <a class="btn btn-secondary jr-analyze" href=analyze_href(id)>"Analyze a photo"</a>
                </Show>
                <Show
                    when=move || confirming.get()
                    fallback=move || view! {
                        <button type="button" class="btn btn-ghost jr-delete" node_ref=delete_button on:click=ask>
                            "Delete entry"
                        </button>
                    }
                >
                    <div class="jr-confirm" role="group" aria-labelledby=question.clone()>
                        <span id=question.clone()>{DELETE_CONFIRM}</span>
                        <button type="button" class="btn btn-danger jr-confirm-delete"
                            disabled=move || deleting.get() on:click=delete>
                            "Delete"
                        </button>
                        <button type="button" class="btn btn-secondary jr-confirm-keep"
                            node_ref=keep_button disabled=move || deleting.get() on:click=keep>
                            "Keep"
                        </button>
                    </div>
                </Show>
            </div>
            <StatusLine status=status />
        </div>
    }
}

/// Focus went with the deleted row: give it to the next row, else the
/// search field. Focus the user has moved elsewhere meanwhile stays put.
fn focus_after_delete(next: Option<i64>) {
    use wasm_bindgen::JsCast;
    request_animation_frame(move || {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let active = doc.active_element();
        let body: Option<web_sys::Element> = doc.body().map(Into::into);
        if active.is_some() && active != body {
            return;
        }
        let selector = match next {
            Some(n) => format!(".journal-page .jr-row[data-entry=\"{n}\"] .jr-row-main"),
            None => ".journal-page .jr-search".to_string(),
        };
        if let Some(el) = doc
            .query_selector(&selector)
            .ok()
            .flatten()
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = el.focus();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn rating(verdict: Option<Verdict>, t: &[&str]) -> Rating {
        Rating {
            verdict,
            tags: tags(t),
        }
    }

    #[test]
    fn the_backend_sentences_are_pinned_and_anything_else_is_the_fallback() {
        assert_eq!(SAVE_FAILED, "The print journal couldn't be saved.");
        assert_eq!(READ_FAILED, "The print journal couldn't be read.");
        assert_eq!(PHOTO_FAILED, "The photo couldn't be saved.");
        assert_eq!(GONE, "That print isn't in the journal any more.");
        assert_eq!(fixed_error(GONE, SAVE_FAILED), GONE);
        assert_eq!(fixed_error(TOO_LARGE, PHOTO_FAILED), TOO_LARGE);
        assert_eq!(fixed_error(NOT_AN_IMAGE, PHOTO_FAILED), NOT_AN_IMAGE);
        assert_eq!(
            fixed_error("invalid type: null, expected a string", SAVE_FAILED),
            SAVE_FAILED
        );
        assert_eq!(
            fixed_error("disk full: /secret", PHOTO_FAILED),
            PHOTO_FAILED
        );
    }

    #[test]
    fn status_lines_read_like_the_other_state_lines() {
        assert_eq!(status_text(Status::Quiet), "");
        assert_eq!(status_kind(Status::Quiet), "");
        assert_eq!(status_text(Status::Saved), "[SAVED]");
        assert_eq!(
            status_text(Status::Failed(SAVE_FAILED)),
            "[ERROR: The print journal couldn't be saved.]"
        );
        assert_eq!(status_kind(Status::Failed(SAVE_FAILED)), "error");
        assert_eq!(
            status_text(Status::Busy(ADDING)),
            "[LOADING: Adding the photo]"
        );
    }

    #[test]
    fn a_photo_over_25_mb_is_refused_before_it_is_read() {
        assert_eq!(refusal(MAX_PHOTO_BYTES), None);
        assert_eq!(refusal(MAX_PHOTO_BYTES + 1.0), Some(TOO_LARGE));
        assert_eq!(refusal(0.0), None);
    }

    #[test]
    fn after_a_delete_focus_goes_to_the_next_row_else_the_one_before() {
        assert_eq!(neighbour(&[2, 1, 4, 5], 1), Some(4));
        assert_eq!(neighbour(&[2, 1, 4, 5], 5), Some(4));
        assert_eq!(neighbour(&[2], 2), None);
        assert_eq!(neighbour(&[2, 1], 9), None);
    }

    #[test]
    fn a_click_changes_one_part_of_the_rating_and_saves_only_that() {
        let r = rating(Some(Verdict::Failed), &["warping"]);
        let (next, update) = r.click_verdict(Verdict::Usable);
        assert_eq!(next, rating(Some(Verdict::Usable), &["warping"]));
        assert_eq!(
            update,
            EntryUpdate {
                verdict: Some(VerdictChoice::Usable),
                ..Default::default()
            }
        );
        let (cleared, update) = next.click_verdict(Verdict::Usable);
        assert_eq!(cleared.verdict, None);
        assert_eq!(update.verdict, Some(VerdictChoice::Clear));
        let (next, update) = r.click_tag("stringing");
        assert_eq!(
            next,
            rating(Some(Verdict::Failed), &["warping", "stringing"])
        );
        assert_eq!(
            update,
            EntryUpdate {
                tags: Some(tags(&["warping", "stringing"])),
                ..Default::default()
            }
        );
        assert_eq!(r.click_tag("warping").0.tags, Vec::<String>::new());
    }

    #[test]
    fn a_saved_click_shows_what_the_backend_stored() {
        let mut s = SaveSync::new(rating(None, &[]));
        let id = s.issue();
        let r = s.settle(id, Ok(rating(Some(Verdict::Good), &[])));
        assert_eq!(r.show, Some(rating(Some(Verdict::Good), &[])));
        assert_eq!(r.status, Some(Status::Saved));
        assert!(r.adopted);
        assert!(s.idle());
    }

    #[test]
    fn a_failed_click_puts_the_last_confirmed_rating_back() {
        let mut s = SaveSync::new(rating(Some(Verdict::Failed), &["warping"]));
        let id = s.issue();
        let r = s.settle(id, Err(SAVE_FAILED));
        assert_eq!(r.show, Some(rating(Some(Verdict::Failed), &["warping"])));
        assert_eq!(r.status, Some(Status::Failed(SAVE_FAILED)));
        assert!(!r.adopted);
    }

    #[test]
    fn rapid_clicks_settle_on_the_newest_success_whatever_the_order() {
        let mut s = SaveSync::new(rating(None, &[]));
        let a = s.issue();
        let b = s.issue();
        let r = s.settle(b, Ok(rating(None, &["warping", "stringing"])));
        assert_eq!(r.show, Some(rating(None, &["warping", "stringing"])));
        assert!(r.adopted);
        assert!(!s.idle());
        // The older answer arrives late: it changes nothing shown or stored.
        let r = s.settle(a, Ok(rating(None, &["warping"])));
        assert_eq!(r.show, Some(rating(None, &["warping", "stringing"])));
        assert_eq!(r.status, Some(Status::Saved));
        assert!(!r.adopted);
        assert!(s.idle());
    }

    #[test]
    fn a_success_followed_by_a_failed_newest_click_is_kept() {
        let mut s = SaveSync::new(rating(None, &[]));
        let a = s.issue();
        let b = s.issue();
        let r = s.settle(a, Ok(rating(Some(Verdict::Good), &[])));
        assert_eq!((r.show, r.status, r.adopted), (None, None, true));
        let r = s.settle(b, Err(SAVE_FAILED));
        assert_eq!(r.show, Some(rating(Some(Verdict::Good), &[])));
        assert_eq!(r.status, Some(Status::Failed(SAVE_FAILED)));
    }

    #[test]
    fn a_newest_failure_then_a_late_success_moves_to_that_success() {
        let mut s = SaveSync::new(rating(None, &[]));
        let a = s.issue();
        let b = s.issue();
        let r = s.settle(b, Err(SAVE_FAILED));
        assert_eq!(r.show, Some(rating(None, &[])));
        let r = s.settle(a, Ok(rating(None, &["z_banding"])));
        assert_eq!(r.show, Some(rating(None, &["z_banding"])));
        assert_eq!(r.status, Some(Status::Failed(SAVE_FAILED)));
        assert!(r.adopted);
    }

    #[test]
    fn notes_save_only_when_they_differ_from_the_journal() {
        let mut n = NotesSync::new("Lifted.".into());
        assert_eq!(n.blur("Lifted."), None);
        let g = n.blur("Lifted. Try a brim.").unwrap();
        // Focus and blur again before the answer: nothing new to save.
        assert_eq!(n.blur("Lifted. Try a brim."), None);
        assert_eq!(n.answer(g, Ok(())), Some(Status::Saved));
        assert_eq!(n.blur("Lifted. Try a brim."), None);
    }

    #[test]
    fn a_failed_note_says_so_and_the_next_blur_tries_again() {
        let mut n = NotesSync::new(String::new());
        let g = n.blur("x").unwrap();
        assert_eq!(
            n.answer(g, Err(SAVE_FAILED)),
            Some(Status::Failed(SAVE_FAILED))
        );
        assert!(n.blur("x").is_some());
    }

    #[test]
    fn only_the_newest_note_save_sets_the_status() {
        let mut n = NotesSync::new(String::new());
        let first = n.blur("a").unwrap();
        let second = n.blur("ab").unwrap();
        assert_eq!(n.answer(first, Err(SAVE_FAILED)), None);
        assert_eq!(n.answer(second, Ok(())), Some(Status::Saved));
        assert_eq!(n.blur("ab"), None);
    }
}
