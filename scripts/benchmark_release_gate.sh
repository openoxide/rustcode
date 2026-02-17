#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: scripts/benchmark_release_gate.sh <baseline.json> <candidate.json>" >&2
  exit 1
fi

baseline_file="$1"
candidate_file="$2"
max_rss_delta_limit="${MAX_RSS_DELTA_LIMIT:-1048576}"
peak_delta_limit="${PEAK_DELTA_LIMIT:-524288}"
p95_delta_limit="${P95_DELTA_LIMIT:-0.100}"
allow_missing="${ALLOW_MISSING_RELEASE_METRICS:-0}"

if [[ ! -f "$baseline_file" ]]; then
  echo "missing baseline artifact: $baseline_file" >&2
  exit 1
fi

if [[ ! -f "$candidate_file" ]]; then
  echo "missing candidate artifact: $candidate_file" >&2
  exit 1
fi

extract_int() {
  local file="$1"
  local pattern="$2"
  local value
  value=$(grep -oE "$pattern" "$file" | head -n1 | grep -oE '[0-9]+' || true)
  if [[ -z "$value" ]]; then
    echo 0
  else
    echo "$value"
  fi
}

extract_float() {
  local file="$1"
  local pattern="$2"
  local value
  value=$(grep -oE "$pattern" "$file" | head -n1 | sed -E 's/.*=([0-9.]+)/\1/' || true)
  if [[ -z "$value" ]]; then
    echo 0
  else
    echo "$value"
  fi
}

extract_sampled_rss() {
  local file="$1"
  local sampled
  sampled=$(grep -oE '[0-9]+[[:space:]]+[0-9]+[[:space:]]+[0-9]+\.[0-9]' "$file" | awk '{print $2}' | sort -nr | head -n1 || true)
  if [[ -z "$sampled" ]]; then
    echo 0
  else
    echo "$sampled"
  fi
}

normalize_rss_and_peak() {
  local file="$1"
  local rss
  local peak
  local sampled

  rss=$(extract_int "$file" '[0-9]+  maximum resident set size')
  peak=$(extract_int "$file" '[0-9]+  peak memory footprint')
  sampled=$(extract_sampled_rss "$file")

  if (( rss == 0 && sampled > 0 )); then
    rss="$sampled"
  fi
  if (( peak == 0 && rss > 0 )); then
    peak="$rss"
  fi

  printf '%s %s\n' "$rss" "$peak"
}

read -r baseline_rss baseline_peak < <(normalize_rss_and_peak "$baseline_file")
read -r candidate_rss candidate_peak < <(normalize_rss_and_peak "$candidate_file")
baseline_p95=$(extract_float "$baseline_file" 'latency_p95_s=[0-9]+(\.[0-9]+)?')
candidate_p95=$(extract_float "$candidate_file" 'latency_p95_s=[0-9]+(\.[0-9]+)?')

rss_delta=$((candidate_rss - baseline_rss))
peak_delta=$((candidate_peak - baseline_peak))
p95_delta=$(awk -v old="$baseline_p95" -v new="$candidate_p95" 'BEGIN { printf "%.6f", new - old }')

printf "release gate baseline=%s candidate=%s\n" "$baseline_file" "$candidate_file"
printf "rss baseline=%s candidate=%s delta=%+d limit=%s\n" "$baseline_rss" "$candidate_rss" "$rss_delta" "$max_rss_delta_limit"
printf "peak baseline=%s candidate=%s delta=%+d limit=%s\n" "$baseline_peak" "$candidate_peak" "$peak_delta" "$peak_delta_limit"
printf "p95 baseline=%s candidate=%s delta=%s limit=%s\n" "$baseline_p95" "$candidate_p95" "$p95_delta" "$p95_delta_limit"

if (( baseline_rss == 0 && baseline_peak == 0 )); then
  if [[ "$allow_missing" == "1" ]]; then
    echo "WARN: missing baseline memory metrics; ALLOW_MISSING_RELEASE_METRICS=1 set"
    exit 0
  fi
  echo "FAIL: baseline memory metrics unavailable" >&2
  exit 1
fi

if (( candidate_rss == 0 && candidate_peak == 0 )); then
  if [[ "$allow_missing" == "1" ]]; then
    echo "WARN: missing candidate memory metrics; ALLOW_MISSING_RELEASE_METRICS=1 set"
    exit 0
  fi
  echo "FAIL: candidate memory metrics unavailable" >&2
  exit 1
fi

if (( rss_delta > max_rss_delta_limit )); then
  echo "FAIL: rss delta exceeds limit" >&2
  exit 1
fi

if (( peak_delta > peak_delta_limit )); then
  echo "FAIL: peak delta exceeds limit" >&2
  exit 1
fi

if awk -v delta="$p95_delta" -v limit="$p95_delta_limit" 'BEGIN { exit !(delta > limit) }'; then
  echo "FAIL: p95 latency delta exceeds limit" >&2
  exit 1
fi

echo "PASS: release benchmark gate"
