#!/usr/bin/env bash
set -euo pipefail

source_file="${1:-benchmarks/latest.json}"
history_dir="${2:-benchmarks/history}"
keep="${KEEP_BENCHMARK_SNAPSHOTS:-20}"

if [[ ! -f "$source_file" ]]; then
  echo "missing source artifact: $source_file" >&2
  exit 1
fi

mkdir -p "$history_dir"

stamp="$(date -u +"%Y%m%dT%H%M%SZ")"
target="$history_dir/$stamp.json"
cp "$source_file" "$target"

echo "snapshot=$target"

# prune oldest snapshots beyond retention
total=$(find "$history_dir" -maxdepth 1 -type f -name '*.json' | wc -l | tr -d ' ')
if (( total > keep )); then
  prune=$((total - keep))
  find "$history_dir" -maxdepth 1 -type f -name '*.json' | sort | head -n "$prune" | while read -r old; do
    rm -f "$old"
    echo "pruned=$old"
  done
fi
