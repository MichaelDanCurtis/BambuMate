<#
.SYNOPSIS
    BambuMate cross-platform test harness (Windows).

.DESCRIPTION
    Runs every stage the CI matrix runs, in the same order, so a green local
    run means a green CI run. Stages are reported individually and the script
    continues after a failure so you get the full picture in one pass.

    This is the Windows twin of scripts/test-harness.sh — keep the two in sync.

.PARAMETER Quick
    Skip the Trunk frontend build (the slowest stage).

.PARAMETER Network
    Include the outbound HTTPS diagnostics check.

.EXAMPLE
    .\scripts\test-harness.ps1
    .\scripts\test-harness.ps1 -Quick -Network
#>
[CmdletBinding()]
param(
    [switch]$Quick,
    [switch]$Network
)

$ErrorActionPreference = 'Continue'

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

$passed = New-Object System.Collections.Generic.List[string]
$failed = New-Object System.Collections.Generic.List[string]

function Invoke-Stage {
    param(
        [string]$Name,
        [scriptblock]$Action
    )
    Write-Host ""
    Write-Host "==> $Name" -ForegroundColor Cyan
    & $Action
    if ($LASTEXITCODE -eq 0) {
        $script:passed.Add($Name)
    }
    else {
        Write-Host "    stage failed: $Name (exit $LASTEXITCODE)" -ForegroundColor Red
        $script:failed.Add($Name)
    }
}

function Test-Command {
    param([string]$Name)
    $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

Write-Host "BambuMate test harness"
Write-Host "  repo:  $root"
Write-Host "  os:    windows $env:PROCESSOR_ARCHITECTURE"
if (Test-Command rustc) { Write-Host "  rustc: $(rustc --version)" }

if (-not (Test-Command cargo)) {
    Write-Host "error: cargo not found - install Rust from https://rustup.rs" -ForegroundColor Red
    exit 1
}

# --- Formatting -------------------------------------------------------------
Invoke-Stage "rustfmt (frontend)" { cargo fmt --check }
Invoke-Stage "rustfmt (backend)"  { cargo fmt --manifest-path src-tauri/Cargo.toml --check }

# --- Lint -------------------------------------------------------------------
# Deliberately not gated on -D warnings: the tree has pre-existing clippy
# warnings, and a harness that is red before you start is one nobody runs.
Invoke-Stage "clippy (backend)" { cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets }

# --- Tests ------------------------------------------------------------------
Invoke-Stage "backend tests" { cargo test --manifest-path src-tauri/Cargo.toml }

# --- Frontend ---------------------------------------------------------------
$targets = rustup target list --installed 2>$null
if ($targets -notcontains 'wasm32-unknown-unknown') {
    Write-Host "  installing wasm32-unknown-unknown target..."
    rustup target add wasm32-unknown-unknown | Out-Null
}
Invoke-Stage "frontend typecheck" { cargo check --target wasm32-unknown-unknown }

if (-not $Quick) {
    if (Test-Command trunk) {
        Invoke-Stage "frontend build" { trunk build }
    }
    else {
        Write-Host "  skipping frontend build: trunk not installed (cargo install trunk)"
    }
}

# --- Platform diagnostics ---------------------------------------------------
# The real payload: exercises this machine's filesystem semantics, external
# tools, Bambu Studio install, credential store and SQLite.
$doctorArgs = @()
if ($Network) { $doctorArgs += '--network' }

Invoke-Stage "diagnostics" {
    cargo run --quiet --manifest-path src-tauri/Cargo.toml --bin bambumate-doctor -- @doctorArgs
}

# Always capture a JSON report, even when the run failed - that is exactly when
# it is most useful to attach to a bug report.
$report = Join-Path $root 'bambumate-report.json'
cargo run --quiet --manifest-path src-tauri/Cargo.toml --bin bambumate-doctor -- --json @doctorArgs 2>$null |
    Set-Content -Path $report -Encoding utf8
Write-Host "  JSON report written to $report"

# --- Summary ----------------------------------------------------------------
Write-Host ""
Write-Host "==> Summary" -ForegroundColor Cyan
foreach ($s in $passed) { Write-Host "  PASS  $s" -ForegroundColor Green }
foreach ($s in $failed) { Write-Host "  FAIL  $s" -ForegroundColor Red }

if ($failed.Count -gt 0) {
    Write-Host ""
    Write-Host "$($failed.Count) stage(s) failed."
    exit 1
}

Write-Host ""
Write-Host "All stages passed."
exit 0
