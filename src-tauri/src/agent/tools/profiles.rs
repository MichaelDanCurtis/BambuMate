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
            description: "Set fields on a user profile. Validates, backs up first, and shows the change in the UI. changes is an object of Bambu Studio keys to JSON values (most values are string arrays, e.g. {\"nozzle_temperature\":[\"215\"]}).",
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
    let joined = if raw.is_absolute() { raw.to_path_buf() } else { dir.join(raw) };
    let canon_dir = dir.canonicalize().map_err(|e| format!("profile folder unavailable: {e}"))?;
    let canon = joined.canonicalize().map_err(|e| format!("{p}: {e}"))?;
    if !canon.starts_with(&canon_dir) {
        return Err(format!("{p} is outside the Bambu Studio user filament folder"));
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
    let dir = match user_dir(reg) { Ok(d) => d, Err(e) => return e };
    let mut rows = Vec::new();
    let entries = match std::fs::read_dir(&dir) { Ok(e) => e, Err(e) => return ToolOutput::error(e.to_string()) };
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
    let p = match arg_str(args, "path") { Ok(v) => v, Err(e) => return e };
    let dir = match user_dir(reg) { Ok(d) => d, Err(e) => return e };
    let raw = Path::new(&p);
    // Reads may target system profiles too, so only relative paths are joined.
    let path = if raw.is_absolute() { raw.to_path_buf() } else { dir.join(raw) };
    let profile = match read_profile(&path) { Ok(p) => p, Err(e) => return ToolOutput::error(e.to_string()) };
    let resolved = args.get("resolved").and_then(|v| v.as_bool()).unwrap_or(false);
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
    let a = match arg_str(args, "path_a") { Ok(v) => v, Err(e) => return e };
    let b = match arg_str(args, "path_b") { Ok(v) => v, Err(e) => return e };
    match crate::commands::profile::compare_profiles(a, b, false) {
        Ok(result) => ToolOutput::json(&serde_json::to_value(result).unwrap_or(Value::Null)),
        Err(e) => ToolOutput::error(e),
    }
}

async fn write(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let p = match arg_str(args, "path") { Ok(v) => v, Err(e) => return e };
    let Some(changes) = args.get("changes").and_then(|v| v.as_object()).cloned() else {
        return ToolOutput::error("'changes' must be an object");
    };
    let dir = match user_dir(reg) { Ok(d) => d, Err(e) => return e };
    let path = match resolve_in(&dir, &p) { Ok(p) => p, Err(e) => return ToolOutput::error(e) };
    if let Err(e) = reg.ensure_write_allowed(&path).await {
        return ToolOutput::error(e);
    }
    let mut profile = match read_profile(&path) { Ok(p) => p, Err(e) => return ToolOutput::error(e.to_string()) };
    let backup = match backup_profile(&path) { Ok(b) => b, Err(e) => return ToolOutput::error(format!("backup failed: {e}")) };
    let keys: Vec<String> = changes.keys().cloned().collect();
    let raw: &mut Map<String, Value> = profile.raw_mut();
    for (k, v) in changes {
        raw.insert(k, v);
    }
    if let Err(e) = write_profile_atomic(&profile, &path) {
        return ToolOutput::error(format!("write failed: {e}"));
    }
    reg.host().emit_ui(UiCommand::Navigate {
        route: "/profiles".into(),
        profile_path: Some(path.to_string_lossy().into_owned()),
    });
    ToolOutput::json(&json!({"path": path.to_string_lossy(), "changed_keys": keys, "backup_path": backup.to_string_lossy()}))
}

async fn rollback(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let p = match arg_str(args, "path") { Ok(v) => v, Err(e) => return e };
    let dir = match user_dir(reg) { Ok(d) => d, Err(e) => return e };
    let path = match resolve_in(&dir, &p) { Ok(p) => p, Err(e) => return ToolOutput::error(e) };
    let backup = match arg_opt_str(args, "backup_path") {
        Some(b) => PathBuf::from(b),
        None => match latest_backup(&path) { Some(b) => b, None => return ToolOutput::error("no backup found") },
    };
    if reg.host().bambu_studio_running()
        && !reg.asks().confirm(reg.session_id(), "Bambu Studio is running. Roll back anyway?").await
    {
        return ToolOutput::error("declined: Bambu Studio is running");
    }
    match restore_from_backup(&backup, &path) {
        Ok(()) => {
            reg.host().emit_ui(UiCommand::Refresh { what: "profiles".into() });
            ToolOutput::json(&json!({"restored_from": backup.to_string_lossy()}))
        }
        Err(e) => ToolOutput::error(e.to_string()),
    }
}

