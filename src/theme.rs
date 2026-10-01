use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct ThemeContext {
    pub theme: ReadSignal<String>,
    pub set_theme: WriteSignal<String>,
}

/// Normalize stored or incoming theme values to the supported set.
/// "light" and the legacy "bambu" both select the light theme; anything else
/// unknown also falls back to light so a corrupt value never hides the UI.
pub fn normalize_theme(theme: &str) -> &'static str {
    match theme {
        "dark" => "dark",
        _ => "bambu",
    }
}

/// Apply the theme by setting `data-theme` on `<html>`.
pub fn apply_theme(theme: &str) {
    if let Some(window) = web_sys::window() {
        if let Some(doc) = window.document() {
            if let Some(html) = doc.document_element() {
                let _ = html.set_attribute("data-theme", normalize_theme(theme));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_theme;

    #[test]
    fn dark_stays_dark() {
        assert_eq!(normalize_theme("dark"), "dark");
    }

    #[test]
    fn light_and_legacy_values_select_the_light_theme() {
        assert_eq!(normalize_theme("light"), "bambu");
        assert_eq!(normalize_theme("bambu"), "bambu");
        assert_eq!(normalize_theme("something-else"), "bambu");
    }
}
