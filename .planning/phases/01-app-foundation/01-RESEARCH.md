# Phase 1: App Foundation - Research

**Researched:** 2026-02-04
**Domain:** Tauri 2.0 desktop app scaffold with Leptos (Rust/WASM) frontend, secure keychain storage, health check, macOS DMG build
**Confidence:** HIGH (core stack verified via official docs; keychain approach at MEDIUM due to community plugin reliance)

## Summary

Phase 1 establishes BambuMate as a running Tauri 2.0 desktop application on macOS with a Leptos/WASM frontend, secure API key storage via the OS keychain, a health check system, and a distributable DMG. The research focused on five domains: (1) Tauri 2.0 + Leptos project scaffolding and configuration, (2) frontend-backend communication from WASM, (3) secure storage for API keys using macOS Keychain, (4) app shell architecture with navigation/routing, and (5) macOS DMG build and distribution.

The standard approach is to use `create-tauri-app` to scaffold a Tauri 2.0 project with the Leptos template (which now targets Leptos 0.8), use Trunk as the WASM bundler, communicate between frontend and backend via `wasm_bindgen` + `window.__TAURI__.core.invoke()`, store API keys using the `keyring` crate (via macOS Keychain) exposed through Tauri commands, use `leptos_router` for client-side navigation between views, and build with `cargo tauri build --bundles dmg` for distribution.

The critical insight from the prior project research is that this project pivoted from a CLI tool to a desktop app. The prior STACK.md research covered CLI-specific crates (clap, colored, indicatif, dialoguer) that are NOT needed for Phase 1. The Tauri backend replaces the CLI entry point, and Leptos replaces terminal UI. However, many backend crates (serde, tokio, dirs, anyhow, thiserror, tracing) remain applicable for the Tauri command layer.

**Primary recommendation:** Scaffold with `cargo create-tauri-app`, use Leptos 0.8 with Trunk for CSR, use `keyring` crate with `apple-native` feature directly in Tauri commands for macOS Keychain access, and `tauri-plugin-store` for non-sensitive preferences.

## Standard Stack

The established libraries/tools for this phase:

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| Tauri | 2.x (latest) | Desktop app framework | Official framework choice per project constraints. Rust backend + webview frontend. |
| Leptos | 0.8.x (latest 0.8.15) | Rust/WASM frontend framework | Project constraint. Fine-grained reactivity, compiles to WASM. create-tauri-app v4.7.0 bumped template to 0.8. |
| leptos_router | 0.8.x | Client-side routing | Official Leptos router. Provides `<Router/>`, `<Routes/>`, `<Route/>` for SPA navigation between views. |
| Trunk | latest | WASM bundler for Leptos | Official build tool for Leptos CSR apps. Compiles Rust to WASM, serves during dev, builds for production. |
| wasm-bindgen | 0.2.x | Rust-JS interop | Required for calling Tauri APIs from Leptos WASM frontend. |
| serde / serde-wasm-bindgen | 1.0.x / latest | Serialization for IPC | Required for serializing arguments and deserializing responses between Leptos frontend and Tauri backend. |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tauri-plugin-store | 2.x | Persistent key-value storage | For non-sensitive preferences (window size, Bambu Studio path, default printer). Stores to app data directory as JSON. |
| keyring | 3.6.x (latest 3.6.3) | OS keychain access | For secure API key storage (Claude, OpenAI). Use with `apple-native` feature for macOS Keychain. Called from Tauri commands, not from WASM directly. |
| serde_json | 1.0.x | JSON handling | For Tauri command serialization and config file handling. |
| anyhow | 1.0.x | Application error handling | For Tauri command error handling on the backend side. |
| thiserror | 2.0.x | Typed error definitions | For defining domain error types in backend modules. |
| tracing | 0.1.x | Structured logging | For backend logging in Tauri commands. |
| tracing-subscriber | 0.3.x | Log output | For configuring log output with RUST_LOG filtering. |
| dirs | 6.0.0 | Platform-specific directories | For resolving Bambu Studio config paths on macOS. |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| keyring crate (direct) | tauri-plugin-keyring (0.1.0) | Community plugin wraps keyring crate as Tauri plugin with JS API. Only v0.1.0, low adoption (111 downloads/month). Using keyring directly in Tauri commands is more reliable and gives full control. |
| keyring crate (direct) | tauri-plugin-secure-storage (1.4.0) | Another community plugin wrapping keyring. Newer (Jul 2025) but also low adoption. Same tradeoff as above. |
| keyring crate (direct) | tauri-plugin-stronghold | Official Tauri plugin for encrypted storage. But: requires user password or storing encryption key elsewhere, heavier dependency, and Tauri maintainers indicated it will be deprecated in v3. |
| tauri-plugin-store | TOML config file | Store plugin gives change events, auto-save, and works from both Rust and WASM sides. TOML file requires manual serialization and no cross-side reactivity. |
| leptos_router | Manual view switching via signals | Simpler for 2-3 views but does not give URL-based navigation, browser back/forward, or deep linking. Router is better even for desktop apps. |

