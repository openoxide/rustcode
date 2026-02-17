#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: scripts/benchmark_compare.sh <old.json> <new.json>" >&2
  exit 1
fi

old_file="$1"
new_file="$2"

extract_value() {
  local file="$1"
  local pattern="$2"
  local fallback="$3"
  local value
  value=$(grep -oE "$pattern" "$file" | head -n1 | grep -oE '[0-9]+' || true)
  if [[ -z "$value" ]]; then
    echo "$fallback"
  else
    echo "$value"
  fi
}

old_rss=$(extract_value "$old_file" '[0-9]+  maximum resident set size' 0)
new_rss=$(extract_value "$new_file" '[0-9]+  maximum resident set size' 0)
old_peak=$(extract_value "$old_file" '[0-9]+  peak memory footprint' 0)
new_peak=$(extract_value "$new_file" '[0-9]+  peak memory footprint' 0)

rss_delta=$((new_rss - old_rss))
peak_delta=$((new_peak - old_peak))

printf "compare old=%s new=%s\n" "$old_file" "$new_file"
printf "max_rss old=%s new=%s delta=%+d\n" "$old_rss" "$new_rss" "$rss_delta"
printf "peak_footprint old=%s new=%s delta=%+d\n" "$old_peak" "$new_peak" "$peak_delta"
