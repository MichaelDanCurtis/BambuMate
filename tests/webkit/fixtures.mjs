// Canned backend responses for the UI flow tests.
//
// The app talks to Rust through `window.__TAURI__.core.invoke`, which does not
// exist in a plain browser. These fixtures stand in for it so the real Leptos
// frontend can be driven end to end without a Tauri host.
//
// Shapes here must match what the frontend deserializes into, because
// `serde_wasm_bindgen::from_value` rejects a missing or misnamed field and the
// UI would show an error state that looks like a rendering bug. Field names are
// snake_case: none of the response structs carry `rename_all`.

const SPECS = {
  serial: "PA-PL-WHTPA0-01",
  brand: "Polymaker",
  material: "PLA",
  nozzle_temp_min: 190,
  nozzle_temp_max: 230,
  bed_temp_min: 35,
  bed_temp_max: 65,
  nozzle_temperature: 220,
  nozzle_temperature_initial_layer: 225,
  hot_plate_temp: 65,
  hot_plate_temp_initial_layer: 65,
  cool_plate_temp: 35,
  cool_plate_temp_initial_layer: 35,
  eng_plate_temp: 65,
  eng_plate_temp_initial_layer: 65,
  textured_plate_temp: 60,
  textured_plate_temp_initial_layer: 60,
  max_volumetric_speed: 21.0,
  filament_flow_ratio: 0.98,
  pressure_advance: 0.04,
  fan_min_speed: 100,
  fan_max_speed: 100,
  overhang_fan_speed: 100,
  close_fan_the_first_x_layers: 1,
  additional_cooling_fan_speed: 80,
  fan_speed_percent: null,
  slow_down_layer_time: 8,
  slow_down_min_speed: 20,
  retraction_distance_mm: 0.8,
  retraction_speed_mm_s: 40,
  deretraction_speed_mm_s: 40,
  bridge_speed: null,
  density_g_cm3: 1.24,
  diameter_mm: 1.75,
  temperature_vitrification: 55,
  filament_cost: 21.99,
  max_speed_mm_s: null,
  source_url: "https://us.polymaker.com/products/polylite-pla",
  extraction_confidence: 0.92,
};

const CATALOG_ENTRY = {
  brand: "Polymaker",
  name: "PolyLite PLA",
  material: "PLA",
  url_slug: "polylite-pla",
  full_url: "https://us.polymaker.com/products/polylite-pla",
};

const USER_PROFILE_PATH =
  "/Users/runner/Library/Application Support/BambuStudio/user/00000001/filament/Polymaker PolyLite PLA @BBL X1C 0.4 nozzle.json";

