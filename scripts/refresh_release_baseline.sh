#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<USAGE
Usage:
  scripts/refresh_release_baseline.sh [--from-latest]

Options:
  --from-latest   Skip benchmark regeneration and promote current benchmarks/latest.json.
USAGE
}

from_latest=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --from-latest)
      from_latest=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
  shift
done

cd "$(dirname "$0")/.."

latest="benchmarks/latest.json"
baseline="benchmarks/release-baseline.json"
previous="$(mktemp /tmp/rustcode-release-baseline-prev.XXXXXX)"
had_previous=0

if [[ -f "$baseline" ]]; then
  cp "$baseline" "$previous"
  had_previous=1
fi

if (( from_latest == 0 )); then
  ./scripts/benchmark_record.sh "$latest"
fi

./scripts/assert_serve_telemetry.sh "$latest"
./scripts/benchmark_assert.sh "$latest"
cp "$latest" "$baseline"

if (( had_previous == 1 )); then
  ./scripts/benchmark_compare.sh "$previous" "$baseline"
  ./scripts/benchmark_release_gate.sh "$previous" "$baseline"
fi

rm -f "$previous"
echo "updated $baseline from $latest"
