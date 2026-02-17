#!/usr/bin/env bash
set -euo pipefail

# Returns first matching integer value for the supplied regex, or 0.
benchmark_extract_first_int() {
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

# Extracts sampled RSS (KB) from ps probe lines in drift/serve sections.
benchmark_extract_sampled_rss_kb() {
  local file="$1"
  local sampled
  sampled=$(grep -oE '[0-9]+[[:space:]]+[0-9]+[[:space:]]+[0-9]+\.[0-9]' "$file" | awk '{print $2}' | sort -nr | head -n1 || true)
  if [[ -z "$sampled" ]]; then
    echo 0
  else
    echo "$sampled"
  fi
}

# Normalized bytes value for max RSS across BSD/GNU time outputs.
benchmark_extract_max_rss_bytes() {
  local file="$1"
  local bsd_bytes
  local gnu_kb
  local sampled_kb

  bsd_bytes=$(benchmark_extract_first_int "$file" '[0-9]+  maximum resident set size')
  if (( bsd_bytes > 0 )); then
    echo "$bsd_bytes"
    return
  fi

  gnu_kb=$(benchmark_extract_first_int "$file" 'Maximum resident set size \(kbytes\):[[:space:]]*[0-9]+')
  if (( gnu_kb > 0 )); then
    echo $((gnu_kb * 1024))
    return
  fi

  sampled_kb=$(benchmark_extract_sampled_rss_kb "$file")
  if (( sampled_kb > 0 )); then
    echo $((sampled_kb * 1024))
    return
  fi

  echo 0
}

# Normalized bytes value for peak footprint; falls back to max RSS bytes.
benchmark_extract_peak_bytes() {
  local file="$1"
  local peak_bytes

  peak_bytes=$(benchmark_extract_first_int "$file" '[0-9]+  peak memory footprint')
  if (( peak_bytes > 0 )); then
    echo "$peak_bytes"
    return
  fi

  benchmark_extract_max_rss_bytes "$file"
}

benchmark_extract_latency_p95_seconds() {
  local file="$1"
  local value
  value=$(grep -oE 'latency_p95_s=[0-9]+(\.[0-9]+)?' "$file" | head -n1 | sed -E 's/.*=([0-9.]+)/\1/' || true)
  if [[ -z "$value" ]]; then
    echo 0
  else
    echo "$value"
  fi
}
