//! `bm_slice` and `bm_slice_result`: slice with the user's Bambu Studio and
//! read back time, weight, cost and warnings. Neither prints or uploads.

use serde_json::{json, Value};

use super::{arg_opt_str, ToolOutput, ToolRegistry, ToolSpec};
use crate::commands::slicer::finished_output;
use crate::slicer::jobs::{JobState, JobView};
use crate::slicer::result::WarningLevel;

/// One job plus up to three comparison filaments: the Slice page's maximum
/// of four columns.
pub const MAX_COMPARE: usize = 3;

/// What `bm_slice` asks the host to queue. Missing presets fall back to the
/// user's slicing defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceToolRequest {
    /// Already validated: an existing `.stl` or `.3mf`, canonical.
    pub model_path: String,
    pub printer: Option<String>,
    pub process: Option<String>,
    pub filament: Option<String>,
    pub compare_filaments: Vec<String>,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bm_slice",
            description: "Slice an .stl or .3mf with the user's Bambu Studio and return per-plate print time, weight, cost, filament use and slicer warnings, plus the sliced file's path. Missing presets use the user's slicing defaults. compare_filaments slices the same model once per extra filament (max 3). Never prints or uploads anything.",
            input_schema: json!({"type":"object","properties":{
                "model_path":{"type":"string","description":"Absolute path of an existing .stl or .3mf model"},
                "printer":{"type":"string","description":"Bambu Studio printer preset name"},
                "process":{"type":"string","description":"Bambu Studio process preset name"},
                "filament":{"type":"string","description":"Bambu Studio filament preset name"},
                "compare_filaments":{"type":"array","items":{"type":"string"},"description":"Up to 3 extra filament presets, sliced with the same printer and process"}
            },"required":["model_path"]}),
        },
        ToolSpec {
            name: "bm_slice_result",
            description: "Status and result summary of a slicing job started by bm_slice or the Slice page.",
            input_schema: json!({"type":"object","properties":{"job_id":{"type":"integer"}},"required":["job_id"]}),
        },
    ]
}

/// `2h 14m`, `14m`, `42s`.
pub fn format_duration(secs: u64) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{secs}s")
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// What the agent sees of a job. The output path is included; the user's
/// presets folder and Bambu Studio's paths are not.
pub fn job_summary(view: &JobView) -> Value {
    let out = json!({
        "job_id": view.id,
        "model": view.model_name,
        "printer": view.printer,
        "process": view.process,
        "filament": view.filament,
    });
    let extra = match &view.state {
        JobState::Queued { position } => json!({"status":"queued","queue_position":position}),
        JobState::Running { progress } => json!({"status":"running","progress":progress}),
        JobState::Cancelled => json!({"status":"cancelled"}),
        JobState::Failed { error } => json!({"status":"failed","error":error.message}),
        JobState::Done { result, cached } => {
            // The sliced file is gone (the cache was cleared): don't hand
            // out its dead path or stale numbers.
            if let Err(cleared) = finished_output(view) {
                return merge(out, json!({"status":"cleared","error":cleared}));
            }
            let plates: Vec<Value> = result
                .plates
                .iter()
                .map(|p| {
                    json!({
                        "plate": p.index,
                        "time": format_duration(p.time_seconds),
                        "time_seconds": p.time_seconds,
                        "weight_g": round2(p.weight_g),
                        "cost": p.cost.map(round2),
                        "filaments": p.filaments.iter().map(|f| json!({
                            "slot": f.slot,
                            "type": f.filament_type,
                            "used_g": round2(f.used_g),
                            "used_m": round2(f.used_m),
                            "cost": f.cost.map(round2),
                        })).collect::<Vec<_>>(),
                        "warnings": p.warnings.iter().map(|w| json!({
                            "level": match w.level { WarningLevel::Warning => "warning", WarningLevel::Notice => "notice" },
                            "message": w.message,
                        })).collect::<Vec<_>>(),
                        "objects": p.objects.iter().map(|o| o.name.clone()).collect::<Vec<_>>(),
                    })
                })
                .collect();
            json!({
                "status": "done",
                "cached": cached,
                "plates": plates,
                "output_path": result.output_path,
                "note": "Sliced only. BambuMate never prints; the user prints from Bambu Studio.",
            })
        }
    };
    merge(out, extra)
}

fn merge(mut out: Value, extra: Value) -> Value {
    if let (Value::Object(o), Value::Object(e)) = (&mut out, extra) {
        o.extend(e);
    }
    out
}