**Installation (backend Cargo.toml in src-tauri/):**

```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-store = "2"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
keyring = { version = "3.6", features = ["apple-native"] }
anyhow = "1.0"
thiserror = "2.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
dirs = "6.0"
```

**Installation (frontend Cargo.toml in project root):**

```toml
[dependencies]
leptos = { version = "0.8", features = ["csr"] }
leptos_router = { version = "0.8", features = ["csr"] }
wasm-bindgen = "0.2"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
serde-wasm-bindgen = "0.6"
```

## Architecture Patterns

### Recommended Project Structure

```
bambumate/
├── Cargo.toml              # Frontend (Leptos) crate
├── index.html              # Trunk entry point
├── Trunk.toml              # Trunk build config
├── src/
│   ├── main.rs             # Leptos mount point
│   ├── app.rs              # Root App component with Router
│   ├── commands.rs         # wasm_bindgen invoke helpers
│   ├── pages/
│   │   ├── mod.rs
│   │   ├── home.rs         # Home/dashboard view
│   │   ├── settings.rs     # API key settings view
│   │   └── health.rs       # Health check view
│   └── components/
│       ├── mod.rs
│       ├── sidebar.rs      # Navigation sidebar
│       ├── status_badge.rs # Health status indicators
│       └── api_key_form.rs # API key input form
├── style/
│   └── main.css            # App stylesheet
├── public/
│   └── (static assets)
└── src-tauri/
    ├── Cargo.toml          # Backend (Tauri) crate
    ├── tauri.conf.json     # Tauri configuration
    ├── capabilities/
    │   └── default.json    # Permission capabilities
    ├── src/
    │   ├── main.rs         # Tauri entry point (generated)
    │   ├── lib.rs          # Plugin registration, command registration
    │   ├── commands/
    │   │   ├── mod.rs
    │   │   ├── keychain.rs # API key get/set/delete via keyring
    │   │   ├── health.rs   # Health check (Bambu Studio detection, etc.)
    │   │   └── config.rs   # App preferences via tauri-plugin-store
    │   └── error.rs        # Backend error types
    └── icons/              # App icons
```

### Pattern 1: Tauri Command Communication from Leptos WASM

**What:** Leptos components call Tauri backend commands via wasm_bindgen. The bridge uses `window.__TAURI__.core.invoke()`.
**When to use:** Every frontend-to-backend call (keychain access, health checks, file system operations).

```rust
// src/commands.rs (frontend)
use wasm_bindgen::prelude::*;
use serde::{Deserialize, Serialize};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

#[derive(Serialize)]
struct SetApiKeyArgs {
    service: String,
    key: String,
}

pub async fn set_api_key(service: &str, key: &str) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&SetApiKeyArgs {
        service: service.to_string(),
        key: key.to_string(),
    }).map_err(|e| e.to_string())?;

    invoke("set_api_key", args)
        .await
        .map(|_| ())
        .map_err(|e| {
            e.as_string().unwrap_or_else(|| "Unknown error".to_string())
        })
}
```

