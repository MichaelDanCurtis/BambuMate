//! Command line and environment for `claude`, per auth mode.
//!
//! Subscription auth is compiled only into private builds. Anthropic does not
//! allow third-party products to offer claude.ai login without approval, so
//! public builds always use an API key and refuse to run without one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::AGENT_INSTRUCTIONS;

pub const PERMISSION_TOOL: &str = "mcp__bambumate__bm_permission";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    ApiKey,
    #[cfg(feature = "claude-subscription")]
    Subscription,
}

pub fn available_modes() -> Vec<AuthMode> {
    #[allow(unused_mut)]
    let mut modes = vec![AuthMode::ApiKey];
    #[cfg(feature = "claude-subscription")]
    modes.push(AuthMode::Subscription);
    modes
}

pub struct LaunchOpts<'a> {
    pub session_uuid: &'a str,
    pub resume: bool,
    pub resume_at: Option<&'a str>,
    pub model: Option<&'a str>,
    pub full_access: bool,
    pub add_dirs: &'a [PathBuf],
    pub mcp_config: &'a str,
}

#[derive(Debug, Clone)]
pub struct ClaudeLaunch {
    pub args: Vec<String>,
    pub env_set: Vec<(String, String)>,
    pub env_remove: Vec<String>,
}

pub fn build_launch(
    mode: AuthMode,
    api_key: Option<&str>,
    o: &LaunchOpts,
) -> Result<ClaudeLaunch, String> {
    let (env_set, env_remove) = match mode {
        AuthMode::ApiKey => {
            let key = api_key
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .ok_or("Claude Agent needs an Anthropic API key. Add one in Settings.")?;
            (
                vec![("ANTHROPIC_API_KEY".to_string(), key.to_string())],
                vec![
                    "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
                    "ANTHROPIC_AUTH_TOKEN".to_string(),
                ],
            )
        }
        #[cfg(feature = "claude-subscription")]
        AuthMode::Subscription => (
            Vec::new(),
            vec![
                "ANTHROPIC_API_KEY".to_string(),
                "ANTHROPIC_AUTH_TOKEN".to_string(),
            ],
        ),
    };

    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--strict-mcp-config",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(["--mcp-config".into(), o.mcp_config.to_string()]);
    args.extend([
        "--append-system-prompt".into(),
        AGENT_INSTRUCTIONS.to_string(),
    ]);
    if o.resume {
        args.extend(["--resume".into(), o.session_uuid.to_string()]);
        if let Some(at) = o.resume_at {
            args.extend(["--resume-session-at".into(), at.to_string()]);
        }
    } else {
        args.extend(["--session-id".into(), o.session_uuid.to_string()]);
    }
    if let Some(m) = o.model {
        args.extend(["--model".into(), m.to_string()]);
    }
    for d in o.add_dirs {
        args.extend(["--add-dir".into(), d.to_string_lossy().into_owned()]);
    }
    if o.full_access {
        args.push("--dangerously-skip-permissions".into());
    } else {
        args.extend(["--permission-mode".into(), "acceptEdits".into()]);
        args.push("--allowedTools".into());
        for t in [
            "mcp__bambumate",
            "Read",
            "Glob",
            "Grep",
            "WebSearch",
            "WebFetch",
            "Edit",
            "Write",
        ] {
            args.push(t.into());
        }
        args.extend(["--permission-prompt-tool".into(), PERMISSION_TOOL.into()]);
    }
    Ok(ClaudeLaunch {
        args,
        env_set,
        env_remove,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts<'a>(dirs: &'a [PathBuf]) -> LaunchOpts<'a> {
        LaunchOpts {
            session_uuid: "11111111-1111-4111-8111-111111111111",
            resume: false,
            resume_at: None,
            model: Some("sonnet"),
            full_access: false,
            add_dirs: dirs,
            mcp_config: "{\"mcpServers\":{}}",
        }
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn api_key_mode_without_a_key_refuses() {
        assert!(build_launch(AuthMode::ApiKey, None, &opts(&[])).is_err());
        assert!(build_launch(AuthMode::ApiKey, Some("  "), &opts(&[])).is_err());
    }

    #[test]
    fn api_key_mode_passes_the_key_and_strips_oauth_token() {
        let l = build_launch(AuthMode::ApiKey, Some("sk-ant-test"), &opts(&[])).unwrap();
        assert!(l
            .env_set
            .contains(&("ANTHROPIC_API_KEY".to_string(), "sk-ant-test".to_string())));
        assert!(l
            .env_remove
            .contains(&"CLAUDE_CODE_OAUTH_TOKEN".to_string()));
    }

    #[test]
    fn args_wire_stream_json_mcp_and_permission_prompt() {
        let dirs = vec![PathBuf::from("/profiles")];
        let l = build_launch(AuthMode::ApiKey, Some("k"), &opts(&dirs)).unwrap();
        let a = &l.args;
        assert_eq!(a[0], "-p");
        assert!(has_pair(a, "--input-format", "stream-json"));
        assert!(has_pair(a, "--output-format", "stream-json"));
        assert!(a.contains(&"--strict-mcp-config".to_string()));
        assert!(has_pair(
            a,
            "--session-id",
            "11111111-1111-4111-8111-111111111111"
        ));
        assert!(has_pair(a, "--permission-prompt-tool", PERMISSION_TOOL));
        assert!(has_pair(a, "--add-dir", "/profiles"));
        assert!(has_pair(a, "--model", "sonnet"));
        assert!(!a.contains(&"--dangerously-skip-permissions".to_string()));
    }

    #[test]
    fn full_access_skips_permissions_and_resume_uses_resume_flag() {
        let mut o = opts(&[]);
        o.full_access = true;
        o.resume = true;
        o.resume_at = Some("u-9");
        let a = build_launch(AuthMode::ApiKey, Some("k"), &o).unwrap().args;
        assert!(a.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(!a.contains(&"--permission-prompt-tool".to_string()));
        assert!(has_pair(
            &a,
            "--resume",
            "11111111-1111-4111-8111-111111111111"
        ));
        assert!(has_pair(&a, "--resume-session-at", "u-9"));
        assert!(!a.contains(&"--session-id".to_string()));
    }

    #[cfg(not(feature = "claude-subscription"))]
    #[test]
    fn public_build_offers_api_key_only() {
        assert_eq!(available_modes(), vec![AuthMode::ApiKey]);
    }

    #[cfg(feature = "claude-subscription")]
    #[test]
    fn private_build_subscription_mode_strips_api_key_env() {
        assert_eq!(
            available_modes(),
            vec![AuthMode::ApiKey, AuthMode::Subscription]
        );
        let l = build_launch(AuthMode::Subscription, None, &opts(&[])).unwrap();
        assert!(l.env_set.iter().all(|(k, _)| k != "ANTHROPIC_API_KEY"));
        assert!(l.env_remove.contains(&"ANTHROPIC_API_KEY".to_string()));
    }
}
