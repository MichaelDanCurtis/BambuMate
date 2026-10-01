//! Slicing with Bambu Studio: shared job state fed by `slicer://job`.

pub mod bridge;
pub mod state;
pub mod types;

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use types::JobView;

/// Every job the app knows, by ascending id. `App` provides it once, so
/// the Slice page and the STL indicator see the same jobs and events that
/// arrive while the page is closed aren't lost.
#[derive(Clone, Copy)]
pub struct SlicerShared {
    pub jobs: RwSignal<Vec<JobView>>,
}

impl Default for SlicerShared {
    fn default() -> Self {
        Self {
            jobs: RwSignal::new(Vec::new()),
        }
    }
}

impl SlicerShared {
    /// One job, as a memo so a row only re-renders when that job changes.
    pub fn job(self, id: u64) -> Memo<Option<JobView>> {
        let jobs = self.jobs;
        Memo::new(move |_| jobs.with(|js| js.iter().find(|j| j.id == id).cloned()))
    }
}

/// Registers the `slicer://job` listener and loads the jobs the backend
/// already has. The listener can't be removed, so this is mounted exactly
/// once, outside anything that can unmount.
#[component]
pub fn SlicerEvents() -> impl IntoView {
    let SlicerShared { jobs } = expect_context::<SlicerShared>();
    // Events for a job arrive in order, so the newest event always wins.
    crate::agent::bridge::listen::<JobView>("slicer://job", move |view| {
        jobs.try_update(|js| state::upsert(js, view));
    });
    spawn_local(async move {
        if let Ok(list) = bridge::jobs().await {
            // The snapshot may be older than events that arrived meanwhile.
            jobs.try_update(|js| {
                for v in list {
                    state::insert_if_absent(js, v);
                }
            });
        }
    });
}
