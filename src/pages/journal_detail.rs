//! One journal entry, expanded in place under its row (spec 2026-10-24 §2):
//! the facts, the verdict and issue tags, notes saved on blur, photos, and
//! Analyze / Delete. There is no modal and no new page.
//!
//! Every write after an await goes through `try_*`. The verdict, tags and
//! notes keep their bookkeeping in a `StoredValue`; an answer that lands once
//! that is gone (the detail closed, or another entry opened) changes nothing,
//! and the list catches up through `journal://changed`. Photo uploads are
//! the exception: they finish even if the detail closes (see [`Uploads`]).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::app::FeatureFlagsContext;
use crate::components::defect_report::defect_display_name;
use crate::journal::format::{
    analyze_href, fact_rows, local_time_text, toggle_tag, verdict_after_click, verdict_key,
    verdict_text, DELETE_CONFIRM, ISSUE_TAGS, MAX_PHOTO_BYTES, NOT_AN_IMAGE, PHOTO_UNAVAILABLE,
    TOO_LARGE, VERDICTS,
};
use crate::journal::types::{EntryUpdate, PrintEntry, Verdict, VerdictChoice};
use crate::journal::{bridge, next_revision};
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

/// What takes focus after `id` goes from `ids` (a row, or a photo): the
/// next one, else the one before it.
fn neighbour(ids: &[i64], id: i64) -> Option<i64> {
    let at = ids.iter().position(|i| *i == id)?;
    ids.get(at + 1)
        .or_else(|| at.checked_sub(1).and_then(|p| ids.get(p)))
        .copied()
}

/// `Photo 2 of 3, added 21 Oct 2026`: each photo named apart from the rest.
fn photo_alt(n: usize, total: usize, added_at: i64, offset_minutes: i32) -> String {
    let at = local_time_text(added_at, offset_minutes);
    let day = at.rsplit_once(' ').map_or(at.as_str(), |(day, _)| day);
    format!("Photo {n} of {total}, added {day}")
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

    /// The rating after verdict `v` is clicked: the pressed one clears.
    fn click_verdict(&self, v: Verdict) -> Self {
        Self {
            verdict: chosen(verdict_after_click(self.verdict, v)),
            tags: self.tags.clone(),
        }
    }

    /// The rating after tag `tag` is clicked.
    fn click_tag(&self, tag: &str) -> Self {
        Self {
            verdict: self.verdict,
            tags: toggle_tag(&self.tags, tag),
        }
    }
}

/// The update that takes `stored` to `wanted`: only the fields that differ.
fn rating_update(stored: &Rating, wanted: &Rating) -> Option<EntryUpdate> {
    let verdict = (stored.verdict != wanted.verdict).then(|| {
        wanted
            .verdict
            .map_or(VerdictChoice::Clear, VerdictChoice::from)
    });
    let tags = (stored.tags != wanted.tags).then(|| wanted.tags.clone());
    (verdict.is_some() || tags.is_some()).then_some(EntryUpdate {
        verdict,
        tags,
        notes: None,
    })
}

/// Saves the verdict and tags one request at a time, so the backend applies
/// them in click order. A click while a save is out only changes what is
/// wanted; when the save answers, the newest wanted rating goes out if it
/// differs from what was stored. A failure puts back what was last stored
/// and drops what was waiting. Once idle, what is shown is what is stored.
#[derive(Debug)]
struct RatingSync {
    /// What the backend last said it stored.
    confirmed: Rating,
    /// What the user last chose; shown on the pills.
    wanted: Rating,
    in_flight: bool,
    /// Clicked since the save in flight went out.
    clicked: bool,
}

/// What the pills do after an answer.
#[derive(Debug, PartialEq)]
struct Answered {
    show: Rating,
    status: Option<Status>,
    /// The next save to send, for clicks made while this one was out.
    send: Option<EntryUpdate>,
}

impl RatingSync {
    fn new(stored: Rating) -> Self {
        Self {
            confirmed: stored.clone(),
            wanted: stored,
            in_flight: false,
            clicked: false,
        }
    }

    /// The list changed: while nothing is out, show what it holds.
    fn loaded(&mut self, stored: Rating) -> Option<Rating> {
        if self.in_flight {
            return None;
        }
        self.confirmed = stored.clone();
        self.wanted = stored.clone();
        Some(stored)
    }

