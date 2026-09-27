use std::io::Cursor;
use std::path::Path;

use base64::Engine;
use serde_json::{json, Value};

use super::{arg_opt_str, arg_str, ToolContent, ToolOutput, ToolRegistry, ToolSpec};
use crate::agent::types::UiCommand;

pub const ROUTES: &[&str] = &[
    "/",
    "/filament",
    "/analysis",
    "/profiles",
    "/batch",
    "/compare",
    "/settings",
    "/health",
    "/about",
];

const MAX_EDGE: u32 = 1568;

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bm_app_state",
            description: "What the user is looking at in BambuMate: route, selected profile/filament, current photo path, last analysis session. Call this first.",
            input_schema: json!({"type":"object","properties":{}}),
        },
        ToolSpec {
            name: "bm_navigate",
            description: "Move the BambuMate UI to a page so the user can follow along. Routes: /, /filament, /analysis, /profiles, /batch, /compare, /settings, /health, /about.",
            input_schema: json!({"type":"object","properties":{"route":{"type":"string"},"profile_path":{"type":"string"}},"required":["route"]}),
        },
        ToolSpec {
            name: "bm_get_photo",
            description: "Return the print photo as an image so you can look at it. Defaults to the photo currently loaded in BambuMate.",
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}}}),
        },
        ToolSpec {
            name: "bm_search_filament",
            description: "Look up manufacturer specs for a filament by name (cached; scrapes the web when needed).",
            input_schema: json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}),
        },
        ToolSpec {
            name: "bm_catalog_search",
            description: "Fuzzy-search BambuMate's local filament catalog.",
            input_schema: json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer"}},"required":["query"]}),
        },
        ToolSpec {
            name: "bm_generate_profile",
            description: "Generate a Bambu Studio profile from filament specs without writing it. Returns a staged_id for bm_install_profile plus the diff against the base profile.",
            input_schema: json!({"type":"object","properties":{"specs":{"type":"object"},"target_printer":{"type":"string"},"base_profile_path":{"type":"string"}},"required":["specs"]}),
        },
        ToolSpec {
            name: "bm_run_analysis",
            description: "Run BambuMate's print-defect analysis on a photo (defaults to the current photo) against an optional profile. Returns defects and rule-engine recommendations.",
            input_schema: json!({"type":"object","properties":{"photo_path":{"type":"string"},"profile_path":{"type":"string"}}}),
        },
        ToolSpec {
            name: "bm_history",
            description: "List past analysis/refinement sessions for a profile.",
            input_schema: json!({"type":"object","properties":{"profile_path":{"type":"string"}},"required":["profile_path"]}),
        },
        ToolSpec {
            name: "bm_bambu_studio",
            description: "action=status reports whether Bambu Studio is running; action=launch opens it (optionally with profile_path).",
            input_schema: json!({"type":"object","properties":{"action":{"type":"string","enum":["status","launch"]},"profile_path":{"type":"string"}},"required":["action"]}),
        },
    ]
}

