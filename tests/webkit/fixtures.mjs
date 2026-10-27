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

// -- slicing ----------------------------------------------------------------
//
// Shapes mirror src-tauri/src/slicer (JobView, SliceResult). Values come from
// the real cube fixture sliced with Bambu Studio 02.08.02.61 for an H2C.

export const SLICE_MODEL = "/Users/runner/models/cube.stl";
export const STL_INBOX_FILE = "/Users/runner/stl-inbox/bracket.stl";
const H2C = "Bambu Lab H2C 0.4 nozzle";
const H2C_PROCESS = "0.20mm Standard @BBL H2C";
const PLA = "Bambu PLA Basic @BBL H2C";
export const PETG = "Generic PETG @BBL H2C 0.4 nozzle";
// A 1x1 PNG; WebKit refuses a data: URL whose bytes aren't the declared type.
const PNG_1X1 =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

export function sliceResult({ time = 843, weight = 3.69, cost = 0.07, warnings = 1 } = {}) {
  return {
    plates: [
      {
        index: 1,
        time_seconds: time,
        weight_g: weight,
        cost,
        filaments: [
          { slot: 1, filament_type: "PLA", color: "#00AE42", used_g: weight, used_m: 1.22, cost },
        ],
        warnings: Array.from({ length: warnings }, () => ({
          level: "warning",
          message:
            "It seems object cube.stl has floating cantilever. Please re-orient the object or enable support generation.",
          code: null,
        })),
        objects: [{ id: "15", name: "cube.stl" }],
        thumbnail: "plate_1.png",
      },
    ],
    printer: H2C,
    printer_model: "Bambu Lab H2C",
    process: H2C_PROCESS,
    filaments: [
      {
        slot: 1,
        preset: PLA,
        filament_type: "PLA",
        color: "#00AE42",
        cost_per_kg: 19.99,
        density: 1.26,
        filament_id: "GFA00",
        material: "pla",
      },
    ],
    bed_type: "Textured PEI Plate",
    bambu_studio_version: "02.08.02.61",
    output_path: "/Users/runner/Library/Application Support/com.bambumate.app/slices/ab/output.gcode.3mf",
  };
}

export function sliceJob(id, state, extra = {}) {
  return {
    id,
    origin: "manual",
    source_path: SLICE_MODEL,
    model_name: "cube.stl",
    printer: H2C,
    process: H2C_PROCESS,
    filament: PLA,
    bed_type: "Textured PEI Plate",
    state,
    ...extra,
  };
}

const SLICER_SETTINGS = {
  saved: { printer: null, process: null, filament: null, bed_type: null, auto_slice: false },
  effective: { printer: H2C, process: H2C_PROCESS, filament: PLA, bed_type: "Textured PEI Plate", auto_slice: false },
  bed_types: ["Cool Plate", "Engineering Plate", "High Temp Plate", "Textured PEI Plate", "Supertack Plate"],
};

// -- printer ------------------------------------------------------------------
//
// Shapes mirror src/printer/types.rs, which mirrors the backend's
// PrinterView (src-tauri/src/printer/service.rs). The slots follow the H2D
// report fixture in src-tauri/src/printer/testdata/h2d_full.json.

export const PRINTER_SERIAL = "0948AB000000001";
export const PRINTER_FINGERPRINT =
  "3A:7F:10:C2:9B:44:E1:08:5D:6A:0F:92:B7:31:CE:04:88:1B:F6:2D:73:A9:50:E3:1C:47:DA:6E:02:B5:98:FF";

const SYSTEM_FILAMENTS = "/Users/runner/Library/Application Support/BambuStudio/system/BBL/filament";
const USER_FILAMENTS = "/Users/runner/Library/Application Support/BambuStudio/user/00000001/filament/base";

// What src-tauri/src/printer/filament_index.rs Detected serializes to.
const detectedAs = (preset_name, dir = SYSTEM_FILAMENTS, is_user = false) => ({
  preset_name,
  path: `${dir}/${preset_name}.json`,
  is_user,
  variants: [preset_name],
});

/** A3 reports GFG99: this is the preset the index names for it. */
export const A3_DETECTED = "Generic PETG @BBL H2D 0.4 nozzle";
export const A3_DETECTED_PATH = `${SYSTEM_FILAMENTS}/${A3_DETECTED}.json`;

const tray = (type, color, idx, sub = "", remain = null) => ({
  id: 0,
  empty: !type && !idx,
  tray_type: type,
  tray_color: color,
  tray_info_idx: idx,
  tray_sub_brands: sub,
  remain,
});

const slotOf = (ams_id, tray_id, label, t, extra = {}) => ({
  ams_id,
  tray_id,
  label,
  tray: { ...t, id: tray_id },
  rfid: false,
  assigned_preset: null,
  assigned_filament_id: null,
  status: "unassigned",
  needs_cloud_sync: false,
  preset_has_no_id: false,
  detected: null,
  not_installed: false,
  other_nozzle: null,
  material: null,
  ...extra,
});

