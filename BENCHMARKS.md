# Rustcode Benchmarks

## Harness
- Script: `scripts/benchmark.sh`
- Modes:
  - `startup [runs]`
  - `memory [prompt]`

Examples:
```bash
./scripts/benchmark.sh startup 5
./scripts/benchmark.sh memory "benchmark memory"
```

## Notes
- The harness builds `rustcode-cli` before running probes.
- On restricted environments, `/usr/bin/time -l` may be unavailable or blocked; the script falls back to portable timing output.
- For full RSS metrics on macOS, run outside sandbox restrictions with `/usr/bin/time -l` enabled.
