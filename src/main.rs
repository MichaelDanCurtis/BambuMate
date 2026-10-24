mod advisor;
mod agent;
mod app;
mod commands;
mod components;
// Task 7 (the Journal page) consumes these items; it removes this allow.
#[allow(dead_code)]
mod journal;
mod pages;
mod printer;
mod routes;
mod slicer;
mod theme;
mod viewport;

use app::App;

fn main() {
    leptos::mount::mount_to_body(App);
}