    /// A click chose `next`; returns the save to send now, if one may go.
    fn click(&mut self, next: Rating) -> Option<EntryUpdate> {
        self.wanted = next;
        if self.in_flight {
            self.clicked = true;
            return None;
        }
        self.send()
    }

    fn send(&mut self) -> Option<EntryUpdate> {
        let update = rating_update(&self.confirmed, &self.wanted)?;
        self.in_flight = true;
        self.clicked = false;
        Some(update)
    }

    /// The answer to the save in flight.
    fn answer(&mut self, result: Result<Rating, &'static str>) -> Answered {
        self.in_flight = false;
        match result {
            Ok(stored) => {
                self.confirmed = stored;
                let send = if self.clicked {
                    self.send()
                } else {
                    self.wanted = self.confirmed.clone();
                    None
                };
                Answered {
                    show: self.wanted.clone(),
                    status: send.is_none().then_some(Status::Saved),
                    send,
                }
            }
            Err(e) => {
                self.wanted = self.confirmed.clone();
                self.clicked = false;
                Answered {
                    show: self.wanted.clone(),
                    status: Some(Status::Failed(e)),
                    send: None,
                }
            }
        }
    }
}

/// Notes save on blur, and only when the text differs from what the journal
/// holds. Each save takes the next generation; only the newest one's answer
/// sets the status or the list. While the box isn't focused, has no unsaved
/// edit and no save is out, it follows the journal's notes.
#[derive(Debug)]
struct NotesSync {
    /// The text the journal holds, as far as the page knows: the last
    /// loaded or sent. `None` after a failed save, so the next blur tries
    /// again and the box keeps the user's text.
    saved: Option<String>,
    generation: u64,
    in_flight: bool,
}

impl NotesSync {
    fn new(stored: String) -> Self {
        Self {
            saved: Some(stored),
            generation: 0,
            in_flight: false,
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
        self.in_flight = true;
        Some(self.generation)
    }

    /// The answer to save `generation`; `None` when a newer save has gone
    /// out since, which owns the status.
    fn answer(&mut self, generation: u64, result: Result<(), &'static str>) -> Option<Status> {
        if generation != self.generation {
            return None;
        }
        self.in_flight = false;
        Some(match result {
            Ok(()) => Status::Saved,
            Err(e) => {
                self.saved = None;
                Status::Failed(e)
            }
        })
    }

    /// The journal's notes are now `stored`. Returns the text to put in the
    /// box, unless the user is in it, has an unsaved edit, or a save is out.
    fn stored_changed(&mut self, stored: &str, draft: &str, focused: bool) -> Option<String> {
        let edited = self.saved.as_deref() != Some(draft);
        if focused || edited || self.in_flight || self.saved.as_deref() == Some(stored) {
            return None;
        }
        self.saved = Some(stored.to_string());
        Some(stored.to_string())
    }
}

/// The photo upload queue: one upload at a time across the app, in the
/// order given. `P` is the file and where its answer goes; tests use text.
#[derive(Debug)]
struct Uploads<P> {
    queue: VecDeque<(i64, P)>,
    /// The entry whose photo is going up now.
    current: Option<i64>,
    /// The last refusal per entry, shown when its uploads end.
    refused: HashMap<i64, &'static str>,
}

/// What the strip does after files are given.
#[derive(Debug, PartialEq, Eq)]
struct Queued {
    /// Nothing was going up: start the queue.
    start: bool,
    /// Nothing of this entry's is waiting: the status to show now.
    settled: Option<Status>,
}

impl<P> Default for Uploads<P> {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            current: None,
            refused: HashMap::new(),
        }
    }
}

impl<P> Uploads<P> {
    fn pending(&self, id: i64) -> bool {
        self.current == Some(id) || self.queue.iter().any(|(i, _)| *i == id)
    }

    fn settled(&mut self, id: i64) -> Status {
        self.refused
            .remove(&id)
            .map_or(Status::Quiet, Status::Failed)
    }

    /// Files for entry `id`, with their sizes. A file over 25 MB is refused
    /// here, before it is read.
    fn add(&mut self, id: i64, files: Vec<(P, f64)>) -> Queued {
        if !self.pending(id) {
            self.refused.remove(&id);
        }
        for (file, size) in files {
            match refusal(size) {
                Some(r) => {
                    self.refused.insert(id, r);
                }
                None => self.queue.push_back((id, file)),
            }
        }
        Queued {
            start: self.current.is_none() && !self.queue.is_empty(),
            settled: (!self.pending(id)).then(|| self.settled(id)),
        }
    }

