#!/usr/bin/env bash
set -euo pipefail

body="${PR_BODY:-}"

if [[ -z "$body" ]]; then
  echo "FAIL: PR body is empty; release checklist is required." >&2
  exit 1
fi

tmp="$(mktemp /tmp/rustcode-release-pr-body.XXXXXX)"
printf '%s\n' "$body" >"$tmp"

require_checked() {
  local pattern="$1"
  local label="$2"
  if ! grep -Eiq "$pattern" "$tmp"; then
    echo "FAIL: release checklist item not checked: $label" >&2
    rm -f "$tmp"
    exit 1
  fi
}

require_checked '\[[xX]\].*ci_matrix\.sh' "ci_matrix"
require_checked '\[[xX]\].*benchmark_release_gate\.sh' "benchmark_release_gate"
require_checked '\[[xX]\].*release_with_gh\.sh.*--dry-run' "release_with_gh dry-run"
require_checked '\[[xX]\].*release notes' "release notes"
require_checked '\[[xX]\].*refresh_release_baseline\.sh.*--from-latest' "release baseline refresh decision"

rm -f "$tmp"
echo "PASS: release PR checklist is complete"
