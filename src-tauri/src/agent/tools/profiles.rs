use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::{arg_opt_str, arg_str, ToolOutput, ToolRegistry, ToolSpec};
use crate::agent::types::UiCommand;
use crate::profile::inheritance::resolve_inheritance;
use crate::profile::reader::read_profile;
use crate::profile::writer::{backup_profile, restore_from_backup, write_profile_atomic};
use crate::profile::ProfileRegistry;

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bm_list_profiles",
            description: "List the user's Bambu Studio filament profiles (name, path, filament_type).",
            input_schema: json!({"type":"object","properties":{}}),
        },
        ToolSpec {
            name: "bm_read_profile",
            description: "Read a filament profile. Path may be absolute or relative to the user filament folder. Set resolved=true to merge the inherits chain.",
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"resolved":{"type":"boolean"}},"required":["path"]}),
        },
        ToolSpec {
            name: "bm_diff_profiles",
            description: "Compare two profiles and return changed fields grouped by category.",
            input_schema: json!({"type":"object","properties":{"path_a":{"type":"string"},"path_b":{"type":"string"}},"required":["path_a","path_b"]}),
        },
        ToolSpec {
            name: "bm_write_profile",
            description: "Set fields on a user profile. Backs up first, writes, then validates the result and automatically restores the backup if the change made the profile invalid (e.g. an empty name); otherwise shows the change in the UI. changes is an object of Bambu Studio keys to JSON values (most values are string arrays, e.g. {\"nozzle_temperature\":[\"215\"]}).",
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"changes":{"type":"object"}},"required":["path","changes"]}),
        },
        ToolSpec {
            name: "bm_rollback",
            description: "Restore a user profile from its most recent backup, or from backup_path if given.",
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"backup_path":{"type":"string"}},"required":["path"]}),
        },
        ToolSpec {
            name: "bm_install_profile",
            description: "Install a profile staged by bm_generate_profile into Bambu Studio.",
            input_schema: json!({"type":"object","properties":{"staged_id":{"type":"string"}},"required":["staged_id"]}),
        },
    ]
}

/// Resolve `p` (absolute or relative) and require it to live inside `dir`.
pub(crate) fn resolve_in(dir: &Path, p: &str) -> Result<PathBuf, String> {
    let raw = Path::new(p);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        dir.join(raw)
    };
    let canon_dir = dir
        .canonicalize()
        .map_err(|e| format!("profile folder unavailable: {e}"))?;
    let canon = joined.canonicalize().map_err(|e| format!("{p}: {e}"))?;
    if !canon.starts_with(&canon_dir) {
        return Err(format!(
            "{p} is outside the Bambu Studio user filament folder"
        ));
    }
    Ok(canon)
}

fn user_dir(reg: &ToolRegistry) -> Result<PathBuf, ToolOutput> {
    reg.host().user_filament_dir().map_err(ToolOutput::error)
}

fn latest_backup(profile: &Path) -> Option<PathBuf> {
    let stem = profile.file_stem()?.to_str()?.to_string();
    let dir = profile.parent()?.join(".backups");
    let mut matches: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&format!("{stem}_")))
                .unwrap_or(false)
        })
        .collect();
    matches.sort();
    matches.pop()
}

pub async fn handle(reg: &ToolRegistry, name: &str, args: &Value) -> Option<ToolOutput> {
    Some(match name {
        "bm_list_profiles" => list(reg),
        "bm_read_profile" => read(reg, args),
        "bm_diff_profiles" => diff(args),
        "bm_write_profile" => write(reg, args).await,
        "bm_rollback" => rollback(reg, args).await,
        "bm_install_profile" => install(reg, args).await,
        _ => return None,
    })
}

fn list(reg: &ToolRegistry) -> ToolOutput {
    let dir = match user_dir(reg) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let mut rows = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => return ToolOutput::error(e.to_string()),
    };
    for p in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(profile) = read_profile(&p) {
            rows.push(json!({
                "name": profile.name(),
                "path": p.to_string_lossy(),
                "filament_type": profile.filament_type(),
            }));
        }
    }
    ToolOutput::json(&json!({"profiles": rows}))
}

