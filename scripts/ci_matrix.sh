#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

echo "[ci] cargo check --workspace"
cargo check --workspace

echo "[ci] cargo test --workspace"
cargo test --workspace

echo "[ci] serve smoke"
allow_serve_smoke_skip=0
if [[ "$(uname -s)" == "Darwin" ]]; then
  # Local Darwin sandboxes may deny binding fixed loopback ports.
  allow_serve_smoke_skip=1
fi
ALLOW_SERVE_SMOKE_SKIP="$allow_serve_smoke_skip" ./scripts/serve_smoke.sh

echo "[ci] benchmark record"
./scripts/benchmark_record.sh benchmarks/latest.json

echo "[ci] serve telemetry assert"
allow_serve_telemetry=0
if [[ "$(uname -s)" == "Darwin" ]]; then
  # Local Darwin sandboxes may deny listener binds used by serve probes.
  allow_serve_telemetry=1
fi
ALLOW_MISSING_SERVE_TELEMETRY="$allow_serve_telemetry" ./scripts/assert_serve_telemetry.sh benchmarks/latest.json

echo "[ci] benchmark assert"
allow_missing=0
max_rss_limit=8000000
if [[ "$(uname -s)" == "Darwin" ]]; then
  # macOS sandboxed runners may not expose detailed time/ps memory metrics.
  allow_missing=1
else
  # Linux CI includes rustls/reqwest auth dependencies that raise steady-state RSS.
  # Keep the gate conservative but aligned with measured baseline.
  max_rss_limit=14000000
fi
ALLOW_MISSING_METRICS="$allow_missing" \
MAX_RSS_LIMIT="$max_rss_limit" \
PEAK_FOOTPRINT_LIMIT="$max_rss_limit" \
./scripts/benchmark_assert.sh benchmarks/latest.json

echo "[ci] PASS"
