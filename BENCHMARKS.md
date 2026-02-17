# Rustcode Benchmarks

## Harness
- Script: `scripts/benchmark.sh`
- Recorder: `scripts/benchmark_record.sh`
- Comparator: `scripts/benchmark_compare.sh`
- Assertions: `scripts/benchmark_assert.sh`
- Release Gate: `scripts/benchmark_release_gate.sh`
- Metrics Parser: `scripts/benchmark_metrics.sh`
- Serve Telemetry Assert: `scripts/assert_serve_telemetry.sh`
- Baseline Refresh: `scripts/refresh_release_baseline.sh`
- Snapshot/Rotation: `scripts/benchmark_snapshot.sh`
- Modes:
  - `startup [runs]`
  - `memory [prompt]`
  - `drift [seconds] [interval]`
  - `serve [seconds] [interval]`
  - `load [iterations] [parallel] [warmup]`

Examples:
```bash
./scripts/benchmark.sh startup 5
./scripts/benchmark.sh memory "benchmark memory"
./scripts/benchmark.sh drift 20 5
./scripts/benchmark.sh serve 20 5
./scripts/benchmark.sh load 10 4 1
./scripts/benchmark_record.sh benchmarks/latest.json
./scripts/benchmark_compare.sh benchmarks/old.json benchmarks/latest.json
./scripts/benchmark_assert.sh benchmarks/latest.json
./scripts/assert_serve_telemetry.sh benchmarks/latest.json
./scripts/benchmark_release_gate.sh benchmarks/release-baseline.json benchmarks/latest.json
./scripts/refresh_release_baseline.sh
./scripts/benchmark_snapshot.sh benchmarks/latest.json benchmarks/history
```

## Notes
- The harness builds `rustcode-cli` before running probes.
- On restricted environments, `/usr/bin/time -l` may be unavailable or blocked; the script falls back to portable timing output.
- For full RSS metrics on macOS, run outside sandbox restrictions with `/usr/bin/time -l` enabled.
- Memory metrics in compare/assert/release-gate scripts are normalized to bytes (BSD + GNU time formats).

## Latest Sample (2026-02-17)
- `./scripts/benchmark.sh startup 5`:
  - `real 0.00` reported across runs on host timer granularity.
- `./scripts/benchmark.sh memory "bench memory probe"`:
  - `maximum resident set size: 6,029,312`
  - `peak memory footprint: 2,605,368`
- `./scripts/benchmark.sh drift 12 4`:
  - observed RSS samples: `32 KB -> 5,392 KB`
  - process remained stable and exited cleanly on cancellation.
- `./scripts/benchmark.sh serve 8 4`:
  - observed RSS samples: `32 KB -> 5,424 KB`
  - service loop remained stable and exited cleanly on cancellation.
- `./scripts/benchmark.sh load 20 2 2`:
  - completed concurrent batch probes successfully (`20` measured batches x `2` workers, `2` warmup batches).
  - emitted latency summary (`latency_p50_s`, `latency_p95_s`, `latency_max_s`).
  - latest sample: `p50=0.005s`, `p95=0.007s`, `max=0.007s`.
- `./scripts/benchmark_record.sh benchmarks/latest.json`:
  - persists startup/memory/drift/serve/load output in JSON artifact format.
- `./scripts/benchmark_compare.sh <old> <new>`:
  - reports deltas for `maximum resident set size` and `peak memory footprint`.
- `./scripts/benchmark_assert.sh <artifact>`:
  - fails when extracted memory metrics exceed conservative limits.
  - configurable via `MAX_RSS_LIMIT` and `PEAK_FOOTPRINT_LIMIT`.
  - falls back to sampled RSS from drift/serve sections when direct memory fields are absent.
  - optional escape hatch: `ALLOW_MISSING_METRICS=1`.
- `./scripts/benchmark_release_gate.sh <baseline> <candidate>`:
  - enforces bounded regression deltas for RSS, peak footprint, and load `latency_p95_s`.
  - defaults:
    - `MAX_RSS_DELTA_LIMIT=1048576`
    - `PEAK_DELTA_LIMIT=524288`
    - `P95_DELTA_LIMIT=0.100`
  - release fallback escape hatch: `ALLOW_MISSING_RELEASE_METRICS=1`.
- `./scripts/assert_serve_telemetry.sh <artifact>`:
  - validates serve section includes configured endpoint, `200`/`404` request telemetry, and clean cancellation.
  - optional escape hatch: `ALLOW_MISSING_SERVE_TELEMETRY=1`.
- `./scripts/ci_matrix.sh`:
  - runs `check`, `test`, `serve_smoke`, `benchmark_record`, `assert_serve_telemetry`, and `benchmark_assert`.
  - enables `ALLOW_MISSING_METRICS=1` automatically on Darwin sandbox environments.
- GitHub Actions CI uploads `benchmarks/latest.json` as artifact `rustcode-benchmarks-latest`.
- GitHub Actions release branches (`release/*`) execute:
  - `./scripts/benchmark_release_gate.sh benchmarks/release-baseline.json benchmarks/latest.json`
  - release gate uses a tracked baseline artifact: `benchmarks/release-baseline.json`.
- GitHub Actions `ci` also supports manual `workflow_dispatch` release-gate runs:
  - `run_release_gate=true`
  - optional `baseline_path` override
- `./scripts/refresh_release_baseline.sh [--from-latest]`:
  - regenerates/validates benchmark artifact and promotes it to `benchmarks/release-baseline.json`.
- `./scripts/benchmark_snapshot.sh <latest> <history_dir>`:
  - writes UTC-timestamped benchmark snapshots.
  - prunes oldest snapshots beyond `KEEP_BENCHMARK_SNAPSHOTS` (default `20`).

## History Policy
- `benchmarks/latest.json` is the current baseline artifact.
- `benchmarks/history/*.json` stores local UTC-stamped snapshots (git-ignored; `.gitkeep` tracked).
- Keep rolling history bounded (`20` by default) to avoid artifact sprawl.