fn read(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let p = match arg_str(args, "path") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let dir = match user_dir(reg) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let raw = Path::new(&p);
    // Reads may target system profiles too, so only relative paths are joined.
    let path = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        dir.join(raw)
    };
    let profile = match read_profile(&path) {
        Ok(p) => p,
        Err(e) => return ToolOutput::error(e.to_string()),
    };
    let resolved = args
        .get("resolved")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !resolved {
        return ToolOutput::json(&Value::Object(profile.raw().clone()));
    }
    let Some(system_dir) = reg.host().system_filament_dir() else {
        return ToolOutput::error("system profile folder unavailable; read with resolved=false");
    };
    let mut registry = match ProfileRegistry::discover_system_profiles(&system_dir) {
        Ok(r) => r,
        Err(e) => return ToolOutput::error(e.to_string()),
    };
    let _ = registry.discover_user_profiles(&dir);
    match resolve_inheritance(&profile, &registry) {
        Ok(full) => ToolOutput::json(&Value::Object(full.raw().clone())),
        Err(e) => ToolOutput::error(e.to_string()),
    }
}

fn diff(args: &Value) -> ToolOutput {
    let a = match arg_str(args, "path_a") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let b = match arg_str(args, "path_b") {
        Ok(v) => v,
        Err(e) => return e,
    };
    match crate::commands::profile::compare_profiles(a, b, false) {
        Ok(result) => ToolOutput::json(&serde_json::to_value(result).unwrap_or(Value::Null)),
        Err(e) => ToolOutput::error(e),
    }
}

async fn write(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let p = match arg_str(args, "path") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(changes) = args.get("changes").and_then(|v| v.as_object()).cloned() else {
        return ToolOutput::error("'changes' must be an object");
    };
    let dir = match user_dir(reg) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let path = match resolve_in(&dir, &p) {
        Ok(p) => p,
        Err(e) => return ToolOutput::error(e),
    };
    if let Err(e) = reg.ensure_write_allowed(&path).await {
        return ToolOutput::error(e);
    }
    let mut profile = match read_profile(&path) {
        Ok(p) => p,
        Err(e) => return ToolOutput::error(e.to_string()),
    };
    let backup = match backup_profile(&path) {
        Ok(b) => b,
        Err(e) => return ToolOutput::error(format!("backup failed: {e}")),
    };
    let keys: Vec<String> = changes.keys().cloned().collect();
    let raw: &mut Map<String, Value> = profile.raw_mut();
    for (k, v) in changes {
        raw.insert(k, v);
    }
    if let Err(e) = write_profile_atomic(&profile, &path) {
        return ToolOutput::error(format!("write failed: {e}"));
    }
    if let Err(reason) = crate::agent::validate::validate_profile_file(&path) {
        return match restore_from_backup(&backup, &path) {
            Ok(()) => ToolOutput::error(format!(
                "change rejected: {reason}; profile restored from backup"
            )),
            Err(re) => ToolOutput::error(format!(
                "change rejected: {reason}; restoring the backup ALSO failed: {re}"
            )),
        };
    }
    reg.host().emit_ui(UiCommand::Navigate {
        route: "/profiles".into(),
        profile_path: Some(path.to_string_lossy().into_owned()),
    });
    ToolOutput::json(
        &json!({"path": path.to_string_lossy(), "changed_keys": keys, "backup_path": backup.to_string_lossy()}),
    )
}

/// Require an explicit `backup_path` to live inside the profile's own
/// `.backups/` directory (where `backup_profile` writes to). Without this,
/// `bm_rollback` could be pointed at any readable JSON file and made to
/// overwrite a user profile with it.
fn resolve_backup_path(profile_path: &Path, backup_path: &str) -> Result<PathBuf, String> {
    let backups_dir = profile_path
        .parent()
        .ok_or_else(|| "profile path has no parent directory".to_string())?
        .join(".backups");
    let canon_backups_dir = backups_dir
        .canonicalize()
        .map_err(|e| format!("no backups directory for this profile: {e}"))?;
    let canon_backup = Path::new(backup_path)
        .canonicalize()
        .map_err(|e| format!("{backup_path}: {e}"))?;
    if !canon_backup.starts_with(&canon_backups_dir) {
        return Err(format!(
            "{backup_path} is outside the profile's backups folder"
        ));
    }
    Ok(canon_backup)
}