async fn install(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let staged = match arg_str(args, "staged_id") { Ok(v) => v, Err(e) => return e };
    let running = reg.host().bambu_studio_running();
    if running
        && !reg
            .asks()
            .confirm(reg.session_id(), "Bambu Studio is running and may overwrite the new profile. Install anyway?")
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
            reg.host().emit_ui(UiCommand::Refresh { what: "profiles".into() });
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

    const PLA: &str = r#"{"name":"My PLA","inherits":"Generic PLA @BBL X1C","from":"User","nozzle_temperature":["220"]}"#;

    fn host_with_profile() -> Arc<FakeHost> {
        let h = Arc::new(FakeHost::new());
        fs::write(h.user_dir.path().join("My PLA.json"), PLA).unwrap();
        h
    }

    async fn answer_next(rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>, reg: &crate::agent::tools::ToolRegistry, answer: &str) {
        loop {
            if let AgentEvent::Ask { request, .. } = rx.recv().await.unwrap() {
                reg.asks().answer(&request.id, vec![answer.into()]).unwrap();
                return;
            }
        }
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
        let out = reg.call("bm_read_profile", json!({"path":"My PLA.json"})).await;
        assert!(out.ok, "{}", out.summary());
        assert!(out.summary().contains("nozzle_temperature"));
    }

    #[tokio::test]
    async fn write_outside_user_dir_is_refused() {
        let h = host_with_profile();
        let (reg, _rx) = registry_with(h);
        let out = reg.call("bm_write_profile", json!({"path":"/etc/hosts","changes":{"a":"b"}})).await;
        assert!(!out.ok);
    }

    #[tokio::test]
    async fn first_write_to_existing_profile_asks_then_backs_up_and_writes() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call("bm_write_profile", json!({"path":"My PLA.json","changes":{"nozzle_temperature":["215"]}})).await
        });
        answer_next(&mut rx, &reg, "Yes").await;
        let out = t.await.unwrap();
        assert!(out.ok, "{}", out.summary());
        let body = fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap();
        assert!(body.contains("215"));
        assert_eq!(fs::read_dir(h.user_dir.path().join(".backups")).unwrap().count(), 1);
        assert!(h.ui.lock().unwrap().iter().any(|c| matches!(c, UiCommand::Navigate { .. })));
    }

    #[tokio::test]
    async fn declined_write_changes_nothing() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call("bm_write_profile", json!({"path":"My PLA.json","changes":{"nozzle_temperature":["199"]}})).await
        });
        answer_next(&mut rx, &reg, "No").await;
        assert!(!t.await.unwrap().ok);
        assert_eq!(fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap(), PLA);
    }

    #[tokio::test]
    async fn rollback_restores_latest_backup() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call("bm_write_profile", json!({"path":"My PLA.json","changes":{"nozzle_temperature":["230"]}})).await
        });
        answer_next(&mut rx, &reg, "Yes").await;
        assert!(t.await.unwrap().ok);
        let out = reg.call("bm_rollback", json!({"path":"My PLA.json"})).await;
        assert!(out.ok, "{}", out.summary());
        let body = fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap();
        assert!(body.contains("220"));
    }

    #[tokio::test]
    async fn install_marks_profile_created_so_later_writes_do_not_ask() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        let out = reg.call("bm_install_profile", json!({"staged_id":"stg1"})).await;
        assert!(out.ok, "{}", out.summary());
        // No ask is pending, so this completes without anyone answering.
        let out = reg
            .call("bm_write_profile", json!({"path":"Polymaker PLA.json","changes":{"filament_flow_ratio":["0.97"]}}))
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
        let t = tokio::spawn(async move { r2.call("bm_install_profile", json!({"staged_id":"stg1"})).await });
        answer_next(&mut rx, &reg, "No").await;
        assert!(!t.await.unwrap().ok);
        assert!(h.calls.lock().unwrap().is_empty(), "nothing installed");
    }
}
