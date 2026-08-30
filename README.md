# BambuMate

**Smart filament profile management and AI-powered print analysis for Bambu Lab printers.**

BambuMate takes the guesswork out of 3D printing. Search for any filament, get an optimized Bambu Studio profile generated from real manufacturer specs, then analyze your test prints with AI vision to fine-tune settings automatically.

## Features

- **Filament Search & Scraping** — Search for filaments by name; BambuMate scrapes manufacturer specs and builds Bambu Studio profiles automatically
- **AI Print Analysis** — Drag-and-drop a photo of your test print for AI-powered defect detection (stringing, warping, layer adhesion, elephant's foot, and more) with specific setting change recommendations
- **Profile Management** — Browse, edit, and manage Bambu Studio filament profiles with visual diffs and one-click installation
- **Auto-Apply Changes** — Recommended profile tweaks are applied directly to your Bambu Studio config with automatic backup
- **OpenSCAD Studio Integration** — Push STLs from OpenSCAD Studio straight to Bambu Studio for slicing

## Screenshots

*Coming soon*

## Installation

### macOS (Pre-built)

1. Download the latest `.dmg` from [Releases](../../releases)
2. Open the DMG and drag **BambuMate** to your Applications folder
3. On first launch, right-click the app and select **Open** (macOS Gatekeeper prompt for unsigned apps)

### Windows (Pre-built)

1. Download the latest `.msi` or `.exe` installer from [Releases](../../releases)
2. Run the installer and follow the prompts

### Build from Source

#### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Trunk](https://trunkrs.dev/) — WASM build tool for Leptos frontend
- WASM target for Rust

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add WASM compilation target
rustup target add wasm32-unknown-unknown

# Install Trunk
cargo install trunk
```

#### macOS Additional Dependencies

No additional dependencies required — macOS includes everything needed.

#### Windows Additional Dependencies

- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the **C++ build tools** workload

#### Build & Run (Development)

```bash
git clone https://github.com/MichaelDanCurtis/BambuMate.git
cd BambuMate
cargo tauri dev
```

This starts the Trunk dev server for the frontend and the Tauri app in development mode with hot reload.

#### Build for Distribution

```bash
cargo tauri build
```

Output locations:
- **macOS**: `src-tauri/target/release/bundle/dmg/BambuMate_*.dmg`
- **Windows**: `src-tauri/target/release/bundle/msi/BambuMate_*.msi`

## Configuration

### AI API Key

BambuMate uses external AI APIs (Claude or GPT-4V) for print analysis. On first launch, go to **Settings** and enter your API key:

- [Get a Claude API key](https://console.anthropic.com/)
- [Get an OpenAI API key](https://platform.openai.com/api-keys)

Your API key is stored securely in your system keychain (macOS Keychain / Windows Credential Manager).

### Bambu Studio Profiles

BambuMate automatically detects your Bambu Studio installation and profile directory:

| Platform | Profile Path |
|----------|-------------|
| macOS | `~/Library/Application Support/BambuStudio/user/<device_id>/filament/` |
| Windows | `%AppData%\BambuStudio\user\<device_id>\filament\` |

## Tech Stack

- **Framework**: [Tauri 2.0](https://v2.tauri.app/) — Rust backend with native webview
- **Frontend**: [Leptos](https://leptos.dev/) — Reactive Rust framework compiled to WASM
- **AI**: Claude / GPT-4V APIs for vision analysis (no local models)
- **Language**: Rust throughout (backend + frontend)

## Testing & Diagnostics

BambuMate ships a cross-platform test harness. Because the app runs on both
macOS (WKWebView) and Windows (WebView2) with different filesystem, process and
credential-store semantics, the same suite runs on every platform.

### Run the full harness

```bash
# macOS / Linux
./scripts/test-harness.sh

# Windows (PowerShell)
.\scripts\test-harness.ps1
```

Both scripts run the identical stages — rustfmt, clippy, backend tests,
frontend typecheck, `trunk build`, and the platform diagnostics — and keep
going after a failure so one run shows every problem. Use `--quick` /
`-Quick` to skip the frontend build, and `--network` / `-Network` to include
the outbound HTTPS check.

### `bambumate-doctor`

The diagnostics engine is also a standalone binary. It probes the things that
actually differ between platforms: case sensitivity and Unicode normalisation
of filenames, atomic file replacement, path-containment guards, external tool
availability (`open`, `pgrep`, `mdfind` on macOS), Spotlight status, the Bambu
Studio installation and config tree, profile write/backup/registration
round-trips, the OS credential store, bundled SQLite, and the image pipeline.

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin bambumate-doctor

# Machine-readable output for bug reports
cargo run --manifest-path src-tauri/Cargo.toml --bin bambumate-doctor -- --json \
  > bambumate-report.json

# Options
#   --json            emit JSON instead of a table
#   --network         include the outbound HTTPS check
#   --no-keychain     skip the credential-store check
#   --no-live-bambu   skip checks needing an installed Bambu Studio
#   --list            print all check ids
```

Exit code is `0` when nothing failed and `1` otherwise, so it can gate a build.
Warnings never fail the run — a machine without Bambu Studio installed stays
green.

### From inside the app

**Settings → Health Check → Run Diagnostics** runs the same suite in-process.

This matters: a packaged macOS `.app` launched from Finder has a reduced `PATH`
(`/usr/bin:/bin:/usr/sbin:/sbin`), must clear Gatekeeper/TCC prompts for file
access, and reaches the keychain under its code signature. A terminal build
reproduces none of that, so bugs that only affect shipped macOS builds are
invisible to `cargo test` alone. **Copy Report** produces a full text snapshot
to paste into an issue.

### Individual test suites

```bash
cargo test --manifest-path src-tauri/Cargo.toml   # backend unit + integration
cargo check --target wasm32-unknown-unknown       # frontend typecheck
```

`src-tauri/tests/platform_tests.rs` holds the platform contract tests: they
assert the harness is internally consistent and that the platform guarantees
the app depends on genuinely hold on the current OS.

## CI/CD

Automated builds run on every push to `main` via GitHub Actions:
- macOS (Apple Silicon + Intel)
- Windows (x64)

`.github/workflows/test.yml` runs the test harness on the same matrix for every
push and pull request, and uploads each platform's diagnostics report as a
build artifact.

Tagged releases (`v*`) automatically create draft GitHub Releases with all platform binaries.

## License

*TBD*
