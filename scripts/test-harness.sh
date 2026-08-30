#!/usr/bin/env bash
#
# BambuMate cross-platform test harness (macOS / Linux).
#
# Runs every stage the CI matrix runs, in the same order, so a green local run
# means a green CI run. Each stage is reported individually and the script
# keeps going after a failure so you get the full picture in one pass.
#
#   ./scripts/test-harness.sh              # full run
#   ./scripts/test-harness.sh --quick      # skip the frontend build
#   ./scripts/test-harness.sh --network    # include the outbound HTTPS check
#
# Exit code is non-zero if any stage failed.

set -uo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"

QUICK=0
NETWORK=""
for arg in "$@"; do
  case "$arg" in
    --quick)   QUICK=1 ;;
    --network) NETWORK="--network" ;;
    -h|--help)
      sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown option: $arg" >&2
      exit 2
      ;;
  esac
done

FAILED_STAGES=()
PASSED_STAGES=()

# Run a stage, record the outcome, never abort the harness early.
stage() {
  local name="$1"; shift
  printf '\n\033[1m==> %s\033[0m\n' "$name"
  if "$@"; then
    PASSED_STAGES+=("$name")
  else
    printf '\033[31m    stage failed: %s\033[0m\n' "$name"
    FAILED_STAGES+=("$name")
  fi
}

have() { command -v "$1" >/dev/null 2>&1; }

echo "BambuMate test harness"
echo "  repo:   $ROOT"
echo "  os:     $(uname -s) $(uname -m)"
echo "  rustc:  $(rustc --version 2>/dev/null || echo 'not found')"

if ! have cargo; then
  echo "error: cargo not found — install Rust from https://rustup.rs" >&2
  exit 1
fi

# --- Formatting -------------------------------------------------------------
stage "rustfmt (frontend)" cargo fmt --check
stage "rustfmt (backend)"  cargo fmt --manifest-path src-tauri/Cargo.toml --check

# --- Lint -------------------------------------------------------------------
# Not gated on -D warnings: the tree has pre-existing clippy warnings, and a
# harness that is red before you start is a harness nobody runs.
stage "clippy (backend)" cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets

# --- Tests ------------------------------------------------------------------
stage "backend tests" cargo test --manifest-path src-tauri/Cargo.toml

# --- Frontend ---------------------------------------------------------------
if ! rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown; then
  echo "  installing wasm32-unknown-unknown target..."
  rustup target add wasm32-unknown-unknown
fi
stage "frontend typecheck" cargo check --target wasm32-unknown-unknown

if [ "$QUICK" -eq 0 ]; then
  if have trunk; then
    stage "frontend build" trunk build
  else
    echo "  skipping frontend build: trunk not installed (cargo install trunk)"
  fi
fi

# --- Platform diagnostics ---------------------------------------------------
# The real payload: exercises this machine's filesystem semantics, external
# tools, Bambu Studio install, keychain and SQLite.
stage "diagnostics" cargo run --quiet --manifest-path src-tauri/Cargo.toml \
  --bin bambumate-doctor -- $NETWORK

# Always capture a JSON report, even when the run failed — that is exactly when
# it is most useful to attach to a bug report.
REPORT="$ROOT/bambumate-report.json"
cargo run --quiet --manifest-path src-tauri/Cargo.toml \
  --bin bambumate-doctor -- --json $NETWORK > "$REPORT" 2>/dev/null || true
echo "  JSON report written to $REPORT"

# --- Summary ----------------------------------------------------------------
printf '\n\033[1m==> Summary\033[0m\n'
for s in "${PASSED_STAGES[@]:-}"; do
  [ -n "$s" ] && printf '  \033[32mPASS\033[0m  %s\n' "$s"
done
for s in "${FAILED_STAGES[@]:-}"; do
  [ -n "$s" ] && printf '  \033[31mFAIL\033[0m  %s\n' "$s"
done

if [ "${#FAILED_STAGES[@]}" -gt 0 ]; then
  printf '\n%d stage(s) failed.\n' "${#FAILED_STAGES[@]}"
  exit 1
fi

printf '\nAll stages passed.\n'