export const PRINTER_SLOTS = [
  slotOf(0, 0, "A1", tray("PLA-S", "FFFFFFFF", "GFS02", "Support for PLA", 1), {
    rfid: true,
    status: "rfid",
    detected: detectedAs("Bambu Support For PLA @BBL H2D"),
  }),
  slotOf(0, 1, "A2", tray("PLA", "FFFFFFFF", "GFA00", "PLA Basic", 31), {
    rfid: true,
    status: "rfid",
    material: "pla",
    detected: detectedAs("Bambu PLA Basic @BBL H2D"),
  }),
  slotOf(0, 2, "A3", tray("PETG", "1F7A3DFF", "GFG99"), { material: "petg", detected: detectedAs(A3_DETECTED) }),
  slotOf(0, 3, "A4", tray("PLA", "FFFFFFFF", "GFA00", "PLA Basic", 100), {
    rfid: true,
    status: "rfid",
    material: "pla",
    detected: detectedAs("Bambu PLA Basic @BBL H2D"),
  }),
  slotOf(1, 0, "B1", tray("PLA", "000000FF", "GFA00", "PLA Basic", 55), {
    rfid: true,
    status: "rfid",
    material: "pla",
    detected: detectedAs("Bambu PLA Basic @BBL H2D"),
  }),
  slotOf(1, 1, "B2", tray("PLA", "F95959FF", "P4d6ae04"), {
    assigned_preset: "Acme Matte PLA",
    assigned_filament_id: "P4d6ae04",
    status: "matches",
    material: "pla",
    detected: detectedAs("Acme Matte PLA", USER_FILAMENTS, true),
  }),
  slotOf(1, 2, "B3", tray("PETG-CF", "000000FF", "GFG50", "PETG-CF", 100), {
    rfid: true,
    status: "rfid",
    material: "pet_cf",
    detected: detectedAs("Bambu PETG-CF @BBL H2D"),
  }),
  slotOf(1, 3, "B4", tray("", "00000000", ""), { status: "empty" }),
  slotOf(255, 254, "Ext-L", tray("PLA", "76D9F4FF", "GFA01"), { material: "pla", not_installed: true }),
  slotOf(255, 255, "Ext-R", tray("", "00000000", ""), { status: "empty" }),
];

export const PRINTER_VIEW = {
  configured: true,
  printer: {
    ip: "192.168.1.20",
    serial: PRINTER_SERIAL,
    name: "Workshop H2D",
    model: "H2D",
    firmware: "01.01.01.00",
  },
  connection: { state: "connected" },
  state: {
    gcode_state: "RUNNING",
    subtask_name: "T-pose - slim H2D dual AMS riser",
    mc_percent: 6,
    mc_remaining_time: 549,
    layer_num: 1,
    total_layer_num: 200,
    bed_temp: 70.0,
    bed_target_temp: 70.0,
    nozzles: [
      { id: 0, temp: 245.0, target_temp: 245.0, diameter: 0.4, nozzle_type: "HS01" },
      { id: 1, temp: 47.0, target_temp: 0.0, diameter: 0.4, nozzle_type: "HS01" },
    ],
    active_nozzle: 0,
    ams_units: [
      { id: 0, humidity_level: 5, humidity_pct: 21, temp: 27.0 },
      { id: 1, humidity_level: 5, humidity_pct: 18, temp: 29.7 },
    ],
  },
  slots: PRINTER_SLOTS,
  errors: [
    {
      kind: "hms",
      code: "0300_0100_0001_0007",
      text: "The heatbed temperature is abnormal; the sensor may have an open circuit.",
      wiki_url: "https://wiki.bambulab.com/en/h2/troubleshooting/hmscode/0300_0100_0001_0007",
    },
    {
      kind: "print_error",
      code: "0300_400C",
      text: null,
      wiki_url: "https://wiki.bambulab.com/en/hms/home",
    },
  ],
};

export const PRINTER_UNCONFIGURED = {
  configured: false,
  printer: null,
  connection: { state: "disconnected" },
  state: null,
  slots: [],
  errors: [],
};

/** PRINTER_VIEW with one slot changed. */
export const withSlot = (label, patch) => ({
  ...PRINTER_VIEW,
  slots: PRINTER_SLOTS.map((s) => (s.label === label ? { ...s, ...patch } : s)),
});

/** A3 after the user assigns PolyLite: the printer still reports PETG. */
export const A3_ASSIGNED = {
  assigned_preset: "Polymaker PolyLite PLA @BBL X1C 0.4 nozzle",
  assigned_filament_id: "PA-PL-WHTPA0-01",
  status: "different",
  needs_cloud_sync: true,
};

// Two presets the Health check reports as not syncing: one with a made-up
// BambuMate_ id (ticked by default) and one that only matches BambuMate's
// file shape (unticked).
export const UNSYNCED_CONFIRMED_PATH =
  "/Users/runner/Library/Application Support/BambuStudio/user/00000001/filament/base/BambuMate_Old_Copy_18f2a.json";
const UNSYNCED_SIGNATURE_PATH =
  "/Users/runner/Library/Application Support/BambuStudio/user/00000001/filament/base/Acme PLA @Bambu Lab X1 Carbon 0.4 nozzle.json";

