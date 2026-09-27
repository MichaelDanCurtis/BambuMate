//! Right-edge agent panel, toggled with the AGENT tab or Cmd/Ctrl+K.

use leptos::prelude::*;
use leptos_router::hooks::{use_location, use_navigate};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use super::bridge;
use super::cards::EntryView;
use super::state::{ChatState, Entry};
use super::types::{
    AgentEvent, AgentModel, AgentSettings, AppState, AuthMode, Provider, Readiness, UiCommand,
};

/// Bumped when the agent changes profiles, so open pages can reload.
#[derive(Clone, Copy)]
pub struct AgentRefresh(pub RwSignal<u32>);

fn provider_label(p: Provider) -> &'static str {
    match p {
        Provider::Codex => "CODEX",
        Provider::Claude => "CLAUDE AGENT",
    }
}

#[component]
pub fn AgentDrawer() -> impl IntoView {
    let open = RwSignal::new(false);
    let show_settings = RwSignal::new(false);
    let provider = RwSignal::new(Provider::Codex);
    let readiness = RwSignal::new(None::<Readiness>);
    let models = RwSignal::new(Vec::<AgentModel>::new());
    let model = RwSignal::new(None::<String>);
    let settings = RwSignal::new(None::<AgentSettings>);
    let chat = RwSignal::new(ChatState::default());
    let draft = RwSignal::new(String::new());
    let attachments = RwSignal::new(Vec::<(String, String)>::new()); // (display name, staged path)
    let refresh = use_context::<AgentRefresh>();

    // Backend events. The drawer mounts once, so these listeners live for the app's lifetime.
    bridge::listen::<AgentEvent>("agent://event", move |ev| chat.update(|c| c.apply(&ev)));
    let navigate = use_navigate();
    bridge::listen::<UiCommand>("agent://ui", move |cmd| match cmd {
        UiCommand::Navigate { route, .. } => navigate(&route, Default::default()),
        UiCommand::Refresh { .. } => {
            if let Some(r) = refresh {
                r.0.update(|n| *n += 1);
            }
        }
    });

    // Tell the backend what the user is looking at.
    let location = use_location();
    Effect::new(move |_| {
        let route = location.pathname.get();
        spawn_local(async move {
            bridge::set_app_state(&AppState {
                route,
                ..Default::default()
            })
            .await;
        });
    });

    let _keys = window_event_listener(leptos::ev::keydown, move |e| {
        if (e.meta_key() || e.ctrl_key()) && e.key().eq_ignore_ascii_case("k") {
            e.prevent_default();
            open.update(|o| *o = !*o);
        }
    });

    // Readiness + models whenever the drawer opens or the provider changes.
    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        let p = provider.get();
        readiness.set(None);
        spawn_local(async move {
            let r = bridge::readiness(p).await.ok();
            if matches!(r, Some(Readiness::Ready { .. })) {
                if let Ok(list) = bridge::models(p).await {
                    model.set(
                        list.iter()
                            .find(|m| m.is_default)
                            .or(list.first())
                            .map(|m| m.id.clone()),
                    );
                    models.set(list);
                }
            }
            readiness.set(r);
            if let Ok(s) = bridge::get_settings().await {
                settings.set(Some(s));
            }
        });
    });

    let switch_provider = move |p: Provider| {
        if provider.get_untracked() == p {
            return;
        }
        provider.set(p);
        chat.set(ChatState::default());
        chat.update(|c| {
            c.entries.push(Entry::Notice {
                text: format!("Switched to {}. New chat.", provider_label(p)),
                is_error: false,
            })
        });
    };

    let send = move || {
        let text = draft.get_untracked().trim().to_string();
        // Enter bypasses the disabled SEND button, so check readiness here too.
        let is_ready = readiness.with_untracked(|r| matches!(r, Some(Readiness::Ready { .. })));
        if text.is_empty() || !is_ready || chat.with_untracked(|c| c.running) {
            return;
        }
        let images: Vec<String> = attachments
            .get_untracked()
            .into_iter()
            .map(|(_, p)| p)
            .collect();
        draft.set(String::new());
        attachments.set(Vec::new());
        chat.update(|c| c.push_user(text.clone(), images.clone()));
        spawn_local(async move {
            let sid = match chat.with_untracked(|c| c.session_id.clone()) {
                Some(s) => s,
                None => match bridge::start(provider.get_untracked(), model.get_untracked(), None)
                    .await
                {
                    Ok(s) => {
                        chat.update(|c| c.session_id = Some(s.clone()));
                        s
                    }
                    Err(e) => {
                        chat.update(|c| {
                            c.entries.push(Entry::Notice {
                                text: e,
                                is_error: true,
                            })
                        });
                        return;
                    }
                },
            };
            if let Err(e) = bridge::send(sid, text, images).await {
                chat.update(|c| {
                    c.entries.push(Entry::Notice {
                        text: e,
                        is_error: true,
                    })
                });
            }
        });
    };

    let on_answer = Callback::new(move |(ask_id, answers): (String, Vec<String>)| {
        chat.update(|c| c.mark_answered(&ask_id, answers.clone()));
        spawn_local(async move {
            let _ = bridge::answer(ask_id, answers).await;
        });
    });

    let on_rewind = Callback::new(move |seq: u32| {
        let Some(sid) = chat.with_untracked(|c| c.session_id.clone()) else {
            return;
        };
        spawn_local(async move {
            match bridge::rewind(sid, seq).await {
                Ok(conversation) => chat.update(|c| {
                    c.rewind_to(seq);
                    let text = if conversation {
                        "Rewound profiles and conversation."
                    } else {
                        "Rewound profiles. The agent will be told on your next message."
                    };
                    c.entries.push(Entry::Notice {
                        text: text.into(),
                        is_error: false,
                    });
                }),
                Err(e) => chat.update(|c| {
                    c.entries.push(Entry::Notice {
                        text: e,
                        is_error: true,
                    })
                }),
            }
        });
    });

    let stage_files = move |files: web_sys::FileList| {
        for i in 0..files.length() {
            let Some(file) = files.get(i) else { continue };
            spawn_local(async move {
                let name = file.name();
                match crate::pages::print_analysis::read_file_as_base64(file).await {
                    Ok((_mime, b64)) => match bridge::stage_image(name.clone(), b64).await {
                        Ok(path) => attachments.update(|a| a.push((name, path))),
                        Err(e) => chat.update(|c| {
                            c.entries.push(Entry::Notice {
                                text: e,
                                is_error: true,
                            })
                        }),
                    },
                    Err(e) => chat.update(|c| {
                        c.entries.push(Entry::Notice {
                            text: e,
                            is_error: true,
                        })
                    }),
                }
            });
        }
    };

    let status_text = move || match readiness.get() {
        None => "[CHECKING…]".to_string(),
        Some(Readiness::Ready { detail }) => format!("READY · {detail}"),
        Some(Readiness::NeedsLogin { .. }) => "SIGN IN REQUIRED".to_string(),
        Some(Readiness::NeedsApiKey) => "API KEY REQUIRED".to_string(),
        Some(Readiness::NotInstalled { hint }) => format!("NOT INSTALLED · {hint}"),
    };
    let ready = move || matches!(readiness.get(), Some(Readiness::Ready { .. }));

    let usage_segments = move || {
        let used = chat.with(|c| c.used_percent).unwrap_or(0.0);
        let filled = ((used / 10.0).round() as usize).min(10);
        (0..10)
            .map(|i| view! { <span class="ag-seg" class:on={i < filled} class:hot={used >= 90.0}></span> })
            .collect_view()
    };

    view! {
        <button class="agent-toggle nd nd-label" on:click=move |_| open.update(|o| *o = !*o)
            title="Agent (Cmd/Ctrl+K)">"AGENT ⌘K"</button>
        <aside class="agent-drawer nd" class:open=move || open.get()
            on:dragover=|e| e.prevent_default()
            on:drop=move |e: web_sys::DragEvent| {
                e.prevent_default();
                if let Some(files) = e.data_transfer().and_then(|dt| dt.files()) {
                    stage_files(files);
                }
            }>
            <header class="ag-header">
                <div class="ag-provider">
                    {[Provider::Codex, Provider::Claude].into_iter().map(|p| view! {
                        <button data-provider=move || if p == Provider::Codex { "codex" } else { "claude" }
                            class="nd-label" class:active=move || provider.get() == p
                            on:click=move |_| switch_provider(p)>{provider_label(p)}</button>
                    }).collect_view()}
                    <button class="ag-gear nd-label" on:click=move |_| show_settings.update(|s| *s = !*s)>"SETTINGS"</button>
                    // The open drawer covers the AGENT tab, so it needs its own close control.
                    <button class="ag-close nd-label" title="Close (Cmd/Ctrl+K)" on:click=move |_| open.set(false)>"CLOSE"</button>
                </div>
                <p class="ag-status nd-mono">{status_text}</p>
                <Show when=move || matches!(readiness.get(), Some(Readiness::NeedsLogin { .. }))>
                    <button class="ag-login" on:click=move |_| {
                        let p = provider.get_untracked();
                        spawn_local(async move {
                            let _ = bridge::login(p).await;
                        });
                    }>"Sign in"</button>
                </Show>
                <Show when=move || matches!(readiness.get(), Some(Readiness::NeedsApiKey))>
                    <a class="ag-login" href="/settings">"Add an Anthropic API key in Settings"</a>
                </Show>
                <Show when=ready>
                    <div class="ag-model-row">
                        <select class="ag-model nd-mono" on:change=move |e| model.set(Some(event_target_value(&e)))>
                            {move || models.get().into_iter().map(|m| {
                                let selected = model.get().as_deref() == Some(m.id.as_str());
                                view! { <option value=m.id.clone() selected=selected>{m.display_name.clone()}</option> }
                            }).collect_view()}
                        </select>
                        <div class="ag-usage" title="Plan usage">{usage_segments}</div>
                    </div>
                </Show>
                <Show when=move || show_settings.get()>
                    <div class="ag-settings">
                        <label class="nd-label">
                            <input type="checkbox"
                                prop:checked=move || settings.get().map(|s| s.full_access).unwrap_or(false)
                                on:change=move |e| {
                                    let on = event_target_checked(&e);
                                    let mode = settings.get_untracked().map(|s| s.claude_auth_mode).unwrap_or(AuthMode::ApiKey);
                                    spawn_local(async move {
                                        if let Ok(s) = bridge::set_settings(on, mode).await { settings.set(Some(s)); }
                                    });
                                } />
                            " FULL ACCESS (agent may touch files outside the profile folder)"
                        </label>
                        <Show when=move || settings.get().map(|s| s.claude_auth_modes.len() > 1).unwrap_or(false)>
                            <label class="nd-label">
                                <input type="checkbox"
                                    prop:checked=move || settings.get().map(|s| s.claude_auth_mode == AuthMode::Subscription).unwrap_or(false)
                                    on:change=move |e| {
                                        let sub = event_target_checked(&e);
                                        let full = settings.get_untracked().map(|s| s.full_access).unwrap_or(false);
                                        let mode = if sub { AuthMode::Subscription } else { AuthMode::ApiKey };
                                        spawn_local(async move {
                                            if let Ok(s) = bridge::set_settings(full, mode).await { settings.set(Some(s)); }
                                        });
                                    } />
                                " CLAUDE: USE MY SUBSCRIPTION (private build)"
                            </label>
                        </Show>
                    </div>
                </Show>
            </header>

            <section class="ag-stream">
                {move || chat.with(|c| c.entries.clone()).into_iter().map(|entry| view! {
                    <EntryView entry=entry on_answer=on_answer on_rewind=on_rewind />
                }).collect_view()}
            </section>

            <Show when=move || chat.with(|c| !c.todos.is_empty())>
                <ol class="ag-todo">
                    {move || chat.with(|c| c.todos.clone()).into_iter().map(|t| view! {
                        <li class:done=t.done><span class="nd-mono">{if t.done { "[x] " } else { "[ ] " }}</span>{t.text}</li>
                    }).collect_view()}
                </ol>
            </Show>

            <footer class="ag-composer">
                <Show when=move || !attachments.get().is_empty()>
                    <p class="ag-attachments nd-label">
                        {move || attachments.get().into_iter().map(|(n, _)| n).collect::<Vec<_>>().join(" · ")}
                    </p>
                </Show>
                <textarea class="ag-input" rows="3"
                    placeholder="Ask about a print, a filament, or a profile. Drop photos here."
                    prop:value=move || draft.get()
                    on:input=move |e| draft.set(event_target_value(&e))
                    on:keydown=move |e: web_sys::KeyboardEvent| {
                        if e.key() == "Enter" && !e.shift_key() {
                            e.prevent_default();
                            send();
                        }
                    }></textarea>
                <div class="ag-actions">
                    <label class="ag-attach nd-label">
                        "PHOTO"
                        <input type="file" accept="image/jpeg,image/png,image/webp" multiple hidden
                            on:change=move |e| {
                                let input: web_sys::HtmlInputElement = e.target().unwrap().unchecked_into();
                                if let Some(files) = input.files() { stage_files(files); }
                                input.set_value("");
                            } />
                    </label>
                    <Show when=move || chat.with(|c| c.running)
                        fallback=move || view! {
                            <button class="ag-send" disabled=move || !ready() on:click=move |_| send()>"SEND"</button>
                        }>
                        <button class="ag-stop" on:click=move |_| {
                            if let Some(sid) = chat.with_untracked(|c| c.session_id.clone()) {
                                spawn_local(async move { let _ = bridge::interrupt(sid).await; });
                            }
                        }>"STOP"</button>
                    </Show>
                </div>
            </footer>
        </aside>
    }
}
