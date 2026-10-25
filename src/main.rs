mod advisor;
mod agent;
mod app;
mod commands;
mod components;
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
