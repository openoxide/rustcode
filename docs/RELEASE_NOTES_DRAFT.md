# Release Notes Draft (v0.1.0)

Date: 2026-02-17
Status: Draft

## Summary
Initial production-oriented Rustcode baseline with typed command routing, trust-gated config resolution, event-driven execution, structured cancellation, plugin boundaries, TUI event-loop contracts, and reproducible benchmark tooling.

## Highlights
- `cli`: strict clap command surface with explicit diagnostics.
- `config`: layered resolution (`global -> user -> project(trusted) -> env -> CLI`) with trust gate.
- `engine`: message-driven execution with structured `Event` emission and explicit cancellation handling.
- `tools`: `list`, `read`, `write`, `edit`, `exec` with workspace-boundary checks and pluggable path policy.
- `events`: versioned envelope (`schema_version`) and backward-compatible deserialization for legacy payloads.
- `plugins`: registry lifecycle (`register`, duplicate guard, `unregister`) and event hook integration tests.
- `tui`: event-consumer adapter model (`UiInput`: `Domain`, `Resize`, `Shutdown`) and testable `UiSummary`.
- `bench`: benchmark harness + JSON recorder (`benchmarks/latest.json`) covering startup/memory/drift/serve.

## Validation Snapshot
- `cargo test --workspace` passed.
- CLI integration tests passed:
  - JSON stream contract
  - SIGINT cancellation behavior
  - TUI route behavior
- Runtime probes passed:
  - trust gate failure/success scenarios
  - workspace escape rejection
  - serve cancellation lifecycle

## Known Limitations
- TUI currently validates event-loop contracts but not full interactive rendering behavior.
- Benchmark metrics depend on host timer granularity and environment privileges.

## Upgrade/Migration Notes
- Event consumers should rely on `schema_version` and tolerate unknown fields.
- Project-local config now requires trust (`--trust-project-config` or trusted project list).
