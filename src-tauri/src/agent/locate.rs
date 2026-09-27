//! Locate agent CLIs. A Finder-launched macOS app has a minimal PATH, so we
//! search the usual install locations explicitly.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub fn candidate_dirs(home: Option<&Path>, path_env: Option<&str>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(p) = path_env {
        dirs.extend(std::env::split_paths(p));
    }
    if let Some(h) = home {
        dirs.push(h.join(".local/bin"));
        dirs.push(h.join(".npm-global/bin"));
        dirs.push(h.join(".bun/bin"));
        dirs.push(h.join(".volta/bin"));
        if cfg!(windows) {
            dirs.push(h.join("AppData/Roaming/npm"));
        }
    }
    if !cfg!(windows) {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

pub fn find_in(program: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![
            format!("{program}.exe"),
            format!("{program}.cmd"),
            program.to_string(),
        ]
    } else {
        vec![program.to_string()]
    };
    for dir in dirs {
        for name in &names {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

pub fn locate(program: &str) -> Option<PathBuf> {
    let path_env = std::env::var("PATH").ok();
    let dirs = candidate_dirs(dirs::home_dir().as_deref(), path_env.as_deref());
    find_in(program, &dirs)
}

/// PATH for a spawned agent CLI: the binary's own dir first (so npm shims
/// find their sibling `node`), then every candidate dir.
pub fn child_path_env(bin: &Path) -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(parent) = bin.parent() {
        dirs.push(parent.to_path_buf());
    }
    let path_env = std::env::var("PATH").ok();
    dirs.extend(candidate_dirs(
        dirs::home_dir().as_deref(),
        path_env.as_deref(),
    ));
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    std::env::join_paths(dirs).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn make_exe(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        });
        fs::write(&p, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    #[test]
    fn finds_executable_in_given_dirs() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let exe = make_exe(b.path(), "codex");
        let found = find_in("codex", &[a.path().to_path_buf(), b.path().to_path_buf()]);
        assert_eq!(found, Some(exe));
    }

    #[test]
    fn returns_none_when_missing() {
        let a = tempfile::tempdir().unwrap();
        assert_eq!(find_in("claude", &[a.path().to_path_buf()]), None);
    }

    #[cfg(unix)]
    #[test]
    fn skips_non_executable_files() {
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("codex"), b"x").unwrap();
        assert_eq!(find_in("codex", &[a.path().to_path_buf()]), None);
    }

    #[test]
    fn candidate_dirs_puts_path_first_adds_home_bins_and_dedups() {
        let home = PathBuf::from("/home/u");
        let path_env = std::env::join_paths([PathBuf::from("/usr/bin"), PathBuf::from("/usr/bin")])
            .unwrap()
            .into_string()
            .unwrap();
        let dirs = candidate_dirs(Some(&home), Some(&path_env));
        assert_eq!(dirs[0], PathBuf::from("/usr/bin"));
        assert_eq!(
            dirs.iter()
                .filter(|d| **d == PathBuf::from("/usr/bin"))
                .count(),
            1
        );
        assert!(dirs.contains(&home.join(".local/bin")));
    }

    #[test]
    fn child_path_env_starts_with_binary_dir() {
        let env = child_path_env(Path::new("/opt/tools/bin/codex"));
        let first = std::env::split_paths(&env).next().unwrap();
        assert_eq!(first, PathBuf::from("/opt/tools/bin"));
    }
}