// -- model viewport ---------------------------------------------------------
//
// A 10 mm cube: its corners as dots, its 12 edges as line segments, and two
// risks (the top centre and a top corner), as `model_view` returns them.
const f32b64 = (arr) => Buffer.from(new Float32Array(arr).buffer).toString("base64");
const CUBE = [0,0,0, 10,0,0, 10,10,0, 0,10,0, 0,0,10, 10,0,10, 10,10,10, 0,10,10];
const E = (a, b) => [...CUBE.slice(a * 3, a * 3 + 3), ...CUBE.slice(b * 3, b * 3 + 3)];
export const MODEL_VIEW = {
  name: "cube.stl",
  triangle_count: 12,
  bounds_min: [0, 0, 0],
  bounds_max: [10, 10, 10],
  dots: f32b64(CUBE),
  dot_normals: f32b64(CUBE.map(() => 0)),
  edges: f32b64([0,1, 1,2, 2,3, 3,0, 4,5, 5,6, 6,7, 7,4, 0,4, 1,5, 2,6, 3,7].reduce((acc, _, i, a) => (i % 2 ? acc : [...acc, ...E(a[i], a[i + 1])]), [])),
  risks: [
    { kind: "overhang", position: [5, 5, 10], value: 64, message: "Overhang · 64 mm² steeper than 45° — may need support" },
    { kind: "tall_thin", position: [10, 10, 10], value: 6, message: "Tall and thin · 6:1 — may wobble; slow down or add support" },
  ],
};

// -- filament advisor ---------------------------------------------------------
export const reason = (need, verdict, detail) => ({ need, verdict, detail });
const asaEntry = (brand) => {
  const slug = brand.toLowerCase().replace(/ /g, "-");
  return { brand, name: "ASA", material: "ASA", url_slug: "asa-asa", full_url: `https://www.spoolscout.com/data-sheets/${slug}/asa-asa` };
};
export const product = (brand, status, slot) => {
  const entry = asaEntry(brand);
  return { name: "ASA", brand, status, slot, buy_url: entry.full_url, entry };
};
/** What advise answers for outdoor + heat ≥80 on an enclosed printer. */
export const ADVICE = {
  needs: ["heat_80", "outdoor_uv"],
  fits: [
    {
      material: "asa", name: "ASA", total: 6, fit_pct: 100,
      reasons: [reason("heat_80", "met", "heat deflection ≈95 °C"), reason("outdoor_uv", "met", "3/3")],
      printer_notes: [], geometry_notes: ["Supports likely, whatever the material"], material_notes: [],
      products: [product("Bambu Lab", "loaded", "A2"), product("Polymaker", "preferred", null), product("Sunlu", "catalog", null)],
    },
    {
      material: "pa_cf", name: "PA-CF", total: 5, fit_pct: 83,
      reasons: [reason("heat_80", "met", "heat deflection ≈120 °C"), reason("outdoor_uv", "partly", "2/3")],
      printer_notes: [], geometry_notes: ["Supports likely, whatever the material"], material_notes: ["Dry the spool before printing."],
      products: [],
    },
    {
      material: "pet_cf", name: "PET-CF", total: 4, fit_pct: 66,
      reasons: [reason("heat_80", "met", "heat deflection ≈80 °C"), reason("outdoor_uv", "partly", "2/3")],
      printer_notes: [], geometry_notes: ["Supports likely, whatever the material"], material_notes: ["Dry the spool before printing."],
      products: [],
    },
  ],
  ruled_out: [],
  printer_line: null,
  model_error: null,
  catalog_empty: false,
};

