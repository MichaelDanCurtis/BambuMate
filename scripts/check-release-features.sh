#!/usr/bin/env bash
#
# Guards against the private `claude-subscription` Cargo feature ever being
# enabled by a release build workflow, whether directly or via a blanket
# `--all-features` build (which would implicitly enable it too).
#
# build.yml has no legitimate reason to mention either the feature name or
# `--all-features` at all, so this is a strict "must not appear anywhere"
# check rather than trying to pattern-match specific invocation forms
# (`--features`, `-F`, a multi-line YAML `args:` list, ...). Comments are
# stripped first so an explanatory comment naming the feature does not trip
# the guard.
#
# Usage: scripts/check-release-features.sh [path-to-workflow-yaml]
#   (defaults to .github/workflows/build.yml so fixtures can be passed in
#   tests)

set -uo pipefail

FILE="${1:-.github/workflows/build.yml}"

if [ ! -f "$FILE" ]; then
  echo "::error::check-release-features.sh: file not found: $FILE" >&2
  exit 1
fi

# Strip `#` comments: a `#` counts as starting a comment when it is at the
# start of the line or preceded by whitespace. Good enough for a GitHub
# Actions workflow file, which never legitimately embeds a bare `#` in a
# command or feature list.
stripped="$(sed -E 's/(^|[[:space:]])#.*$//' "$FILE")"

matches="$(printf '%s\n' "$stripped" | grep -nE 'claude-subscription|--all-features')"

if [ -n "$matches" ]; then
  echo "::error::$FILE must not enable the private claude-subscription feature (directly or via --all-features)"
  echo "$matches"
  exit 1
fi

exit 0
