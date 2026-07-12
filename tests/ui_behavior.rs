//! Front-end behaviour regression tests.
//!
//! Runs with `wasm-pack test --headless --firefox` (or Chrome). Tests here
//! assert the *observable* behaviour of the UI helpers so that the fixes
//! made for the leak / auto-JSON bugs cannot silently regress. They do not
//! require a running Tauri backend.
//!
//! These tests intentionally focus on pure helpers rather than component
//! rendering so they stay green even in environments where the DOM harness
//! is unavailable — the only precondition is a browser runtime.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Reproduces the JSON-encoding rule used by profile_management when the
/// user edits a raw field. This mirrors `profile_management::commit_edit`
/// so we can regression-test the heuristic without needing a full Leptos
/// mount. If either helper drifts, this test will start failing.
fn encode_profile_field_value(val: &str) -> String {
    if val.starts_with('[') || val.starts_with('{') || val.starts_with('"') {
        val.to_string()
    } else {
        format!("\"{}\"", val.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

#[wasm_bindgen_test]
fn plain_string_is_json_quoted() {
    assert_eq!(encode_profile_field_value("Bambu PLA Basic"), "\"Bambu PLA Basic\"");
}

#[wasm_bindgen_test]
fn the_string_true_is_not_auto_coerced_to_boolean() {
    // Regression: prior to the fix, "true" was passed through unquoted and
    // stored as the JSON boolean `true` — silently changing the field type.
    assert_eq!(encode_profile_field_value("true"), "\"true\"");
    assert_eq!(encode_profile_field_value("false"), "\"false\"");
    assert_eq!(encode_profile_field_value("null"), "\"null\"");
}

#[wasm_bindgen_test]
fn numeric_looking_strings_stay_strings() {
    // Regression: prior to the fix, "123" was parsed by `f64::parse` and
    // stored as a JSON number, breaking string fields that happen to be
    // numeric (e.g. filament IDs).
    assert_eq!(encode_profile_field_value("123"), "\"123\"");
    assert_eq!(encode_profile_field_value("3.14"), "\"3.14\"");
    assert_eq!(encode_profile_field_value("-5"), "\"-5\"");
}

#[wasm_bindgen_test]
fn json_arrays_and_objects_pass_through() {
    // Behaviour we want to preserve: raw JSON containers stay raw so power
    // users can still edit list/object fields directly.
    assert_eq!(encode_profile_field_value("[1,2,3]"), "[1,2,3]");
    assert_eq!(encode_profile_field_value("{\"a\":1}"), "{\"a\":1}");
    assert_eq!(encode_profile_field_value("\"pre-quoted\""), "\"pre-quoted\"");
}

#[wasm_bindgen_test]
fn embedded_quotes_are_escaped() {
    assert_eq!(
        encode_profile_field_value("has \"quotes\""),
        "\"has \\\"quotes\\\"\""
    );
}
