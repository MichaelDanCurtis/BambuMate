mod agent;
mod app;
mod commands;
mod components;
mod pages;
mod printer;
mod theme;

use app::App;

fn main() {
    leptos::mount::mount_to_body(App);
}