```rust
// src-tauri/src/commands/keychain.rs (backend)
use keyring::Entry;

#[tauri::command]
pub fn set_api_key(service: &str, key: &str) -> Result<(), String> {
    let entry = Entry::new(service, "bambumate")
        .map_err(|e| e.to_string())?;
    entry.set_password(key)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_api_key(service: &str) -> Result<Option<String>, String> {
    let entry = Entry::new(service, "bambumate")
        .map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn delete_api_key(service: &str) -> Result<(), String> {
    let entry = Entry::new(service, "bambumate")
        .map_err(|e| e.to_string())?;
    entry.delete_credential()
        .map_err(|e| e.to_string())
}
```

### Pattern 2: Leptos Router for Desktop App Navigation

**What:** Use leptos_router for client-side routing between views (Home, Settings, Health Check). Even in a desktop app, URL-based routing provides structure and enables the back/forward pattern.
**When to use:** App shell layout with sidebar navigation.

```rust
// src/app.rs
use leptos::prelude::*;
use leptos_router::components::*;
use leptos_router::path;

#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <div class="app-layout">
                <Sidebar />
                <main class="content">
                    <Routes fallback=|| "Page not found">
                        <Route path=path!("/") view=HomePage />
                        <Route path=path!("/settings") view=SettingsPage />
                        <Route path=path!("/health") view=HealthPage />
                    </Routes>
                </main>
            </div>
        </Router>
    }
}
```

### Pattern 3: Health Check via Tauri Commands

**What:** Health check runs on the backend (Tauri side) where it has full filesystem and process access. Results are returned as a structured response to the frontend.
**When to use:** FNDN-03 requirement -- validate Bambu Studio installation, profile directory access, API key configuration.

```rust
// src-tauri/src/commands/health.rs
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct HealthReport {
    pub bambu_studio_installed: bool,
    pub bambu_studio_path: Option<String>,
    pub profile_dir_accessible: bool,
    pub profile_dir_path: Option<String>,
    pub claude_api_key_set: bool,
    pub openai_api_key_set: bool,
}

#[tauri::command]
pub fn run_health_check() -> Result<HealthReport, String> {
    let bs_path = PathBuf::from("/Applications/BambuStudio.app");
    let bs_installed = bs_path.exists();

    let profile_dir = dirs::data_dir()
        .map(|d| d.join("BambuStudio"))
        .unwrap_or_default();
    let profile_accessible = profile_dir.exists() && profile_dir.is_dir();

    let claude_key = keyring::Entry::new("claude-api", "bambumate")
        .and_then(|e| e.get_password())
        .is_ok();
    let openai_key = keyring::Entry::new("openai-api", "bambumate")
        .and_then(|e| e.get_password())
        .is_ok();

    Ok(HealthReport {
        bambu_studio_installed: bs_installed,
        bambu_studio_path: if bs_installed {
            Some(bs_path.to_string_lossy().to_string())
        } else {
            None
        },
        profile_dir_accessible: profile_accessible,
        profile_dir_path: if profile_accessible {
            Some(profile_dir.to_string_lossy().to_string())
        } else {
            None
        },
        claude_api_key_set: claude_key,
        openai_api_key_set: openai_key,
    })
}
```

### Pattern 4: Tauri Configuration for Leptos

**What:** The specific configuration files needed to wire Tauri 2.0 with a Leptos CSR frontend using Trunk.

```json
// src-tauri/tauri.conf.json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "BambuMate",
  "version": "0.1.0",
  "identifier": "com.bambumate.app",
  "build": {
    "beforeDevCommand": "trunk serve",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "trunk build",
    "frontendDist": "../dist"
  },
  "app": {
    "withGlobalTauri": true,
    "windows": [
      {
        "title": "BambuMate",
        "width": 1200,
        "height": 800,
        "resizable": true,
        "fullscreen": false
      }
    ]
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "macOS": {
      "dmg": {
        "appPosition": { "x": 180, "y": 170 },
        "applicationFolderPosition": { "x": 480, "y": 170 },
        "windowSize": { "width": 660, "height": 400 }
      },
      "minimumSystemVersion": "10.15"
    }
  }
}
```

```toml
# Trunk.toml
[build]
target = "./index.html"

[watch]
ignore = ["./src-tauri"]

[serve]
port = 1420
open = false
ws_protocol = "ws"
```

