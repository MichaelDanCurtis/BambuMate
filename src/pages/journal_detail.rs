//! One journal entry, expanded in place under its row (spec 2026-10-24 §2):
//! the facts, the verdict and issue tags, notes saved on blur, photos, and
//! Analyze / Delete. There is no modal and no new page.
//!
//! Every write after an await goes through `try_*`. The verdict, tags and
//! notes are saved one request at a time per entry, by books kept outside
//! the detail ([`RatingBook`], [`NotesBook`]), so a save waiting behind
//! another still goes out if the detail closes; an answer that lands once
//! the detail is gone changes nothing on the page, and the list catches up
//! through `journal://changed`. Photo uploads also finish if the detail
//! closes (see [`Uploads`]).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::app::FeatureFlagsContext;
use crate::components::defect_report::defect_display_name;
use crate::journal::format::{
    analyze_href, fact_rows, local_time_text, purpose_link, toggle_tag, verdict_after_click,
    verdict_key, verdict_text, DELETE_CONFIRM, ISSUE_TAGS, MAX_PHOTO_BYTES, NOT_AN_IMAGE,
    PHOTO_UNAVAILABLE, TOO_LARGE, VERDICTS,
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

/// The local UTC offset, in minutes east, at Unix time `unix_secs`: each
/// time gets its own, so a DST change since then doesn't shift it.
pub(crate) fn local_offset_minutes(unix_secs: i64) -> i32 {
    let at = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(unix_secs as f64 * 1000.0));
    -(at.get_timezone_offset() as i32)
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
/// holds (or will, once the saves queued for the entry land). While the box
/// isn't focused, has no unsaved edit and no save of the entry is out, it
/// follows the journal's notes. The saves themselves go through
/// [`NotesBook`], one at a time.
#[derive(Debug)]
struct NotesSync {
    /// The text the journal holds, as far as the box knows: the last loaded
    /// or sent. `None` after a failed save, so the next blur tries again and
    /// the box keeps the user's text.
    saved: Option<String>,
}

impl NotesSync {
    fn new(stored: String) -> Self {
        Self {
            saved: Some(stored),
        }
    }

    /// On blur: whether `draft` is to be saved, i.e. differs from what the
    /// journal holds as far as the box knows.
    fn blur(&mut self, draft: &str) -> bool {
        if self.saved.as_deref() == Some(draft) {
            return false;
        }
        self.saved = Some(draft.to_string());
        true
    }

    /// A save failed: the journal may not hold what the box shows.
    fn failed(&mut self) {
        self.saved = None;
    }

    /// The journal's notes are now `stored`. Returns the text to put in the
    /// box, unless the user is in it, has an unsaved edit, or a save is out
    /// (`saving`).
    fn stored_changed(
        &mut self,
        stored: &str,
        draft: &str,
        focused: bool,
        saving: bool,
    ) -> Option<String> {
        let edited = self.saved.as_deref() != Some(draft);
        if focused || edited || saving || self.saved.as_deref() == Some(stored) {
            return None;
        }
        self.saved = Some(stored.to_string());
        Some(stored.to_string())
    }
}

/// One entry's note saves while one is out.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NoteSaves {
    /// The text of the save out.
    sent: String,
    /// The newest text blurred since it went out, if it differs.
    waiting: Option<String>,
}

/// What the notes box does after a save answers.
#[derive(Debug, PartialEq, Eq)]
struct NoteAnswer {
    /// `None` while a follow-up save goes out.
    status: Option<Status>,
    /// The next save to send: the newest text blurred meanwhile.
    send: Option<String>,
}

/// The note saves of every entry, kept outside the detail: one save in
/// flight per entry, so the backend (which runs each on its own thread)
/// can't store an older text last. A blur while a save is out only replaces
/// the text waiting; when the save answers, that text goes out if it differs
/// from what was stored. A failure drops it. An entry is here only while a
/// save is out.
#[derive(Debug, Default)]
struct NotesBook {
    saves: HashMap<i64, NoteSaves>,
}

impl NotesBook {
    fn saving(&self, id: i64) -> bool {
        self.saves.contains_key(&id)
    }