async fn rollback(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let p = match arg_str(args, "path") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let dir = match user_dir(reg) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let path = match resolve_in(&dir, &p) {
        Ok(p) => p,
        Err(e) => return ToolOutput::error(e),
    };
    let backup = match arg_opt_str(args, "backup_path") {
        Some(b) => match resolve_backup_path(&path, &b) {
            Ok(p) => p,
            Err(e) => return ToolOutput::error(e),
        },
        None => match latest_backup(&path) {
            Some(b) => b,
            None => return ToolOutput::error("no backup found"),
        },
    };
    if reg.host().bambu_studio_running()
        && !reg
            .asks()
            .confirm(
                reg.session_id(),
                "Bambu Studio is running. Roll back anyway?",
            )
            .await
    {
        return ToolOutput::error("declined: Bambu Studio is running");
    }
    match restore_from_backup(&backup, &path) {
        Ok(()) => {
            reg.host().emit_ui(UiCommand::Refresh {
                what: "profiles".into(),
            });
            ToolOutput::json(&json!({"restored_from": backup.to_string_lossy()}))
        }
        Err(e) => ToolOutput::error(e.to_string()),
    }
}

async fn install(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let staged = match arg_str(args, "staged_id") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let running = reg.host().bambu_studio_running();
    if running
        && !reg
            .asks()
            .confirm(
                reg.session_id(),
                "Bambu Studio is running and may overwrite the new profile. Install anyway?",
            )
            .await
    {
        return ToolOutput::error("declined: Bambu Studio is running");
    }
    match reg.host().install_staged(&staged, running).await {
        Ok(v) => {
            if let Some(p) = v.get("installed_path").and_then(|p| p.as_str()) {
                let p = PathBuf::from(p);
                reg.mark_created(&p.canonicalize().unwrap_or(p));
            }
            reg.host().emit_ui(UiCommand::Refresh {
                what: "profiles".into(),
            });
            ToolOutput::json(&v)
        }
        Err(e) => ToolOutput::error(e),
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::tools::fake_host::{registry_with, FakeHost};
    use crate::agent::types::{AgentEvent, UiCommand};
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;

    const PLA: &str = r#"{"name":"My PLA","inherits":"Generic PLA @BBL X1C","from":"User","nozzle_temperature":["220"]}"#;

    /// How long a test is willing to wait for an `Ask` or a spawned tool call
    /// before treating it as a hung regression rather than a real failure.
    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    fn host_with_profile() -> Arc<FakeHost> {
        let h = Arc::new(FakeHost::new());
        fs::write(h.user_dir.path().join("My PLA.json"), PLA).unwrap();
        h
    }

    /// Waits for the next `Ask` event and answers it. Wrapped in a timeout so
    /// a regression that stops emitting the expected `Ask` fails the test
    /// instead of hanging the suite forever.
    async fn answer_next(
        rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>,
        reg: &crate::agent::tools::ToolRegistry,
        answer: &str,
    ) {
        tokio::time::timeout(TEST_TIMEOUT, async {
            loop {
                if let AgentEvent::Ask { request, .. } = rx.recv().await.unwrap() {
                    reg.asks().answer(&request.id, vec![answer.into()]).unwrap();
                    return;
                }
            }
        })
        .await
        .expect("timed out waiting for an Ask event");
    }

    /// Awaits a spawned tool call under a timeout, so a regression that never
    /// resolves the call fails fast instead of hanging the suite.
    async fn await_timeout(
        handle: tokio::task::JoinHandle<super::ToolOutput>,
    ) -> super::ToolOutput {
        tokio::time::timeout(TEST_TIMEOUT, handle)
            .await
            .expect("timed out waiting for the spawned tool call")
            .expect("spawned tool call task panicked")
    }

    #[tokio::test]
    async fn list_profiles_lists_user_json_files() {
        let h = host_with_profile();
        let (reg, _rx) = registry_with(h);
        let out = reg.call("bm_list_profiles", json!({})).await;
        assert!(out.ok, "{}", out.summary());
        assert!(out.summary().contains("My PLA"));
    }

    #[tokio::test]
    async fn read_profile_returns_raw_json() {
        let h = host_with_profile();
        let (reg, _rx) = registry_with(h);
        let out = reg
            .call("bm_read_profile", json!({"path":"My PLA.json"}))
            .await;
        assert!(out.ok, "{}", out.summary());
        assert!(out.summary().contains("nozzle_temperature"));
    }

    #[tokio::test]
    async fn write_outside_user_dir_is_refused() {
        let h = host_with_profile();
        let (reg, _rx) = registry_with(h);
        let out = reg
            .call(
                "bm_write_profile",
                json!({"path":"/etc/hosts","changes":{"a":"b"}}),
            )
            .await;
        assert!(!out.ok);
    }

    #[tokio::test]
    async fn first_write_to_existing_profile_asks_then_backs_up_and_writes() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call(
                "bm_write_profile",
                json!({"path":"My PLA.json","changes":{"nozzle_temperature":["215"]}}),
            )
            .await
        });
        answer_next(&mut rx, &reg, "Yes").await;
        let out = await_timeout(t).await;
        assert!(out.ok, "{}", out.summary());
        let body = fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap();
        assert!(body.contains("215"));
        assert_eq!(
            fs::read_dir(h.user_dir.path().join(".backups"))
                .unwrap()
                .count(),
            1
        );
        assert!(h
            .ui
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, UiCommand::Navigate { .. })));
    }

    #[tokio::test]
    async fn declined_write_changes_nothing() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call(
                "bm_write_profile",
                json!({"path":"My PLA.json","changes":{"nozzle_temperature":["199"]}}),
            )
            .await
        });
        answer_next(&mut rx, &reg, "No").await;
        assert!(!await_timeout(t).await.ok);
        assert_eq!(
            fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap(),
            PLA
        );
    }

    #[tokio::test]
    async fn rollback_restores_latest_backup() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call(
                "bm_write_profile",
                json!({"path":"My PLA.json","changes":{"nozzle_temperature":["230"]}}),
            )
            .await
        });
        answer_next(&mut rx, &reg, "Yes").await;
        assert!(await_timeout(t).await.ok);
        let out = reg.call("bm_rollback", json!({"path":"My PLA.json"})).await;
        assert!(out.ok, "{}", out.summary());
        let body = fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap();
        assert!(body.contains("220"));
    }

    #[tokio::test]
    async fn install_marks_profile_created_so_later_writes_do_not_ask() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        let out = reg
            .call("bm_install_profile", json!({"staged_id":"stg1"}))
            .await;
        assert!(out.ok, "{}", out.summary());
        // No ask is pending, so this completes without anyone answering.
        let out = reg
            .call(
                "bm_write_profile",
                json!({"path":"Polymaker PLA.json","changes":{"filament_flow_ratio":["0.97"]}}),
            )
            .await;
        assert!(out.ok, "{}", out.summary());
    }

    #[tokio::test]
    async fn install_while_studio_running_asks_first() {
        let h = Arc::new(FakeHost::new());
        h.bs_running.store(true, Ordering::SeqCst);
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call("bm_install_profile", json!({"staged_id":"stg1"}))
                .await
        });
        answer_next(&mut rx, &reg, "No").await;
        assert!(!await_timeout(t).await.ok);
        assert!(h.calls.lock().unwrap().is_empty(), "nothing installed");
    }

    // --- Fix round 1 ---

    /// Fix 1: `bm_rollback` must refuse a `backup_path` that doesn't live
    /// inside the profile's own `.backups/` directory, so it can't be pointed
    /// at an arbitrary file to overwrite the profile with it.
    #[tokio::test]
    async fn rollback_refuses_backup_path_outside_backups_dir() {
        let h = host_with_profile();
        fs::create_dir_all(h.user_dir.path().join(".backups")).unwrap();
        let outside = h.user_dir.path().join("evil.json");
        fs::write(&outside, r#"{"name":"Evil"}"#).unwrap();
        let (reg, _rx) = registry_with(h.clone());
        let out = reg
            .call(
                "bm_rollback",
                json!({"path":"My PLA.json","backup_path": outside.to_string_lossy()}),
            )
            .await;
        assert!(!out.ok, "{}", out.summary());
        assert_eq!(
            fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap(),
            PLA
        );
    }

    /// Fix 3: a write that produces an invalid profile (e.g. an empty name)
    /// must be rejected and the pre-write backup restored, leaving the file
    /// exactly as it was before the call.
    #[tokio::test]
    async fn write_profile_validates_result_and_restores_on_failure() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call(
                "bm_write_profile",
                json!({"path":"My PLA.json","changes":{"name":""}}),
            )
            .await
        });
        answer_next(&mut rx, &reg, "Yes").await;
        let out = await_timeout(t).await;
        assert!(!out.ok, "{}", out.summary());
        // The backup is restored via `write_profile_atomic`, which re-serializes
        // with 4-space indentation, so compare parsed values rather than bytes
        // (same pattern as `rollback_restores_latest_backup` below).
        let body = fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap();
        let restored: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            restored["name"], "My PLA",
            "the original name must be restored, not the empty one"
        );
        assert_eq!(restored["nozzle_temperature"], json!(["220"]));
    }

    /// Fix 4(a): `bm_diff_profiles` reports the field that actually changed.
    #[tokio::test]
    async fn diff_profiles_reports_the_changed_field() {
        let h = host_with_profile();
        let other = h.user_dir.path().join("Other PLA.json");
        // Same name as the fixture so "name" isn't itself reported as a diff —
        // otherwise "Identity & Metadata" would sort before "Temperature" and
        // push the field we care about past the 200-char summary window.
        fs::write(
            &other,
            r#"{"name":"My PLA","inherits":"Generic PLA @BBL X1C","from":"User","nozzle_temperature":["230"]}"#,
        )
        .unwrap();
        let (reg, _rx) = registry_with(h.clone());
        let out = reg
            .call(
                "bm_diff_profiles",
                json!({
                    "path_a": h.user_dir.path().join("My PLA.json").to_string_lossy(),
                    "path_b": other.to_string_lossy(),
                }),
            )
            .await;
        assert!(out.ok, "{}", out.summary());
        let summary = out.summary();
        assert!(
            summary.contains("nozzle_temperature") || summary.contains("Temperature"),
            "expected the changed field or its category in: {summary}"
        );
    }

    /// Fix 4(b): `bm_read_profile` with `resolved: true` merges in a field
    /// that only exists on the parent profile in the system dir.
    #[tokio::test]
    async fn read_profile_resolved_merges_inherited_field() {
        let sys_dir = tempfile::tempdir().unwrap();
        fs::write(
            sys_dir.path().join("Generic PLA @BBL X1C.json"),
            r#"{"name":"Generic PLA @BBL X1C","filament_max_volumetric_speed":["12"]}"#,
        )
        .unwrap();
        let mut h = FakeHost::new();
        fs::write(h.user_dir.path().join("My PLA.json"), PLA).unwrap();
        h.system_dir = Some(sys_dir.path().to_path_buf());
        let h = Arc::new(h);
        let (reg, _rx) = registry_with(h);
        let out = reg
            .call(
                "bm_read_profile",
                json!({"path":"My PLA.json","resolved":true}),
            )
            .await;
        assert!(out.ok, "{}", out.summary());
        assert!(
            out.summary().contains("filament_max_volumetric_speed"),
            "expected the inherited field in: {}",
            out.summary()
        );
    }

    /// Fix 5: two concurrent writes to the same existing, unconfirmed profile
    /// must trigger exactly one `Ask`; once it's answered "Yes", both calls
    /// succeed without a second prompt.
    #[tokio::test]
    async fn concurrent_writes_to_same_profile_ask_only_once() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r1 = reg.clone();
        let r2 = reg.clone();
        let t1 = tokio::spawn(async move {
            r1.call(
                "bm_write_profile",
                json!({"path":"My PLA.json","changes":{"nozzle_temperature":["215"]}}),
            )
            .await
        });
        let t2 = tokio::spawn(async move {
            r2.call(
                "bm_write_profile",
                json!({"path":"My PLA.json","changes":{"filament_flow_ratio":["0.97"]}}),
            )
            .await
        });
        answer_next(&mut rx, &reg, "Yes").await;
        let out1 = await_timeout(t1).await;
        let out2 = await_timeout(t2).await;
        assert!(out1.ok, "{}", out1.summary());
        assert!(out2.ok, "{}", out2.summary());
        // No second Ask should have been emitted for the other writer.
        let second_event = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        assert!(
            second_event.is_err(),
            "expected no second Ask event, got {second_event:?}"
        );
    }
}
