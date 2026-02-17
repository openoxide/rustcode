# Rustcode Benchmarks

## Harness
- Script: `scripts/benchmark.sh`
- Modes:
  - `startup [runs]`
  - `memory [prompt]`
  - `drift [seconds] [interval]`

Examples:
```bash
./scripts/benchmark.sh startup 5
./scripts/benchmark.sh memory "benchmark memory"
./scripts/benchmark.sh drift 20 5
```

## Notes
- The harness builds `rustcode-cli` before running probes.
- On restricted environments, `/usr/bin/time -l` may be unavailable or blocked; the script falls back to portable timing output.
- For full RSS metrics on macOS, run outside sandbox restrictions with `/usr/bin/time -l` enabled.

## Latest Sample (2026-02-17)
- `./scripts/benchmark.sh startup 5`:
  - `real 0.00` reported across runs on host timer granularity.
- `./scripts/benchmark.sh memory "bench memory probe"`:
  - `maximum resident set size: 5,242,880`
  - `peak memory footprint: 2,064,696`
- `./scripts/benchmark.sh drift 12 4`:
  - observed RSS samples: `32 KB -> 5,424 KB -> 5,424 KB`
  - process remained stable and exited cleanly on cancellation.
