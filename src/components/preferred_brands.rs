//! Settings → General: the brands Filament → Advise lists first, after the
//! AMS (spec 2026-10-23 §2). Saved on every toggle. The picker shows the
//! toggle at once, then settles on what the backend confirmed: a failed save
//! puts the last confirmed set back.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::advisor::bridge;
use crate::advisor::format::{toggle_brand, CATALOG_EMPTY};
use crate::commands;
use crate::components::patterns::{state_text, StateKind, StateText};

/// Shown when a save fails. Fixed text: the raw error says nothing the user
/// can act on.
pub const SAVE_FAILED: &str = "Couldn't save your preferred brands.";
/// Shown instead of the picker while the stored set can't be read. Toggling
/// without it would write a list built from nothing and wipe the stored one.
pub const READ_FAILED: &str = "Couldn't read your preferred brands.";
/// Shown when the catalog's brands can't be listed.
pub const BRANDS_FAILED: &str = "Couldn't load the catalog's brands.";

/// What the status line under the picker says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Quiet,
    Saved,
    Failed,
}

/// The save bookkeeping, apart from the signals so it can be tested. Each
/// toggle is a request with a rising id; `confirmed` is the set the backend
/// last said it stored, taken from the newest request that succeeded.
#[derive(Debug, Default)]
struct BrandSync {
    confirmed: Vec<String>,
    confirmed_id: u64,
    issued: u64,
    pending: u32,
    latest_ok: bool,
}

/// What the picker does after an answer: `show` is the set to display now.
#[derive(Debug, PartialEq, Eq)]
struct Settled {
    show: Option<Vec<String>>,
    status: Option<Status>,
}

impl BrandSync {
    /// The stored set, read when the picker opened.
    fn loaded(&mut self, stored: Vec<String>) {
        self.confirmed = stored;
    }

    /// A toggle is about to be sent; returns its id.
    fn issue(&mut self) -> u64 {
        self.issued += 1;
        self.pending += 1;
        self.latest_ok = true;
        self.issued
    }

    /// The answer to request `id`. A success always advances `confirmed` (if
    /// no newer request already did), even when a newer toggle has been made
    /// since, so a dropped success is not lost if the newer save then fails.
    /// The view changes when the newest request answers, and again when the
    /// last request in flight answers, so what is shown, what is stored and
    /// what the status says agree once the picker is idle.
    fn settle(&mut self, id: u64, result: Result<Vec<String>, ()>) -> Settled {
        self.pending = self.pending.saturating_sub(1);
        let ok = result.is_ok();
        if let Ok(list) = result {
            if id > self.confirmed_id {
                self.confirmed = list;
                self.confirmed_id = id;
            }
        }
        if id == self.issued {
            self.latest_ok = ok;
        }
        if id == self.issued || self.pending == 0 {
            Settled {
                show: Some(self.confirmed.clone()),
                status: Some(if self.latest_ok {
                    Status::Saved
                } else {
                    Status::Failed
                }),
            }
        } else {
            Settled {
                show: None,
                status: None,
            }
        }
    }
}