    /// The box was left with `text`, which differs from what it last knew
    /// the journal to hold. Returns the save to send now, if one may go.
    fn queue(&mut self, id: i64, text: &str) -> Option<String> {
        if let Some(out) = self.saves.get_mut(&id) {
            out.waiting = (out.sent != text).then(|| text.to_string());
            return None;
        }
        self.saves.insert(
            id,
            NoteSaves {
                sent: text.to_string(),
                waiting: None,
            },
        );
        Some(text.to_string())
    }

    /// The answer to entry `id`'s save in flight. The entry leaves the book
    /// once nothing more goes out.
    fn answer(&mut self, id: i64, result: Result<(), &'static str>) -> Option<NoteAnswer> {
        let out = self.saves.remove(&id)?;
        Some(match (result, out.waiting) {
            (Ok(()), Some(next)) => {
                self.saves.insert(
                    id,
                    NoteSaves {
                        sent: next.clone(),
                        waiting: None,
                    },
                );
                NoteAnswer {
                    status: None,
                    send: Some(next),
                }
            }
            (Ok(()), None) => NoteAnswer {
                status: Some(Status::Saved),
                send: None,
            },
            (Err(e), _) => NoteAnswer {
                status: Some(Status::Failed(e)),
                send: None,
            },
        })
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
            let answer = match read_file_as_base64(file).await {
                Ok((_, data)) => {
                    // Taken as the photo is sent, not while it is read: a
                    // reload asked for during a long read is older.
                    let revision = next_revision();
                    bridge::add_photo(id, data).await.map(|e| (e, revision))
                }
                Err(_) => Err(NOT_AN_IMAGE.to_string()),
            };
            let result = match answer {
                Ok((saved, revision)) => {
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
    let facts = move || {
        entry
            .with(|e| e.as_ref().map(|e| fact_rows(e, local_offset_minutes)))
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
            {move || entry.with(|e| e.as_ref().and_then(purpose_link)).map(|(chip, href)| view! {
                <p class="jr-purpose-line"><a class="jr-purpose-link" href=href>{chip}</a></p>
            })}
            <dl class="jr-facts">{facts}</dl>
            <RatingControls id=id entry=entry entries=entries />
            <Notes id=id entry=entry entries=entries />
            <Photos id=id entry=entry entries=entries />
            <Actions id=id entries=entries open=open
                analysis=Signal::derive(move || flags.get().analysis_enabled) />
        </div>
    }
}

/// The rating saves of every entry, kept outside the detail: one save in
/// flight per entry, and the follow-up for clicks made meanwhile goes out
/// even if the detail closes. An entry is here only while a save is out, so
/// the book doesn't grow; a detail that opens meanwhile shows what is wanted.
#[derive(Debug, Default)]
struct RatingBook {
    syncs: HashMap<i64, RatingSync>,
}

impl RatingBook {
    /// What a detail opening on entry `id` shows: the wanted rating while a
    /// save is out, else what the list holds.
    fn opening(&self, id: i64, stored: Rating) -> Rating {
        self.syncs.get(&id).map_or(stored, |s| s.wanted.clone())
    }

    /// The list changed. While a save is out it is ignored (the answer
    /// settles it); else it is what to show.
    fn loaded(&self, id: i64, stored: Rating) -> Option<Rating> {
        (!self.syncs.contains_key(&id)).then_some(stored)
    }

    /// A click chose `next`; `stored` is the list's rating, used when no save
    /// is out. Returns the save to send now, if one may go.
    fn click(&mut self, id: i64, stored: Rating, next: Rating) -> Option<EntryUpdate> {
        let sync = self
            .syncs
            .entry(id)
            .or_insert_with(|| RatingSync::new(stored));
        let send = sync.click(next);
        if !sync.in_flight {
            self.syncs.remove(&id);
        }
        send
    }

    /// The answer to entry `id`'s save in flight. The entry leaves the book
    /// once nothing more goes out.
    fn answer(&mut self, id: i64, result: Result<Rating, &'static str>) -> Option<Answered> {
        let answered = self.syncs.get_mut(&id)?.answer(result);
        if answered.send.is_none() {
            self.syncs.remove(&id);
        }
        Some(answered)
    }
}

/// The pills and status line of an entry's open detail, if one is open.
type RatingView = (RwSignal<Rating>, RwSignal<Status>);

thread_local! {
    static RATINGS: RefCell<RatingBook> = RefCell::new(RatingBook::default());
    static RATING_VIEWS: RefCell<HashMap<i64, RatingView>> = RefCell::new(HashMap::new());
}

/// Sends entry `id`'s rating save, then the follow-up the answer asks for.
/// Runs to the end whether or not a detail is open.
fn send_rating(id: i64, entries: Entries, update: EntryUpdate) {
    spawn_local(async move {
        let revision = next_revision();
        let answer = bridge::update(id, &update)
            .await
            .map_err(|e| fixed_error(&e, SAVE_FAILED));
        let rating = answer.as_ref().map(Rating::of).map_err(|e| *e);
        let Some(answered) = RATINGS.with_borrow_mut(|b| b.answer(id, rating)) else {
            return;
        };
        if let Ok(saved) = answer {
            replace_entry(entries, saved, revision);
        }
        if let Some((shown, status)) = RATING_VIEWS.with_borrow(|v| v.get(&id).copied()) {
            shown.try_set(answered.show);
            if let Some(s) = answered.status {
                status.try_set(s);
            }
        }
        if let Some(next) = answered.send {
            send_rating(id, entries, next);
        }
    });
}

/// The verdict pills and the issue-tag chips. Each click shows at once and
/// is saved, one save at a time per entry (see [`RatingSync`], [`RatingBook`]).
#[component]
fn RatingControls(id: i64, entry: Memo<Option<PrintEntry>>, entries: Entries) -> impl IntoView {
    let stored = Memo::new(move |_| entry.with(|e| e.as_ref().map(Rating::of).unwrap_or_default()));
    let shown = RwSignal::new(RATINGS.with_borrow(|b| b.opening(id, stored.get_untracked())));
    let status = RwSignal::new(Status::Quiet);
    RATING_VIEWS.with_borrow_mut(|v| v.insert(id, (shown, status)));
    on_cleanup(move || {
        RATING_VIEWS.with_borrow_mut(|v| {
            if v.get(&id) == Some(&(shown, status)) {
                v.remove(&id);
            }
        });
    });

    // The list changed (a reload, or a save's answer): follow it while no
    // save of this entry is out.
    Effect::new(move |_| {
        let now = stored.get();
        if let Some(show) = RATINGS.with_borrow(|b| b.loaded(id, now)) {
            shown.try_set(show);
        }
    });

    let click = move |next: Rating| {
        shown.set(next.clone());
        status.set(Status::Quiet);
        let update = RATINGS.with_borrow_mut(|b| b.click(id, stored.get_untracked(), next));
        if let Some(update) = update {
            send_rating(id, entries, update);
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
            <StatusLine status=status />
        </section>
    }
}

/// The status line and bookkeeping of an entry's open notes box, if one is
/// open.
type NotesView = (RwSignal<Status>, StoredValue<NotesSync>);

thread_local! {
    static NOTES: RefCell<NotesBook> = RefCell::new(NotesBook::default());
    static NOTES_VIEWS: RefCell<HashMap<i64, NotesView>> = RefCell::new(HashMap::new());
}

/// Sends entry `id`'s note save, then the follow-up the answer asks for.
/// Runs to the end whether or not a detail is open.
fn send_notes(id: i64, entries: Entries, text: String) {
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
        let Some(answered) = NOTES.with_borrow_mut(|b| b.answer(id, result)) else {
            return;
        };
        if let Ok(saved) = answer {
            replace_entry(entries, saved, revision);
        }
        if let Some((status, sync)) = NOTES_VIEWS.with_borrow(|v| v.get(&id).copied()) {
            if result.is_err() {
                sync.try_update_value(NotesSync::failed);
            }
            if let Some(s) = answered.status {
                status.try_set(s);
            }
        }
        if let Some(next) = answered.send {
            send_notes(id, entries, next);
        }
    });
}

/// The notes box. It saves when it loses focus, and only if the text changed,
/// one save at a time per entry; see [`NotesSync`] for when it follows the
/// journal, and [`NotesBook`] for the order of saves.
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
    NOTES_VIEWS.with_borrow_mut(|v| v.insert(id, (status, sync)));
    on_cleanup(move || {
        NOTES_VIEWS.with_borrow_mut(|v| {
            if v.get(&id) == Some(&(status, sync)) {
                v.remove(&id);
            }
        });
    });

    // Takes the journal's notes into the box if `stored_changed` allows.
    let follow = move |now: &str, text: &str, in_use: bool| {
        let saving = NOTES.with_borrow(|b| b.saving(id));
        let next = sync
            .try_update_value(|s| s.stored_changed(now, text, in_use, saving))
            .flatten();
        if let Some(next) = &next {
            draft.set(next.clone());
            in_box.set(next.clone());
        }
        next.is_some()
    };

    Effect::new(move |_| {
        let now = stored.get();
        follow(&now, &draft.get_untracked(), focused.get_untracked());
    });

    let on_blur = move |_| {
        focused.set(false);
        let text = draft.get_untracked();
        // Notes that arrived while the box was in use, and nothing typed:
        // take them now, rather than keep showing (and later save over
        // them) the older text.
        if follow(&stored.get_untracked(), &text, false) {
            return;
        }
        if !sync.try_update_value(|s| s.blur(&text)).unwrap_or(false) {
            return;
        }
        status.set(Status::Quiet);
        if let Some(send) = NOTES.with_borrow_mut(|b| b.queue(id, &text)) {
            send_notes(id, entries, send);
        }
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
fn Photos(id: i64, entry: Memo<Option<PrintEntry>>, entries: Entries) -> impl IntoView {
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
                            entries=entries status=status />
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
    let offset = local_offset_minutes(added_at);
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
        assert_eq!(a.send, None);
    }

    #[test]
    fn the_book_keeps_one_save_per_entry_across_a_closed_detail() {
        let usable = rating(Some(Verdict::Usable), &[]);
        let good = rating(Some(Verdict::Good), &[]);
        let failed = rating(Some(Verdict::Failed), &[]);
        let mut b = RatingBook::default();
        assert_eq!(b.opening(1, usable.clone()), usable);
        assert_eq!(
            b.click(1, usable.clone(), good.clone()),
            Some(verdict(VerdictChoice::Good))
        );
        assert_eq!(b.click(1, usable.clone(), failed.clone()), None);
        // The detail closes and opens again before Good answers: it shows
        // Failed, not the list's Usable, and a reload doesn't move it.
        assert_eq!(b.opening(1, usable.clone()), failed);
        assert_eq!(b.loaded(1, usable.clone()), None);
        // Another entry is its own: its click goes out at once.
        assert!(b.click(2, usable.clone(), good.clone()).is_some());
        // Good's answer sends the waiting Failed; Failed's answer settles.
        let a = b.answer(1, Ok(good)).unwrap();
        assert_eq!(a.send, Some(verdict(VerdictChoice::Failed)));
        let a = b.answer(1, Ok(failed.clone())).unwrap();
        assert_eq!((a.show, a.send), (failed.clone(), None));
        // Settled entries leave the book.
        assert!(!b.syncs.contains_key(&1));
        assert_eq!(b.loaded(1, failed.clone()), Some(failed));
        assert_eq!(b.answer(1, Err(SAVE_FAILED)), None, "nothing out for 1");
        b.answer(2, Err(SAVE_FAILED));
        assert!(b.syncs.is_empty());
        // A click back to what is stored sends nothing and leaves nothing.
        assert_eq!(b.click(3, usable.clone(), usable), None);
        assert!(b.syncs.is_empty());
    }

    #[test]
    fn notes_that_arrived_while_the_box_was_in_use_are_taken_on_blur() {
        let mut n = NotesSync::new("Lifted.".into());
        // The reload lands while the box is focused: skipped then.
        assert_eq!(
            n.stored_changed("Use a brim.", "Lifted.", true, false),
            None
        );
        // On blur with nothing typed, the box takes them, and saves nothing.
        assert_eq!(
            n.stored_changed("Use a brim.", "Lifted.", false, false),
            Some("Use a brim.".to_string())
        );
        assert!(!n.blur("Use a brim."));
    }

    #[test]
    fn notes_save_only_when_they_differ_from_the_journal() {
        let mut n = NotesSync::new("Lifted.".into());
        assert!(!n.blur("Lifted."));
        assert!(n.blur("Lifted. Try a brim."));
        // Focus and blur again, before the answer or after: nothing new.
        assert!(!n.blur("Lifted. Try a brim."));
    }

    #[test]
    fn a_failed_note_keeps_the_text_and_the_next_blur_tries_again() {
        let mut n = NotesSync::new(String::new());
        let mut b = NotesBook::default();
        assert!(n.blur("x"));
        assert_eq!(b.queue(1, "x"), Some("x".to_string()));
        assert_eq!(
            b.answer(1, Err(SAVE_FAILED)),
            Some(NoteAnswer {
                status: Some(Status::Failed(SAVE_FAILED)),
                send: None
            })
        );
        n.failed();
        assert_eq!(
            n.stored_changed("", "x", false, false),
            None,
            "a failed save keeps the text"
        );
        assert!(n.blur("x"));
        assert_eq!(b.queue(1, "x"), Some("x".to_string()));
    }

    #[test]
    fn note_saves_go_out_one_at_a_time_and_the_newest_text_is_sent_last() {
        let mut n = NotesSync::new(String::new());
        let mut b = NotesBook::default();
        assert!(n.blur("first"));
        assert_eq!(b.queue(1, "first"), Some("first".to_string()));
        assert!(b.saving(1));
        // Edited and left twice while it is out: only the newest waits.
        assert!(n.blur("second draft"));
        assert_eq!(b.queue(1, "second draft"), None);
        assert!(n.blur("second"));
        assert_eq!(b.queue(1, "second"), None);
        // Another entry is its own: its save goes out at once.
        assert_eq!(b.queue(2, "other"), Some("other".to_string()));
        assert_eq!(
            b.answer(1, Ok(())),
            Some(NoteAnswer {
                status: None,
                send: Some("second".to_string())
            })
        );
        assert!(b.saving(1), "the follow-up is out");
        assert_eq!(
            n.stored_changed("first", "second", false, b.saving(1)),
            None,
            "the first answer's reload doesn't move the box"
        );
        assert_eq!(
            b.answer(1, Ok(())),
            Some(NoteAnswer {
                status: Some(Status::Saved),
                send: None
            })
        );
        // Settled entries leave the book; the stored text is the box's.
        assert!(!b.saving(1));
        assert_eq!(n.stored_changed("second", "second", false, false), None);
        assert!(!n.blur("second"));
        assert_eq!(b.answer(1, Ok(())), None, "nothing out for 1");
        b.answer(2, Ok(()));
        assert!(b.saves.is_empty());
    }

    #[test]
    fn a_note_edited_back_to_the_text_out_sends_nothing_more() {
        let mut b = NotesBook::default();
        b.queue(1, "a");
        b.queue(1, "ab");
        b.queue(1, "a");
        assert_eq!(
            b.answer(1, Ok(())),
            Some(NoteAnswer {
                status: Some(Status::Saved),
                send: None
            })
        );
        assert!(b.saves.is_empty());
    }

    #[test]
    fn a_failed_note_save_drops_the_text_waiting_behind_it() {
        let mut b = NotesBook::default();
        b.queue(1, "first");
        b.queue(1, "second");
        assert_eq!(
            b.answer(1, Err(SAVE_FAILED)),
            Some(NoteAnswer {
                status: Some(Status::Failed(SAVE_FAILED)),
                send: None
            })
        );
        assert!(!b.saving(1));
        // Idle again: the next blur's save goes out at once.
        assert_eq!(b.queue(1, "second"), Some("second".to_string()));
    }

    #[test]
    fn the_box_follows_newer_notes_and_never_saves_the_stale_text_over_them() {
        // Reopened before a save landed: the box shows the old text.
        let mut n = NotesSync::new("Lifted.".into());
        // The reload brings the saved text; the box isn't in use, so it follows.
        assert_eq!(
            n.stored_changed("Lifted. Try a brim.", "Lifted.", false, false),
            Some("Lifted. Try a brim.".to_string())
        );
        // Leaving the box now saves nothing: it holds what is stored.
        assert!(!n.blur("Lifted. Try a brim."));
        // Unchanged notes don't touch the box.
        assert_eq!(
            n.stored_changed("Lifted. Try a brim.", "Lifted. Try a brim.", false, false),
            None
        );
    }

    #[test]
    fn the_box_keeps_the_users_text_while_in_use_unsaved_or_saving() {
        let mut n = NotesSync::new("a".into());
        assert_eq!(n.stored_changed("b", "a", true, false), None, "focused");
        assert_eq!(
            n.stored_changed("b", "a typed", false, false),
            None,
            "edited"
        );
        assert!(n.blur("c"));
        assert_eq!(
            n.stored_changed("b", "c", false, true),
            None,
            "a save is out"
        );
        n.failed();
        assert_eq!(
            n.stored_changed("b", "c", false, false),
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
