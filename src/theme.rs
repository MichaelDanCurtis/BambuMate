use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct ThemeContext {
    pub theme: ReadSignal<String>,
    pub set_theme: WriteSignal<String>,
}

/// Apply the theme by setting or removing the `data-theme` attribute on `<html>`.
/// - "light" → forces light
/// - "dark" → forces dark
/// - anything else ("system") → removes attribute, CSS @media handles it
pub fn apply_theme(theme: &str) {
    if let Some(window) = web_sys::window() {
        if let Some(doc) = window.document() {
            if let Some(html) = doc.document_element() {
                match theme {
                    "light" => {
                        let _ = html.set_attribute("data-theme", "light");
                    }
                    "dark" => {
                        let _ = html.set_attribute("data-theme", "dark");
                    }
                    _ => {
                        let _ = html.remove_attribute("data-theme");
                    }
                }
            }
        }
    }
}
