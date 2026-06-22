use leptos::prelude::*;

use crate::commands::FilamentSpecs;

/// A single setting that can be merged between two sources.
struct MergeableSetting {
    label: &'static str,
    ai_value: Option<String>,
    base_value: Option<String>,
    field_key: &'static str,
}

#[component]
pub fn SettingsMerge(
    ai_specs: FilamentSpecs,
    base_specs: FilamentSpecs,
    base_name: String,
    #[prop(into)] on_apply: Callback<FilamentSpecs>,
    #[prop(into)] on_skip: Callback<()>,
) -> impl IntoView {
    // Build list of mergeable settings (only those where at least one side has a value)
    let settings = build_mergeable_settings(&ai_specs, &base_specs);

    // Create a signal for each setting's selection ("ai" or "base")
    let selections: Vec<(String, RwSignal<String>)> = settings
        .iter()
        .map(|s| {
            // Default to AI value if it exists, otherwise base
            let default = if s.ai_value.is_some() { "ai" } else { "base" };
            (s.field_key.to_string(), RwSignal::new(default.to_string()))
        })
        .collect();

    let selections_for_apply = selections.clone();
    let ai_for_apply = ai_specs.clone();
    let base_for_apply = base_specs.clone();

    let on_apply_click = move |_| {
        let merged = apply_merge(&ai_for_apply, &base_for_apply, &selections_for_apply);
        on_apply.run(merged);
    };

    let on_skip_click = move |_| {
        on_skip.run(());
    };

    view! {
        <div class="settings-merge-container">
            <div class="merge-header-row">
                <span class="merge-col-label">"Setting"</span>
                <span class="merge-col-label">"AI / Web"</span>
                <span class="merge-col-label">{format!("Base ({})", base_name)}</span>
            </div>
            <div class="merge-rows">
                {settings.into_iter().enumerate().map(|(i, setting)| {
                    let selection = selections[i].1;
                    let ai_val = setting.ai_value.clone().unwrap_or_else(|| "--".to_string());
                    let base_val = setting.base_value.clone().unwrap_or_else(|| "--".to_string());
                    let has_ai = setting.ai_value.is_some();
                    let has_base = setting.base_value.is_some();
                    let differs = setting.ai_value != setting.base_value;
                    view! {
                        <div class={if differs { "merge-row differs" } else { "merge-row" }}>
                            <span class="merge-setting-label">{setting.label}</span>
                            <label class={move || if selection.get() == "ai" { "merge-option selected" } else { "merge-option" }}>
                                <input
                                    type="radio"
                                    name={format!("merge_{}", i)}
                                    value="ai"
                                    prop:checked=move || selection.get() == "ai"
                                    on:change=move |_| selection.set("ai".to_string())
                                    disabled=!has_ai
                                />
                                <span class="merge-value">{ai_val}</span>
                            </label>
                            <label class={move || if selection.get() == "base" { "merge-option selected" } else { "merge-option" }}>
                                <input
                                    type="radio"
                                    name={format!("merge_{}", i)}
                                    value="base"
                                    prop:checked=move || selection.get() == "base"
                                    on:change=move |_| selection.set("base".to_string())
                                    disabled=!has_base
                                />
                                <span class="merge-value">{base_val}</span>
                            </label>
                        </div>
                    }
                }).collect::<Vec<_>>()}
            </div>
            <div class="merge-actions">
                <button class="btn btn-primary" on:click=on_apply_click>
                    "Apply Merged Settings"
                </button>
                <button class="btn btn-secondary" on:click=on_skip_click>
                    "Skip (Use AI Only)"
                </button>
            </div>
        </div>
    }
}

