use leptos::prelude::*;

use crate::commands::FilamentSpecs;

pub const PRINTER_OPTIONS: &[&str] = &[
    "Bambu Lab H2C 0.4 nozzle",
    "Bambu Lab H2D 0.4 nozzle",
    "Bambu Lab X1C 0.4 nozzle",
    "Bambu Lab P1S 0.4 nozzle",
    "Bambu Lab A1 0.4 nozzle",
    "Bambu Lab A1 mini 0.4 nozzle",
];

#[component]
pub fn SpecsEditor(
    specs: FilamentSpecs,
    #[prop(into)] on_generate: Callback<(FilamentSpecs, String)>,
    #[prop(into)] on_cancel: Callback<()>,
    #[prop(default = "Generate Profile")] action_label: &'static str,
    #[prop(default = "Back")] cancel_label: &'static str,
    #[prop(default = true)] show_printer: bool,
    #[prop(default = true)] fill_defaults: bool,
) -> impl IntoView {
    // Optionally fill in derived defaults
    let mut specs = specs;
    if fill_defaults {
        specs.fill_derived_defaults();
    }
    let original = specs.clone();

    // Printer selector
    let (selected_printer, set_selected_printer) = signal(PRINTER_OPTIONS[0].to_string());

    // Identity signals
    let (profile_name, set_profile_name) = signal(specs.name.clone());
    let (brand, set_brand) = signal(specs.brand.clone());
    let (material, set_material) = signal(specs.material.clone());

    // Temperature signals
    let (nozzle_temp_min, set_nozzle_temp_min) = signal(specs.nozzle_temp_min.map(|v| v.to_string()).unwrap_or_default());
    let (nozzle_temp_max, set_nozzle_temp_max) = signal(specs.nozzle_temp_max.map(|v| v.to_string()).unwrap_or_default());
    let (bed_temp_min, set_bed_temp_min) = signal(specs.bed_temp_min.map(|v| v.to_string()).unwrap_or_default());
    let (bed_temp_max, set_bed_temp_max) = signal(specs.bed_temp_max.map(|v| v.to_string()).unwrap_or_default());
    let (nozzle_temperature, set_nozzle_temperature) = signal(specs.nozzle_temperature.map(|v| v.to_string()).unwrap_or_default());
    let (nozzle_temperature_initial_layer, set_nozzle_temperature_initial_layer) = signal(specs.nozzle_temperature_initial_layer.map(|v| v.to_string()).unwrap_or_default());
    let (hot_plate_temp, set_hot_plate_temp) = signal(specs.hot_plate_temp.map(|v| v.to_string()).unwrap_or_default());
    let (cool_plate_temp, set_cool_plate_temp) = signal(specs.cool_plate_temp.map(|v| v.to_string()).unwrap_or_default());
    let (eng_plate_temp, set_eng_plate_temp) = signal(specs.eng_plate_temp.map(|v| v.to_string()).unwrap_or_default());
    let (textured_plate_temp, set_textured_plate_temp) = signal(specs.textured_plate_temp.map(|v| v.to_string()).unwrap_or_default());

    // Speed/Flow signals
    let (max_volumetric_speed, set_max_volumetric_speed) = signal(specs.max_volumetric_speed.map(|v| format!("{:.1}", v)).unwrap_or_default());
    let (filament_flow_ratio, set_filament_flow_ratio) = signal(specs.filament_flow_ratio.map(|v| format!("{:.2}", v)).unwrap_or_default());
    let (pressure_advance, set_pressure_advance) = signal(specs.pressure_advance.map(|v| format!("{:.3}", v)).unwrap_or_default());
    let (max_speed_mm_s, set_max_speed_mm_s) = signal(specs.max_speed_mm_s.map(|v| v.to_string()).unwrap_or_default());

    // Fan/Cooling signals
    let (fan_min_speed, set_fan_min_speed) = signal(specs.fan_min_speed.map(|v| v.to_string()).unwrap_or_default());
    let (fan_max_speed, set_fan_max_speed) = signal(specs.fan_max_speed.map(|v| v.to_string()).unwrap_or_default());
    let (overhang_fan_speed, set_overhang_fan_speed) = signal(specs.overhang_fan_speed.map(|v| v.to_string()).unwrap_or_default());
    let (close_fan_first_layers, set_close_fan_first_layers) = signal(specs.close_fan_the_first_x_layers.map(|v| v.to_string()).unwrap_or_default());
    let (additional_cooling_fan_speed, set_additional_cooling_fan_speed) = signal(specs.additional_cooling_fan_speed.map(|v| v.to_string()).unwrap_or_default());
    let (slow_down_layer_time, set_slow_down_layer_time) = signal(specs.slow_down_layer_time.map(|v| v.to_string()).unwrap_or_default());
    let (slow_down_min_speed, set_slow_down_min_speed) = signal(specs.slow_down_min_speed.map(|v| v.to_string()).unwrap_or_default());

    // Retraction signals
    let (retraction_distance_mm, set_retraction_distance_mm) = signal(specs.retraction_distance_mm.map(|v| format!("{:.1}", v)).unwrap_or_default());
    let (retraction_speed_mm_s, set_retraction_speed_mm_s) = signal(specs.retraction_speed_mm_s.map(|v| v.to_string()).unwrap_or_default());
    let (deretraction_speed_mm_s, set_deretraction_speed_mm_s) = signal(specs.deretraction_speed_mm_s.map(|v| v.to_string()).unwrap_or_default());

    // Physical signals
    let (density_g_cm3, set_density_g_cm3) = signal(specs.density_g_cm3.map(|v| format!("{:.2}", v)).unwrap_or_default());
    let (diameter_mm, set_diameter_mm) = signal(specs.diameter_mm.map(|v| format!("{:.2}", v)).unwrap_or_default());
    let (filament_cost, set_filament_cost) = signal(specs.filament_cost.map(|v| format!("{:.2}", v)).unwrap_or_default());
    let (temperature_vitrification, set_temperature_vitrification) = signal(specs.temperature_vitrification.map(|v| v.to_string()).unwrap_or_default());

    // Reset handler
    let do_reset = move |_| {
        let o = &original;
        set_profile_name.set(o.name.clone());
        set_brand.set(o.brand.clone());
        set_material.set(o.material.clone());
        set_nozzle_temp_min.set(o.nozzle_temp_min.map(|v| v.to_string()).unwrap_or_default());
        set_nozzle_temp_max.set(o.nozzle_temp_max.map(|v| v.to_string()).unwrap_or_default());
        set_bed_temp_min.set(o.bed_temp_min.map(|v| v.to_string()).unwrap_or_default());
        set_bed_temp_max.set(o.bed_temp_max.map(|v| v.to_string()).unwrap_or_default());
        set_nozzle_temperature.set(o.nozzle_temperature.map(|v| v.to_string()).unwrap_or_default());
        set_nozzle_temperature_initial_layer.set(o.nozzle_temperature_initial_layer.map(|v| v.to_string()).unwrap_or_default());
        set_hot_plate_temp.set(o.hot_plate_temp.map(|v| v.to_string()).unwrap_or_default());
        set_cool_plate_temp.set(o.cool_plate_temp.map(|v| v.to_string()).unwrap_or_default());
        set_eng_plate_temp.set(o.eng_plate_temp.map(|v| v.to_string()).unwrap_or_default());
        set_textured_plate_temp.set(o.textured_plate_temp.map(|v| v.to_string()).unwrap_or_default());
        set_max_volumetric_speed.set(o.max_volumetric_speed.map(|v| format!("{:.1}", v)).unwrap_or_default());
        set_filament_flow_ratio.set(o.filament_flow_ratio.map(|v| format!("{:.2}", v)).unwrap_or_default());
        set_pressure_advance.set(o.pressure_advance.map(|v| format!("{:.3}", v)).unwrap_or_default());
        set_max_speed_mm_s.set(o.max_speed_mm_s.map(|v| v.to_string()).unwrap_or_default());
        set_fan_min_speed.set(o.fan_min_speed.map(|v| v.to_string()).unwrap_or_default());
        set_fan_max_speed.set(o.fan_max_speed.map(|v| v.to_string()).unwrap_or_default());
        set_overhang_fan_speed.set(o.overhang_fan_speed.map(|v| v.to_string()).unwrap_or_default());
        set_close_fan_first_layers.set(o.close_fan_the_first_x_layers.map(|v| v.to_string()).unwrap_or_default());
        set_additional_cooling_fan_speed.set(o.additional_cooling_fan_speed.map(|v| v.to_string()).unwrap_or_default());
        set_slow_down_layer_time.set(o.slow_down_layer_time.map(|v| v.to_string()).unwrap_or_default());
        set_slow_down_min_speed.set(o.slow_down_min_speed.map(|v| v.to_string()).unwrap_or_default());
        set_retraction_distance_mm.set(o.retraction_distance_mm.map(|v| format!("{:.1}", v)).unwrap_or_default());
        set_retraction_speed_mm_s.set(o.retraction_speed_mm_s.map(|v| v.to_string()).unwrap_or_default());
        set_deretraction_speed_mm_s.set(o.deretraction_speed_mm_s.map(|v| v.to_string()).unwrap_or_default());
        set_density_g_cm3.set(o.density_g_cm3.map(|v| format!("{:.2}", v)).unwrap_or_default());
        set_diameter_mm.set(o.diameter_mm.map(|v| format!("{:.2}", v)).unwrap_or_default());
        set_filament_cost.set(o.filament_cost.map(|v| format!("{:.2}", v)).unwrap_or_default());
        set_temperature_vitrification.set(o.temperature_vitrification.map(|v| v.to_string()).unwrap_or_default());
        set_selected_printer.set(PRINTER_OPTIONS[0].to_string());
    };

    // Build edited specs
    let specs_for_generate = specs.clone();
    let do_generate = move |_| {
        let mut edited = specs_for_generate.clone();

        // Identity fields
        edited.name = profile_name.get();
        edited.brand = brand.get();
        edited.material = material.get();

        // Parse helpers
        let parse_u16 = |s: &str| -> Option<u16> { s.trim().parse().ok() };
        let parse_u8 = |s: &str| -> Option<u8> { s.trim().parse().ok() };
        let parse_f32 = |s: &str| -> Option<f32> { s.trim().parse().ok() };

        edited.nozzle_temp_min = parse_u16(&nozzle_temp_min.get());
        edited.nozzle_temp_max = parse_u16(&nozzle_temp_max.get());
        edited.bed_temp_min = parse_u16(&bed_temp_min.get());
        edited.bed_temp_max = parse_u16(&bed_temp_max.get());
        edited.nozzle_temperature = parse_u16(&nozzle_temperature.get());
        edited.nozzle_temperature_initial_layer = parse_u16(&nozzle_temperature_initial_layer.get());
        edited.hot_plate_temp = parse_u16(&hot_plate_temp.get());
        edited.hot_plate_temp_initial_layer = edited.hot_plate_temp;
        edited.cool_plate_temp = parse_u16(&cool_plate_temp.get());
        edited.cool_plate_temp_initial_layer = edited.cool_plate_temp;
        edited.eng_plate_temp = parse_u16(&eng_plate_temp.get());
        edited.eng_plate_temp_initial_layer = edited.eng_plate_temp;
        edited.textured_plate_temp = parse_u16(&textured_plate_temp.get());
        edited.textured_plate_temp_initial_layer = edited.textured_plate_temp;
        edited.max_volumetric_speed = parse_f32(&max_volumetric_speed.get());
        edited.filament_flow_ratio = parse_f32(&filament_flow_ratio.get());
        edited.pressure_advance = parse_f32(&pressure_advance.get());
        edited.max_speed_mm_s = parse_u16(&max_speed_mm_s.get());
        edited.fan_min_speed = parse_u8(&fan_min_speed.get());
        edited.fan_max_speed = parse_u8(&fan_max_speed.get());
        edited.overhang_fan_speed = parse_u8(&overhang_fan_speed.get());
        edited.close_fan_the_first_x_layers = parse_u8(&close_fan_first_layers.get());
        edited.additional_cooling_fan_speed = parse_u8(&additional_cooling_fan_speed.get());
        edited.slow_down_layer_time = parse_u8(&slow_down_layer_time.get());
        edited.slow_down_min_speed = parse_u16(&slow_down_min_speed.get());
        edited.retraction_distance_mm = parse_f32(&retraction_distance_mm.get());
        edited.retraction_speed_mm_s = parse_u16(&retraction_speed_mm_s.get());
        edited.deretraction_speed_mm_s = parse_u16(&deretraction_speed_mm_s.get());
        edited.density_g_cm3 = parse_f32(&density_g_cm3.get());
        edited.diameter_mm = parse_f32(&diameter_mm.get());
        edited.filament_cost = parse_f32(&filament_cost.get());
        edited.temperature_vitrification = parse_u16(&temperature_vitrification.get());

        on_generate.run((edited, selected_printer.get()));
    };

    view! {
        <div class="specs-editor">
            <h3 class="specs-editor-title">"Edit Specifications"</h3>
            <p class="specs-editor-subtitle">
                "Review and adjust values before generating the profile. Hover "
                <span class="tooltip-icon-inline">"?"</span>
                " for guidance."
            </p>

            // Identity section
            <div class="specs-section">
                <h4 class="specs-section-title">"Profile Identity"</h4>
                <div class="spec-fields-grid">
                    {spec_input("Name", "", profile_name, set_profile_name,
                        "Profile display name as it appears in Bambu Studio's filament list.")}
                    {spec_input("Brand", "", brand, set_brand,
                        "Filament manufacturer/brand name (e.g., SUNLU, Bambu Lab, eSUN).")}
                    {spec_input("Material", "", material, set_material,
                        "Material type (e.g., PLA, PETG, ABS, TPU). Determines which base profile is used.")}
                </div>
            </div>

            // Printer selector (optional)
            <Show when=move || show_printer>
                <div class="specs-section">
                    <h4 class="specs-section-title">"Target Printer"</h4>
                    <div class="spec-field full-width">
                        <label class="spec-field-label">"Printer / Nozzle"</label>
                        <select
                            class="spec-field-select"
                            on:change=move |ev| set_selected_printer.set(event_target_value(&ev))
                        >
                            {PRINTER_OPTIONS.iter().map(|p| {
                                let p_str = p.to_string();
                                view! { <option value={*p}>{p_str}</option> }
                            }).collect::<Vec<_>>()}
                        </select>
                    </div>
                </div>
            </Show>

            // Temperature section
            <div class="specs-section">
                <h4 class="specs-section-title">"Temperature"</h4>
                <div class="spec-fields-grid">
                    {spec_input("Nozzle Temp", "C", nozzle_temperature, set_nozzle_temperature,
                        "Main printing temperature. Usually near the top of the manufacturer's range. PLA: 200-220, PETG: 230-250, ABS: 240-260.")}
                    {spec_input("Nozzle Initial Layer", "C", nozzle_temperature_initial_layer, set_nozzle_temperature_initial_layer,
                        "First layer temp, usually 5-10C above nozzle temp for better bed adhesion.")}
                    {spec_input("Nozzle Min", "C", nozzle_temp_min, set_nozzle_temp_min,
                        "Lowest safe nozzle temp. Below this, extrusion becomes unreliable. Sets the low end of the BS temp slider.")}
                    {spec_input("Nozzle Max", "C", nozzle_temp_max, set_nozzle_temp_max,
                        "Highest safe nozzle temp. Above this, the material may degrade or produce fumes. Sets the high end of the BS temp slider.")}
                    {spec_input("Bed Min", "C", bed_temp_min, set_bed_temp_min,
                        "Lowest useful bed temp. Used for cool/textured plates that need less heat.")}
                    {spec_input("Bed Max", "C", bed_temp_max, set_bed_temp_max,
                        "Highest useful bed temp. Used for hot/engineering plates.")}
                    {spec_input("Hot Plate", "C", hot_plate_temp, set_hot_plate_temp,
                        "Temp for the smooth PEI hot plate. Usually near bed_temp_max. PLA: 55-60, PETG: 70-80, ABS: 100-110.")}
                    {spec_input("Cool Plate", "C", cool_plate_temp, set_cool_plate_temp,
                        "Temp for the cool/smooth PEI plate. Usually near bed_temp_min. Some materials (PETG, ABS) should not use cool plate.")}
                    {spec_input("Eng Plate", "C", eng_plate_temp, set_eng_plate_temp,
                        "Temp for the engineering/textured PEI plate. Usually close to hot plate temp.")}
                    {spec_input("Textured Plate", "C", textured_plate_temp, set_textured_plate_temp,
                        "Temp for the textured PEI plate. Usually 5C below cool plate to prevent over-adhesion on textured surface.")}
                </div>
            </div>

            // Speed/Flow section
            <div class="specs-section">
                <h4 class="specs-section-title">"Speed & Flow"</h4>
                <div class="spec-fields-grid">
                    {spec_input("Max Vol. Speed", "mm\u{00b3}/s", max_volumetric_speed, set_max_volumetric_speed,
                        "THE key speed limit in Bambu Studio. Caps how fast plastic melts. PLA: 18-24, PETG: 14-20, ABS: 14-18, TPU: 5-10. Higher = faster prints but risk of underextrusion.")}
                    {spec_input("Flow Ratio", "", filament_flow_ratio, set_filament_flow_ratio,
                        "Extrusion multiplier (0.90-1.05). Increase if walls look thin/gaps, decrease if blobby/overextruded. Default 0.98. Fine-tune with a single-wall cube test.")}
                    {spec_input("Pressure Advance", "", pressure_advance, set_pressure_advance,
                        "Compensates for pressure buildup in the nozzle. PLA: 0.02-0.06, PETG: 0.01-0.04. Too high causes gaps at corners, too low causes blobs. Tune with a PA calibration print.")}
                    {spec_input("Max Speed", "mm/s", max_speed_mm_s, set_max_speed_mm_s,
                        "Maximum print head speed. Usually 100-300 for Bambu printers. Actual speed is limited by max volumetric speed, so this is a secondary limit.")}
                </div>
            </div>

            // Fan/Cooling section
            <div class="specs-section">
                <h4 class="specs-section-title">"Fan & Cooling"</h4>
                <div class="spec-fields-grid">
                    {spec_input("Fan Min", "%", fan_min_speed, set_fan_min_speed,
                        "Minimum part cooling fan speed (0-100). PLA: 80-100, PETG: 15-30, ABS: 0. Low fan = better layer adhesion but worse overhangs.")}
                    {spec_input("Fan Max", "%", fan_max_speed, set_fan_max_speed,
                        "Maximum part cooling fan speed (0-100). Fan ramps from min to max based on layer time. PLA: 100, PETG: 30-50, ABS: 0-30.")}
                    {spec_input("Overhang Fan", "%", overhang_fan_speed, set_overhang_fan_speed,
                        "Fan speed when printing overhangs (0-100). Usually higher than normal to help bridges and overhangs solidify. 80-100 for most materials.")}
                    {spec_input("Fan Off Layers", "", close_fan_first_layers, set_close_fan_first_layers,
                        "Number of initial layers with fan completely off. Helps bed adhesion. PLA: 1, PETG/ABS: 2-3. More layers = better adhesion but potential elephant's foot.")}
                    {spec_input("Aux Fan", "%", additional_cooling_fan_speed, set_additional_cooling_fan_speed,
                        "Auxiliary/chamber fan speed (0-100). Helps cool the print chamber. PLA: 60-100, ABS: 0 (ABS needs a hot chamber). Only relevant for enclosed printers.")}
                    {spec_input("Slowdown Time", "s", slow_down_layer_time, set_slow_down_layer_time,
                        "Minimum time per layer in seconds. If a layer would print faster, speed is reduced. Prevents overheating on small parts. 5-12s typical.")}
                    {spec_input("Slowdown Min", "mm/s", slow_down_min_speed, set_slow_down_min_speed,
                        "Minimum speed when slowing down for cooling. Printer won't go below this even for tiny layers. 10-25 mm/s typical.")}
                </div>
            </div>

            // Retraction section
            <div class="specs-section">
                <h4 class="specs-section-title">"Retraction"</h4>
                <div class="spec-fields-grid">
                    {spec_input("Distance", "mm", retraction_distance_mm, set_retraction_distance_mm,
                        "How far filament is pulled back during travel moves. Direct drive: 0.5-2.0mm, Bowden: 3-6mm. Too much can cause clogs, too little causes stringing.")}
                    {spec_input("Speed", "mm/s", retraction_speed_mm_s, set_retraction_speed_mm_s,
                        "How fast filament is retracted. 25-50 mm/s typical. Faster reduces stringing but may grind filament. TPU needs slower (15-25).")}
                    {spec_input("Deretraction", "mm/s", deretraction_speed_mm_s, set_deretraction_speed_mm_s,
                        "Speed at which filament is pushed back after retraction. Often same as retraction speed. Lower values can help with blobs at restart points.")}
                </div>
            </div>

            // Physical section
            <div class="specs-section">
                <h4 class="specs-section-title">"Physical Properties"</h4>
                <div class="spec-fields-grid">
                    {spec_input("Density", "g/cm\u{00b3}", density_g_cm3, set_density_g_cm3,
                        "Material density for weight/cost estimates. PLA: 1.24, PETG: 1.27, ABS: 1.04, TPU: 1.21, Nylon: 1.14. Check the filament spool or datasheet.")}
                    {spec_input("Diameter", "mm", diameter_mm, set_diameter_mm,
                        "Filament diameter. Almost always 1.75mm for Bambu printers. Some specialty filaments are 2.85mm. Check the spool label.")}
                    {spec_input("Cost", "$/kg", filament_cost, set_filament_cost,
                        "Price per kilogram. Used by Bambu Studio for print cost estimates. Check the purchase price and spool weight.")}
                    {spec_input("Vitrification", "C", temperature_vitrification, set_temperature_vitrification,
                        "Glass transition temperature. Above this, the material softens. Important for AMS drying limits and part heat resistance. PLA: 55-60, PETG: 80, ABS: 105.")}
                </div>
            </div>

            // Actions
            <div class="specs-editor-actions">
                <button class="btn btn-secondary" on:click=move |_| on_cancel.run(())>
                    {cancel_label}
                </button>
                <button class="btn btn-secondary" on:click=do_reset>
                    "Reset"
                </button>
                <button class="btn btn-primary" on:click=do_generate>
                    {action_label}
                </button>
            </div>
        </div>
    }
}

fn spec_input(
    label: &'static str,
    unit: &'static str,
    value: ReadSignal<String>,
    set_value: WriteSignal<String>,
    tooltip: &'static str,
) -> impl IntoView {
    view! {
        <div class="spec-field">
            <div class="spec-field-label-row">
                <label class="spec-field-label">{label}</label>
                <span class="tooltip-wrapper">
                    <span class="tooltip-icon">"?"</span>
                    <span class="tooltip-content">{tooltip}</span>
                </span>
            </div>
            <div class="spec-field-input-wrapper">
                <input
                    type="text"
                    class="spec-field-input"
                    prop:value=move || value.get()
                    on:input=move |ev| set_value.set(event_target_value(&ev))
                    placeholder="--"
                />
                {if !unit.is_empty() {
                    view! { <span class="spec-field-unit">{unit}</span> }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }}
            </div>
        </div>
    }
}
