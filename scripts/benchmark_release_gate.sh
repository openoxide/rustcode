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
source "$(dirname "$0")/benchmark_metrics.sh"

if [[ ! -f "$baseline_file" ]]; then
  echo "missing baseline artifact: $baseline_file" >&2
  exit 1
fi

if [[ ! -f "$candidate_file" ]]; then
  echo "missing candidate artifact: $candidate_file" >&2
  exit 1
fi

baseline_rss=$(benchmark_extract_max_rss_bytes "$baseline_file")
candidate_rss=$(benchmark_extract_max_rss_bytes "$candidate_file")
baseline_peak=$(benchmark_extract_peak_bytes "$baseline_file")
candidate_peak=$(benchmark_extract_peak_bytes "$candidate_file")
baseline_p95=$(benchmark_extract_latency_p95_seconds "$baseline_file")
candidate_p95=$(benchmark_extract_latency_p95_seconds "$candidate_file")

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
