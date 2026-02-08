use leptos::prelude::*;

/// A single option in the searchable select.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
    pub group: String,
}

/// A searchable dropdown select component.
///
/// Replaces a native `<select>` with a text input that filters options
/// and a dropdown list grouped by category.
#[component]
pub fn SearchableSelect(
    /// Unique ID for this select instance.
    id: &'static str,
    /// Placeholder text shown when nothing is selected.
    placeholder: &'static str,
    /// All available options.
    options: Signal<Vec<SelectOption>>,
    /// The currently selected value.
    value: ReadSignal<String>,
    /// Callback when a value is selected.
    on_select: impl Fn(String) + 'static + Copy + Send + Sync,
) -> impl IntoView {
    let (is_open, set_is_open) = signal(false);
    let (search_text, set_search_text) = signal(String::new());

    // Get display label for current selection
    let display_label = move || {
        let val = value.get();
        if val.is_empty() {
            return String::new();
        }
        options
            .get()
            .iter()
            .find(|o| o.value == val)
            .map(|o| o.label.clone())
            .unwrap_or(val)
    };

    // Filter and group options based on search text
    let grouped_options = move || {
        let query = search_text.get().to_lowercase();
        let all = options.get();

        let filtered: Vec<_> = if query.is_empty() {
            all
        } else {
            all.into_iter()
                .filter(|o| o.label.to_lowercase().contains(&query))
                .collect()
        };

        let total = filtered.len();
        let mut groups: Vec<(String, Vec<SelectOption>)> = Vec::new();
        for opt in filtered {
            if let Some(g) = groups.iter_mut().find(|(name, _)| *name == opt.group) {
                g.1.push(opt);
            } else {
                groups.push((opt.group.clone(), vec![opt]));
            }
        }
        (groups, total)
    };

    let on_input_focus = move |_: leptos::ev::FocusEvent| {
        set_is_open.set(true);
        set_search_text.set(String::new());
    };

    let on_input_change = move |ev: leptos::ev::Event| {
        set_search_text.set(event_target_value(&ev));
        set_is_open.set(true);
    };

    let on_select_option = move |val: String| {
        on_select(val);
        set_is_open.set(false);
        set_search_text.set(String::new());
    };

    let on_clear = move |ev: leptos::ev::MouseEvent| {
        ev.prevent_default();
        ev.stop_propagation();
        on_select(String::new());
        set_search_text.set(String::new());
    };

    // Close dropdown when clicking outside
    let container_ref = NodeRef::<leptos::html::Div>::new();

    Effect::new(move |_| {
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        if !is_open.get() {
            return;
        }

        let el = container_ref.get();
        if el.is_none() {
            return;
        }
        let container = el.unwrap();

        let closure = Closure::<dyn Fn(web_sys::MouseEvent)>::new(move |ev: web_sys::MouseEvent| {
            if let Some(target) = ev.target() {
                if let Some(node) = target.dyn_ref::<web_sys::Node>() {
                    if !container.contains(Some(node)) {
                        set_is_open.set(false);
                    }
                }
            }
        });

        let window = web_sys::window().unwrap();
        let _ = window.add_event_listener_with_callback(
            "mousedown",
            closure.as_ref().unchecked_ref(),
        );

        closure.forget();
    });

    let dropdown_id = format!("{}-dropdown", id);

    view! {
        <div
            class="searchable-select"
            class:open=move || is_open.get()
            node_ref=container_ref
        >
            <style>{include_str!("searchable_select.css")}</style>

            {move || {
                if is_open.get() {
                    // Open state: show search input
                    view! {
                        <input
                            type="text"
                            class="ss-search input"
                            placeholder="Type to search..."
                            prop:value=move || search_text.get()
                            on:input=on_input_change
                            on:focus=on_input_focus
                            autofocus=true
                        />
                    }.into_any()
                } else {
                    // Closed state: show selected value or placeholder
                    let has_value = !value.get().is_empty();
                    let label = display_label();
                    let display_text = if label.is_empty() {
                        placeholder.to_string()
                    } else {
                        label
                    };

                    if has_value {
                        view! {
                            <div
                                class="ss-display has-value"
                                on:click=move |_| { set_is_open.set(true); set_search_text.set(String::new()); }
                            >
                                <span class="ss-display-text">{display_text}</span>
                                <button
                                    class="ss-clear"
                                    on:click=on_clear
                                    title="Clear selection"
                                >
                                    "\u{2715}"
                                </button>
                                <span class="ss-chevron">"\u{25BE}"</span>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div
                                class="ss-display"
                                on:click=move |_| { set_is_open.set(true); set_search_text.set(String::new()); }
                            >
                                <span class="ss-display-text">{display_text}</span>
                                <span class="ss-chevron">"\u{25BE}"</span>
                            </div>
                        }.into_any()
                    }
                }
            }}

            {move || {
                if !is_open.get() {
                    return view! { <div style="display:none"></div> }.into_any();
                }

                let (groups, count) = grouped_options();

                if count == 0 {
                    return view! {
                        <div class="ss-dropdown">
                            <div class="ss-empty">"No matching profiles"</div>
                        </div>
                    }.into_any();
                }

                let group_views: Vec<_> = groups.into_iter().map(|(group_name, items)| {
                    let item_views: Vec<_> = items.into_iter().map(|opt| {
                        let val = opt.value.clone();
                        let is_selected = value.get() == val;
                        let selected_class = if is_selected { "ss-option selected" } else { "ss-option" };
                        view! {
                            <div
                                class={selected_class}
                                on:mousedown=move |_| on_select_option(val.clone())
                            >
                                {opt.label}
                            </div>
                        }
                    }).collect();

                    view! {
                        <div class="ss-group">
                            <div class="ss-group-label">{group_name}</div>
                            {item_views}
                        </div>
                    }
                }).collect();

                let count_label = format!("{} profile{}", count, if count == 1 { "" } else { "s" });

                view! {
                    <div class="ss-dropdown" id={dropdown_id.clone()}>
                        <div class="ss-options">
                            {group_views}
                            <div class="ss-count">{count_label}</div>
                        </div>
                    </div>
                }.into_any()
            }}
        </div>
    }
}