export const FIXTURES = {
  // -- boot --
  get_preference: null,
  set_preference: null,
  get_feature_flags: { profiles_enabled: true, analysis_enabled: true },
  check_setup_complete: {
    bambu_studio_path: "/Applications/BambuStudio.app",
    ai_provider: "openai",
    has_api_key: true,
    setup_complete: true,
  },
  check_for_updates: {
    has_update: false,
    latest_version: "1.3.0",
    release_url: "https://github.com/MichaelDanCurtis/BambuMate/releases/tag/v1.3.0",
    release_notes: null,
  },
  list_received_stls: [],

  // -- filament search --
  get_catalog_status: { entry_count: 847, needs_refresh: false },
  search_catalog: [
    { entry: CATALOG_ENTRY, score: 0.97 },
    {
      entry: {
        brand: "Polymaker",
        name: "PolyTerra PLA",
        material: "PLA",
        url_slug: "polyterra-pla",
        full_url: "https://us.polymaker.com/products/polyterra-pla",
      },
      score: 0.84,
    },
  ],
  fetch_filament_from_catalog: SPECS,
  search_filament: SPECS,
  search_base_profiles: [
    {
      name: "Bambu PLA Basic @BBL X1C 0.4 nozzle",
      path: "/Applications/BambuStudio.app/Contents/Resources/profiles/BBL/filament/Bambu PLA Basic @BBL X1C 0.4 nozzle.json",
      filament_type: "PLA",
    },
    {
      name: "Generic PLA @BBL X1C 0.4 nozzle",
      path: "/Applications/BambuStudio.app/Contents/Resources/profiles/BBL/filament/Generic PLA @BBL X1C 0.4 nozzle.json",
      filament_type: "PLA",
    },
  ],
  // Model names are bare here because format_target_printer_label prepends
  // "Bambu Lab " when building the display label. Mirrors the shape of
  // default_target_printer_options in src/commands.rs; an earlier version of
  // this fixture carried the prefix already and produced "Bambu Lab Bambu Lab
  // X1 Carbon 0.4 nozzle" on screen.
  list_target_printer_options: {
    printer_models: ["H2C", "H2D", "X1 Carbon", "X1E", "P1P", "P1S", "A1", "A1 mini"],
    nozzle_sizes: ["0.4", "0.2", "0.6", "0.8"],
    default_printer_model: "H2C",
    default_nozzle_size: "0.4",
  },
  tune_specs_for_nozzle: {
    specs: { ...SPECS, max_volumetric_speed: 18.0 },
    nozzle_diameter: 0.4,
    flow_cap: 21.0,
    changes: [
      {
        field: "max_volumetric_speed",
        from: "21.0",
        to: "18.0",
        source: "limit",
      },
    ],
    notes: ["Volumetric speed capped to 18.0 mm\u00b3/s for the 0.4 mm nozzle."],
    confidence: 0.88,
  },
  generate_profile_from_specs: {
    profile_name: "Polymaker PolyLite PLA @BBL X1C 0.4 nozzle",
    filament_id: "PA-PL-WHTPA0-01",
    profile_json: '{"name":"Polymaker PolyLite PLA @BBL X1C 0.4 nozzle"}',
    metadata_info: '{"sync_info":"sync"}',
    filename: "Polymaker PolyLite PLA @BBL X1C 0.4 nozzle.json",
    field_count: 47,
    base_profile_used: "Bambu PLA Basic @BBL X1C 0.4 nozzle",
    specs_applied: {
      nozzle_temp: "220\u00b0C",
      bed_temp: "65\u00b0C",
      fan_speed: "100%",
      retraction: "0.8 mm",
    },
    diffs: [
      {
        key: "nozzle_temperature",
        label: "Nozzle Temperature",
        base_value: "220",
        new_value: "220",
      },
      {
        key: "hot_plate_temp",
        label: "Hot Plate Temp",
        base_value: "55",
        new_value: "65",
      },
      {
        key: "max_volumetric_speed",
        label: "Max Volumetric Speed",
        base_value: "21",
        new_value: "18",
      },
    ],
    warnings: [],
    bambu_studio_running: false,
  },
  install_generated_profile: {
    installed_path: USER_PROFILE_PATH,
    profile_name: "Polymaker PolyLite PLA @BBL X1C 0.4 nozzle",
    bambu_studio_was_running: false,
  },

  // -- print analysis --
  list_profiles: [
    {
      name: "Polymaker PolyLite PLA @BBL X1C 0.4 nozzle",
      filament_type: "PLA",
      filament_id: "PA-PL-WHTPA0-01",
      path: USER_PROFILE_PATH,
      is_user_profile: true,
    },
    {
      name: "Bambu PLA Basic @BBL X1C 0.4 nozzle",
      filament_type: "PLA",
      filament_id: "GFA00",
      path: "/Users/runner/Library/Application Support/BambuStudio/user/00000001/filament/Bambu PLA Basic @BBL X1C 0.4 nozzle.json",
      is_user_profile: true,
    },
  ],
  analyze_print: {
    defect_report: {
      defects: [
        { defect_type: "stringing", severity: 0.65, confidence: 0.88 },
        { defect_type: "poor_overhangs", severity: 0.35, confidence: 0.71 },
      ],
      overall_quality: "fair",
      notes: "Visible stringing between towers; layer adhesion looks good.",
    },
    recommendations: [
      {
        defect: "stringing",
        parameter: "nozzle_temperature",
        parameter_label: "Nozzle Temperature",
        current_value: 225.0,
        recommended_value: 215.0,
        change_display: "225 \u2192 215",
        unit: "\u00b0C",
        priority: 1,
        rationale: "Lowering nozzle temperature reduces ooze during travel moves.",
        was_clamped: false,
      },
      {
        defect: "stringing",
        parameter: "retraction_distance_mm",
        parameter_label: "Retraction Distance",
        current_value: 0.8,
        recommended_value: 1.2,
        change_display: "0.8 \u2192 1.2",
        unit: "mm",
        priority: 2,
        rationale: "More retraction pulls filament back before long travels.",
        was_clamped: false,
      },
      {
        defect: "poor_overhangs",
        parameter: "overhang_fan_speed",
        parameter_label: "Overhang Fan Speed",
        current_value: 80.0,
        recommended_value: 100.0,
        change_display: "80 \u2192 100",
        unit: "%",
        priority: 3,
        rationale: "Maximum cooling on overhangs helps bridges solidify sooner.",
        was_clamped: true,
      },
    ],
    conflicts: [
      {
        parameter: "nozzle_temperature",
        conflicting_defects: ["stringing", "poor_layer_adhesion"],
        description:
          "Stringing wants a lower nozzle temperature while layer adhesion wants a higher one.",
      },
    ],
    current_values: {
      nozzle_temperature: 225.0,
      retraction_distance_mm: 0.8,
      overhang_fan_speed: 80.0,
    },
    material_type: "PLA",
    session_id: 42,
  },
  // The results screen loads prior refinement sessions for the chosen profile.
  list_history_sessions: [
    { id: 41, created_at: "2026-08-24T18:02:11Z", was_applied: true },
    { id: 39, created_at: "2026-08-19T09:47:03Z", was_applied: false },
  ],
  apply_recommendations: {    backup_path: `${USER_PROFILE_PATH}.bak`,
    changes_applied: [
      { parameter: "nozzle_temperature", old_value: 225.0, new_value: 215.0 },
      { parameter: "retraction_distance_mm", old_value: 0.8, new_value: 1.2 },
    ],
    profile_path: USER_PROFILE_PATH,
  },
  revert_to_backup: null,
  launch_bambu_studio: null,
};

