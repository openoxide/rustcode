#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<USAGE
Usage:
  scripts/nightly_benchmark_drift.sh [candidate_json] [report_txt]

Defaults:
  candidate_json=/tmp/rustcode-nightly-candidate.json
  report_txt=/tmp/rustcode-nightly-report.txt
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

candidate="${1:-/tmp/rustcode-nightly-candidate.json}"
report="${2:-/tmp/rustcode-nightly-report.txt}"
baseline="${BASELINE_PATH:-benchmarks/release-baseline.json}"

cd "$(dirname "$0")/.."

if [[ ! -f "$baseline" ]]; then
  echo "missing baseline artifact: $baseline" >&2
  exit 1
fi

allow_serve=0
allow_memory=0
if [[ "$(uname -s)" == "Darwin" ]]; then
  # Local Darwin sandboxes may block bind/time metrics.
  allow_serve=1
  allow_memory=1
fi

./scripts/benchmark_record.sh "$candidate"
ALLOW_MISSING_SERVE_TELEMETRY="$allow_serve" ./scripts/assert_serve_telemetry.sh "$candidate"
ALLOW_MISSING_METRICS="$allow_memory" ./scripts/benchmark_assert.sh "$candidate"
./scripts/benchmark_compare.sh "$baseline" "$candidate" | tee "$report"
ALLOW_MISSING_RELEASE_METRICS="$allow_memory" ./scripts/benchmark_release_gate.sh "$baseline" "$candidate"

echo "nightly candidate=$candidate"
echo "nightly report=$report"
