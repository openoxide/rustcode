#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<USAGE
Usage:
  scripts/benchmark.sh startup [runs]
  scripts/benchmark.sh memory [prompt]
USAGE
}

if [[ $# -lt 1 ]]; then
  usage
  exit 1
fi

mode="$1"
shift || true

cd "$(dirname "$0")/.."

cargo build -q -p rustcode-cli

bin="./target/debug/rustcode-cli"

run_time() {
  if /usr/bin/time -p "$@" >/dev/null 2>&1; then
    /usr/bin/time -p "$@" >/dev/null
  else
    time "$@" >/dev/null
  fi
}

run_memory() {
  if /usr/bin/time -l "$@" >/dev/null 2>&1; then
    /usr/bin/time -l "$@" >/dev/null
  elif /usr/bin/time -p "$@" >/dev/null 2>&1; then
    /usr/bin/time -p "$@" >/dev/null
  else
    time "$@" >/dev/null
  fi
}

case "$mode" in
  startup)
    runs="${1:-5}"
    echo "Benchmark: startup ($runs runs)"
    for i in $(seq 1 "$runs"); do
      echo "run=$i"
      run_time "$bin" --json version
    done
    ;;
  memory)
    prompt="${1:-memory benchmark prompt}"
    echo "Benchmark: memory"
    run_memory "$bin" --json run "$prompt"
    ;;
  *)
    usage
    exit 1
    ;;
esac
