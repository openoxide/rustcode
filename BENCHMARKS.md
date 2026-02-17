# Rustcode Benchmarks

## Harness
- Script: `scripts/benchmark.sh`
- Recorder: `scripts/benchmark_record.sh`
- Comparator: `scripts/benchmark_compare.sh`
- Assertions: `scripts/benchmark_assert.sh`
- Snapshot/Rotation: `scripts/benchmark_snapshot.sh`
- Modes:
  - `startup [runs]`
  - `memory [prompt]`
  - `drift [seconds] [interval]`
  - `serve [seconds] [interval]`
  - `load [iterations] [parallel]`

Examples:
```bash
./scripts/benchmark.sh startup 5
./scripts/benchmark.sh memory "benchmark memory"
./scripts/benchmark.sh drift 20 5
./scripts/benchmark.sh serve 20 5
./scripts/benchmark.sh load 10 4
./scripts/benchmark_record.sh benchmarks/latest.json
./scripts/benchmark_compare.sh benchmarks/old.json benchmarks/latest.json
./scripts/benchmark_assert.sh benchmarks/latest.json
./scripts/benchmark_snapshot.sh benchmarks/latest.json benchmarks/history
```

## Notes
- The harness builds `rustcode-cli` before running probes.
- On restricted environments, `/usr/bin/time -l` may be unavailable or blocked; the script falls back to portable timing output.
- For full RSS metrics on macOS, run outside sandbox restrictions with `/usr/bin/time -l` enabled.

## Latest Sample (2026-02-17)
- `./scripts/benchmark.sh startup 5`:
  - `real 0.00` reported across runs on host timer granularity.
- `./scripts/benchmark.sh memory "bench memory probe"`:
  - `maximum resident set size: 5,488,640`
  - `peak memory footprint: 2,113,848`
- `./scripts/benchmark.sh drift 12 4`:
  - observed RSS samples: `32 KB -> 5,392 KB`
  - process remained stable and exited cleanly on cancellation.
- `./scripts/benchmark.sh serve 8 4`:
  - observed RSS samples: `32 KB -> 5,424 KB`
  - service loop remained stable and exited cleanly on cancellation.
- `./scripts/benchmark.sh load 3 2`:
  - completed concurrent batch probes successfully (`3` batches x `2` workers).
- `./scripts/benchmark_record.sh benchmarks/latest.json`:
  - persists startup/memory/drift/serve/load output in JSON artifact format.
- `./scripts/benchmark_compare.sh <old> <new>`:
  - reports deltas for `maximum resident set size` and `peak memory footprint`.
- `./scripts/benchmark_assert.sh <artifact>`:
  - fails when extracted memory metrics exceed conservative limits.
  - configurable via `MAX_RSS_LIMIT` and `PEAK_FOOTPRINT_LIMIT`.
  - falls back to sampled RSS from drift/serve sections when direct memory fields are absent.
  - optional escape hatch: `ALLOW_MISSING_METRICS=1`.
- `./scripts/ci_matrix.sh`:
  - runs `check`, `test`, `benchmark_record`, and `benchmark_assert`.
  - enables `ALLOW_MISSING_METRICS=1` automatically on Darwin sandbox environments.
- `./scripts/benchmark_snapshot.sh <latest> <history_dir>`:
  - writes UTC-timestamped benchmark snapshots.
  - prunes oldest snapshots beyond `KEEP_BENCHMARK_SNAPSHOTS` (default `20`).

## History Policy
- `benchmarks/latest.json` is the current baseline artifact.
- `benchmarks/history/*.json` stores local UTC-stamped snapshots (git-ignored; `.gitkeep` tracked).
- Keep rolling history bounded (`20` by default) to avoid artifact sprawl.
