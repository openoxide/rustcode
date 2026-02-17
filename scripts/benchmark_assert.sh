#!/usr/bin/env bash
set -euo pipefail

file="${1:-benchmarks/latest.json}"
max_rss_limit="${MAX_RSS_LIMIT:-8000000}"
peak_limit="${PEAK_FOOTPRINT_LIMIT:-4000000}"

extract_value() {
  local pattern="$1"
  local value
  value=$(grep -oE "$pattern" "$file" | head -n1 | grep -oE '[0-9]+' || true)
  if [[ -z "$value" ]]; then
    echo 0
  else
    echo "$value"
  fi
}

max_rss=$(extract_value '[0-9]+  maximum resident set size')
peak=$(extract_value '[0-9]+  peak memory footprint')

printf "file=%s\n" "$file"
printf "max_rss=%s limit=%s\n" "$max_rss" "$max_rss_limit"
printf "peak_footprint=%s limit=%s\n" "$peak" "$peak_limit"

if (( max_rss > max_rss_limit )); then
  echo "FAIL: max_rss exceeds limit" >&2
  exit 1
fi

if (( peak > peak_limit )); then
  echo "FAIL: peak_footprint exceeds limit" >&2
  exit 1
fi

echo "PASS: benchmark assertions satisfied"