// -- print journal ------------------------------------------------------------
//
// Shapes mirror src-tauri/src/journal/types.rs (PrintEntry, MonthSummary).
// Times are Unix seconds around 2026-10-24. The list is newest first, as
// `journal_list` returns it; ids 4 and 5 are older prints added after the
// first three, so ids don't follow the start times.
export const JOURNAL_NOW = 1792850700;
const DAY = 86400;
const usedTray = (ams_id, tray_id, label, tray_type, color, brand = "") => ({ ams_id, tray_id, label, tray_type, color, brand });
const journalEntry = (fields) => ({
  job_key: "",
  gcode_file: null,
  plate: null,
  printer_model: "H2D",
  start_approx: false,
  ended_at: null,
  error_code: null,
  hms: [],
  verdict: null,
  tags: [],
  notes: "",
  slice: null,
  trays: [],
  photos: [],
  analyses: [],
  updated_at: JOURNAL_NOW,
  ...fields,
});
export const JOURNAL_PRINTING = journalEntry({
  id: 3,
  job_key: "local:Gear housing|/data/Metadata/plate_1.gcode",
  name: "Gear housing",
  gcode_file: "/data/Metadata/plate_1.gcode",
  plate: 1,
  started_at: JOURNAL_NOW - 1800,
  outcome: "printing",
  trays: [usedTray(0, 1, "A2", "PLA", "FFFFFFFF", "PLA Basic")],
});
export const JOURNAL_FINISHED = journalEntry({
  id: 2,
  job_key: "local:T-pose - slim H2D dual AMS riser|",
  name: "T-pose - slim H2D dual AMS riser",
  plate: 1,
  started_at: JOURNAL_NOW - DAY - 7080,
  ended_at: JOURNAL_NOW - DAY,
  outcome: "finished",
  slice: {
    model_name: "cube.stl",
    model_path: SLICE_MODEL,
    printer: H2C,
    process: H2C_PROCESS,
    filament: "Bambu PLA Basic @BBL X1C 0.4 nozzle",
    plate: 1,
    time_seconds: 843,
    weight_g: 3.69,
    filaments: [{ slot: 1, filament_type: "PLA", color: "#00AE42", used_g: 3.69 }],
    has_thumbnail: true,
  },
  trays: [usedTray(0, 1, "A2", "PLA", "FFFFFFFF", "PLA Basic"), usedTray(1, 2, "B3", "PETG-CF", "000000FF", "PETG-CF")],
  photos: [{ id: 7, file_name: "5f0c.jpg", added_at: JOURNAL_NOW - DAY + 600 }],
});
export const JOURNAL_FAILED = journalEntry({
  id: 1,
  job_key: "local:Bracket v2|/data/Metadata/plate_2.gcode",
  name: "Bracket v2",
  gcode_file: "/data/Metadata/plate_2.gcode",
  plate: 2,
  started_at: JOURNAL_NOW - 3 * DAY - 2400,
  start_approx: true,
  ended_at: JOURNAL_NOW - 3 * DAY,
  outcome: "failed",
  error_code: 0x0300400c,
  hms: ["0300_0100_0001_0007"],
  verdict: "failed",
  tags: ["warping"],
  notes: "Lifted at the front corner.",
  trays: [usedTray(1, 2, "B3", "PETG-CF", "000000FF", "PETG-CF")],
});
// Finished and rated: counts as "good" in the month, not as "finished".
export const JOURNAL_GOOD = journalEntry({
  id: 4,
  job_key: "4f1d8a2c-task",
  name: "Filament swatch card",
  gcode_file: "/data/Metadata/plate_1.gcode",
  plate: 1,
  started_at: JOURNAL_NOW - 4 * DAY - 5400,
  ended_at: JOURNAL_NOW - 4 * DAY,
  outcome: "finished",
  verdict: "good",
  trays: [usedTray(0, 0, "A1", "PLA", "00AE42FF", "PLA Matte"), usedTray(254, 254, "Ext-L", "PLA", "76D9F4FF", "PLA Basic")],
});
// The app was closed through the end: no end time, no duration.
export const JOURNAL_UNKNOWN = journalEntry({
  id: 5,
  job_key: "local:Cable clip x12|/data/Metadata/plate_1.gcode",
  name: "Cable clip x12",
  gcode_file: "/data/Metadata/plate_1.gcode",
  plate: 1,
  started_at: JOURNAL_NOW - 6 * DAY,
  outcome: "unknown",
  trays: [usedTray(0, 3, "A4", "TPU", "1A1A1AFF", "TPU for AMS")],
});
export const JOURNAL_ENTRIES = [JOURNAL_PRINTING, JOURNAL_FINISHED, JOURNAL_FAILED, JOURNAL_GOOD, JOURNAL_UNKNOWN];
// What the backend counts for JOURNAL_ENTRIES: a verdict counts first, the
// unknown and printing entries only in the total, and the hours only where
// an end was seen (7080 + 2400 + 5400 s).
export const JOURNAL_SUMMARY = { prints: 5, good: 1, usable: 0, finished: 1, failed: 1, printing_seconds: 14880 };

// -- calibration --------------------------------------------------------------
//
// Shapes mirror src-tauri/src/commands/calibration.rs (RunView, PreviewView,
// SaveAnswer), as src/calibration/wire.json records them: the page's mirrors
// default none of the always-sent fields, so a missing one fails to decode.
// A2 holds an RFID Bambu PLA Basic spool, so its run calibrates the system
// preset and the first save makes the calibrated copy. The maths is the
// backend's: these answers are what it would give.

export const CAL_SYSTEM_PATH = `${SYSTEM_FILAMENTS}/Bambu PLA Basic @BBL H2D.json`;
export const CAL_COPY = "Bambu PLA Basic (calibrated) @BBL H2D";
const CAL_COPY_PATH = `${USER_FILAMENTS}/${CAL_COPY}.json`;
const CAL_STEPS = ["temperature", "flow", "retraction", "max_speed", "pressure_advance"];
const calSteps = (patch = {}) =>
  CAL_STEPS.map((step) => ({ step, status: { state: "not_run" }, fine_ready: false, opened_at: null, ...(patch[step] ?? {}) }));
const saved = (result_id, value) => ({ status: { state: "saved", result_id, value, at: JOURNAL_NOW } });

export const CAL_RUN = {
  id: 7,
  preset_name: "Bambu PLA Basic @BBL H2D",
  preset_path: CAL_SYSTEM_PATH,
  target_name: "Bambu PLA Basic @BBL H2D",
  target_path: CAL_SYSTEM_PATH,
  is_system: true,
  will_save_as: CAL_COPY,
  filament_id: "GFA00",
  material: "PLA",
  printer_model: "H2D",
  nozzle: "0.4",
  slot_label: "A2",
  variant: "Direct Drive Standard",
  variants: ["Direct Drive Standard", "Direct Drive High Flow", "Direct Drive E3D High Flow"],
  needs_variant: false,
  preset_error: null,
  steps: calSteps(),
  results: [],
  created_at: JOURNAL_NOW - 3600,
  updated_at: JOURNAL_NOW - 3600,
};