#[component]
pub fn PreferredBrands() -> impl IntoView {
    // The catalog's brands; `None` until listed.
    let brands = RwSignal::new(None::<Vec<String>>);
    let chosen = RwSignal::new(Vec::<String>::new());
    // Why the picker can't be shown (a fixed sentence); `None` while it can.
    let load_error = RwSignal::new(None::<&'static str>);
    // True once the stored set has been read: toggles are not offered before.
    let stored_read = RwSignal::new(false);
    let status = RwSignal::new(Status::Quiet);
    let sync = StoredValue::new(BrandSync::default());

    let load = move || {
        load_error.set(None);
        spawn_local(async move {
            let mine = bridge::get_preferred_brands().await;
            let all = commands::list_catalog_brands().await;
            match mine {
                Ok(list) => {
                    if sync.try_update_value(|s| s.loaded(list.clone())).is_none() {
                        return;
                    }
                    chosen.try_set(list);
                    stored_read.try_set(true);
                }
                Err(_) => {
                    load_error.try_set(Some(READ_FAILED));
                }
            }
            match all {
                Ok(list) => {
                    brands.try_set(Some(list));
                }
                Err(_) => {
                    // Keep the first problem if there is one.
                    if load_error.try_get_untracked().flatten().is_none() {
                        load_error.try_set(Some(BRANDS_FAILED));
                    }
                }
            }
        });
    };
    load();

    let toggle = move |brand: String| {
        let next = toggle_brand(&chosen.get_untracked(), &brand);
        chosen.set(next.clone());
        status.set(Status::Quiet);
        let id = sync.try_update_value(|s| s.issue()).unwrap_or(0);
        spawn_local(async move {
            let answer = bridge::set_preferred_brands(&next).await;
            let Some(settled) = sync.try_update_value(|s| s.settle(id, answer.map_err(|_| ())))
            else {
                return;
            };
            if let Some(show) = settled.show {
                chosen.try_set(show);
            }
            if let Some(s) = settled.status {
                status.try_set(s);
            }
        });
    };

    view! {
        <section class="settings-section preferred-brands">
            <h2 class="nd-label settings-section-title">"Preferred brands"</h2>
            <p class="section-description">
                "Filament → Advise lists these brands' products right after what's loaded in your AMS."
            </p>
            {move || match (load_error.get(), brands.get(), stored_read.get()) {
                (Some(e), _, _) => view! {
                    <StateText kind=StateKind::Error message=e />
                    <button type="button" class="btn btn-secondary btn-sm pb-retry" on:click=move |_| load()>
                        "Try again"
                    </button>
                }
                .into_any(),
                (None, Some(list), true) if list.is_empty() => view! {
                    <StateText kind=StateKind::Empty message=CATALOG_EMPTY />
                }
                .into_any(),
                (None, Some(list), true) => view! {
                    <div class="pb-brands" role="group" aria-label="Preferred brands">
                        {list.into_iter().map(|b| {
                            let key = b.clone();
                            let label = b.clone();
                            view! {
                                <button type="button" class="nd-tab pb-brand"
                                    aria-pressed=move || chosen.with(|c| c.iter().any(|x| x.eq_ignore_ascii_case(&key))).to_string()
                                    on:click=move |_| toggle(b.clone())>
                                    {label}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                }
                .into_any(),
                (None, _, _) => view! { <StateText kind=StateKind::Loading /> }.into_any(),
            }}
            // Mounted once and empty, so a screen reader announces the text
            // that arrives in it.
            <p class="nd-state pb-status" role="status"
                data-kind=move || match status.get() {
                    Status::Quiet => "",
                    Status::Saved => "saved",
                    Status::Failed => "error",
                }>
                {move || match status.get() {
                    Status::Quiet => String::new(),
                    Status::Saved => "[SAVED]".to_string(),
                    Status::Failed => state_text(StateKind::Error, Some(SAVE_FAILED)),
                }}
            </p>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn synced(stored: &[&str]) -> BrandSync {
        let mut s = BrandSync::default();
        s.loaded(l(stored));
        s
    }

    #[test]
    fn a_success_shows_what_the_backend_stored() {
        let mut s = synced(&[]);
        let id = s.issue();
        let r = s.settle(id, Ok(l(&["Polymaker"])));
        assert_eq!(r.show, Some(l(&["Polymaker"])));
        assert_eq!(r.status, Some(Status::Saved));
    }

    #[test]
    fn a_failure_puts_the_last_confirmed_set_back() {
        let mut s = synced(&["Bambu Lab"]);
        let id = s.issue();
        let r = s.settle(id, Err(()));
        assert_eq!(r.show, Some(l(&["Bambu Lab"])));
        assert_eq!(r.status, Some(Status::Failed));
    }

    #[test]
    fn a_dropped_success_then_a_failed_latest_reverts_to_that_success() {
        // Toggle A saves, but B (the newest) answers first and fails.
        let mut s = synced(&[]);
        let a = s.issue();
        let b = s.issue();
        let r = s.settle(b, Err(()));
        assert_eq!(r.show, Some(l(&[])));
        assert_eq!(r.status, Some(Status::Failed));
        // A's success arrives last: the view moves to what is stored.
        let r = s.settle(a, Ok(l(&["Polymaker"])));
        assert_eq!(r.show, Some(l(&["Polymaker"])));
        assert_eq!(r.status, Some(Status::Failed));
    }

    #[test]
    fn a_dropped_success_before_a_failed_latest_is_kept() {
        let mut s = synced(&[]);
        let a = s.issue();
        let b = s.issue();
        assert_eq!(
            s.settle(a, Ok(l(&["Polymaker"]))),
            Settled {
                show: None,
                status: None
            }
        );
        let r = s.settle(b, Err(()));
        assert_eq!(r.show, Some(l(&["Polymaker"])));
        assert_eq!(r.status, Some(Status::Failed));
    }

    #[test]
    fn the_newest_success_wins_whatever_order_answers_arrive() {
        let mut s = synced(&[]);
        let a = s.issue();
        let b = s.issue();
        let r = s.settle(b, Ok(l(&["Polymaker", "Prusament"])));
        assert_eq!(r.show, Some(l(&["Polymaker", "Prusament"])));
        // The older answer arrives late and must not roll the view back.
        let r = s.settle(a, Ok(l(&["Polymaker"])));
        assert_eq!(r.show, Some(l(&["Polymaker", "Prusament"])));
        assert_eq!(r.status, Some(Status::Saved));
    }

    #[test]
    fn a_failed_older_save_beside_a_good_latest_one_is_not_an_error() {
        let mut s = synced(&[]);
        let a = s.issue();
        let b = s.issue();
        s.settle(b, Ok(l(&["Prusament"])));
        let r = s.settle(a, Err(()));
        assert_eq!(r.show, Some(l(&["Prusament"])));
        assert_eq!(r.status, Some(Status::Saved));
    }
}
