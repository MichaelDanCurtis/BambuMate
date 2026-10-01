//! Slice page: slice a model with Bambu Studio before printing, and compare
//! filaments side by side. BambuMate never prints; "Open in Bambu Studio"
//! hands the sliced file to the user.
//!
//! Every `spawn_local` here reads signals before its first `.await` and only
//! uses `try_*` accessors after it: the page can be gone by then. Everything
//! that `slicer://job` changes is keyed per job or memo-driven, so a progress
//! event updates text in place and never rebuilds a result or a picker.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, StlFile};
use crate::slicer::state::{
    compare_rows, format_cost, format_duration, format_grams, insert_if_absent, keyed_rows,
    remember_choices, status_line, swatch_color, totals, Totals,
};
use crate::slicer::types::{
    ErrorView, JobState, PresetLists, PresetOption, PresetSource, SlicerStatus, WarningLevel,
    FILES_CLEARED_KIND,
};
use crate::slicer::{bridge, SlicerShared};

/// Up to three filaments next to the main one: four columns at most.
const MAX_EXTRA: usize = 3;
/// Jobs listed under "Recent".
const RECENT: usize = 10;
/// The backend's staging cap (`slicer_stage_model`). Larger files are refused
/// before they are read into memory and sent over IPC.
const MAX_DROP_BYTES: f64 = 512.0 * 1024.0 * 1024.0;
const TOO_LARGE: &str = "That file is too large to slice from BambuMate.";
/// How often the "From watch folder" list is refreshed, like the STL
/// indicator's.
const WATCH_POLL: std::time::Duration = std::time::Duration::from_secs(5);

