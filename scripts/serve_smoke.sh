#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build -q -p rustcode-cli
bin="./target/debug/rustcode-cli"
listen="${1:-127.0.0.1:4319}"
health_url="http://$listen/health"
missing_url="http://$listen/does-not-exist"
allow_skip="${ALLOW_SERVE_SMOKE_SKIP:-0}"

log_file="$(mktemp /tmp/rustcode-serve-smoke.XXXXXX)"
health_body="$(mktemp /tmp/rustcode-serve-health.XXXXXX)"
missing_body="$(mktemp /tmp/rustcode-serve-missing.XXXXXX)"

cleanup() {
  if [[ -n "${pid:-}" ]]; then
    kill -INT "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -f "$log_file" "$health_body" "$missing_body"
}
trap cleanup EXIT

"$bin" serve --listen "$listen" >"$log_file" 2>&1 &
pid="$!"

ready=0
for _ in $(seq 1 50); do
  if curl -s --max-time 1 "$health_url" >"$health_body"; then
    ready=1
    break
  fi
  sleep 0.1
done

if (( ready == 0 )); then
  if grep -q "failed to bind" "$log_file" && [[ "$allow_skip" == "1" ]]; then
    echo "WARN: serve smoke skipped due bind restriction and ALLOW_SERVE_SMOKE_SKIP=1"
    exit 0
  fi
  echo "FAIL: serve smoke could not connect to $health_url" >&2
  tail -n 40 "$log_file" >&2 || true
  exit 1
fi

if ! grep -q '{"ok":true}' "$health_body"; then
  echo "FAIL: health response body mismatch" >&2
  cat "$health_body" >&2
  exit 1
fi

status_code="$(curl -sS --max-time 1 -o "$missing_body" -w '%{http_code}' "$missing_url")"
if [[ "$status_code" != "404" ]]; then
  echo "FAIL: missing route status expected 404, got $status_code" >&2
  cat "$missing_body" >&2
  exit 1
fi

if ! grep -q '{"error":"not found"}' "$missing_body"; then
  echo "FAIL: missing route body mismatch" >&2
  cat "$missing_body" >&2
  exit 1
fi

echo "PASS: serve smoke"
