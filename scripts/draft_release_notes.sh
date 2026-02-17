#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<USAGE
Usage:
  scripts/draft_release_notes.sh <version> [from_ref] [to_ref] [output_file]

Defaults:
  from_ref: latest tag matching v*
  to_ref: HEAD
  output_file: docs/RELEASE_NOTES_DRAFT.md
USAGE
}

if [[ $# -lt 1 ]]; then
  usage
  exit 1
fi

version="${1#v}"
from_ref="${2:-}"
to_ref="${3:-HEAD}"
output_file="${4:-docs/RELEASE_NOTES_DRAFT.md}"

cd "$(dirname "$0")/.."

if [[ -z "$from_ref" ]]; then
  from_ref="$(git tag --list 'v*' --sort=-version:refname | head -n1 || true)"
fi

range="$to_ref"
if [[ -n "$from_ref" ]]; then
  range="$from_ref..$to_ref"
fi

collect_section() {
  local section="$1"
  local regex="$2"
  git log "$range" --pretty=format:'%s|%h' | grep -E "$regex" || true
}

render_section() {
  local title="$1"
  local body="$2"
  if [[ -z "$body" ]]; then
    return
  fi
  {
    echo "## $title"
    while IFS='|' read -r subject sha; do
      [[ -z "${subject:-}" ]] && continue
      echo "- $subject ($sha)"
    done <<< "$body"
    echo
  } >> "$output_file"
}

mkdir -p "$(dirname "$output_file")"

{
  echo "# Release v$version"
  echo
  echo "Date: $(date -u +%Y-%m-%d)"
  echo
  if [[ -n "$from_ref" ]]; then
    echo "Range: \`$from_ref..$to_ref\`"
  else
    echo "Range: \`$to_ref\`"
  fi
  echo
} > "$output_file"

features="$(collect_section "Features" '^(feat|feature)(\(|:)')"
fixes="$(collect_section "Fixes" '^fix(\(|:)')"
perf="$(collect_section "Performance" '^perf(\(|:)')"
docs="$(collect_section "Documentation" '^docs(\(|:)')"
ci="$(collect_section "CI/Release" '^(ci|chore\(release\)|chore\(bench\))')"
other="$(git log "$range" --pretty=format:'%s|%h' | grep -Ev '^(feat|feature|fix|perf|docs|ci|chore\(release\)|chore\(bench\))(\(|:)' || true)"

render_section "Features" "$features"
render_section "Fixes" "$fixes"
render_section "Performance" "$perf"
render_section "Documentation" "$docs"
render_section "CI and Release" "$ci"
render_section "Other" "$other"

echo "wrote $output_file"
