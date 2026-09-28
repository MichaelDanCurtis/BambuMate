//! Checks run over profile files an agent turn changed, so a direct file edit
//! that Bambu Studio would silently reject is surfaced with a restore option.

use std::path::{Path, PathBuf};

use crate::profile::reader::read_profile;

pub fn validate_profile_file(path: &Path) -> Result<(), String> {
    let profile = read_profile(path).map_err(|e| format!("not a readable profile: {e}"))?;
    match profile.name() {
        Some(n) if !n.trim().is_empty() => {}
        _ => return Err("missing or empty \"name\"".to_string()),
    }
    let has_inherits = profile
        .inherits()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_id = profile
        .filament_id()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if !has_inherits && !has_id {
        return Err("needs a non-empty \"inherits\" or \"filament_id\"".to_string());
    }
    Ok(())
}

pub fn invalid_profiles(paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
    paths
        .iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| validate_profile_file(p).err().map(|e| (p.clone(), e)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn accepts_a_minimal_user_profile() {
        let d = tempfile::tempdir().unwrap();
        let p = write(
            d.path(),
            "ok.json",
            r#"{"name":"My PLA","inherits":"Generic PLA @BBL X1C","from":"User"}"#,
        );
        assert_eq!(validate_profile_file(&p), Ok(()));
    }

    #[test]
    fn rejects_broken_json() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), "bad.json", "{\"name\": ");
        assert!(validate_profile_file(&p).is_err());
    }

    #[test]
    fn rejects_missing_name() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), "noname.json", r#"{"inherits":"Generic PLA"}"#);
        assert!(validate_profile_file(&p).unwrap_err().contains("name"));
    }

    #[test]
    fn rejects_profile_with_neither_inherits_nor_filament_id() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), "orphan.json", r#"{"name":"Orphan"}"#);
        assert!(validate_profile_file(&p).unwrap_err().contains("inherits"));
    }

    #[test]
    fn invalid_profiles_ignores_info_files_and_reports_bad_json() {
        let d = tempfile::tempdir().unwrap();
        let good = write(
            d.path(),
            "g.json",
            r#"{"name":"G","filament_id":"P1234567"}"#,
        );
        let bad = write(d.path(), "b.json", "not json");
        let info = write(d.path(), "g.info", "garbage");
        let out = invalid_profiles(&[good, bad.clone(), info]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, bad);
    }
}