// -- test images ------------------------------------------------------------
//
// Built here rather than committed as binaries so the bytes are inspectable,
// and so the magic numbers the frontend sniffs are visible in the source. The
// frontend picks the MIME type from these leading bytes; WKWebView then honours
// that declared type strictly, so a wrong guess renders nothing on macOS while
// Chromium's content sniffing hides the mistake on Windows.

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

export function crc32(buf) {
  let c = 0xffffffff;
  for (const byte of buf) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** A real PNG of the given size, filled with a diagonal colour ramp. */
export function makePng(width, height, deflateSync) {
  const raw = Buffer.alloc(height * (width * 3 + 1));
  let p = 0;
  for (let y = 0; y < height; y++) {
    raw[p++] = 0; // filter: none
    for (let x = 0; x < width; x++) {
      raw[p++] = (x * 255) / width;
      raw[p++] = (y * 255) / height;
      raw[p++] = 160;
    }
  }

  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body));
    return Buffer.concat([len, body, crc]);
  };

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // colour type: truecolour
  ihdr[10] = 0; // deflate
  ihdr[11] = 0; // adaptive filtering
  ihdr[12] = 0; // no interlace

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/** A minimal but valid 1x1 GIF89a, to exercise a second sniffing branch. */
export const GIF_1X1 = Buffer.from(
  "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
  "base64"
);