/** A3's run: a system PETG preset, a different run from A2's. */
export const CAL_RUN_A3 = {
  ...CAL_RUN,
  id: 8,
  preset_name: A3_DETECTED,
  preset_path: A3_DETECTED_PATH,
  target_name: A3_DETECTED,
  target_path: A3_DETECTED_PATH,
  will_save_as: "Generic PETG (calibrated) @BBL H2D 0.4 nozzle",
  filament_id: "GFG99",
  material: "PETG",
  slot_label: "A3",
};

/** A preset with several nozzle variants and none chosen: the card waits for the user. */
export const CAL_RUN_NEEDS_VARIANT = { ...CAL_RUN, variant: null, needs_variant: true };

const calResult = (id, test, value, extra = {}) => {
  const result = {
    id,
    test,
    value,
    preset_key: "nozzle_temperature",
    old_value: "220",
    new_value: "225",
    target_name: CAL_COPY,
    variant: "Direct Drive Standard",
    saved: true,
    undone: false,
    created_at: JOURNAL_NOW,
    photo: null,
    journal_entry_id: null,
    can_undo: true,
    created_copy: false,
    ...extra,
  };
  // What the page shows for the old value; `printer default` only for a "nil" override.
  return { old_display: result.old_value, ...result };
};

/** The first save: it made the copy, so Undo moves the copy to the backups. */
export const CAL_RESULT_TEMP = calResult(11, "temperature", "225 °C", { created_copy: true });

/** After saving 225 °C: the copy exists and is the target. */
export const CAL_RUN_TEMP_SAVED = {
  ...CAL_RUN,
  target_name: CAL_COPY,
  target_path: CAL_COPY_PATH,
  is_system: false,
  will_save_as: null,
  steps: calSteps({ temperature: saved(11, "225 °C") }),
  results: [CAL_RESULT_TEMP],
  updated_at: JOURNAL_NOW,
};

/** After Undo: back to the system preset. The result stays, undone. */
export const CAL_RUN_UNDONE = {
  ...CAL_RUN,
  results: [{ ...CAL_RESULT_TEMP, undone: true, can_undo: false }],
  updated_at: JOURNAL_NOW,
};

/** After the coarse flow pass: the fine pass is open. */
export const CAL_RUN_COARSE_SAVED = {
  ...CAL_RUN_TEMP_SAVED,
  steps: calSteps({
    temperature: saved(11, "225 °C"),
    flow: { ...saved(12, "1.029"), fine_ready: true },
  }),
  results: [
    calResult(12, "flow_coarse", "1.029", { preset_key: "filament_flow_ratio", old_value: "0.98", new_value: "1.029" }),
    { ...CAL_RESULT_TEMP, can_undo: false },
  ],
};

/** After the coarse pass on Standard, High Flow picked: its own coarse pass isn't saved, so fine stays closed. */
export const CAL_RUN_HIGH_FLOW_NO_COARSE = {
  ...CAL_RUN_COARSE_SAVED,
  variant: "Direct Drive High Flow",
  steps: calSteps({ temperature: saved(11, "225 °C"), flow: saved(12, "1.029") }),
};

export const CAL_RUN_PA_RECORDED = {
  ...CAL_RUN,
  steps: calSteps({ pressure_advance: { status: { state: "recorded", result_id: 13, value: "K 0.025", at: JOURNAL_NOW } } }),
  results: [
    calResult(13, "pressure_advance", "K 0.025", {
      preset_key: null,
      old_value: null,
      old_display: null,
      new_value: "0.025",
      target_name: "",
      saved: false,
      can_undo: false,
    }),
  ],
};

const CAL_BANDS = [230, 225, 220, 215, 210, 205, 200, 195, 190].map((t, index) => ({ index, label: `${t} °C` }));
export const CAL_TEMP_DEFAULTS = { setup: { test: "temperature", start: 230, end: 190 }, current: "220", material_label: "PLA" };
/** The tower, and the 225 °C band's value and diff. The page shows the value only for a request that had a reading. */
export const CAL_TEMP_PREVIEW = {
  diagram: { kind: "bands", bands: CAL_BANDS },
  labels: CAL_BANDS.map((b) => b.label),
  value: {
    value: 225,
    text: "225",
    display: "225 °C",
    change: {
      preset_key: "nozzle_temperature",
      old: "220",
      old_display: "220",
      new: "225",
      extra: { key: "nozzle_temperature_initial_layer", old: "220", new: "225" },
      target_name: CAL_COPY,
      will_copy: true,
      variant: "Direct Drive Standard",
    },
    change_error: null,
  },
};

const CAL_MODS = [-20, -15, -10, -5, 0, 5, 10, 15, 20].map((m) => ({ modifier: m, label: m > 0 ? `+${m}` : `${m}` }));
export const CAL_FLOW_DEFAULTS = { setup: { test: "flow_coarse", base: 0.98 }, current: "0.98", material_label: null };
export const CAL_FLOW_PREVIEW = {
  diagram: { kind: "patches", patches: CAL_MODS },
  labels: CAL_MODS.map((p) => p.label),
  value: {
    value: 1.029,
    text: "1.029",
    display: "1.029",
    change: {
      preset_key: "filament_flow_ratio",
      old: "0.98",
      old_display: "0.98",
      new: "1.029",
      extra: null,
      target_name: CAL_COPY,
      will_copy: false,
      variant: "Direct Drive Standard",
    },
    change_error: null,
  },
};

