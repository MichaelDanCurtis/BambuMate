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

## CI/CD

Automated builds run on every push to `main` via GitHub Actions:
- macOS (Apple Silicon + Intel)
- Windows (x64)

Tagged releases (`v*`) automatically create draft GitHub Releases with all platform binaries.

## License

*TBD*