/// Reads a dropped file and stages it in the backend, which needs a path.
async fn stage_dropped(file: web_sys::File) -> Result<String, String> {
    use js_sys::{ArrayBuffer, Uint8Array};
    if file.size() > MAX_DROP_BYTES {
        return Err(TOO_LARGE.to_string());
    }
    let buf: ArrayBuffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
        .await
        .map_err(|_| "Couldn't read that file.".to_string())?
        .dyn_into()
        .map_err(|_| "Couldn't read that file.".to_string())?;
    let bytes = Uint8Array::new(&buf).to_vec();
    bridge::stage_model(
        file.name(),
        crate::pages::print_analysis::base64_encode(&bytes),
    )
    .await
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// A preset `<select>`; user presets first, under their own group.
#[component]
fn PresetSelect(
    id: &'static str,
    label: &'static str,
    options: Signal<Vec<PresetOption>>,
    value: RwSignal<String>,
) -> impl IntoView {
    let group = move |source: PresetSource| {
        options
            .get()
            .into_iter()
            .filter(move |o| o.source == source)
            .map(|o| {
                let selected = o.name == value.get_untracked();
                let label = o.name.clone();
                view! { <option value=o.name selected=selected>{label}</option> }
            })
            .collect::<Vec<_>>()
    };
    view! {
        <label class="sl-field" for=id>
            <span class="nd-label">{label}</span>
            <select
                id=id
                prop:value=move || value.get()
                on:change=move |ev| value.set(event_target_value(&ev))
            >
                <optgroup label="Your presets">{move || group(PresetSource::User)}</optgroup>
                <optgroup label="Bambu presets">{move || group(PresetSource::System)}</optgroup>
            </select>
        </label>
    }
}

#[component]
pub fn SlicePage() -> impl IntoView {
    let shared = expect_context::<SlicerShared>();
    let status = RwSignal::new(None::<SlicerStatus>);
    let lists = RwSignal::new(PresetLists::default());
    let bed_types = RwSignal::new(Vec::<String>::new());
    // Whether the saved settings could be read. Until they are, a slice
    // saves nothing, so it can't overwrite them (auto-slice) with defaults.
    let settings_loaded = RwSignal::new(false);
    let printer = RwSignal::new(String::new());
    let process = RwSignal::new(String::new());
    let filament = RwSignal::new(String::new());
    let bed_type = RwSignal::new(String::new());
    let model = RwSignal::new(None::<String>);
    let watched = RwSignal::new(Vec::<StlFile>::new());
    let comparing = RwSignal::new(false);
    let extra = RwSignal::new(Vec::<String>::new());
    let compare_ids = RwSignal::new(Vec::<u64>::new());
    let selected = RwSignal::new(None::<u64>);
    let error = RwSignal::new(None::<String>);
    let preset_error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let drag_over = RwSignal::new(false);
    // Only the newest presets request may write its answer.
    let preset_gen = StoredValue::new(0u64);

    // `/slice?job=ID` opens that job (the STL indicator links here).
    let query = use_query_map();
    Effect::new(move |_| {
        if let Some(id) = query.with(|q| q.get("job")).and_then(|s| s.parse().ok()) {
            selected.set(Some(id));
            comparing.set(false);
        }
    });

    // Called after awaits too, so it only uses `try_*` accessors.
    let load_presets = move |for_printer: Option<String>| {
        let Some(gen) = preset_gen.try_update_value(|g| {
            *g += 1;
            *g
        }) else {
            return;
        };
        spawn_local(async move {
            let answer = bridge::presets(for_printer).await;
            if preset_gen.try_get_value() != Some(gen) {
                return;
            }
            let l = match answer {
                Ok(l) => l,
                Err(e) => {
                    preset_error.try_set(Some(e));
                    return;
                }
            };
            preset_error.try_set(None);
            let fix = |sig: RwSignal<String>, opts: &[PresetOption]| {
                let current = sig.try_get_untracked().unwrap_or_default();
                if !opts.iter().any(|o| o.name == current) {
                    sig.try_set(opts.first().map(|o| o.name.clone()).unwrap_or_default());
                }
            };
            fix(process, &l.processes);
            fix(filament, &l.filaments);
            // A compare pick the new printer doesn't offer goes back to
            // "Choose a filament", so what is shown is what gets sliced.
            extra.try_update(|picks| {
                for pick in picks.iter_mut() {
                    if !l.filaments.iter().any(|o| o.name == *pick) {
                        pick.clear();
                    }
                }
            });
            if printer.try_get_untracked().unwrap_or_default().is_empty() {
                printer.try_set(
                    l.printers
                        .first()
                        .map(|o| o.name.clone())
                        .unwrap_or_default(),
                );
            }
            lists.try_set(l);
        });
    };

    spawn_local(async move {
        match bridge::status().await {
            Ok(st) => {
                status.try_set(Some(st));
            }
            Err(e) => {
                error.try_set(Some(e));
            }
        }
        match bridge::get_settings().await {
            Ok(view) => {
                let eff = view.effective;
                settings_loaded.try_set(true);
                bed_types.try_set(view.bed_types);
                process.try_set(eff.process.unwrap_or_default());
                filament.try_set(eff.filament.unwrap_or_default());
                bed_type.try_set(eff.bed_type.unwrap_or_default());
                printer.try_set(eff.printer.unwrap_or_default());
            }
            Err(e) => {
                error.try_set(Some(e));
            }
        }
        // `None` once the page is gone: nothing to load for.
        if printer.try_get_untracked().is_some_and(|p| p.is_empty()) {
            load_presets(None);
        }
        if let Ok(files) = commands::list_received_stls().await {
            watched.try_set(files);
        }
    });

    // New STLs arrive while the page is open; only a changed list is set.
    let refresh_watched = move || {
        spawn_local(async move {
            if let Ok(files) = commands::list_received_stls().await {
                if watched.try_with_untracked(|w| *w != files) == Some(true) {
                    watched.try_set(files);
                }
            }
        });
    };
    if let Ok(handle) = set_interval_with_handle(refresh_watched, WATCH_POLL) {
        on_cleanup(move || handle.clear());
    }

    // Processes and filaments follow the printer.
    Effect::new(move |_| {
        let p = printer.get();
        if !p.is_empty() {
            load_presets(Some(p));
        }
    });

    let processes = Signal::derive(move || lists.with(|l| l.processes.clone()));
    let filaments = Signal::derive(move || lists.with(|l| l.filaments.clone()));
    let printers = Signal::derive(move || lists.with(|l| l.printers.clone()));
    let can_slice = move || {
        model.with(|m| m.is_some())
            && !printer.with(String::is_empty)
            && !process.with(String::is_empty)
            && !filament.with(String::is_empty)
            && !busy.get()
            && status.with(|s| s.as_ref().is_some_and(|s| s.supported))
    };

    // The native picker returns a real path, so nothing is copied over IPC.
    let browse = move |_| {
        error.set(None);
        spawn_local(async move {
            match bridge::pick_model().await {
                Ok(Some(path)) => {
                    model.try_set(Some(path));
                }
                Ok(None) => {}
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    };

    // A dropped file has no path in the webview, so its bytes are staged.
    let on_drop = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        drag_over.set(false);
        let Some(file) = ev
            .data_transfer()
            .and_then(|dt| dt.files())
            .and_then(|f| f.get(0))
        else {
            return;
        };
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match stage_dropped(file).await {
                Ok(path) => {
                    model.try_set(Some(path));
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
            busy.try_set(false);
        });
    };

    let slice = move |_| {
        let Some(path) = model.get_untracked() else {
            return;
        };
        let (p, q, f, b) = (
            printer.get_untracked(),
            process.get_untracked(),
            filament.get_untracked(),
            bed_type.get_untracked(),
        );
        let mut all = vec![f.clone()];
        if comparing.get_untracked() {
            all.extend(extra.get_untracked().into_iter().filter(|e| !e.is_empty()));
        }
        let remember = settings_loaded.get_untracked();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let mut ids = Vec::new();
            for fil in all {
                match bridge::slice(path.clone(), p.clone(), q.clone(), fil, Some(b.clone())).await
                {
                    Ok(job) => {
                        ids.push(job.id);
                        // The job's own events may already be here and newer.
                        shared.jobs.try_update(|js| insert_if_absent(js, job));
                    }
                    Err(e) => {
                        error.try_set(Some(e));
                        break;
                    }
                }
            }
            let Some(first) = ids.first().copied() else {
                // Nothing was queued: keep the slice error and the settings.
                busy.try_set(false);
                return;
            };
            selected.try_set(Some(first));
            compare_ids.try_set(if ids.len() > 1 { ids } else { Vec::new() });
            // The presets just used become the defaults, saved on top of the
            // settings as they are now (Settings may have changed auto-slice
            // since the page loaded). Nothing is saved when they can't be read.
            if remember {
                let saved = match bridge::get_settings().await {
                    Ok(view) => bridge::set_settings(remember_choices(view.saved, p, q, f, b))
                        .await
                        .map(|_| ()),
                    Err(e) => Err(e),
                };
                if let Err(e) = saved {
                    error.try_set(Some(e));
                }
            }
            busy.try_set(false);
        });
    };

    let toggle_compare = move |_| {
        let on = !comparing.get_untracked();
        comparing.set(on);
        if on && extra.with_untracked(Vec::is_empty) {
            extra.set(vec![String::new()]);
        }
    };

    // Only the count drives the picker list, so choosing a filament in one
    // picker doesn't rebuild the others.
    let extra_len = Memo::new(move |_| extra.with(Vec::len));
    let recent_ids = Memo::new(move |_| {
        shared.jobs.with(|js| {
            js.iter()
                .rev()
                .take(RECENT)
                .map(|j| j.id)
                .collect::<Vec<_>>()
        })
    });

    view! {
        <div class="page slice-page nd">
            <header class="sl-head">
                <h2>"Slice"</h2>
                <p class="nd-label sl-version">
                    {move || match status.get() {
                        Some(s) if s.supported => format!("Bambu Studio {}", s.version.unwrap_or_default()),
                        Some(_) => "Bambu Studio unavailable".to_string(),
                        None => "Checking Bambu Studio…".to_string(),
                    }}
                </p>
            </header>

            {move || status.get().and_then(|s| s.message).map(|m| view! {
                <p class="sl-error sl-status-error">{m}</p>
            })}

            <section class="sl-input">
                <div
                    class="sl-drop"
                    class:sl-drop-over=move || drag_over.get()
                    on:dragover=move |ev: web_sys::DragEvent| {
                        ev.prevent_default();
                        drag_over.set(true);
                    }
                    on:dragleave=move |_| drag_over.set(false)
                    on:drop=on_drop
                >
                    <p class="sl-model">
                        {move || match model.get() {
                            Some(p) => file_name(&p),
                            None => "Drop an STL or 3MF here".to_string(),
                        }}
                    </p>
                    <div class="sl-drop-actions">
                        <button class="sl-btn sl-browse" on:click=browse>"Choose file"</button>
                        <Show when=move || !watched.with(Vec::is_empty)>
                            <select
                                class="sl-watch"
                                on:change=move |ev| {
                                    let v = event_target_value(&ev);
                                    if !v.is_empty() {
                                        model.set(Some(v));
                                    }
                                }
                            >
                                <option value="">"From watch folder"</option>
                                <For each=move || watched.get() key=|f| f.path.clone() let:f>
                                    <option value=f.path.clone()>{f.filename.clone()}</option>
                                </For>
                            </select>
                        </Show>
                    </div>
                </div>

                <div class="sl-presets">
                    <PresetSelect id="sl-printer" label="Printer" options=printers value=printer />
                    <PresetSelect id="sl-process" label="Process" options=processes value=process />
                    <PresetSelect id="sl-filament" label="Filament" options=filaments value=filament />
                    <label class="sl-field" for="sl-bed">
                        <span class="nd-label">"Plate"</span>
                        <select
                            id="sl-bed"
                            prop:value=move || bed_type.get()
                            on:change=move |ev| bed_type.set(event_target_value(&ev))
                        >
                            {move || bed_types.get().into_iter().map(|b| view! {
                                <option value=b.clone()>{b.clone()}</option>
                            }).collect::<Vec<_>>()}
                        </select>
                    </label>
                </div>
                {move || preset_error.get().map(|e| view! { <p class="sl-error sl-preset-error">{e}</p> })}

                <Show when=move || comparing.get()>
                    <div class="sl-compare-pickers">
                        <For each=move || 0..extra_len.get() key=|i| *i let:i>
                            <label class="sl-field">
                                <span class="nd-label">{format!("Compare with {}", i + 2)}</span>
                                <select
                                    class="sl-compare-filament"
                                    prop:value=move || extra.with(|e| e.get(i).cloned().unwrap_or_default())
                                    on:change=move |ev| {
                                        let v = event_target_value(&ev);
                                        extra.update(|e| if let Some(slot) = e.get_mut(i) { *slot = v });
                                    }
                                >
                                    <option value="">"Choose a filament"</option>
                                    {move || {
                                        let pick = extra.with_untracked(|e| e.get(i).cloned().unwrap_or_default());
                                        filaments.get().into_iter().map(|o| {
                                            let selected = o.name == pick;
                                            view! { <option value=o.name.clone() selected=selected>{o.name.clone()}</option> }
                                        }).collect::<Vec<_>>()
                                    }}
                                </select>
                            </label>
                        </For>
                        <Show when=move || extra_len.get() < MAX_EXTRA>
                            <button class="sl-btn sl-add-filament" on:click=move |_| extra.update(|e| e.push(String::new()))>
                                "Add filament"
                            </button>
                        </Show>
                    </div>
                </Show>

                <div class="sl-actions">
                    <button class="sl-btn sl-primary sl-slice" disabled=move || !can_slice() on:click=slice>
                        {move || if comparing.get() { "Slice and compare" } else { "Slice" }}
                    </button>
                    <button class="sl-btn sl-compare-toggle" class:active=move || comparing.get() on:click=toggle_compare>
                        "Compare"
                    </button>
                </div>
                {move || error.get().map(|e| view! { <p class="sl-error sl-input-error">{e}</p> })}
            </section>

            <Show when=move || !compare_ids.with(Vec::is_empty)>
                <CompareTable ids=compare_ids.into() />
            </Show>

            {move || selected.get().map(|id| view! { <JobCard id=id selected=selected /> })}

            <section class="sl-recent">
                <h3 class="nd-label">"Recent"</h3>
                <ul>
                    <For each=move || recent_ids.get() key=|id| *id let:id>
                        <RecentRow id=id selected=selected />
                    </For>
                </ul>
            </section>
        </div>
    }
}

#[component]
fn RecentRow(id: u64, selected: RwSignal<Option<u64>>) -> impl IntoView {
    let job = expect_context::<SlicerShared>().job(id);
    view! {
        <li
            class="sl-recent-row"
            data-job=id.to_string()
            class:active=move || selected.get() == Some(id)
            on:click=move |_| selected.set(Some(id))
        >
            <span class="sl-recent-name">{move || job.with(|j| j.as_ref().map(|j| j.model_name.clone()).unwrap_or_default())}</span>
            <span class="sl-recent-filament">{move || job.with(|j| j.as_ref().map(|j| j.filament.clone()).unwrap_or_default())}</span>
            <span class="nd-mono sl-recent-state">{move || job.with(|j| j.as_ref().map(status_line).unwrap_or_default())}</span>
        </li>
    }
}

/// What a job card shows below its status line. Progress events don't
/// change it, so they never rebuild the result.
#[derive(Clone, PartialEq)]
enum Phase {
    Pending,
    Done,
    Failed(String),
}

#[component]
fn JobCard(id: u64, selected: RwSignal<Option<u64>>) -> impl IntoView {
    let shared = expect_context::<SlicerShared>();
    let job = shared.job(id);
    // An inline message from Cancel, the thumbnail, Open or Slice again.
    let notice = RwSignal::new(None::<ErrorView>);
    let cancelling = RwSignal::new(false);
    let reslicing = RwSignal::new(false);
    let phase = Memo::new(move |_| {
        job.with(|j| match j.as_ref().map(|j| &j.state) {
            Some(JobState::Done { .. }) => Phase::Done,
            Some(JobState::Failed { error }) => Phase::Failed(error.message.clone()),
            _ => Phase::Pending,
        })
    });
    let running =
        Memo::new(move |_| job.with(|j| j.as_ref().is_some_and(|j| !j.state.is_terminal())));
    let percent = move || {
        job.with(|j| match j.as_ref().map(|j| &j.state) {
            Some(JobState::Running { progress: Some(p) }) => p.percent,
            _ => 0,
        })
    };
    // `true` only means the cancel was taken; the job's final state arrives
    // as a `slicer://job` event.
    let cancel = move |_| {
        cancelling.set(true);
        notice.set(None);
        spawn_local(async move {
            if let Err(e) = bridge::cancel(id).await {
                notice.try_set(Some(ErrorView::text(e)));
            }
            cancelling.try_set(false);
        });
    };
    // Queues the same model and presets again, for a job whose files were
    // cleared.
    let reslice = move |_| {
        let Some(j) = job.get_untracked() else {
            return;
        };
        reslicing.set(true);
        notice.set(None);
        spawn_local(async move {
            match bridge::slice(
                j.source_path,
                j.printer,
                j.process,
                j.filament,
                Some(j.bed_type),
            )
            .await
            {
                Ok(new) => {
                    let new_id = new.id;
                    shared.jobs.try_update(|js| insert_if_absent(js, new));
                    selected.try_set(Some(new_id));
                }
                Err(e) => {
                    notice.try_set(Some(ErrorView::text(e)));
                }
            }
            reslicing.try_set(false);
        });
    };
    let cleared = move || notice.with(|n| n.as_ref().is_some_and(|n| n.kind == FILES_CLEARED_KIND));
    // Whether a cleared job's model is still there to slice again: a dropped
    // model is cleared along with the cache. `None` until checked.
    let source_ok = RwSignal::new(None::<bool>);
    Effect::new(move |_| {
        if !cleared() || source_ok.get_untracked().is_some() {
            return;
        }
        let Some(path) = job.with_untracked(|j| j.as_ref().map(|j| j.source_path.clone())) else {
            return;
        };
        spawn_local(async move {
            let ok = bridge::model_exists(path).await;
            source_ok.try_set(Some(ok));
        });
    });
    view! {
        <section class="sl-job">
            <header class="sl-job-head">
                <div>
                    <p class="sl-job-model">{move || job.with(|j| j.as_ref().map(|j| j.model_name.clone()).unwrap_or_default())}</p>
                    <p class="nd-label">
                        {move || job.with(|j| j.as_ref().map(|j| format!("{} · {} · {}", j.printer, j.process, j.filament)).unwrap_or_default())}
                    </p>
                </div>
                <Show when=move || running.get()>
                    <button class="sl-btn sl-cancel" disabled=move || cancelling.get() on:click=cancel>"Cancel"</button>
                </Show>
            </header>
            <p class="nd-mono sl-job-status">{move || job.with(|j| j.as_ref().map(status_line).unwrap_or_else(|| "This job is no longer available.".into()))}</p>
            <Show when=move || running.get()>
                <div class="sl-progress"><div class="sl-progress-bar" style:width=move || format!("{}%", percent())></div></div>
            </Show>
            {move || match phase.get() {
                Phase::Failed(message) => view! { <p class="sl-error sl-job-error">{message}</p> }.into_any(),
                Phase::Done => view! { <ResultView id=id notice=notice /> }.into_any(),
                Phase::Pending => ().into_any(),
            }}
            {move || notice.get().map(|n| view! {
                <div class="sl-notice">
                    <p class="sl-error sl-action-error">{n.message}</p>
                    <Show when=cleared>
                        {move || match source_ok.get() {
                            Some(true) => view! {
                                <button class="sl-btn sl-reslice" disabled=move || reslicing.get() on:click=reslice>"Slice again"</button>
                            }.into_any(),
                            Some(false) => view! {
                                <p class="sl-reslice-gone">"The model was cleared too; drop it again."</p>
                            }.into_any(),
                            None => ().into_any(),
                        }}
                    </Show>
                </div>
            })}
        </section>
    }
}

#[component]
fn ResultView(id: u64, notice: RwSignal<Option<ErrorView>>) -> impl IntoView {
    let job = expect_context::<SlicerShared>().job(id);
    let result = Memo::new(move |_| {
        job.with(|j| match j.as_ref().map(|j| &j.state) {
            Some(JobState::Done { result, .. }) => Some(result.clone()),
            _ => None,
        })
    });
    let plate_ids = Memo::new(move |_| {
        result.with(|r| {
            r.as_ref()
                .map(|r| r.plates.iter().map(|p| p.index).collect::<Vec<_>>())
                .unwrap_or_default()
        })
    });
    let plate = RwSignal::new(plate_ids.with_untracked(|ids| ids.first().copied().unwrap_or(1)));
    let current = Memo::new(move |_| {
        let p = plate.get();
        result.with(|r| {
            r.as_ref()
                .and_then(|r| r.plates.iter().find(|x| x.index == p).cloned())
        })
    });
    let thumb = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        let p = plate.get();
        let has_thumb = current.with(|c| c.as_ref().is_some_and(|c| c.thumbnail.is_some()));
        thumb.set(None);
        if !has_thumb {
            return;
        }
        spawn_local(async move {
            match bridge::thumbnail(id, p).await {
                // Only the plate still shown may write; a `data:` PNG only.
                Ok(Some(url)) if url.starts_with("data:image/png;base64,") => {
                    if plate.try_get_untracked() == Some(p) {
                        thumb.try_set(Some(url));
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    notice.try_set(Some(e));
                }
            }
        });
    });
    let open = move |_| {
        notice.set(None);
        spawn_local(async move {
            if let Err(e) = bridge::open_in_bambu_studio(id).await {
                notice.try_set(Some(e));
            }
        });
    };
    // Rows are keyed on the plate, the position and the whole row, so a row
    // is rebuilt whenever anything it shows differs (another plate's grams,
    // a warning's level), and kept when nothing does.
    let warnings = Memo::new(move |_| {
        current.with(|c| {
            c.as_ref()
                .map(|p| keyed_rows(p.index, &p.warnings))
                .unwrap_or_default()
        })
    });
    let filaments = Memo::new(move |_| {
        current.with(|c| {
            c.as_ref()
                .map(|p| keyed_rows(p.index, &p.filaments))
                .unwrap_or_default()
        })
    });
    let field = move |f: fn(&crate::slicer::types::PlateResult) -> String| {
        move || current.with(|c| c.as_ref().map(f).unwrap_or_default())
    };
    view! {
        <div class="sl-result">
            <Show when=move || plate_ids.with(|ids| ids.len() > 1)>
                <div class="sl-plate-tabs" role="tablist">
                    <For each=move || plate_ids.get() key=|idx| *idx let:idx>
                        <button
                            class="sl-plate-tab"
                            role="tab"
                            class:active=move || plate.get() == idx
                            aria-selected=move || (plate.get() == idx).to_string()
                            on:click=move |_| plate.set(idx)
                        >
                            {format!("Plate {idx}")}
                        </button>
                    </For>
                </div>
            </Show>
            <div class="sl-result-body">
                <div class="sl-thumb">
                    {move || thumb.get().map(|src| view! { <img src=src alt="Plate preview" /> })}
                </div>
                <div class="sl-metrics">
                    <p class="sl-hero nd-mono">{field(|p| format_duration(p.time_seconds))}</p>
                    <dl class="sl-stats">
                        <div><dt class="nd-label">"Weight"</dt><dd class="sl-weight">{field(|p| format_grams(p.weight_g))}</dd></div>
                        <div><dt class="nd-label">"Cost"</dt><dd class="sl-cost">{field(|p| p.cost.map(format_cost).unwrap_or_else(|| "—".into()))}</dd></div>
                    </dl>
                    <ul class="sl-filaments">
                        <For each=move || filaments.get() key=|(k, _)| k.clone() let:row>
                            {
                                let (_, f) = row;
                                view! {
                                    <li class="sl-filament">
                                        <span class="sl-swatch" style:background=swatch_color(&f.color).unwrap_or_default()></span>
                                        {format!("Slot {} · {} · {} · {:.2} m", f.slot, f.filament_type, format_grams(f.used_g), f.used_m)}
                                    </li>
                                }
                            }
                        </For>
                    </ul>
                    <ul class="sl-warnings">
                        <For each=move || warnings.get() key=|(k, _)| k.clone() let:row>
                            {
                                let (_, w) = row;
                                let level = match w.level {
                                    WarningLevel::Warning => "warning",
                                    WarningLevel::Notice => "notice",
                                };
                                view! {
                                    <li class=format!("sl-warning sl-warning-{level}")>
                                        <span class="nd-label">{level}</span>" "{w.message}
                                    </li>
                                }
                            }
                        </For>
                    </ul>
                </div>
            </div>
            <button class="sl-btn sl-open" on:click=open>"Open in Bambu Studio"</button>
        </div>
    }
}