fn build_mergeable_settings(ai: &FilamentSpecs, base: &FilamentSpecs) -> Vec<MergeableSetting> {
    let mut settings = Vec::new();

    macro_rules! add_opt_i32 {
        ($label:expr, $field:ident, $key:expr) => {
            if ai.$field.is_some() || base.$field.is_some() {
                settings.push(MergeableSetting {
                    label: $label,
                    ai_value: ai.$field.map(|v| v.to_string()),
                    base_value: base.$field.map(|v| v.to_string()),
                    field_key: $key,
                });
            }
        };
    }

    macro_rules! add_opt_f64 {
        ($label:expr, $field:ident, $key:expr) => {
            if ai.$field.is_some() || base.$field.is_some() {
                settings.push(MergeableSetting {
                    label: $label,
                    ai_value: ai.$field.map(|v| format!("{:.2}", v)),
                    base_value: base.$field.map(|v| format!("{:.2}", v)),
                    field_key: $key,
                });
            }
        };
    }

    // Temperature settings
    add_opt_i32!("Nozzle Temperature", nozzle_temperature, "nozzle_temperature");
    add_opt_i32!("Nozzle Temp (Initial Layer)", nozzle_temperature_initial_layer, "nozzle_temperature_initial_layer");
    add_opt_i32!("Nozzle Temp Min", nozzle_temp_min, "nozzle_temp_min");
    add_opt_i32!("Nozzle Temp Max", nozzle_temp_max, "nozzle_temp_max");
    add_opt_i32!("Bed Temp Min", bed_temp_min, "bed_temp_min");
    add_opt_i32!("Bed Temp Max", bed_temp_max, "bed_temp_max");
    add_opt_i32!("Hot Plate Temp", hot_plate_temp, "hot_plate_temp");
    add_opt_i32!("Cool Plate Temp", cool_plate_temp, "cool_plate_temp");
    add_opt_i32!("Engineering Plate Temp", eng_plate_temp, "eng_plate_temp");
    add_opt_i32!("Textured Plate Temp", textured_plate_temp, "textured_plate_temp");

    // Speed/Flow
    add_opt_f64!("Max Volumetric Speed", max_volumetric_speed, "max_volumetric_speed");
    add_opt_f64!("Filament Flow Ratio", filament_flow_ratio, "filament_flow_ratio");
    add_opt_f64!("Pressure Advance", pressure_advance, "pressure_advance");
    add_opt_i32!("Max Speed (mm/s)", max_speed_mm_s, "max_speed_mm_s");

    // Fan/Cooling
    add_opt_i32!("Fan Min Speed", fan_min_speed, "fan_min_speed");
    add_opt_i32!("Fan Max Speed", fan_max_speed, "fan_max_speed");
    add_opt_i32!("Overhang Fan Speed", overhang_fan_speed, "overhang_fan_speed");
    add_opt_i32!("Close Fan Below Layer", close_fan_the_first_x_layers, "close_fan_the_first_x_layers");

    // Retraction
    add_opt_f64!("Retraction Length", retraction_length, "retraction_length");
    add_opt_i32!("Retraction Speed", retraction_speed, "retraction_speed");
    add_opt_i32!("Z Hop", z_hop, "z_hop");

    settings
}

fn apply_merge(
    ai: &FilamentSpecs,
    base: &FilamentSpecs,
    selections: &[(String, RwSignal<String>)],
) -> FilamentSpecs {
    let mut merged = ai.clone();

    for (key, signal) in selections {
        let source = signal.get_untracked();
        if source == "base" {
            match key.as_str() {
                "nozzle_temperature" => merged.nozzle_temperature = base.nozzle_temperature,
                "nozzle_temperature_initial_layer" => merged.nozzle_temperature_initial_layer = base.nozzle_temperature_initial_layer,
                "nozzle_temp_min" => merged.nozzle_temp_min = base.nozzle_temp_min,
                "nozzle_temp_max" => merged.nozzle_temp_max = base.nozzle_temp_max,
                "bed_temp_min" => merged.bed_temp_min = base.bed_temp_min,
                "bed_temp_max" => merged.bed_temp_max = base.bed_temp_max,
                "hot_plate_temp" => merged.hot_plate_temp = base.hot_plate_temp,
                "cool_plate_temp" => merged.cool_plate_temp = base.cool_plate_temp,
                "eng_plate_temp" => merged.eng_plate_temp = base.eng_plate_temp,
                "textured_plate_temp" => merged.textured_plate_temp = base.textured_plate_temp,
                "max_volumetric_speed" => merged.max_volumetric_speed = base.max_volumetric_speed,
                "filament_flow_ratio" => merged.filament_flow_ratio = base.filament_flow_ratio,
                "pressure_advance" => merged.pressure_advance = base.pressure_advance,
                "max_speed_mm_s" => merged.max_speed_mm_s = base.max_speed_mm_s,
                "fan_min_speed" => merged.fan_min_speed = base.fan_min_speed,
                "fan_max_speed" => merged.fan_max_speed = base.fan_max_speed,
                "overhang_fan_speed" => merged.overhang_fan_speed = base.overhang_fan_speed,
                "close_fan_the_first_x_layers" => merged.close_fan_the_first_x_layers = base.close_fan_the_first_x_layers,
                "retraction_length" => merged.retraction_length = base.retraction_length,
                "retraction_speed" => merged.retraction_speed = base.retraction_speed,
                "z_hop" => merged.z_hop = base.z_hop,
                _ => {}
            }
        }
    }

    merged
}
