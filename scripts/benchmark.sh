#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<USAGE
Usage:
  scripts/benchmark.sh startup [runs]
  scripts/benchmark.sh memory [prompt]
  scripts/benchmark.sh drift [seconds] [interval]
  scripts/benchmark.sh serve [seconds] [interval]
  scripts/benchmark.sh load [iterations] [parallel]
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
  elif /usr/bin/time -v "$@" >/dev/null 2>&1; then
    /usr/bin/time -v "$@" >/dev/null
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
  drift)
    seconds="${1:-20}"
    interval="${2:-5}"
    echo "Benchmark: drift (seconds=$seconds interval=$interval)"

    output_file="$(mktemp /tmp/rustcode-drift.XXXXXX)"
    "$bin" exec sleep 120 >"$output_file" 2>&1 &
    pid="$!"
    echo "pid=$pid"

    elapsed=0
    while [[ "$elapsed" -lt "$seconds" ]]; do
      ps -o pid=,rss=,%cpu=,etime= -p "$pid" || true
      sleep "$interval"
      elapsed=$((elapsed + interval))
    done

    kill -INT "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    echo "---tail---"
    tail -n 20 "$output_file"
    ;;
  serve)
    seconds="${1:-20}"
    interval="${2:-5}"
    echo "Benchmark: serve (seconds=$seconds interval=$interval)"

    output_file="$(mktemp /tmp/rustcode-serve.XXXXXX)"
    "$bin" serve --listen 127.0.0.1:4317 >"$output_file" 2>&1 &
    pid="$!"
    echo "pid=$pid"

    elapsed=0
    while [[ "$elapsed" -lt "$seconds" ]]; do
      ps -o pid=,rss=,%cpu=,etime= -p "$pid" || true
      sleep "$interval"
      elapsed=$((elapsed + interval))
    done

    kill -INT "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    echo "---tail---"
    tail -n 20 "$output_file"
    ;;
  load)
    iterations="${1:-10}"
    parallel="${2:-4}"
    echo "Benchmark: load (iterations=$iterations parallel=$parallel)"

    run_batch() {
      local batch="$1"
      local pids=()
      local i
      for i in $(seq 1 "$parallel"); do
        "$bin" --json run "load probe batch=$batch worker=$i" >/dev/null 2>&1 &
        pids+=("$!")
      done

      for pid in "${pids[@]}"; do
        wait "$pid"
      done
    }

    for batch in $(seq 1 "$iterations"); do
      echo "batch=$batch"
      run_batch "$batch"
    done
    ;;
  *)
    usage
    exit 1
    ;;
esac
