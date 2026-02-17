#!/usr/bin/env bash
set -euo pipefail

out_file="${1:-benchmarks/latest.json}"
cd "$(dirname "$0")/.."

json_escape() {
  local value="$1"
  value=${value//\\/\\\\}
  value=${value//\"/\\\"}
  value=${value//$'\n'/\\n}
  value=${value//$'\r'/\\r}
  printf '%s' "$value"
}

startup_output="$(./scripts/benchmark.sh startup 3 2>&1)"
memory_output="$(./scripts/benchmark.sh memory "record probe" 2>&1)"
drift_output="$(./scripts/benchmark.sh drift 8 4 2>&1)"

mkdir -p "$(dirname "$out_file")"

cat > "$out_file" <<JSON
{
  "generated_at_utc": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "startup": "$(json_escape "$startup_output")",
  "memory": "$(json_escape "$memory_output")",
  "drift": "$(json_escape "$drift_output")"
}
JSON

echo "wrote $out_file"