#[component]
fn CompareTable(ids: Signal<Vec<u64>>) -> impl IntoView {
    let shared = expect_context::<SlicerShared>();
    // Totals only change when a column's result does, not on progress.
    let columns = Memo::new(move |_| {
        let ids = ids.get();
        shared.jobs.with(|js| {
            ids.iter()
                .map(|id| {
                    js.iter()
                        .find(|j| j.id == *id)
                        .and_then(|j| match &j.state {
                            JobState::Done { result, .. } => Some(totals(result)),
                            _ => None,
                        })
                })
                .collect::<Vec<Option<Totals>>>()
        })
    });
    let rows = Memo::new(move |_| columns.with(|c| compare_rows(c)));
    view! {
        <section class="sl-compare">
            <table class="sl-compare-table">
                <thead>
                    <tr>
                        <th></th>
                        <For each=move || ids.get() key=|id| *id let:id>
                            <CompareHead id=id />
                        </For>
                    </tr>
                </thead>
                <tbody>
                    {move || rows.get().into_iter().map(|row| view! {
                        <tr>
                            <th class="nd-label">{row.label}</th>
                            {row.cells.iter().enumerate().map(|(i, cell)| {
                                let best = row.best == Some(i);
                                let delta = row.deltas.get(i).cloned().flatten();
                                view! {
                                    <td class:best=best>
                                        <span class="nd-mono">{cell.clone().unwrap_or_else(|| "—".into())}</span>
                                        {delta.map(|d| view! { <span class="sl-delta">{d}</span> })}
                                    </td>
                                }
                            }).collect::<Vec<_>>()}
                        </tr>
                    }).collect::<Vec<_>>()}
                </tbody>
            </table>
        </section>
    }
}

#[component]
fn CompareHead(id: u64) -> impl IntoView {
    let job = expect_context::<SlicerShared>().job(id);
    view! {
        <th>
            <span class="sl-col-name">{move || job.with(|j| j.as_ref().map(|j| j.filament.clone()).unwrap_or_default())}</span>
            <span class="nd-label sl-col-state">{move || job.with(|j| j.as_ref().map(status_line).unwrap_or_default())}</span>
        </th>
    }
}