### Anti-Patterns to Avoid

- **Using Leptos SSR with Tauri:** Tauri does not support server-based solutions. Use CSR (client-side rendering) only. The `csr` feature flag must be enabled on leptos and leptos_router.
- **Accessing filesystem directly from WASM:** The Leptos frontend runs in a webview sandbox. All filesystem, keychain, and process operations must go through Tauri commands on the backend. Never try to use `std::fs` from the frontend crate.
- **Storing API keys in tauri-plugin-store:** The store plugin writes unencrypted JSON to the app data directory. API keys MUST go through the keyring crate (OS Keychain) via Tauri commands. Only use the store for non-sensitive preferences.
- **Using Leptos nightly features in Tauri builds:** While Leptos supports nightly-only features (function-call syntax for signals), the Tauri build pipeline is simpler with stable Rust. Use `signal.get()` and `signal.set()` explicitly rather than the nightly function-call syntax.
- **Blocking the main thread in Tauri commands:** Long operations (health checks with filesystem scanning, network calls) should use `#[tauri::command]` with async where appropriate, or be documented as fast enough to not need it. Keychain operations are fast and synchronous, which is fine.

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| OS keychain access | Custom macOS Keychain bindings | `keyring` crate with `apple-native` feature | Handles macOS Keychain, Windows Credential Manager, Linux Secret Service. Cross-platform from day one. |
| Persistent preferences | Custom JSON file management | `tauri-plugin-store` | Handles file location, auto-save with debounce, change events, works from both Rust and WASM sides. |
| WASM-to-Tauri IPC | Custom message passing | `wasm_bindgen` + `window.__TAURI__.core.invoke()` | Official Tauri pattern. Handles serialization, error propagation, async. |
| Client-side routing | Manual view switching with signals | `leptos_router` | Provides URL-based navigation, nested routes, fallbacks. Standard Leptos pattern. |
| DMG packaging | Custom packaging scripts | `cargo tauri build --bundles dmg` | Tauri CLI handles icon placement, Applications symlink, window layout. |
| WASM bundling | Custom wasm-pack workflow | Trunk | Official Leptos build tool. Handles WASM compilation, asset hashing, dev server with hot-reload. |
| App data directory location | Hardcoded paths | `dirs` crate + Tauri path resolver | Cross-platform directory resolution. `dirs::data_dir()` gives `~/Library/Application Support/` on macOS. |

**Key insight:** The Tauri 2.0 ecosystem provides most of the desktop plumbing (window management, IPC, bundling, packaging). The main implementation work is the Leptos frontend components and the Tauri backend commands, not infrastructure.

## Common Pitfalls

### Pitfall 1: withGlobalTauri Not Enabled

**What goes wrong:** Leptos WASM code tries to call `window.__TAURI__.core.invoke()` but gets undefined because Tauri APIs are not exposed to the webview.
**Why it happens:** The default Tauri configuration does not expose `__TAURI__` to the window object. This must be explicitly enabled.
**How to avoid:** Set `"withGlobalTauri": true` in `tauri.conf.json` under the `app` section. This is required for any Rust/WASM frontend that uses `wasm_bindgen` to access Tauri APIs.
**Warning signs:** JavaScript errors in the webview console about `__TAURI__` being undefined. Commands silently fail.

### Pitfall 2: Leptos CSR vs SSR Feature Confusion

**What goes wrong:** Build errors or runtime panics because the wrong feature flag is enabled. Leptos compiled with `ssr` feature tries to do server-side things that don't exist in a Tauri webview.
**Why it happens:** Leptos supports both CSR and SSR. Many tutorials and examples use SSR with Axum/Actix. Tauri requires CSR only.
**How to avoid:** Enable `features = ["csr"]` on both `leptos` and `leptos_router` in the frontend Cargo.toml. Never enable `ssr` or `hydrate` features for a Tauri app.
**Warning signs:** Compilation errors mentioning server functions, or runtime errors about missing server context.

### Pitfall 3: Trunk Build Port Mismatch