// The fine pass slices from the saved coarse value: -9 to 0 in steps of 1.
const CAL_FINE_MODS = [-9, -8, -7, -6, -5, -4, -3, -2, -1, 0].map((m) => ({ modifier: m, label: `${m}` }));
export const CAL_FINE_DEFAULTS = { setup: { test: "flow_fine", base: 1.029 }, current: "1.029", material_label: null };
export const CAL_FINE_PREVIEW = {
  diagram: { kind: "patches", patches: CAL_FINE_MODS },
  labels: CAL_FINE_MODS.map((p) => p.label),
  value: null,
};

export const CAL_PA_DEFAULTS = { setup: { test: "pressure_advance" }, current: null, material_label: null };
export const CAL_PA_PREVIEW = {
  diagram: { kind: "none" },
  labels: [],
  value: { value: 0.025, text: "0.025", display: "K 0.025", change: null, change_error: null },
};

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
      instantiable: true,
    },
    {
      name: "Bambu PLA Basic @BBL X1C 0.4 nozzle",
      filament_type: "PLA",
      filament_id: "GFA00",
      path: "/Users/runner/Library/Application Support/BambuStudio/user/00000001/filament/Bambu PLA Basic @BBL X1C 0.4 nozzle.json",
      is_user_profile: true,
      instantiable: true,
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

  // -- profile management (/profiles) --
  //
  // read_profile is the one command whose invoke string does not match its
  // wrapper name: commands::read_profile invokes "read_profile_command".
  read_profile_command: {
    name: "Polymaker PolyLite PLA @BBL X1C 0.4 nozzle",
    filament_type: "PLA",
    filament_id: "PA-PL-WHTPA0-01",
    inherits: "fdm_filament_pla",
    field_count: 42,
    nozzle_temperature: ["220", "220"],
    bed_temperature: ["35"],
    compatible_printers: ["Bambu Lab X1 Carbon 0.4 nozzle"],
    metadata: {
      sync_info: "synced",
      user_id: "00000001",
      setting_id: "PFUSb4a1c2",
      base_id: "GFL01",
      updated_time: 1756400000,
    },
    raw_json: '{"name":"Polymaker PolyLite PLA","nozzle_temperature":["220","220"]}',
  },
  extract_specs_from_profile: SPECS,
  update_profile_field: null,
  duplicate_profile: null,
  delete_profile: null,
  save_profile_specs: null,

  // -- compare profiles (/compare) --
  //
  // The picker groups user profiles under "My Profiles" and these under
  // "Bambu Lab Factory Profiles", so both lists must be non-empty for the
  // compare flow to have two sides to choose between.
  list_system_profiles: [
    {
      name: "Bambu PLA Basic @BBL X1C",
      filament_type: "PLA",
      filament_id: "GFA00",
      path: "/Applications/BambuStudio.app/Contents/Resources/profiles/BBL/filament/Bambu PLA Basic @BBL X1C.json",
      is_user_profile: false,
      instantiable: true,
    },
  ],
  compare_profiles: {
    profile_a_name: "Polymaker PolyLite PLA @BBL X1C 0.4 nozzle",
    profile_b_name: "Bambu PLA Basic @BBL X1C",
    categories: [
      {
        category: "Temperature",
        diffs: [
          {
            key: "nozzle_temperature",
            label: "Nozzle Temperature",
            base_value: "220",
            new_value: "215",
          },
          {
            key: "hot_plate_temp",
            label: "Bed Temperature",
            base_value: "35",
            new_value: "40",
          },
        ],
      },
      {
        category: "Retraction",
        diffs: [
          {
            key: "retraction_distance_mm",
            label: "Retraction Distance",
            base_value: "0.4",
            new_value: "0.8",
          },
        ],
      },
    ],
    total_fields: 42,
    changed_fields: 3,
  },

  // -- batch generate (/batch) --
  list_catalog_brands: ["Bambu Lab", "Polymaker", "Prusament"],
  refresh_catalog: { entry_count: 847, needs_refresh: false },
  get_ai_capabilities: { provider: "openai", model_id: "gpt-5.4", text: true, vision: true },
  advise_interpret: ["heat_80", "outdoor_uv"],
  advise: ADVICE,
  // -- preferred brands (Settings → General) --
  get_preferred_brands: [],
  set_preferred_brands: [],
  // One failure among the successes on purpose: it exercises the .row-fail
  // branch and the error-hint span, which the all-success path never renders.
  batch_generate_brand: {
    total: 3,
    completed: 3,
    succeeded: 2,
    failed: 1,
    results: [
      {
        filament_name: "PolyLite PLA",
        brand: "Polymaker",
        material: "PLA",
        success: true,
        profile_name: "Polymaker PolyLite PLA @BBL X1C 0.4 nozzle",
        error: null,
      },
      {
        filament_name: "PolyTerra PLA",
        brand: "Polymaker",
        material: "PLA",
        success: true,
        profile_name: "Polymaker PolyTerra PLA @BBL X1C 0.4 nozzle",
        error: null,
      },
      {
        filament_name: "PolyMide PA6-CF",
        brand: "Polymaker",
        material: "PA-CF",
        success: false,
        profile_name: null,
        error: "no base profile found for PA-CF on a 0.4 nozzle",
      },
    ],
  },

  // -- health check (/health) --
  run_health_check: {
    bambu_studio_installed: true,
    bambu_studio_path: "/Applications/BambuStudio.app",
    profile_dir_accessible: true,
    profile_dir_path: "/Users/runner/Library/Application Support/BambuStudio",
    claude_api_key_set: false,
    openai_api_key_set: true,
    kimi_api_key_set: false,
    openrouter_api_key_set: false,
  },
  // status is a plain string the panel matches on: "pass" | "warn" | "fail",
  // anything else renders as SKIP. One of each so every badge branch renders.
  run_diagnostics: {
    os: "macOS",
    arch: "aarch64",
    app_version: "1.3.0",
    generated_at: "2026-08-31T09:52:25Z",
    bundled: false,
    checks: [
      {
        id: "bambu-studio-installed",
        name: "Bambu Studio Installed",
        category: "environment",
        status: "pass",
        detail: "Found at /Applications/BambuStudio.app",
        remedy: null,
        duration_ms: 12,
      },
      {
        id: "profile-dir-writable",
        name: "Profile Directory Writable",
        category: "filesystem",
        status: "warn",
        detail: "Directory exists but no user profiles were found",
        remedy: "Create a profile in Bambu Studio first.",
        duration_ms: 4,
      },
      {
        id: "bambu.preset_sync",
        name: "BambuMate presets are set to sync to Bambu Cloud",
        category: "bambu",
        status: "warn",
        detail: "2 presets won't sync to Bambu Cloud",
        remedy:
          "Choose Review and repair, tick the presets missing from your printer, then open Bambu Studio while signed in so it uploads them.",
        duration_ms: 3,
        action: { id: "repair_preset_sync", label: "Review and repair" },
      },
      {
        id: "network-catalog-reachable",
        name: "Catalog Reachable",
        category: "network",
        status: "skip",
        detail: "Network checks were not requested",
        remedy: null,
        duration_ms: 0,
      },
    ],
    summary: { passed: 1, warned: 2, failed: 0, skipped: 1 },
  },

  list_unsynced_presets: {
    presets: [
      {
        path: UNSYNCED_CONFIRMED_PATH,
        profile_name: "Old Copy",
        file_name: "BambuMate_Old_Copy_18f2a.json",
        source: "confirmed",
      },
      {
        path: UNSYNCED_SIGNATURE_PATH,
        profile_name: "Acme PLA @Bambu Lab X1 Carbon 0.4 nozzle",
        file_name: "Acme PLA @Bambu Lab X1 Carbon 0.4 nozzle.json",
        source: "signature",
      },
    ],
    bambu_studio_running: false,
  },
  repair_preset_sync: { repaired: [UNSYNCED_CONFIRMED_PATH], skipped: [] },
  latest_generated_preset: { profile_name: "Polymaker PETG Red", created_at: 1791158400 },

  // -- settings (/settings) --
  // Option<String>: null is "no key stored", which is a real state and keeps
  // anything secret-shaped out of the repo.
  get_api_key: null,
  get_stl_watch_dir: null,
  set_stl_watch_dir: null,
  list_models: {
    models: [
      {
        id: "gpt-5.4",
        name: "GPT-5.4",
        recommended: true,
        vision: true,
        is_preview: false,
        input_cost: 2.5,
        output_cost: 10.0,
        release_date: "2026-02-11",
        context: 400000,
        quality_tier: 1,
        unverified: false,
      },
      {
        id: "gpt-5-mini",
        name: "GPT-5 mini",
        recommended: false,
        vision: true,
        is_preview: false,
        input_cost: 0.25,
        output_cost: 2.0,
        release_date: null,
        context: null,
        quality_tier: 2,
        unverified: false,
      },
    ],
    vision_available: true,
    recommended_id: "gpt-5.4",
  },
  validate_model: {
    text_ok: true,
    vision_ok: true,
    text_message: "Model supports text.",
    vision_message: "Model supports vision.",
  },
  validate_bambu_studio_path: {
    valid: true,
    has_system_profiles: true,
    has_config_file: true,
    message: "Valid BambuStudio configuration directory",
  },
  search_bambu_studio_config: "/Users/runner/Library/Application Support/BambuStudio",
  pick_config_folder: null,
  refresh_model_catalog: 2,
  reset_to_clean_install: null,

  // -- about (/about) --
  get_app_version: { current_version: "1.3.0" },
  open_external_url: null,

  // -- agent panel --
  agent_readiness: { state: "ready", detail: "test@example.com · plus" },
  agent_models: [
    { id: "gpt-test", display_name: "GPT Test", efforts: ["low", "medium", "high"], is_default: true },
    { id: "gpt-6.1-sol", display_name: "GPT-6.1 Sol", efforts: ["low", "medium", "high"], is_default: false },
  ],
  agent_get_settings: { full_access: false, claude_auth_mode: "api_key", claude_auth_modes: ["api_key"], codex_model: "gpt-6.1-sol", codex_effort: "medium" },
  agent_set_settings: { full_access: false, claude_auth_mode: "api_key", claude_auth_modes: ["api_key"], codex_model: "gpt-6.1-sol", codex_effort: "medium" },
  agent_set_app_state: null,
  agent_start: "sess-1",
  agent_send: 1,
  agent_answer: null,
  agent_interrupt: null,
  agent_rewind: true,
  agent_rewind_preview: { delete: [], overwrite: ["/p/A.json"] },
  agent_delete_session: null,
  agent_stage_image: "/tmp/agent-upload.png",
  agent_login: null,

  // -- slicing --
  slicer_status: {
    installed: true,
    version: "02.08.02.61",
    supported: true,
    min_version: "02.08.00.00",
    tested_version: "02.08.02.61",
    message: null,
  },
  slicer_presets: {
    printers: [
      { name: H2C, source: "system" },
      { name: "Bambu Lab H2S 0.4 nozzle", source: "system" },
    ],
    processes: [
      { name: "My 0.16 Fine", source: "user" },
      { name: H2C_PROCESS, source: "system" },
    ],
    filaments: [
      { name: "SUNLU PETG High Speed @Bambu Lab H2C 0.4 nozzle", source: "user" },
      { name: PLA, source: "system" },
      { name: PETG, source: "system" },
    ],
  },
  slicer_get_settings: SLICER_SETTINGS,
  slicer_set_settings: SLICER_SETTINGS,
  slicer_jobs: [],
  // Each Slice press returns the next queued job; the last one repeats.
  slicer_slice: {
    __sequence: [1, 2, 3, 4].map((id) => sliceJob(id, { state: "queued", position: 0 })),
  },
  slicer_cancel: true,
  slicer_thumbnail: PNG_1X1,
  slicer_open_in_bambu_studio: { launched: true, app_path: "/Applications/BambuStudio.app", was_already_running: false },
  slicer_clear_cache: 12582912,
  slicer_pick_model: SLICE_MODEL,
  slicer_stage_model: SLICE_MODEL,
  slicer_model_exists: true,
  // The viewport shows this cube whatever path it is asked for.
  model_view: MODEL_VIEW,

  // -- printer --
  // Not set up in Settings yet, so the discovery flow has an empty form.
  printer_get_config: null,
  printer_view: PRINTER_VIEW,
  printer_discover: { printers: [
    { ip: "192.168.1.20", serial: PRINTER_SERIAL, name: "Workshop H2D", model: "H2D", conflict: false },
  ], warnings: [], listen_seconds: 15 },
  printer_test_connection: {
    connection: { state: "cert_untrusted", fingerprint: PRINTER_FINGERPRINT },
    got_report: false,
    model: null,
  },
  printer_save: {
    ip: "192.168.1.20",
    serial: PRINTER_SERIAL,
    name: "Workshop H2D",
    model: "H2D",
    pinned_fingerprint: PRINTER_FINGERPRINT,
    has_access_code: true,
  },
  printer_remove: null,
  printer_assign_slot: withSlot("A3", A3_ASSIGNED),
  printer_clear_slot: PRINTER_VIEW,

  // -- print journal --
  journal_list: JOURNAL_ENTRIES,
  journal_month_summary: JOURNAL_SUMMARY,
  journal_get: JOURNAL_FINISHED,
  journal_last: JOURNAL_FINISHED,
  journal_update: JOURNAL_FINISHED,
  journal_add_photo: { ...JOURNAL_FINISHED, photos: [...JOURNAL_FINISHED.photos, { id: 8, file_name: "9a1e.jpg", added_at: JOURNAL_NOW }] },
  journal_remove_photo: { ...JOURNAL_FINISHED, photos: [] },
  journal_delete: null,
  journal_image: PNG_1X1,

  // -- calibration --
  calibration_runs: [{ id: 7, preset_name: "Bambu PLA Basic @BBL H2D", slot_label: "A2", updated_at: JOURNAL_NOW, result_ids: [11] }],
  calibration_start: CAL_RUN,
  calibration_set_variant: CAL_RUN,
  calibration_defaults: CAL_TEMP_DEFAULTS,
  calibration_preview: CAL_TEMP_PREVIEW,
  calibration_save: { outcome: "saved", run: CAL_RUN_TEMP_SAVED, result_id: 11, studio_running: false, assign_error: null },
  calibration_record: CAL_RUN_PA_RECORDED,
  calibration_undo: CAL_RUN_UNDONE,
  calibration_skip: CAL_RUN,
  // Never a real model: the suggestion is canned, so no test spends tokens.
  calibration_suggest: { photo: "c0ffee.jpg", suggestion: { pick: "225 °C", confidence: 0.8, reason: "Least stringing." }, note: null },
  calibration_image: PNG_1X1,
  calibration_link_candidates: { suggested: 2, entries: [JOURNAL_FINISHED, JOURNAL_FAILED] },
  calibration_link_print: { ...CAL_RUN_TEMP_SAVED, results: [{ ...CAL_RESULT_TEMP, journal_entry_id: 2 }] },
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
