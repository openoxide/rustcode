#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<USAGE
Usage:
  scripts/release_with_gh.sh <version> [--dry-run] [--skip-checks] [--notes <file>] [--benchmark <file>]

Examples:
  scripts/release_with_gh.sh 0.1.0 --dry-run
  scripts/release_with_gh.sh v0.1.0 --notes docs/RELEASE_NOTES_DRAFT.md
USAGE
}

notes_file="docs/RELEASE_NOTES_DRAFT.md"
benchmark_file="benchmarks/latest.json"
dry_run=0
skip_checks=0
version=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      dry_run=1
      ;;
    --skip-checks)
      skip_checks=1
      ;;
    --notes)
      notes_file="${2:-}"
      shift
      ;;
    --benchmark)
      benchmark_file="${2:-}"
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

if [[ ! -f "$notes_file" ]]; then
  echo "missing notes file: $notes_file" >&2
  exit 1
fi

if [[ ! -f "$benchmark_file" ]]; then
  echo "missing benchmark artifact: $benchmark_file" >&2
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
  echo "git tag -a $tag -m 'release $tag'"
  echo "git push origin main"
  echo "git push origin $tag"
  echo "gh release create $tag --title $tag --notes-file $notes_file $benchmark_file"
  exit 0
fi

git tag -a "$tag" -m "release $tag"
git push origin main
git push origin "$tag"
gh release create "$tag" \
  --title "$tag" \
  --notes-file "$notes_file" \
  "$benchmark_file"

echo "release complete: $tag"