**What goes wrong:** `cargo tauri dev` launches but shows a blank white window because the Trunk dev server is not running on the expected port.
**Why it happens:** Tauri's `devUrl` in `tauri.conf.json` must match Trunk's serve port exactly. The default Trunk port is 8080, but the Tauri template expects 1420.
**How to avoid:** Ensure `Trunk.toml` has `port = 1420` under `[serve]`, and `tauri.conf.json` has `"devUrl": "http://localhost:1420"`. These must match.
**Warning signs:** Blank window on `cargo tauri dev`. Console shows connection refused.

### Pitfall 4: Tauri Command Permissions Not Configured

**What goes wrong:** Commands are registered in `lib.rs` but the frontend gets permission errors when trying to invoke them.
**Why it happens:** Tauri 2.0 requires explicit capability/permission configuration for all commands. Custom commands need permissions defined in `build.rs` and granted in capability files.
**How to avoid:** Configure `build.rs` to expose commands, and add permissions to `src-tauri/capabilities/default.json`. See the capabilities section below.
**Warning signs:** Invoke calls return permission denied errors. Works in dev but fails in production builds.

### Pitfall 5: macOS Keychain Access Without Signing

**What goes wrong:** The `keyring` crate fails to access macOS Keychain because the app is not code-signed, or the Keychain prompts the user for permission on every access.
**Why it happens:** macOS restricts Keychain access based on code signing identity. Unsigned apps get limited access. Development builds may behave differently than release builds.
**How to avoid:** For development, macOS usually allows unsigned apps to create and access their own Keychain items. For distribution, sign the DMG/app bundle. Use a consistent `service` name with the keyring crate so all BambuMate entries are grouped.
**Warning signs:** First launch prompts user for Keychain access. Keychain items are not found after rebuilding the app with a different signing identity.

### Pitfall 6: GUI Apps Don't Inherit Shell PATH

**What goes wrong:** Health check tries to find Bambu Studio or other tools using PATH, but the PATH is not set correctly because macOS GUI apps do not inherit shell dotfiles (.zshrc, .bashrc).
**Why it happens:** macOS launches GUI applications with a minimal environment. The user's shell PATH additions are not available.
**How to avoid:** Use `fix-path-env-rs` crate (mentioned in Tauri docs) or hardcode known application paths (e.g., `/Applications/BambuStudio.app`). Do not rely on PATH for finding applications.
**Warning signs:** Health check reports Bambu Studio not found even though it's installed. Works in `cargo tauri dev` but fails in the packaged DMG.

## Code Examples

Verified patterns from official sources:

### Leptos Component with Tauri Command Call

```rust
// Source: Tauri v2 + Leptos integration pattern
// (combined from v2.tauri.app/start/frontend/leptos/ and forgestream guide)
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use serde::{Deserialize, Serialize};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

#[derive(Serialize)]
struct GreetArgs {
    name: String,
}

#[component]
pub fn GreetButton() -> impl IntoView {
    let (message, set_message) = signal(String::new());

    let greet = move |_| {
        leptos::spawn::spawn_local(async move {
            let args = serde_wasm_bindgen::to_value(&GreetArgs {
                name: "BambuMate User".to_string(),
            }).unwrap();
            match invoke("greet", args).await {
                Ok(result) => {
                    let msg: String = serde_wasm_bindgen::from_value(result).unwrap();
                    set_message.set(msg);
                }
                Err(e) => {
                    set_message.set(format!("Error: {:?}", e));
                }
            }
        });
    };

    view! {
        <button on:click=greet>"Greet"</button>
        <p>{message}</p>
    }
}
```

### Tauri Backend Command Registration

```rust
// Source: Tauri v2 official docs
// src-tauri/src/lib.rs

mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::keychain::get_api_key,
            commands::keychain::set_api_key,
            commands::keychain::delete_api_key,
            commands::health::run_health_check,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### Leptos Router Setup for App Shell

```rust
// Source: Leptos book (book.leptos.dev/router/16_routes.html)
use leptos::prelude::*;
use leptos_router::components::*;
use leptos_router::path;

