//! One view per chat entry. Nothing styling: labels in Space Mono caps,
//! status as bracketed text, red only for failures.

use leptos::prelude::*;

use super::state::{ActivityKind, Entry};

fn kind_label(k: ActivityKind) -> &'static str {
    match k {
        ActivityKind::Tool => "TOOL",
        ActivityKind::File => "FILE",
        ActivityKind::Command => "CMD",
        ActivityKind::Search => "WEB",
        ActivityKind::Image => "IMAGE",
    }
}

#[component]
pub fn EntryView(
    entry: Entry,
    on_answer: Callback<(String, Vec<String>)>,
    on_rewind: Callback<u32>,
    /// The user's answer on a rewind confirm card: (seq, go ahead).
    on_rewind_decide: Callback<(u32, bool)>,
    /// A turn is running: rewinding is refused by the backend, so disable it.
    #[prop(into)]
    busy: Signal<bool>,
) -> impl IntoView {
    match entry {
        Entry::User { seq, text, images } => view! {
            <div class="ag-user">
                <p class="ag-user-text">{text}</p>
                {(!images.is_empty()).then(|| view! {
                    <p class="nd-label">{format!("{} PHOTO(S) ATTACHED", images.len())}</p>
                })}
                {seq.map(|s| view! {
                    <button class="ag-rewind nd-label" title="Restore profiles and conversation to before this message"
                        disabled=move || busy.get() on:click=move |_| on_rewind.run(s)>"REWIND"</button>
                })}
            </div>
        }
        .into_any(),
        Entry::Agent { text, done, .. } => view! {
            <div class="ag-agent" class:ag-streaming=!done>{text}</div>
        }
        .into_any(),
        Entry::Activity { kind, title, detail, ok, .. } => {
            let status = match ok {
                None => "[…]",
                Some(true) => "[OK]",
                Some(false) => "[FAIL]",
            };
            view! {
                <div class="ag-activity" class:ag-fail={ok == Some(false)}>
                    <div class="ag-activity-head">
                        <span class="nd-label">{kind_label(kind)}</span>
                        <span class="ag-activity-title nd-mono">{title}</span>
                        <span class="ag-activity-status nd-mono">{status}</span>
                    </div>
                    {(!detail.is_empty()).then(|| view! {
                        <details class="ag-activity-detail"><summary class="nd-label">"DETAIL"</summary><pre>{detail}</pre></details>
                    })}
                </div>
            }
            .into_any()
        }
        Entry::Ask { request, answered } => {
            let id = request.id.clone();
            let allow_other = request.allow_other;
            let other = RwSignal::new(String::new());
            let submit_other = {
                let id = id.clone();
                move || {
                    let text = other.get_untracked().trim().to_string();
                    if !text.is_empty() {
                        on_answer.run((id.clone(), vec![text]));
                    }
                }
            };
            let submit_on_enter = submit_other.clone();
            view! {
                <div class="ag-ask">
                    <p class="nd-label">{request.header.to_uppercase()}</p>
                    <p class="ag-ask-question">{request.question.clone()}</p>
                    {match answered {
                        Some(a) => view! { <p class="nd-label">{format!("[ANSWERED: {}]", a.join(", "))}</p> }.into_any(),
                        None => view! {
                            <div class="ag-ask-options">
                                {request.options.iter().map(|o| {
                                    let (id, label) = (id.clone(), o.label.clone());
                                    view! {
                                        <button class="ag-option" title=o.description.clone()
                                            on:click=move |_| on_answer.run((id.clone(), vec![label.clone()]))>
                                            {o.label.clone()}
                                        </button>
                                    }
                                }).collect_view()}
                                {allow_other.then(|| view! {
                                    <div class="ag-ask-other">
                                        <input class="ag-other-input" type="text" placeholder="Or type an answer"
                                            prop:value=move || other.get()
                                            on:input=move |e| other.set(event_target_value(&e))
                                            on:keydown=move |e: web_sys::KeyboardEvent| {
                                                if e.key() == "Enter" {
                                                    e.prevent_default();
                                                    submit_on_enter();
                                                }
                                            } />
                                        <button class="ag-other-send nd-label"
                                            disabled=move || other.with(|t| t.trim().is_empty())
                                            on:click=move |_| submit_other()>"SEND"</button>
                                    </div>
                                })}
                            </div>
                        }.into_any(),
                    }}
                </div>
            }
            .into_any()
        }
        Entry::RewindConfirm { seq, delete, overwrite } => {
            let nothing = delete.is_empty() && overwrite.is_empty();
            let list = |label: &'static str, names: Vec<String>| {
                (!names.is_empty()).then(|| view! {
                    <p class="nd-label">{label}</p>
                    <ul class="ag-confirm-files nd-mono">
                        {names.into_iter().map(|n| view! { <li>{n}</li> }).collect_view()}
                    </ul>
                })
            };
            view! {
                <div class="ag-ask ag-confirm">
                    <p class="nd-label">{format!("REWIND TO BEFORE MESSAGE {seq}?")}</p>
                    {list("WILL DELETE", delete)}
                    {list("WILL OVERWRITE", overwrite)}
                    <p class="ag-ask-question">
                        {if nothing {
                            "No profile files change. A copy of the current files is kept first."
                        } else {
                            "A copy of the current files is kept first."
                        }}
                    </p>
                    <div class="ag-ask-options">
                        <button class="ag-option ag-confirm-rewind" disabled=move || busy.get()
                            on:click=move |_| on_rewind_decide.run((seq, true))>"Rewind"</button>
                        <button class="ag-option ag-confirm-cancel"
                            on:click=move |_| on_rewind_decide.run((seq, false))>"Cancel"</button>
                    </div>
                </div>
            }
            .into_any()
        }
        Entry::Notice { text, is_error } => view! {
            <p class="ag-notice nd-mono" class:ag-error=is_error>{text}</p>
        }
        .into_any(),
    }
}