pub(crate) fn load_photo(path: &Path) -> Result<(String, String), String> {
    let img =
        image::open(path).map_err(|e| format!("cannot open photo {}: {e}", path.display()))?;
    let img = if img.width().max(img.height()) > MAX_EDGE {
        img.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut buf = Cursor::new(Vec::new());
    img.to_rgb8()
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .map_err(|e| format!("cannot encode photo: {e}"))?;
    Ok((
        "image/jpeg".into(),
        base64::engine::general_purpose::STANDARD.encode(buf.into_inner()),
    ))
}

fn host_result(r: Result<Value, String>) -> ToolOutput {
    match r {
        Ok(v) => ToolOutput::json(&v),
        Err(e) => ToolOutput::error(e),
    }
}

pub async fn handle(reg: &ToolRegistry, name: &str, args: &Value) -> Option<ToolOutput> {
    let host = reg.host();
    Some(match name {
        "bm_app_state" => {
            ToolOutput::json(&serde_json::to_value(host.app_state()).unwrap_or(Value::Null))
        }
        "bm_navigate" => {
            let route = match arg_str(args, "route") {
                Ok(v) => v,
                Err(e) => return Some(e),
            };
            if !ROUTES.contains(&route.as_str()) {
                return Some(ToolOutput::error(format!(
                    "unknown route '{route}'; valid: {}",
                    ROUTES.join(", ")
                )));
            }
            host.emit_ui(UiCommand::Navigate {
                route: route.clone(),
                profile_path: arg_opt_str(args, "profile_path"),
            });
            ToolOutput::text(format!("navigated to {route}"))
        }
        "bm_get_photo" => {
            let Some(path) = arg_opt_str(args, "path").or_else(|| host.app_state().photo_path)
            else {
                return Some(ToolOutput::error(
                    "no photo loaded; ask the user to drop one into the panel",
                ));
            };
            match load_photo(Path::new(&path)) {
                Ok((mime, base64)) => ToolOutput {
                    ok: true,
                    content: vec![
                        ToolContent::Text(format!("photo: {path}")),
                        ToolContent::Image { mime, base64 },
                    ],
                },
                Err(e) => ToolOutput::error(e),
            }
        }
        "bm_search_filament" => {
            let n = match arg_str(args, "name") {
                Ok(v) => v,
                Err(e) => return Some(e),
            };
            host_result(host.search_filament(&n).await)
        }
        "bm_catalog_search" => {
            let q = match arg_str(args, "query") {
                Ok(v) => v,
                Err(e) => return Some(e),
            };
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            host_result(host.catalog_search(&q, limit).await)
        }
        "bm_generate_profile" => {
            let Some(specs) = args.get("specs").cloned() else {
                return Some(ToolOutput::error("missing 'specs' object"));
            };
            host_result(
                host.generate_profile(
                    specs,
                    arg_opt_str(args, "target_printer"),
                    arg_opt_str(args, "base_profile_path"),
                )
                .await,
            )
        }
        "bm_run_analysis" => {
            let Some(photo) =
                arg_opt_str(args, "photo_path").or_else(|| host.app_state().photo_path)
            else {
                return Some(ToolOutput::error(
                    "no photo loaded; ask the user to drop one into the panel",
                ));
            };
            host.emit_ui(UiCommand::Navigate {
                route: "/analysis".into(),
                profile_path: None,
            });
            host_result(
                host.run_analysis(&photo, arg_opt_str(args, "profile_path"))
                    .await,
            )
        }
        "bm_history" => {
            let p = match arg_str(args, "profile_path") {
                Ok(v) => v,
                Err(e) => return Some(e),
            };
            host_result(host.history(&p).await)
        }
        "bm_bambu_studio" => match arg_str(args, "action").as_deref() {
            Ok("status") => ToolOutput::json(&json!({"running": host.bambu_studio_running()})),
            Ok("launch") => host_result(
                host.launch_bambu_studio(arg_opt_str(args, "profile_path"))
                    .await,
            ),
            Ok(other) => ToolOutput::error(format!("unknown action '{other}'")),
            Err(e) => e.clone(),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::agent::tools::fake_host::{registry_with, FakeHost};
    use crate::agent::tools::ToolContent;
    use crate::agent::types::UiCommand;
    use serde_json::json;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn write_png(path: &std::path::Path, w: u32, h: u32) {
        image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30]))
            .save(path)
            .unwrap();
    }

    #[tokio::test]
    async fn app_state_reports_current_route() {
        let h = Arc::new(FakeHost::new());
        h.state.lock().unwrap().route = "/analysis".into();
        let (reg, _rx) = registry_with(h);
        assert!(reg
            .call("bm_app_state", json!({}))
            .await
            .summary()
            .contains("/analysis"));
    }

    #[tokio::test]
    async fn navigate_rejects_unknown_routes_and_emits_known_ones() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        assert!(!reg.call("bm_navigate", json!({"route":"/nope"})).await.ok);
        assert!(
            reg.call("bm_navigate", json!({"route":"/compare"}))
                .await
                .ok
        );
        assert!(
            matches!(&h.ui.lock().unwrap()[0], UiCommand::Navigate { route, .. } if route == "/compare")
        );
    }

    #[tokio::test]
    async fn get_photo_returns_downscaled_jpeg_from_app_state() {
        let h = Arc::new(FakeHost::new());
        let photo = h.user_dir.path().join("print.png");
        write_png(&photo, 3000, 1000);
        h.state.lock().unwrap().photo_path = Some(photo.to_string_lossy().into_owned());
        let (reg, _rx) = registry_with(h);
        let out = reg.call("bm_get_photo", json!({})).await;
        assert!(out.ok, "{}", out.summary());
        let (mime, b64) = out
            .content
            .iter()
            .find_map(|c| match c {
                ToolContent::Image { mime, base64 } => Some((mime.clone(), base64.clone())),
                _ => None,
            })
            .expect("image content");
        assert_eq!(mime, "image/jpeg");
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!(img.width(), 1568);
    }

    #[tokio::test]
    async fn get_photo_without_a_photo_is_an_error() {
        let (reg, _rx) = registry_with(Arc::new(FakeHost::new()));
        assert!(!reg.call("bm_get_photo", json!({})).await.ok);
    }

    #[tokio::test]
    async fn host_backed_tools_forward_arguments() {
        let h = Arc::new(FakeHost::new());
        h.state.lock().unwrap().photo_path = Some("/tmp/p.jpg".into());
        let (reg, _rx) = registry_with(h.clone());
        assert!(
            reg.call("bm_search_filament", json!({"name":"PolyLite PLA"}))
                .await
                .ok
        );
        assert!(
            reg.call("bm_catalog_search", json!({"query":"petg"}))
                .await
                .ok
        );
        let gen = reg.call("bm_generate_profile", json!({"specs":{"brand":"Polymaker"},"target_printer":"Bambu Lab X1 Carbon 0.4 nozzle"})).await;
        assert!(gen.summary().contains("stg1"));
        assert!(reg.call("bm_run_analysis", json!({})).await.ok);
        assert!(
            reg.call("bm_history", json!({"profile_path":"/x.json"}))
                .await
                .ok
        );
        let calls = h.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                "search_filament:PolyLite PLA",
                "catalog_search:petg:10",
                "generate_profile:Bambu Lab X1 Carbon 0.4 nozzle",
                "run_analysis:/tmp/p.jpg:",
                "history:/x.json",
            ]
        );
    }

    #[tokio::test]
    async fn bambu_studio_status_and_launch() {
        let h = Arc::new(FakeHost::new());
        h.bs_running.store(true, Ordering::SeqCst);
        let (reg, _rx) = registry_with(h.clone());
        assert!(reg
            .call("bm_bambu_studio", json!({"action":"status"}))
            .await
            .summary()
            .contains("true"));
        assert!(
            reg.call("bm_bambu_studio", json!({"action":"launch"}))
                .await
                .ok
        );
        assert!(
            !reg.call("bm_bambu_studio", json!({"action":"explode"}))
                .await
                .ok
        );
    }

    #[test]
    fn total_tool_count_stays_small() {
        assert_eq!(crate::agent::tools::ToolRegistry::specs().len(), 18);
    }
}
