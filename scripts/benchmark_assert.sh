#!/usr/bin/env bash
set -euo pipefail

file="${1:-benchmarks/latest.json}"
max_rss_limit="${MAX_RSS_LIMIT:-8000000}"
peak_limit="${PEAK_FOOTPRINT_LIMIT:-$max_rss_limit}"
allow_missing="${ALLOW_MISSING_METRICS:-0}"
source "$(dirname "$0")/benchmark_metrics.sh"

max_rss=$(benchmark_extract_max_rss_bytes "$file")
peak=$(benchmark_extract_peak_bytes "$file")
sampled_rss_kb=$(benchmark_extract_sampled_rss_kb "$file")
sampled_rss_bytes=$((sampled_rss_kb * 1024))

printf "file=%s\n" "$file"
printf "max_rss=%s limit=%s\n" "$max_rss" "$max_rss_limit"
printf "peak_footprint=%s limit=%s\n" "$peak" "$peak_limit"
printf "sampled_rss_fallback_kb=%s\n" "$sampled_rss_kb"
printf "sampled_rss_fallback_bytes=%s\n" "$sampled_rss_bytes"

if (( max_rss == 0 && peak == 0 )); then
  if [[ "$allow_missing" == "1" ]]; then
    echo "WARN: no measurable memory metrics found; ALLOW_MISSING_METRICS=1 set"
    exit 0
  fi
  echo "FAIL: no measurable memory metrics found in artifact" >&2
  exit 1
fi

if (( max_rss > max_rss_limit )); then
  echo "FAIL: max_rss exceeds limit" >&2
  exit 1
fi

if (( peak > peak_limit )); then
  echo "FAIL: peak_footprint exceeds limit" >&2
  exit 1
fi

echo "PASS: benchmark assertions satisfied"
