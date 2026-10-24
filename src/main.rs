// Task 7 builds the Advise page that uses the bridge, the formatting and the
// Search hand-off. Until then the placeholder page uses almost none of them;
// Task 7 removes this allow.
#[allow(dead_code)]
mod advisor;
mod agent;
mod app;
mod commands;
mod components;
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