    /// The next upload; `None` when the queue is done.
    fn next(&mut self) -> Option<(i64, P)> {
        let next = self.queue.pop_front();
        self.current = next.as_ref().map(|(id, _)| *id);
        next
    }

    /// The upload of entry `id` answered. Returns its status once nothing
    /// more of that entry's is waiting.
    fn done(&mut self, id: i64, result: Result<(), &'static str>) -> Option<Status> {
        if let Err(e) = result {
            self.refused.insert(id, e);
        }
        self.current = None;
        (!self.queue.iter().any(|(i, _)| *i == id)).then(|| self.settled(id))
    }
}

/// The app's photo uploads, and the status line of each entry's open strip.
/// Kept outside the detail, so photos queued before it closes still go up,
/// one at a time; their answers go into the list, and refusals show if the
/// entry is open again by then.
#[derive(Default)]
struct PhotoUploads {
    uploads: Uploads<(web_sys::File, Entries)>,
    shown: HashMap<i64, RwSignal<Status>>,
}

thread_local! {
    static UPLOADS: RefCell<PhotoUploads> = RefCell::new(PhotoUploads::default());
}

fn show_upload(id: i64, status: Status) {
    if let Some(line) = UPLOADS.with_borrow(|u| u.shown.get(&id).copied()) {
        line.try_set(status);
    }
}

fn drain_uploads() {
    spawn_local(async move {
        while let Some((id, (file, entries))) = UPLOADS.with_borrow_mut(|u| u.uploads.next()) {
            show_upload(id, Status::Busy(ADDING));
            let revision = next_revision();
            let answer = match read_file_as_base64(file).await {
                Ok((_, data)) => bridge::add_photo(id, data).await,
                Err(_) => Err(NOT_AN_IMAGE.to_string()),
            };
            let result = match answer {
                Ok(saved) => {
                    replace_entry(entries, saved, revision);
                    Ok(())
                }
                Err(e) => Err(fixed_error(&e, PHOTO_FAILED)),
            };
            if let Some(s) = UPLOADS.with_borrow_mut(|u| u.uploads.done(id, result)) {
                show_upload(id, s);
            }
        }
    });
}

/// Moves focus to `selector` if the element that had it went away (focus
/// fell back to the page). Focus the user moved elsewhere stays put.
fn focus_if_lost(selector: String) {
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
            <Photos id=id entry=entry entries=entries offset=offset />
            <Actions id=id entries=entries open=open
                analysis=Signal::derive(move || flags.get().analysis_enabled) />
        </div>
    }
}

/// What a rating save needs once its answer is in.
#[derive(Clone, Copy)]
struct RatingSaves {
    id: i64,
    entries: Entries,
    sync: StoredValue<RatingSync>,
    shown: RwSignal<Rating>,
    status: RwSignal<Status>,
}

impl RatingSaves {
    fn send(self, update: EntryUpdate) {
        spawn_local(async move {
            let revision = next_revision();
            let answer = bridge::update(self.id, &update)
                .await
                .map_err(|e| fixed_error(&e, SAVE_FAILED));
            let rating = answer.as_ref().map(Rating::of).map_err(|e| *e);
            let Some(answered) = self.sync.try_update_value(|s| s.answer(rating)) else {
                return;
            };
            if let Ok(saved) = answer {
                replace_entry(self.entries, saved, revision);
            }
            self.shown.try_set(answered.show);
            if let Some(s) = answered.status {
                self.status.try_set(s);
            }
            if let Some(next) = answered.send {
                self.send(next);
            }
        });
    }
}

