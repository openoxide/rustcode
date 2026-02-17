#!/usr/bin/env bash
set -euo pipefail

file="${1:-benchmarks/latest.json}"
allow_missing="${ALLOW_MISSING_SERVE_TELEMETRY:-0}"

if [[ ! -f "$file" ]]; then
  echo "missing artifact: $file" >&2
  exit 1
fi

contains() {
  local pattern="$1"
  grep -Fq "$pattern" "$file"
}

if contains "failed to bind"; then
  if [[ "$allow_missing" == "1" ]]; then
    echo "WARN: serve telemetry assertion skipped (bind restriction + ALLOW_MISSING_SERVE_TELEMETRY=1)"
    exit 0
  fi
  echo "FAIL: serve benchmark failed to bind listener" >&2
  exit 1
fi

if contains "WARN: curl unavailable; skipping serve HTTP probes"; then
  if [[ "$allow_missing" == "1" ]]; then
    echo "WARN: serve telemetry assertion skipped (curl unavailable + ALLOW_MISSING_SERVE_TELEMETRY=1)"
    exit 0
  fi
  echo "FAIL: serve telemetry probes unavailable (curl missing)" >&2
  exit 1
fi

required=(
  "serve endpoint configured"
  "ServeRequest { method: \\\"GET\\\", path: \\\"/health\\\", status: 200 }"
  "ServeRequest { method: \\\"GET\\\", path: \\\"/does-not-exist\\\", status: 404 }"
  "execution cancelled"
)

for token in "${required[@]}"; do
  if ! contains "$token"; then
    echo "FAIL: missing serve telemetry token: $token" >&2
    exit 1
  fi
done

echo "PASS: serve telemetry assertions satisfied"
