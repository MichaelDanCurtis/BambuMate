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
                            </div>
                        }.into_any(),
                    }}
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