/// The verdict pills and the issue-tag chips. Each click shows at once and
/// is saved, one save at a time (see [`RatingSync`]).
#[component]
fn RatingControls(id: i64, entry: Memo<Option<PrintEntry>>, entries: Entries) -> impl IntoView {
    let stored = Memo::new(move |_| entry.with(|e| e.as_ref().map(Rating::of).unwrap_or_default()));
    let saves = RatingSaves {
        id,
        entries,
        sync: StoredValue::new(RatingSync::new(stored.get_untracked())),
        shown: RwSignal::new(stored.get_untracked()),
        status: RwSignal::new(Status::Quiet),
    };
    let shown = saves.shown;

    // The list changed (a reload, or a save's answer): follow it while idle.
    Effect::new(move |_| {
        let now = stored.get();
        if let Some(Some(show)) = saves.sync.try_update_value(|s| s.loaded(now)) {
            shown.try_set(show);
        }
    });

    let click = move |next: Rating| {
        shown.set(next.clone());
        saves.status.set(Status::Quiet);
        if let Some(Some(update)) = saves.sync.try_update_value(|s| s.click(next)) {
            saves.send(update);
        }
    };

    let verdicts = VERDICTS
        .iter()
        .map(|&v| {
            view! {
                <button type="button" class="nd-tab jr-verdict" data-verdict=verdict_key(v)
                    aria-pressed=move || shown.with(|r| r.verdict == Some(v)).to_string()
                    on:click=move |_| click(shown.with_untracked(|r| r.click_verdict(v)))>
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
                    on:click=move |_| click(shown.with_untracked(|r| r.click_tag(tag)))>
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
            <StatusLine status=saves.status />
        </section>
    }
}

/// The notes box. It saves when it loses focus, and only if the text changed;
/// see [`NotesSync`] for when it follows the journal.
#[component]
fn Notes(id: i64, entry: Memo<Option<PrintEntry>>, entries: Entries) -> impl IntoView {
    let stored =
        Memo::new(move |_| entry.with(|e| e.as_ref().map(|e| e.notes.clone()).unwrap_or_default()));
    let initial = stored.get_untracked();
    // What the user typed, and what the box is set to (on mount and when it
    // follows the journal).
    let draft = RwSignal::new(initial.clone());
    let in_box = RwSignal::new(initial.clone());
    let focused = RwSignal::new(false);
    let status = RwSignal::new(Status::Quiet);
    let sync = StoredValue::new(NotesSync::new(initial));
    let box_id = format!("jr-notes-{id}");

    Effect::new(move |_| {
        let now = stored.get();
        let text = draft.get_untracked();
        let follow = focused.get_untracked();
        if let Some(Some(next)) = sync.try_update_value(|s| s.stored_changed(&now, &text, follow)) {
            draft.set(next.clone());
            in_box.set(next);
        }
    });

    let on_blur = move |_| {
        focused.set(false);
        let text = draft.get_untracked();
        let Some(generation) = sync.try_update_value(|s| s.blur(&text)).flatten() else {
            return;
        };
        status.set(Status::Quiet);
        spawn_local(async move {
            let revision = next_revision();
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
                replace_entry(entries, saved, revision);
            }
            status.try_set(s);
        });
    };

    view! {
        <div class="jr-section jr-notes-field">
            <label class="nd-label" for=box_id.clone()>"Notes"</label>
            <textarea class="jr-notes" id=box_id rows="3" maxlength=NOTES_MAX
                prop:value=move || in_box.get()
                on:focus=move |_| focused.set(true)
                on:input=move |ev| draft.set(event_target_value(&ev))
                on:blur=on_blur></textarea>
            <StatusLine status=status />
        </div>
    }
}

/// The photo strip: add with the picker or by dropping files on it, and
/// remove each one. Files go up through the app's queue ([`PhotoUploads`]),
/// one at a time, and finish even if the detail closes meanwhile.
#[component]
fn Photos(
    id: i64,
    entry: Memo<Option<PrintEntry>>,
    entries: Entries,
    offset: i32,
) -> impl IntoView {
    let status = RwSignal::new(Status::Quiet);
    // Drags over the strip, counted so that crossing a child doesn't clear it.
    let over = RwSignal::new(0u32);
    let input = NodeRef::<leptos::html::Input>::new();
    let heading = format!("jr-photos-label-{id}");
    let labelled_by = heading.clone();

    // This strip shows the entry's upload status, including uploads queued
    // before it opened.
    let busy = UPLOADS.with_borrow_mut(|u| {
        u.shown.insert(id, status);
        u.uploads.pending(id)
    });
    if busy {
        status.set(Status::Busy(ADDING));
    }
    on_cleanup(move || {
        UPLOADS.with_borrow_mut(|u| {
            if u.shown.get(&id) == Some(&status) {
                u.shown.remove(&id);
            }
        });
    });

    let add = move |files: Vec<web_sys::File>| {
        if files.is_empty() {
            return;
        }
        let files = files
            .into_iter()
            .map(|f| {
                let size = f.size();
                ((f, entries), size)
            })
            .collect();
        let queued = UPLOADS.with_borrow_mut(|u| u.uploads.add(id, files));
        status.set(queued.settled.unwrap_or(Status::Busy(ADDING)));
        if queued.start {
            drain_uploads();
        }
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
    let photos = Memo::new(move |_| {
        entry.with(|e| {
            e.as_ref()
                .map(|e| {
                    e.photos
                        .iter()
                        .map(|p| (p.id, p.added_at))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    });
    let photo_ids =
        Memo::new(move |_| photos.with(|p| p.iter().map(|(id, _)| *id).collect::<Vec<_>>()));

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
                    each=move || photos.get()
                    key=|(photo_id, _)| *photo_id
                    children=move |(photo_id, added_at)| view! {
                        <PhotoThumb id=id photo_id=photo_id added_at=added_at photos=photo_ids
                            offset=offset entries=entries status=status />
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
/// reads "Photo unavailable". After a removal, focus moves to the next
/// photo's Remove, else "Add photo".
#[component]
fn PhotoThumb(
    id: i64,
    photo_id: i64,
    added_at: i64,
    photos: Memo<Vec<i64>>,
    offset: i32,
    entries: Entries,
    status: RwSignal<Status>,
) -> impl IntoView {
    // `None` while loading.
    let src = RwSignal::new(None::<Result<String, ()>>);
    let broken = RwSignal::new(false);
    let removing = RwSignal::new(false);
    let place = Memo::new(move |_| {
        photos.with(|p| {
            let n = p.iter().position(|x| *x == photo_id).map_or(0, |i| i + 1);
            (n, p.len())
        })
    });
    let alt = move || {
        let (n, total) = place.get();
        photo_alt(n, total, added_at, offset)
    };
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
        let next = neighbour(&photos.get_untracked(), photo_id);
        spawn_local(async move {
            let revision = next_revision();
            let answer = bridge::remove_photo(id, photo_id).await;
            // Gone with the detail, or with a reload that dropped the photo.
            if removing.try_get_untracked().is_none() {
                return;
            }
            match answer {
                Ok(saved) => {
                    replace_entry(entries, saved, revision);
                    focus_if_lost(match next {
                        Some(n) => format!(
                            "#jr-detail-{id} .jr-photo[data-photo=\"{n}\"] .jr-photo-remove"
                        ),
                        None => format!("#jr-detail-{id} .jr-photo-add"),
                    });
                }
                Err(e) => {
                    status.try_set(Status::Failed(fixed_error(&e, SAVE_FAILED)));
                    removing.try_set(false);
                }
            }
        });
    };
    view! {
        <figure class="jr-photo" data-photo=photo_id.to_string()>
            {move || match (src.get(), broken.get()) {
                (Some(Ok(url)), false) => view! {
                    <img src=url alt=alt on:error=move |_| broken.set(true) />
                }
                .into_any(),
                (None, _) => view! { <span class="jr-photo-missing" aria-hidden="true"></span> }.into_any(),
                _ => view! {
                    <span class="jr-photo-missing" role="img" aria-label=alt>{PHOTO_UNAVAILABLE}</span>
                }
                .into_any(),
            }}
            <button type="button" class="btn btn-ghost jr-photo-remove"
                aria-label=move || format!("Remove photo {}", place.get().0)
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

    // The focused button goes away with each swap, so focus follows it.
    let focus_delete = move || {
        request_animation_frame(move || {
            if let Some(Some(b)) = delete_button.try_get_untracked() {
                let _ = b.focus();
            }
        });
    };
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
        focus_delete();
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
                    focus_if_lost(match next {
                        Some(n) => {
                            format!(".journal-page .jr-row[data-entry=\"{n}\"] .jr-row-main")
                        }
                        None => ".journal-page .jr-search".to_string(),
                    });
                }
                Err(e) => {
                    status.try_set(Status::Failed(fixed_error(&e, SAVE_FAILED)));
                    deleting.try_set(false);
                    confirming.try_set(false);
                    focus_delete();
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

    fn verdict(v: VerdictChoice) -> EntryUpdate {
        EntryUpdate {
            verdict: Some(v),
            ..Default::default()
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
    fn each_photo_is_named_by_its_place_and_day() {
        // 2026-10-21 09:00 UTC.
        assert_eq!(
            photo_alt(2, 3, 1_792_573_200, 0),
            "Photo 2 of 3, added 21 Oct 2026"
        );
        assert_eq!(
            photo_alt(1, 1, 1_792_573_200, -600),
            "Photo 1 of 1, added 20 Oct 2026"
        );
    }

    #[test]
    fn a_click_changes_one_part_of_the_rating_and_saves_only_that() {
        let r = rating(Some(Verdict::Failed), &["warping"]);
        let next = r.click_verdict(Verdict::Usable);
        assert_eq!(next, rating(Some(Verdict::Usable), &["warping"]));
        assert_eq!(
            rating_update(&r, &next),
            Some(verdict(VerdictChoice::Usable))
        );
        let cleared = next.click_verdict(Verdict::Usable);
        assert_eq!(
            rating_update(&next, &cleared),
            Some(verdict(VerdictChoice::Clear))
        );
        let tagged = r.click_tag("stringing");
        assert_eq!(
            rating_update(&r, &tagged),
            Some(EntryUpdate {
                tags: Some(tags(&["warping", "stringing"])),
                ..Default::default()
            })
        );
        assert_eq!(r.click_tag("warping").tags, Vec::<String>::new());
        assert_eq!(rating_update(&r, &r), None);
    }

    #[test]
    fn one_rating_save_is_out_at_a_time_and_the_last_click_wins() {
        let mut s = RatingSync::new(rating(Some(Verdict::Usable), &[]));
        // Good goes out; Failed waits for it.
        let good = rating(Some(Verdict::Good), &[]);
        let failed = rating(Some(Verdict::Failed), &[]);
        assert_eq!(s.click(good.clone()), Some(verdict(VerdictChoice::Good)));
        assert_eq!(s.click(failed.clone()), None);
        assert_eq!(
            s.loaded(rating(None, &[])),
            None,
            "a reload mid-save is ignored"
        );
        // Good's answer arrives late: Failed goes out next, still shown.
        let a = s.answer(Ok(good));
        assert_eq!(a.show, failed);
        assert_eq!(a.send, Some(verdict(VerdictChoice::Failed)));
        assert_eq!(a.status, None);
        let a = s.answer(Ok(failed.clone()));
        assert_eq!(
            (a.show, a.status, a.send),
            (failed, Some(Status::Saved), None)
        );
    }

    #[test]
    fn rapid_clicks_coalesce_into_one_save_of_the_newest_rating() {
        let mut s = RatingSync::new(rating(None, &["warping"]));
        let first = s.click(rating(None, &["warping", "stringing"]));
        assert!(first.is_some());
        // Three more clicks while it is out: tag, verdict, tag.
        assert_eq!(
            s.click(rating(None, &["warping", "stringing", "z_banding"])),
            None
        );
        assert_eq!(
            s.click(rating(
                Some(Verdict::Good),
                &["warping", "stringing", "z_banding"]
            )),
            None
        );
        assert_eq!(
            s.click(rating(Some(Verdict::Good), &["warping", "z_banding"])),
            None
        );
        let a = s.answer(Ok(rating(None, &["warping", "stringing"])));
        assert_eq!(
            a.send,
            Some(EntryUpdate {
                verdict: Some(VerdictChoice::Good),
                tags: Some(tags(&["warping", "z_banding"])),
                notes: None,
            })
        );
        // Clicking back to what is stored while a save is out sends nothing more.
        let mut s = RatingSync::new(rating(None, &[]));
        s.click(rating(Some(Verdict::Good), &[]));
        s.click(rating(None, &[]));
        s.click(rating(Some(Verdict::Good), &[]));
        let a = s.answer(Ok(rating(Some(Verdict::Good), &[])));
        assert_eq!((a.send, a.status), (None, Some(Status::Saved)));
    }

    #[test]
    fn a_failure_mid_sequence_puts_back_what_was_stored_and_drops_the_rest() {
        let stored = rating(Some(Verdict::Usable), &["warping"]);
        let mut s = RatingSync::new(stored.clone());
        let good = rating(Some(Verdict::Good), &["warping"]);
        s.click(good.clone());
        let a = s.answer(Ok(good.clone()));
        assert_eq!(a.status, Some(Status::Saved));
        // Failed goes out, Usable waits, and Failed's save fails.
        s.click(rating(Some(Verdict::Failed), &["warping"]));
        s.click(rating(Some(Verdict::Usable), &["warping"]));
        let a = s.answer(Err(SAVE_FAILED));
        assert_eq!(a.show, good, "back to the last stored rating");
        assert_eq!(a.status, Some(Status::Failed(SAVE_FAILED)));
        assert_eq!(a.send, None, "the waiting click is dropped");
        // Idle again: the next click goes out at once.
        assert!(s.click(stored).is_some());
    }

    #[test]
    fn an_answer_that_differs_from_the_click_is_what_shows() {
        // The backend keeps only known tags: show what it stored.
        let mut s = RatingSync::new(rating(None, &[]));
        s.click(rating(None, &["stringing", "bogus"]));
        let a = s.answer(Ok(rating(None, &["stringing"])));
        assert_eq!(a.show, rating(None, &["stringing"]));
        assert_eq!(
            s.loaded(rating(None, &["stringing"])),
            Some(rating(None, &["stringing"]))
        );
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

    #[test]
    fn the_box_follows_newer_notes_and_never_saves_the_stale_text_over_them() {
        // Reopened before a save landed: the box shows the old text.
        let mut n = NotesSync::new("Lifted.".into());
        // The reload brings the saved text; the box isn't in use, so it follows.
        assert_eq!(
            n.stored_changed("Lifted. Try a brim.", "Lifted.", false),
            Some("Lifted. Try a brim.".to_string())
        );
        // Leaving the box now saves nothing: it holds what is stored.
        assert_eq!(n.blur("Lifted. Try a brim."), None);
        // Unchanged notes don't touch the box.
        assert_eq!(
            n.stored_changed("Lifted. Try a brim.", "Lifted. Try a brim.", false),
            None
        );
    }

    #[test]
    fn the_box_keeps_the_users_text_while_in_use_or_unsaved() {
        let mut n = NotesSync::new("a".into());
        assert_eq!(n.stored_changed("b", "a", true), None, "focused");
        assert_eq!(n.stored_changed("b", "a typed", false), None, "edited");
        let g = n.blur("c").unwrap();
        assert_eq!(n.stored_changed("b", "c", false), None, "a save is out");
        n.answer(g, Err(SAVE_FAILED));
        assert_eq!(
            n.stored_changed("b", "c", false),
            None,
            "a failed save keeps the text"
        );
    }

    #[test]
    fn uploads_go_one_at_a_time_in_order_across_entries() {
        let mut u = Uploads::<&str>::default();
        let q = u.add(2, vec![("a", 10.0), ("b", 10.0)]);
        assert_eq!(
            q,
            Queued {
                start: true,
                settled: None
            }
        );
        assert_eq!(u.next(), Some((2, "a")));
        // Added while one is going up: queued, not started again.
        let q = u.add(1, vec![("c", 10.0)]);
        assert_eq!(
            q,
            Queued {
                start: false,
                settled: None
            }
        );
        assert_eq!(u.done(2, Ok(())), None, "entry 2 still has b waiting");
        assert_eq!(u.next(), Some((2, "b")));
        assert_eq!(u.done(2, Ok(())), Some(Status::Quiet));
        assert_eq!(u.next(), Some((1, "c")));
        assert_eq!(
            u.done(1, Err(NOT_AN_IMAGE)),
            Some(Status::Failed(NOT_AN_IMAGE))
        );
        assert_eq!(u.next(), None);
        assert!(!u.pending(1) && !u.pending(2));
    }

    #[test]
    fn an_oversized_file_is_refused_without_joining_the_queue() {
        let mut u = Uploads::<&str>::default();
        let q = u.add(2, vec![("huge", MAX_PHOTO_BYTES + 1.0)]);
        assert_eq!(
            q,
            Queued {
                start: false,
                settled: Some(Status::Failed(TOO_LARGE))
            }
        );
        assert_eq!(u.next(), None);
        // A refusal in a batch shows once the batch ends.
        let q = u.add(2, vec![("ok", 1.0), ("huge", MAX_PHOTO_BYTES + 1.0)]);
        assert_eq!(
            q,
            Queued {
                start: true,
                settled: None
            }
        );
        u.next();
        assert_eq!(u.done(2, Ok(())), Some(Status::Failed(TOO_LARGE)));
        // A new batch starts clean.
        let q = u.add(2, vec![("ok", 1.0)]);
        u.next();
        assert!(q.start);
        assert_eq!(u.done(2, Ok(())), Some(Status::Quiet));
    }
}
