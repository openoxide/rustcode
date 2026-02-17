#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: scripts/benchmark_compare.sh <old.json> <new.json>" >&2
  exit 1
fi

old_file="$1"
new_file="$2"
source "$(dirname "$0")/benchmark_metrics.sh"

old_rss=$(benchmark_extract_max_rss_bytes "$old_file")
new_rss=$(benchmark_extract_max_rss_bytes "$new_file")
old_peak=$(benchmark_extract_peak_bytes "$old_file")
new_peak=$(benchmark_extract_peak_bytes "$new_file")
old_p95=$(benchmark_extract_latency_p95_seconds "$old_file")
new_p95=$(benchmark_extract_latency_p95_seconds "$new_file")

printf "compare old=%s new=%s\n" "$old_file" "$new_file"

if (( old_rss == 0 || new_rss == 0 )); then
  printf "max_rss old=%s new=%s delta=n/a (missing metrics)\n" "$old_rss" "$new_rss"
else
  rss_delta=$((new_rss - old_rss))
  printf "max_rss old=%s new=%s delta=%+d\n" "$old_rss" "$new_rss" "$rss_delta"
fi

if (( old_peak == 0 || new_peak == 0 )); then
  printf "peak_footprint old=%s new=%s delta=n/a (missing metrics)\n" "$old_peak" "$new_peak"
else
  peak_delta=$((new_peak - old_peak))
  printf "peak_footprint old=%s new=%s delta=%+d\n" "$old_peak" "$new_peak" "$peak_delta"
fi

if grep -q 'latency_p95_s=' "$old_file" && grep -q 'latency_p95_s=' "$new_file"; then
  p95_delta=$(awk -v old="$old_p95" -v new="$new_p95" 'BEGIN { printf "%.6f", new - old }')
  printf "latency_p95_s old=%s new=%s delta=%s\n" "$old_p95" "$new_p95" "$p95_delta"
else
  printf "latency_p95_s old=%s new=%s delta=n/a (missing metrics)\n" "$old_p95" "$new_p95"
fi