/// A preset argument, trimmed; blank means "use the default".
fn arg_preset(args: &Value, key: &str) -> Option<String> {
    arg_opt_str(args, key)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub async fn handle(reg: &ToolRegistry, name: &str, args: &Value) -> Option<ToolOutput> {
    let host = reg.host();
    Some(match name {
        "bm_slice" => {
            let Some(raw) = arg_opt_str(args, "model_path") else {
                return Some(ToolOutput::error("missing string argument 'model_path'"));
            };
            let model_path = match crate::slicer::validate_model_path(&raw) {
                Ok((p, _)) => p.to_string_lossy().into_owned(),
                Err(e) => return Some(ToolOutput::error(e.to_string())),
            };
            let compare: Vec<String> =
                match args.get("compare_filaments") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::Array(items)) => {
                        let mut names = Vec::new();
                        for item in items {
                            match item.as_str().map(str::trim) {
                                Some(s) if !s.is_empty() => names.push(s.to_string()),
                                _ => return Some(ToolOutput::error(
                                    "'compare_filaments' entries must be non-blank preset names",
                                )),
                            }
                        }
                        names
                    }
                    Some(_) => {
                        return Some(ToolOutput::error(
                            "'compare_filaments' must be an array of preset names",
                        ))
                    }
                };
            if compare.len() > MAX_COMPARE {
                return Some(ToolOutput::error(format!(
                    "compare at most {MAX_COMPARE} extra filaments"
                )));
            }
            let req = SliceToolRequest {
                model_path,
                printer: arg_preset(args, "printer"),
                process: arg_preset(args, "process"),
                filament: arg_preset(args, "filament"),
                compare_filaments: compare,
            };
            // Stopping the agent drops the wait, which cancels the jobs it
            // queued (see `TauriToolHost::slice`).
            let mut stopped = reg.cancel_watch();
            tokio::select! {
                res = host.slice(req) => match res {
                    Ok(views) => ToolOutput::json(&json!({
                        "jobs": views.iter().map(job_summary).collect::<Vec<_>>()
                    })),
                    Err(e) => ToolOutput::error(e),
                },
                _ = stopped.fired() => ToolOutput::error("stopped; the queued slicing jobs were cancelled"),
            }
        }
        "bm_slice_result" => {
            let Some(id) = args.get("job_id").and_then(Value::as_u64) else {
                return Some(ToolOutput::error("missing integer argument 'job_id'"));
            };
            match host.slice_job(id) {
                Some(v) => ToolOutput::json(&job_summary(&v)),
                None => ToolOutput::error(format!("no slicing job {id}")),
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::fake_host::{registry_with, FakeHost};
    use std::sync::Arc;

    #[test]
    fn durations_read_like_the_slice_page() {
        assert_eq!(format_duration(8040), "2h 14m");
        assert_eq!(format_duration(843), "14m");
        assert_eq!(format_duration(42), "42s");
    }

    #[tokio::test]
    async fn slice_validates_the_model_path() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        let txt = h.user_dir.path().join("notes.txt");
        std::fs::write(&txt, b"x").unwrap();
        for args in [
            json!({}),
            json!({"model_path": txt.to_string_lossy()}),
            json!({"model_path": "/nope/cube.stl"}),
        ] {
            let out = reg.call("bm_slice", args.clone()).await;
            assert!(!out.ok, "{args}");
        }
        assert!(
            h.calls.lock().unwrap().is_empty(),
            "nothing reached the host"
        );
    }

    #[tokio::test]
    async fn slice_forwards_presets_and_summarises_results() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        let stl = h.user_dir.path().join("cube.stl");
        std::fs::write(&stl, b"solid cube").unwrap();
        let out = reg
            .call(
                "bm_slice",
                json!({"model_path": stl.to_string_lossy(), "filament":"Bambu PLA Basic @BBL H2C","compare_filaments":["Generic PETG @BBL H2C 0.4 nozzle"]}),
            )
            .await;
        assert!(out.ok, "{}", out.summary());
        let calls = h.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("slice:"));
        assert!(calls[0].ends_with(":Bambu PLA Basic @BBL H2C:Generic PETG @BBL H2C 0.4 nozzle"));
        let crate::agent::tools::ToolContent::Text(text) = &out.content[0] else {
            panic!()
        };
        let v: Value = serde_json::from_str(text).unwrap();
        let job = &v["jobs"][0];
        assert_eq!(job["status"], "done");
        assert_eq!(job["plates"][0]["time"], "14m");
        assert_eq!(job["plates"][0]["weight_g"], 3.69);
        assert_eq!(job["plates"][0]["cost"], 0.07);
        assert!(job["note"].as_str().unwrap().contains("never prints"));
    }

    #[tokio::test]
    async fn stopping_the_agent_ends_the_wait_and_drops_the_queued_jobs() {
        let h = Arc::new(FakeHost::new());
        h.slice_hangs
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (reg, _rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let stl = h.user_dir.path().join("cube.stl");
        std::fs::write(&stl, b"solid").unwrap();
        let call = {
            let reg = reg.clone();
            let args = json!({"model_path": stl.to_string_lossy()});
            tokio::spawn(async move { reg.call("bm_slice", args).await })
        };
        while !h.slice_waiting.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        assert!(!h.slice_dropped.load(std::sync::atomic::Ordering::SeqCst));
        reg.cancel_calls();
        let out = tokio::time::timeout(std::time::Duration::from_secs(30), call)
            .await
            .expect("the wait ended")
            .unwrap();
        assert!(!out.ok);
        assert!(out.summary().contains("cancelled"));
        assert!(
            h.slice_dropped.load(std::sync::atomic::Ordering::SeqCst),
            "the host's wait was dropped, which cancels its jobs"
        );
    }

    #[tokio::test]
    async fn a_stop_before_a_call_does_not_cancel_it() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        reg.cancel_calls();
        let stl = h.user_dir.path().join("cube.stl");
        std::fs::write(&stl, b"solid").unwrap();
        let out = reg
            .call("bm_slice", json!({"model_path": stl.to_string_lossy()}))
            .await;
        assert!(out.ok, "{}", out.summary());
    }

    #[test]
    fn a_cleared_output_reports_the_clear_copy_and_no_path() {
        let mut job = crate::agent::tools::fake_host::done_job(3);
        if let JobState::Done { result, .. } = &mut job.state {
            result.output_path = "/nope/output.gcode.3mf".into();
        }
        let v = job_summary(&job);
        assert_eq!(v["status"], "cleared");
        assert_eq!(
            v["error"],
            "That slice's files were cleared; slice it again."
        );
        assert!(v.get("output_path").is_none());
        assert!(v.get("plates").is_none());
        // A job whose file is still there keeps its path.
        let ok = job_summary(&crate::agent::tools::fake_host::done_job(3));
        assert_eq!(ok["status"], "done");
        assert!(std::path::Path::new(ok["output_path"].as_str().unwrap()).is_file());
    }

    #[tokio::test]
    async fn bad_compare_entries_are_refused_and_preset_args_are_trimmed() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        let stl = h.user_dir.path().join("cube.stl");
        std::fs::write(&stl, b"solid").unwrap();
        for bad in [json!([1]), json!(["a", "  "]), json!([null]), json!("a")] {
            let out = reg
                .call(
                    "bm_slice",
                    json!({"model_path": stl.to_string_lossy(), "compare_filaments": bad}),
                )
                .await;
            assert!(!out.ok, "{bad}");
        }
        assert!(h.calls.lock().unwrap().is_empty());
        let out = reg
            .call(
                "bm_slice",
                json!({"model_path": stl.to_string_lossy(), "filament": "  PLA  ", "compare_filaments": [" PETG "]}),
            )
            .await;
        assert!(out.ok);
        assert!(h.calls.lock().unwrap()[0].ends_with(":PLA:PETG"));
        // Blank presets mean "use the default".
        let out = reg
            .call(
                "bm_slice",
                json!({"model_path": stl.to_string_lossy(), "filament": "   "}),
            )
            .await;
        assert!(out.ok);
        assert!(h.calls.lock().unwrap()[1].ends_with("::"));
    }

    #[tokio::test]
    async fn too_many_comparisons_are_refused() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        let stl = h.user_dir.path().join("cube.stl");
        std::fs::write(&stl, b"solid").unwrap();
        let out = reg
            .call(
                "bm_slice",
                json!({"model_path": stl.to_string_lossy(), "compare_filaments":["a","b","c","d"]}),
            )
            .await;
        assert!(!out.ok);
    }

    #[tokio::test]
    async fn slice_result_reports_status_or_an_unknown_id() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        let out = reg.call("bm_slice_result", json!({"job_id": 7})).await;
        let crate::agent::tools::ToolContent::Text(text) = &out.content[0] else {
            panic!()
        };
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["status"], "failed");
        assert_eq!(
            v["error"],
            "Bambu Studio couldn't slice this model: No valid nozzle found. Please check nozzle count."
        );
        let cleared = reg.call("bm_slice_result", json!({"job_id": 8})).await;
        let crate::agent::tools::ToolContent::Text(text) = &cleared.content[0] else {
            panic!()
        };
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            v["error"],
            "That slice's files were cleared; slice it again."
        );
        assert!(!text.contains("/nope"));
        assert!(!reg.call("bm_slice_result", json!({"job_id": 99})).await.ok);
        assert!(!reg.call("bm_slice_result", json!({})).await.ok);
    }
}
