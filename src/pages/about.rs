use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, UpdateInfo};
use crate::components::branding::BrandMark;

#[component]
pub fn AboutPage() -> impl IntoView {
    let (current_version, set_current_version) = signal(String::from("…"));
    let (update_info, set_update_info) = signal::<Option<UpdateInfo>>(None);
    let (checking, set_checking) = signal(false);
    let (check_error, set_check_error) = signal::<Option<String>>(None);

    // Load current version on mount
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(info) = commands::get_app_version().await {
                set_current_version.set(info.current_version);
            }
        });
    });

    let check_updates = move |_| {
        set_checking.set(true);
        set_check_error.set(None);
        set_update_info.set(None);
        spawn_local(async move {
            match commands::check_for_updates().await {
                Ok(info) => set_update_info.set(Some(info)),
                Err(e) => set_check_error.set(Some(e)),
            }
            set_checking.set(false);
        });
    };

    let open_releases = move |_| {
        spawn_local(async move {
            let _ = commands::open_external_url(
                "https://github.com/MichaelDanCurtis/BambuMate/releases",
            )
            .await;
        });
    };

    view! {
        <div class="page about-page">
            <div class="about-header">
                <div class="about-logo">
                    <BrandMark />
                </div>
                <h2 class="about-title">"BambuMate"</h2>
                <p class="about-subtitle">"Studio-inspired filament workflow"</p>
            </div>

            <div class="about-version-card">
                <div class="about-version-row">
                    <span class="about-version-label">"Current Version"</span>
                    <span class="about-version-value">
                        {move || format!("v{}", current_version.get())}
                    </span>
                </div>

                {move || {
                    update_info.get().map(|info| {
                        if info.has_update {
                            let download_url = info.release_url.clone();
                            let notes = info.release_notes.clone();
                            view! {
                                <div class="about-update-available">
                                    <div class="about-update-header">
                                        <span class="about-update-badge">"✨ Update Available"</span>
                                        <span class="about-version-value">
                                            {format!("v{}", info.latest_version)}
                                        </span>
                                    </div>
                                    {notes.map(|n| view! {
                                        <p class="about-release-notes">{n}</p>
                                    })}
                                    <button
                                        class="btn btn-primary about-download-btn"
                                        on:click=move |_| {
                                            let url = download_url.clone();
                                            spawn_local(async move {
                                               let _ = commands::open_external_url(&url).await;
                                            });
                                        }
                                    >
                                        "Download Update"
                                    </button>
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div class="about-up-to-date">
                                    <span class="about-check-icon">"✅"</span>
                                    " You're up to date!"
                                </div>
                            }.into_any()
                        }
                    })
                }}

                {move || {
                    check_error.get().map(|e| view! {
                        <div class="about-check-error">
                            <span class="status-text status-error">
                                {format!("Could not check for updates: {}", e)}
                            </span>
                        </div>
                    })
                }}

                <div class="about-update-actions">
                    <button
                        class="btn btn-secondary"
                        on:click=check_updates
                        disabled=move || checking.get()
                    >
                        {move || if checking.get() { "Checking…" } else { "Check for Updates" }}
                    </button>
                    <button class="btn btn-ghost" on:click=open_releases>
                        "View All Releases"
                    </button>
                </div>
            </div>

            <div class="about-links">
                <h3>"Links"</h3>
                <ul class="about-link-list">
                    <li>
                        <button
                            class="about-link-btn"
                            on:click=move |_| {
                                spawn_local(async move {
                                    let _ = commands::open_external_url(
                                        "https://github.com/MichaelDanCurtis/BambuMate",
                                    ).await;
                                });
                            }
                        >
                            "🐙  GitHub Repository"
                        </button>
                    </li>
                    <li>
                        <button
                            class="about-link-btn"
                            on:click=move |_| {
                                spawn_local(async move {
                                    let _ = commands::open_external_url(
                                        "https://github.com/MichaelDanCurtis/BambuMate/issues",
                                    ).await;
                                });
                            }
                        >
                            "🐛  Report an Issue"
                        </button>
                    </li>
                </ul>
            </div>
        </div>
    }
}
