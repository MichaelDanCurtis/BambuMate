mod app;
mod commands;
mod components;
mod pages;

use app::App;

fn main() {
    leptos::mount::mount_to_body(App);
}
