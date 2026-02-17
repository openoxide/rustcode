#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<USAGE
Usage:
  scripts/release_with_gh.sh <version> [--dry-run] [--skip-checks] [--allow-unsigned-tag] [--auto-notes] [--notes <file>] [--benchmark <file>] [--bundle-output <dir>]

Examples:
  scripts/release_with_gh.sh 0.1.0 --dry-run
  scripts/release_with_gh.sh v0.1.0 --notes docs/RELEASE_NOTES_DRAFT.md
USAGE
}

notes_file="docs/RELEASE_NOTES_DRAFT.md"
benchmark_file="benchmarks/latest.json"
bundle_output="${BUNDLE_OUTPUT_DIR:-/tmp/rustcode-release-dist}"
dry_run=0
skip_checks=0
allow_unsigned_tag=0
auto_notes=0
version=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      dry_run=1
      ;;
    --skip-checks)
      skip_checks=1
      ;;
    --allow-unsigned-tag)
      allow_unsigned_tag=1
      ;;
    --auto-notes)
      auto_notes=1
      ;;
    --notes)
      notes_file="${2:-}"
      shift
      ;;
    --benchmark)
      benchmark_file="${2:-}"
      shift
      ;;
    --bundle-output)
      bundle_output="${2:-}"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "unknown argument: $1" >&2
      usage
      exit 1
      ;;
    *)
      if [[ -z "$version" ]]; then
        version="$1"
      else
        echo "unexpected positional argument: $1" >&2
        usage
        exit 1
      fi
      ;;
  esac
  shift
done

if [[ -z "$version" ]]; then
  usage
  exit 1
fi

tag="v${version#v}"

if [[ -z "$notes_file" || -z "$benchmark_file" ]]; then
  echo "notes and benchmark paths must be non-empty" >&2
  exit 1
fi

cd "$(dirname "$0")/.."

if (( auto_notes == 1 )); then
  generated_notes="/tmp/rustcode-release-notes-${tag}.md"
  ./scripts/draft_release_notes.sh "$tag" "" HEAD "$generated_notes"
  notes_file="$generated_notes"
fi

if [[ ! -f "$notes_file" ]]; then
  echo "missing notes file: $notes_file" >&2
  exit 1
fi

if [[ ! -f "$benchmark_file" ]]; then
  echo "missing benchmark artifact: $benchmark_file" >&2
  exit 1
fi

if [[ -z "$bundle_output" ]]; then
  echo "bundle output path must be non-empty" >&2
  exit 1
fi

if git rev-parse "$tag" >/dev/null 2>&1; then
  echo "tag already exists locally: $tag" >&2
  exit 1
fi

if (( dry_run == 0 )); then
  if git ls-remote --tags origin "refs/tags/$tag" | grep -q "$tag"; then
    echo "tag already exists on origin: $tag" >&2
    exit 1
  fi

  if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "working tree must be clean before release" >&2
    exit 1
  fi

  if (( skip_checks == 0 )); then
    ./scripts/ci_matrix.sh
    ./scripts/benchmark_release_gate.sh benchmarks/release-baseline.json "$benchmark_file"
  fi
fi

if (( dry_run == 1 )); then
  echo "DRY RUN"
  echo "./scripts/build_release_bundle.sh $tag $bundle_output"
  echo "<verify checksum for generated bundle>"
  echo "git tag -s $tag -m 'release $tag' (fallback to -a only with --allow-unsigned-tag)"
  echo "git push origin main"
  echo "git push origin $tag"
  echo "gh release create $tag --title $tag --notes-file $notes_file $benchmark_file <bundle.tar.gz> <bundle.tar.gz.sha256>"
  exit 0
fi

./scripts/build_release_bundle.sh "$tag" "$bundle_output"

archive="$(ls "$bundle_output"/rustcode-"$tag"-*.tar.gz 2>/dev/null | head -n1 || true)"
if [[ -z "$archive" ]]; then
  echo "failed to locate release archive in $bundle_output" >&2
  exit 1
fi
checksum="${archive}.sha256"
if [[ ! -f "$checksum" ]]; then
  echo "missing checksum file: $checksum" >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum -c "$checksum"
elif command -v shasum >/dev/null 2>&1; then
  shasum -a 256 -c "$checksum"
else
  echo "no checksum verifier available (sha256sum/shasum)" >&2
  exit 1
fi

if ! git tag -s "$tag" -m "release $tag"; then
  if (( allow_unsigned_tag == 1 )); then
    echo "WARN: signed tag failed; falling back to unsigned annotated tag"
    git tag -a "$tag" -m "release $tag"
  else
    echo "FAIL: signed tag creation failed; rerun with --allow-unsigned-tag to permit fallback" >&2
    exit 1
  fi
fi

git push origin main
git push origin "$tag"
gh release create "$tag" \
  --title "$tag" \
  --notes-file "$notes_file" \
  "$benchmark_file" \
  "$archive" \
  "$checksum"

echo "release complete: $tag"