#[component]
fn Sidebar() -> impl IntoView {
    view! {
        <nav class="sidebar">
            <div class="sidebar-header">
                <h1>"BambuMate"</h1>
            </div>
            <ul class="nav-list">
                <li><a href="/">"Home"</a></li>
                <li><a href="/settings">"Settings"</a></li>
                <li><a href="/health">"Health Check"</a></li>
            </ul>
        </nav>
    }
}

#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <div class="app-layout">
                <Sidebar />
                <main class="content">
                    <Routes fallback=|| view! { <p>"Page not found"</p> }>
                        <Route path=path!("/") view=HomePage />
                        <Route path=path!("/settings") view=SettingsPage />
                        <Route path=path!("/health") view=HealthPage />
                    </Routes>
                </main>
            </div>
        </Router>
    }
}
```

### Building the DMG

```bash
# Source: v2.tauri.app/distribute/dmg/
# Development
cargo tauri dev

# Production DMG build (must run on macOS)
cargo tauri build --bundles dmg

# Output: src-tauri/target/release/bundle/dmg/BambuMate_0.1.0_aarch64.dmg
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Leptos 0.6 in Tauri template | Leptos 0.8 in Tauri template | create-tauri-app v4.7.0 (Jan 2025) | Use 0.8 imports (`leptos::prelude::*`), `signal()` returns `(ReadSignal, WriteSignal)` |
| Tauri Stronghold for secrets | keyring crate / community plugins | Ongoing (Stronghold deprecated for v3) | Use OS-native keychain via keyring crate directly |
| `tauri.bundle.dmg` config path | `bundle.macOS.dmg` config path | Tauri 2.0 | Configuration structure changed; use new path in tauri.conf.json |
| `use leptos::*` imports | `use leptos::prelude::*` imports | Leptos 0.7 | Module restructuring for better discoverability |
| Leptos required nightly Rust | Leptos works on stable Rust | Leptos 0.7+ | Nightly only needed for optional `nightly` feature (function-call signal syntax) |

**Deprecated/outdated:**
- **Leptos 0.6:** Tauri docs page says "accurate as of Leptos version 0.6" but create-tauri-app actually generates Leptos 0.8 projects as of v4.7.0. Use 0.8.
- **tauri-plugin-stronghold for new projects:** Will be deprecated in Tauri v3. Use keyring crate directly for new projects.
- **`tauri.bundle.dmg` config key:** Moved to `bundle.macOS.dmg` in Tauri 2.0.

## Open Questions

Things that could not be fully resolved:

1. **Exact Leptos version in current create-tauri-app template**
   - What we know: create-tauri-app v4.7.0 (Jan 2025) bumped Leptos to 0.8. DeepWiki mentions 0.7. Latest Leptos is 0.8.15.
   - What's unclear: Whether current template pins to 0.8.0 or a later 0.8.x release. The exact generated Cargo.toml was not fully verified.
   - Recommendation: Use Leptos 0.8 (latest 0.8.x). After scaffolding, update Cargo.toml to latest 0.8.x if template is behind.

2. **Tauri command permission configuration for custom commands**
   - What we know: Tauri 2.0 uses capabilities system. Custom commands need permissions in `build.rs` and capability files.
   - What's unclear: Exact build.rs configuration for exposing custom commands. Default behavior may allow all commands unless restricted.
   - Recommendation: Start with default capabilities that allow all commands. Tighten permissions later when the command surface is stable.

3. **keyring crate behavior in unsigned macOS development builds**
   - What we know: keyring 3.6.3 with `apple-native` uses macOS Keychain. Code signing affects Keychain access.
   - What's unclear: Whether `cargo tauri dev` builds (which are unsigned) can reliably create and retrieve Keychain items without user prompts.
   - Recommendation: Test early in Phase 1. If unsigned builds have Keychain issues, implement a fallback for development (environment variables or plaintext dev-only storage).

4. **Tauri 2.0 + Leptos CSS/styling approach**
   - What we know: Trunk supports SCSS and Tailwind. Community templates exist for Leptos + Tailwind.
   - What's unclear: Best CSS approach for a desktop app (Tailwind, plain CSS, SCSS). No strong project constraint.
   - Recommendation: Start with plain CSS for Phase 1 (minimal styling needed). Add Tailwind in later phases if the UI complexity warrants it.

## Sources

