use leptos::prelude::*;

#[component]
pub fn HealthPage() -> impl IntoView {
    let (checking, set_checking) = signal(false);
    let (results, set_results) = signal::<Option<Vec<(String, bool, String)>>>(None);

    // Placeholder health check -- will be wired to backend in Plan 02
    let run_check = move |_| {
        set_checking.set(true);
        set_results.set(Some(vec![
            ("Bambu Studio".to_string(), false, "Not yet checked".to_string()),
            ("Profile Directory".to_string(), false, "Not yet checked".to_string()),
            ("Claude API Key".to_string(), false, "Not yet checked".to_string()),
            ("OpenAI API Key".to_string(), false, "Not yet checked".to_string()),
        ]));
        set_checking.set(false);
    };

    view! {
        <div class="page health-page">
            <h2>"Health Check"</h2>
            <p class="page-description">
                "Verify that BambuMate can access all required services and directories."
            </p>

            <button
                class="btn btn-primary"
                on:click=run_check
                disabled=move || checking.get()
            >
                {move || if checking.get() { "Checking..." } else { "Run Health Check" }}
            </button>

            <div class="health-results">
                {move || {
                    results.get().map(|items| {
                        items.into_iter().map(|(name, passed, detail)| {
                            let status_class = if passed { "status-pass" } else { "status-fail" };
                            let icon = if passed { "OK" } else { "X" };
                            view! {
                                <div class="health-item">
                                    <span class={format!("status-badge {}", status_class)}>{icon}</span>
                                    <span class="health-name">{name}</span>
                                    <span class="health-detail">{detail}</span>
                                </div>
                            }
                        }).collect_view()
                    })
                }}
            </div>
        </div>
    }
}
