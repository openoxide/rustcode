#!/usr/bin/env bash
set -euo pipefail

file="${1:-benchmarks/latest.json}"
max_rss_limit="${MAX_RSS_LIMIT:-8000000}"
peak_limit="${PEAK_FOOTPRINT_LIMIT:-4000000}"
allow_missing="${ALLOW_MISSING_METRICS:-0}"

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
sampled_rss=$(grep -oE '[0-9]+[[:space:]]+[0-9]+[[:space:]]+[0-9]+\.[0-9]' "$file" | awk '{print $2}' | sort -nr | head -n1 || true)

if [[ -z "$sampled_rss" ]]; then
  sampled_rss=0
fi

if (( max_rss == 0 && sampled_rss > 0 )); then
  max_rss="$sampled_rss"
fi

if (( peak == 0 && max_rss > 0 )); then
  peak="$max_rss"
fi

printf "file=%s\n" "$file"
printf "max_rss=%s limit=%s\n" "$max_rss" "$max_rss_limit"
printf "peak_footprint=%s limit=%s\n" "$peak" "$peak_limit"
printf "sampled_rss_fallback=%s\n" "$sampled_rss"

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