### Primary (HIGH confidence)
- [Tauri v2 Leptos Frontend Guide](https://v2.tauri.app/start/frontend/leptos/) -- CSR setup, Trunk config, withGlobalTauri
- [Tauri v2 Create Project](https://v2.tauri.app/start/create-project/) -- Scaffolding commands and template options
- [Tauri v2 DMG Distribution](https://v2.tauri.app/distribute/dmg/) -- DMG build commands and configuration
- [Tauri v2 Store Plugin](https://v2.tauri.app/plugin/store/) -- Store API, permissions, Rust and JS usage
- [Tauri v2 Capabilities](https://v2.tauri.app/security/capabilities/) -- Permission model and configuration
- [Leptos Router Documentation](https://book.leptos.dev/router/16_routes.html) -- Route definition, Router component, CSR routing
- [Leptos GitHub Releases](https://github.com/leptos-rs/leptos/releases) -- Version history, latest 0.8.15
- [create-tauri-app Releases](https://github.com/tauri-apps/create-tauri-app/releases) -- v4.7.0 bumped Leptos to 0.8
- [keyring crate docs.rs](https://docs.rs/keyring) -- v3.6.3 API, platform support, feature flags
- [tauri-wasm crate docs.rs](https://docs.rs/tauri-wasm/latest/tauri_wasm/) -- v0.2.0, invoke API for WASM

### Secondary (MEDIUM confidence)
- [ForgeStream: Getting Started with Leptos and Tauri](https://forgestream.idverse.com/blog/20251020-getting-started-with-leptos-and-tauri/) -- End-to-end walkthrough with wasm_bindgen patterns
- [Tauri GitHub Discussion #7846](https://github.com/tauri-apps/tauri/discussions/7846) -- Secure storage options, maintainer recommendations
- [tauri-plugin-keyring](https://github.com/HuakunShen/tauri-plugin-keyring) -- Community plugin API (v0.1.0)
- [tauri-plugin-secure-storage](https://lib.rs/crates/tauri-plugin-secure-storage) -- Community plugin (v1.4.0, wraps keyring)
- [Leptos start-trunk template](https://github.com/leptos-rs/start-trunk) -- CSR project structure reference
- [DeepWiki: Rust Templates for create-tauri-app](https://deepwiki.com/tauri-apps/create-tauri-app/7.2-rust-templates) -- Generated template structure details

### Tertiary (LOW confidence)
- [Tauri v2 Stronghold Plugin](https://v2.tauri.app/plugin/stronghold/) -- Official docs say no deprecation notice, but GitHub discussion says deprecated in v3. Contradictory.
- CSS/styling approach -- No strong data on best CSS framework for Leptos desktop apps. Decision deferred.

### Prior Project Research (HIGH confidence for applicable parts)
- `.planning/research/STACK.md` -- Rust crate versions verified (serde, tokio, anyhow, thiserror, tracing, dirs, keyring). CLI-specific crates (clap, colored, indicatif, dialoguer) not applicable to Phase 1.
- `.planning/research/ARCHITECTURE.md` -- Bambu Studio profile paths, JSON schema, inheritance chains all verified and applicable.
- `.planning/research/PITFALLS.md` -- Profile stability pitfalls, cloud sync issues, and filesystem patterns all applicable. CLI-specific pitfalls not applicable.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- Tauri 2 + Leptos verified via official docs and create-tauri-app releases. All crate versions checked via docs.rs or crates.io.
- Architecture: HIGH -- Project structure follows official Tauri + Leptos template. IPC pattern verified from multiple sources.
- Pitfalls: MEDIUM-HIGH -- Most pitfalls identified from official docs and community reports. Keychain signing behavior in dev builds is LOW confidence (needs empirical validation).
- Keychain approach: MEDIUM -- keyring crate is well-established (v3.6.3, multiple platforms) but no official Tauri plugin for keychain. Community plugins are v0.1.x/v1.x with low adoption. Direct keyring usage in Tauri commands is the pragmatic choice but not officially blessed.

**Research date:** 2026-02-04
**Valid until:** 30 days (Tauri and Leptos ecosystems are active but not rapidly breaking)
