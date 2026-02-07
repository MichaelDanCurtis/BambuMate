use leptos::prelude::*;

use crate::commands::FilamentSpecs;

#[component]
pub fn FilamentCard(
    specs: FilamentSpecs,
    #[prop(into)] on_generate: Callback<()>,
    #[prop(default = false)] generating: bool,
) -> impl IntoView {
    let confidence_pct = (specs.extraction_confidence * 100.0) as u8;
    let confidence_class = if confidence_pct >= 70 {
        "confidence-high"
    } else if confidence_pct >= 40 {
        "confidence-medium"
    } else {
        "confidence-low"
    };

    // Format temperature ranges
    let nozzle_temp = match (specs.nozzle_temp_min, specs.nozzle_temp_max) {
        (Some(min), Some(max)) => format!("{}-{}C", min, max),
        (None, Some(max)) => format!("{}C", max),
        (Some(min), None) => format!("{}C+", min),
        (None, None) => "--".to_string(),
    };

    let bed_temp = match (specs.bed_temp_min, specs.bed_temp_max) {
        (Some(min), Some(max)) => format!("{}-{}C", min, max),
        (None, Some(max)) => format!("{}C", max),
        (Some(min), None) => format!("{}C+", min),
        (None, None) => "--".to_string(),
    };

    let max_speed = specs
        .max_speed_mm_s
        .map(|s| format!("{} mm/s", s))
        .unwrap_or_else(|| "--".to_string());

    let fan_speed = match (specs.fan_min_speed, specs.fan_max_speed) {
        (Some(min), Some(max)) if min != max => format!("{}-{}%", min, max),
        (_, Some(max)) => format!("{}%", max),
        (Some(min), None) => format!("{}%", min),
        (None, None) => specs
            .fan_speed_percent
            .map(|f| format!("{}%", f))
            .unwrap_or_else(|| "--".to_string()),
    };

    let retraction = match (specs.retraction_distance_mm, specs.retraction_speed_mm_s) {
        (Some(dist), Some(spd)) => format!("{:.1}mm @ {}mm/s", dist, spd),
        (Some(dist), None) => format!("{:.1}mm", dist),
        (None, Some(spd)) => format!("@ {}mm/s", spd),
        (None, None) => "--".to_string(),
    };

    let density = specs
        .density_g_cm3
        .map(|d| format!("{:.2} g/cm3", d))
        .unwrap_or_else(|| "--".to_string());

    let diameter = specs
        .diameter_mm
        .map(|d| format!("{:.2}mm", d))
        .unwrap_or_else(|| "--".to_string());

    let source_url = specs.source_url.clone();
    let is_http_source = source_url.starts_with("http");
    let source_label = if source_url == "ai-knowledge" {
        "AI Knowledge".to_string()
    } else if !is_http_source {
        source_url.clone()
    } else {
        String::new()
    };

    view! {
        <div class="filament-card">
            <div class="filament-card-header">
                <div class="filament-card-title">
                    <span class="filament-brand">{specs.brand.clone()}</span>
                    <span class="filament-name">{specs.name.clone()}</span>
                </div>
                <span class="filament-material">{specs.material.clone()}</span>
            </div>

            <div class="filament-card-specs">
                <div class="spec-row">
                    <span class="spec-label">"Nozzle Temp"</span>
                    <span class="spec-value">{nozzle_temp}</span>
                </div>
                <div class="spec-row">
                    <span class="spec-label">"Bed Temp"</span>
                    <span class="spec-value">{bed_temp}</span>
                </div>
                <div class="spec-row">
                    <span class="spec-label">"Max Speed"</span>
                    <span class="spec-value">{max_speed}</span>
                </div>
                <div class="spec-row">
                    <span class="spec-label">"Fan Speed"</span>
                    <span class="spec-value">{fan_speed}</span>
                </div>
                <div class="spec-row">
                    <span class="spec-label">"Retraction"</span>
                    <span class="spec-value">{retraction}</span>
                </div>
                <div class="spec-row">
                    <span class="spec-label">"Density"</span>
                    <span class="spec-value">{density}</span>
                </div>
                <div class="spec-row">
                    <span class="spec-label">"Diameter"</span>
                    <span class="spec-value">{diameter}</span>
                </div>
            </div>

            <div class="filament-card-footer">
                <div class="confidence-wrapper">
                    <span class="confidence-label">"Confidence:"</span>
                    <span class={format!("confidence-value {}", confidence_class)}>
                        {format!("{}%", confidence_pct)}
                    </span>
                </div>
                {if is_http_source {
                    view! {
                        <a href={source_url} target="_blank" class="source-link">"View Source"</a>
                    }.into_any()
                } else {
                    view! {
                        <span class="source-link">{source_label}</span>
                    }.into_any()
                }}
            </div>

            <button
                class="btn btn-primary filament-card-generate-btn"
                on:click=move |_| on_generate.run(())
                disabled=generating
            >
                {if generating { "Generating..." } else { "Generate Profile" }}
            </button>
        </div>
    }
}
