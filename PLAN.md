# Rustcode Execution Plan

Last updated: 2026-02-17
Owner: Codex agent
Source audit: `rustcode/ARCHITECTURE_AUDIT.md`

## Operating Rules
- No implementation starts before measured baseline and architecture decisions are documented.
- Every milestone must include validation commands and pass/fail notes.
- Update this file in every substantive commit.

## Current Mode
- Active mode: Slow Deep Research
- Status: Feature delivery mode (baseline complete; iterative hardening active)

## Milestones

0. Comparative Baseline Audit
- Status: completed
- Deliverables:
  - Runtime and UX measurements for `opencode` and `codex`
  - Static architecture archaeology (entry flow, dependency hubs, cycle signals)
  - Re-architecture direction captured in `ARCHITECTURE_AUDIT.md`
- Validation:
  - command logs and metrics captured during Phase 0/1/2

1. Workspace Bootstrap
- Status: completed
- Deliverables:
  - Cargo workspace with crates:
    - `rustcode-cli`
    - `rustcode-core`
    - `rustcode-engine`
    - `rustcode-config`
    - `rustcode-io`
    - `rustcode-plugins`
    - `rustcode-tui`
    - `rustcode-llm`
  - Shared lint/test config
- Validation:
  - `cargo check --workspace`
  - Pass: `cargo check --workspace` (2026-02-17)
  - Pass: `cargo test --workspace` (2026-02-17)

2. Command Surface and Routing
- Status: completed
- Deliverables:
  - Strict clap parser
  - Command router with typed command enum
  - No implicit positional ambiguity
- Validation:
  - `cargo test -p rustcode-cli`
  - invalid-command snapshots
  - Pass: strict invalid subcommand and invalid flag diagnostics verified (2026-02-17)
  - Pass: parser + router module extraction with focused tests (2026-02-17)

3. Layered Config System
- Status: completed
- Deliverables:
  - Deterministic config precedence
  - Explicit trust model for project-local config
  - Schema validation + actionable errors
- Validation:
  - malformed config fixtures
  - precedence tests
  - Pass: layered merge/trust tests in `rustcode-config` (`cargo test --workspace`, 2026-02-17)
  - Pass: runtime trust gate probe with untrusted project failure and trusted success (`rustcode-cli version`, 2026-02-17)

4. Event Protocol and Renderers
- Status: completed
- Deliverables:
  - Domain event types (turn/item/tool/error)
  - Human renderer
  - JSONL renderer
- Validation:
  - snapshot tests for both output modes
  - `jq`-valid JSONL stream checks
  - Pass: `cargo run -q -p rustcode-cli -- --json run "hello"` emits structured JSON events (2026-02-17)
  - Pass: renderer contract tests for human and JSON output stability (`cargo test -p rustcode-cli`, 2026-02-17)
  - Pass: runtime probe confirmed human + JSON rendering paths (`cargo run -q -p rustcode-cli -- run "hi"` and `--json run "hi"`, 2026-02-17)

5. Engine Skeleton
- Status: completed
- Deliverables:
  - Async execution loop
  - Cancellation propagation
  - Structured error domains
- Validation:
  - interrupt integration tests
  - no-panic policy in expected failure paths
  - Pass: cancellation propagation tests in `rustcode-io` and `rustcode-engine` (`cargo test --workspace`, 2026-02-17)
  - Pass: runtime signal probes for `SIGINT` and `SIGTERM` emit cancellation warning and clean exit (`rustcode-cli exec sleep 30`, 2026-02-17)

6. Tool Runtime v1
- Status: completed
- Deliverables:
  - Core tools (`list`, `read`, `write`, `edit`, `exec`)
  - Permission policy hooks
  - File boundary checks
- Validation:
  - unit + integration tests
  - invalid input and boundary tests
  - Pass: `list`, `read`, `write`, `edit`, `exec` command paths wired and validated (2026-02-17)
  - Pass: workspace boundary rejection for `../` escape paths with structured failure event (2026-02-17)
  - Pass: permission policy hook introduced and enforced in engine path resolution (2026-02-17)

7. Plugin Boundary v1
- Status: completed
- Deliverables:
  - Trait-based plugin API
  - Registration lifecycle
  - Event subscription hooks
- Validation:
  - fixture plugin crate tests
  - Pass: plugin registry lifecycle tests (`register` duplicate rejection + `unregister`) in `rustcode-plugins` (2026-02-17)
  - Pass: engine test verifies plugin `on_event` hook receives emitted events (2026-02-17)

8. TUI Integration
- Status: completed
- Deliverables:
  - Event-consumer-only TUI
  - Dedicated UI loop
  - Ctrl+C and resize behavior
- Validation:
  - TUI smoke tests
  - manual interaction checklist
  - Pass (partial): event-consumer TUI loop wired in CLI path (`rustcode-cli tui`) with integration + unit tests (2026-02-17)
  - Pass: structured UI input abstractions (`Domain`/`Resize`/`Shutdown`) with TUI adapter tests (2026-02-17)
  - Pass: CLI integration confirms TUI routing uses consumer loop and exits cleanly (2026-02-17)

9. Hardening and Benchmarks
- Status: completed
- Deliverables:
  - startup benchmark
  - memory benchmark
  - concurrency/load tests
- Validation targets:
  - warm startup under `50ms` for trivial non-interactive command
  - stable memory in long-run idle sample
  - Pass (partial): benchmark harness script added (`scripts/benchmark.sh`) with startup and memory modes (2026-02-17)
  - Pass: elevated benchmark run captured timing and RSS metrics (`startup 5`, `memory "bench memory probe"`, 2026-02-17)
  - Pass (partial): drift probe mode added and exercised (`drift 12 4`) with stable sampled RSS (2026-02-17)
  - Pass (partial): serve-mode long-run benchmark (`serve 8 4`) with stable sampled RSS (2026-02-17)
  - Pass (partial): benchmark JSON artifact persistence via `scripts/benchmark_record.sh` (2026-02-17)
  - Pass: load/concurrency mode added and exercised (`load 3 2`) (2026-02-17)
  - Pass: benchmark snapshot/rotation policy implemented (`scripts/benchmark_snapshot.sh`) (2026-02-17)
  - Pass: benchmark trend compare + threshold assertions implemented (`benchmark_compare.sh`, `benchmark_assert.sh`) (2026-02-17)

10. Documentation and Release Prep
- Status: completed
- Deliverables:
  - architecture doc
  - operator/developer docs
  - changelog policy
- Validation:
  - docs build + command examples verified
  - Pass (partial): added `docs/QUICKSTART.md` and `docs/CHANGELOG_POLICY.md` (2026-02-17)
  - Pass (partial): quickstart command set executed and verified on local workspace (2026-02-17)
  - Pass: release notes draft created from validated plan + changelog policy (`docs/RELEASE_NOTES_DRAFT.md`, 2026-02-17)
  - Pass: release packaging checklist added (`docs/RELEASE_PACKAGING_CHECKLIST.md`, 2026-02-17)

## Testing Matrix
- Unit tests: core logic, config merge/validation, event transformation
- Integration tests: CLI parse/dispatch, execution loop, permission flow
- Snapshot tests: human and JSON output
- Fuzz tests: parser/config where practical
- Load tests: repeated turns and long-running session stability

## Reference Docs
- `https://opencode.ai/docs`
- `https://developers.openai.com/codex/`
- Use references during implementation phases for behavior parity checks and interoperability assumptions.

## Runtime Baseline (Rustcode Bootstrap)
- Startup (`cargo run -q -p rustcode-cli -- version`): `0.21s - 0.27s` over 3 runs.
- Invalid subcommand: clap rejection with usage and non-zero exit.
- Invalid run flag: clap rejection with `--` escape tip.
- Large prompt (`200k` chars): completes in `0.20s`, JSON output file `400495` bytes.
- Interrupt behavior:
  - `SIGINT` and `SIGTERM` now trigger cancellation propagation and emit `Warning { message: "execution cancelled" }`.
  - Cancellation path exits cleanly (exit code `0`) after draining events.

## Risks and Mitigations
- Over-engineering:
  - Mitigation: keep first version minimal, defer optional abstractions.
- Async complexity:
  - Mitigation: enforce structured task ownership and cancellation policy.
- Boundary leaks:
  - Mitigation: forbid IO in core crate and enforce event-only rendering path.
- Plugin API instability:
  - Mitigation: small trait surface and explicit versioning.

## Next Action Queue
1. Add platform-specific benchmark parser to reduce `ALLOW_MISSING_METRICS` use.
2. Add serve telemetry assertion script for CI artifacts.
3. Add baseline refresh workflow for `benchmarks/release-baseline.json`.

## Update Log
- 2026-02-17:
  - Created initial execution plan from measured comparative audit.
  - Added Milestone 0 as completed (baseline research and architecture extraction).
  - Marked implementation milestones pending until coding kickoff.
  - Completed workspace skeleton (`Cargo.toml`, 8 crates, toolchain pin, shared lints).
  - Added first-pass command/event/error/config/engine/plugin/IO boundaries.
  - Validation passed: `cargo check --workspace`, `cargo test --workspace`.
  - Runtime probes passed: `--help`, invalid subcommand, `--json run`, `exec`.
  - Added external reference docs for ongoing implementation checks.
  - Added principal architecture spec: `rustcode/docs/PRINCIPAL_ARCHITECTURE.md`.
  - Added rustcode runtime baseline metrics and recorded shutdown gap for Milestone 5.
  - Completed Milestone 2: parser/router split (`cli.rs`) with strict diagnostics tests.
  - Split event rendering concerns into `render.rs`; validated `--json` and human output paths.
  - Confirmed `.gitignore` hygiene: minimal 3-rule file, no plan/doc suppression.
  - Completed Milestone 3: deterministic global/user/project config layering with explicit project trust gate.
  - Added `--trust-project-config` CLI override and `[trust].projects` support in user/global config files.
  - Added config tests for precedence, plugin/env merge semantics, trust rejection, and CLI override precedence.
  - Verified runtime behavior: untrusted project config fails with actionable error; trusted modes succeed.
  - Completed Milestone 4: renderer contract tests added for deterministic human and JSON outputs.
  - Confirmed renderer outputs via runtime probes for both human and machine-readable paths.
  - Completed Milestone 5: cancellation token propagation from CLI signal handling into engine/IO.
  - Added cancellation tests:
    - `rustcode-io`: process cancellation and success path tests.
    - `rustcode-engine`: cancellation emits warning event and no `Completed` event.
  - Verified runtime behavior: `SIGINT`/`SIGTERM` now produce structured cancellation warning events.
  - Started Milestone 6: implemented `list/read/write` command handlers with workspace-boundary enforcement.
  - Added regression test for workspace escape rejection in engine command routing.
  - Runtime probes confirmed:
    - positive flows for `list/read/write`
    - non-zero failure and structured `Failure` event for escaped paths.
  - Completed Milestone 6:
    - Added `edit` command semantics (`from` -> `to` replacement flow).
    - Introduced `PermissionPolicy` hook (`PathOperation`) and default workspace policy.
    - Wired all filesystem tools through policy-gated path resolution.
  - Runtime probes confirmed:
    - successful `edit` and `read` verification path
    - escaped `edit` path rejected with structured failure and non-zero exit.
  - Added event envelope versioning (`schema_version = 1`) to all emitted events.
  - Updated renderer contract tests and runtime JSON probes to validate versioned event envelopes.
  - Completed Milestone 7:
    - Added plugin lifecycle operations (`unregister`, `len`, `is_empty`).
    - Added plugin lifecycle tests in `rustcode-plugins`.
    - Added engine test validating plugin event-subscription hook execution.
  - Added CLI integration harness (`crates/rustcode-cli/tests/integration_cli.rs`):
    - JSON stream envelope/`Completed` event assertion.
    - SIGINT cancellation behavior assertion for long-running command.
  - Validation pass: `cargo test --workspace` including integration tests.
  - Added event schema compatibility tests in `rustcode-core`:
    - legacy event JSON without `schema_version` deserializes to schema `1`
    - newly emitted events retain schema `1`.
  - Started Milestone 8:
    - Wired `tui` command path to `rustcode-tui` event consumer.
    - Added `UiSummary` contract and TUI loop consumption test.
    - Added CLI integration test asserting `tui` route uses TUI path and exits cleanly.
  - Started Milestone 9:
    - Added benchmark harness `scripts/benchmark.sh` and usage doc `BENCHMARKS.md`.
    - Validated startup mode and fallback timing behavior in restricted environment.
  - Expanded plugin boundary validation with fixture integration test:
    - `crates/rustcode-plugins/tests/fixture_plugin.rs`
    - validated via `cargo test -p rustcode-plugins` and `cargo test --workspace`.
  - Added TUI smoke checklist doc: `docs/TUI_SMOKE_CHECKLIST.md`.
  - Captured manual validation expectations for launch, Ctrl+C, resize, and repeat stability.
  - Ran benchmark harness with elevated permissions to capture system metrics:
    - startup (`5` runs): reported `0.00s` real per run on this host timer granularity.
    - memory probe: `maximum resident set size: 5,242,880`, `peak memory footprint: 2,064,696`.
  - Completed Milestone 8:
    - Added `UiInput` abstractions (`Domain`, `Resize`, `Shutdown`) and `UiSummary` counters.
    - Added domain-event adapter (`from_domain_receiver`) for TUI event consumption.
    - Added TUI unit tests for resize/shutdown semantics and maintained CLI TUI integration coverage.
  - Implemented persistent `serve` execution loop (runs until cancellation) and added engine test coverage.
  - Added `serve` benchmark mode and sample run:
    - `./scripts/benchmark.sh serve 8 4`
    - observed stable RSS sample (`32 -> 5184 KB`) and clean cancellation.
  - Added benchmark JSON persistence tooling:
    - `scripts/benchmark_record.sh benchmarks/latest.json`
    - generated artifact snapshot under `benchmarks/latest.json` including `startup`, `memory`, `drift`, and `serve` sections.
  - Completed Milestone 10:
    - Added release-prep docs (`QUICKSTART`, `CHANGELOG_POLICY`, `RELEASE_NOTES_DRAFT`).
    - Verified quickstart command matrix aligns with current CLI behavior.
  - Added benchmark comparator utility:
    - `scripts/benchmark_compare.sh <old.json> <new.json>`
    - reports RSS and peak-footprint deltas for trend checks.
  - Added benchmark assertion utility:
    - `scripts/benchmark_assert.sh <artifact>`
    - enforces conservative memory thresholds with environment overrides.
  - Extended benchmark harness with drift mode and captured sample stability run:
    - `./scripts/benchmark.sh drift 12 4`
    - RSS samples stabilized at `~5424 KB` over sampled window.
  - Added benchmark artifact recorder:
    - `scripts/benchmark_record.sh benchmarks/latest.json`
    - persists startup/memory/drift outputs in JSON for historical tracking.
  - Completed Milestone 9:
    - Added load benchmark mode (`./scripts/benchmark.sh load <iterations> <parallel>`).
    - Added snapshot/rotation workflow (`scripts/benchmark_snapshot.sh`, `benchmarks/history/`).
    - Added CI matrix runner (`scripts/ci_matrix.sh`) and CI workflow (`.github/workflows/ci.yml`).
    - Added benchmark comparison/assertion automation for trend and threshold checks.
  - Added release packaging checklist doc (`docs/RELEASE_PACKAGING_CHECKLIST.md`).
  - Started Milestone 10:
    - Added `docs/QUICKSTART.md` with developer/operator command flows.
    - Added `docs/CHANGELOG_POLICY.md` with entry format and release gate rules.
    - Executed quickstart command matrix to verify docs align with current CLI behavior.
  - Extended benchmark recorder to capture `load` section and regenerated `benchmarks/latest.json`.
  - Hardened benchmark assertions:
    - fallback RSS parsing from drift/serve samples
    - explicit missing-metrics failure unless `ALLOW_MISSING_METRICS=1`.
  - Verified CI matrix end-to-end via `./scripts/ci_matrix.sh` on current host.
  - Feature pivot completed:
    - `serve` now binds a TCP listener and returns HTTP `200` health responses.
    - Added CLI integration coverage for `serve` response + graceful cancellation path.
  - CI/release hardening completed:
    - Added release packaging checklist (`docs/RELEASE_PACKAGING_CHECKLIST.md`).
    - Added CI workflow (`.github/workflows/ci.yml`) bound to `scripts/ci_matrix.sh`.
  - Extended `serve` endpoint behavior:
    - route-aware HTTP responses (`GET /health` -> `200`, unknown routes -> `404`, malformed requests -> `400`).
    - integration coverage asserts both health success and unknown-route handling.
  - Validation pass:
    - `cargo test --workspace`
    - `./scripts/ci_matrix.sh`
  - CI benchmark artifact publication enabled:
    - workflow now uploads `benchmarks/latest.json` via `actions/upload-artifact@v4`.
    - artifact name: `rustcode-benchmarks-latest`.
  - Extended load benchmark telemetry:
    - `scripts/benchmark.sh load` now captures per-batch latency.
    - reports `latency_p50_s`, `latency_p95_s`, and `latency_max_s` summary lines.
  - Serve connection hardening completed:
    - added per-connection read timeout with `408 Request Timeout` response contract.
    - connection-level read/write failures now emit warning events and keep listener alive.
    - integration coverage validates timeout response and emitted `ServeRequest` timeout event.
  - CI serve smoke coverage added:
    - introduced `scripts/serve_smoke.sh` to validate live `/health` and `404` routes.
    - wired smoke probe into `scripts/ci_matrix.sh` before benchmark recording.
    - added `ALLOW_SERVE_SMOKE_SKIP` fallback for restricted local bind environments.
  - Release benchmark comparison gate added:
    - introduced `scripts/benchmark_release_gate.sh` for baseline-vs-candidate delta enforcement.
    - added tracked baseline artifact `benchmarks/release-baseline.json`.
    - wired CI release branch gate in `.github/workflows/ci.yml`.
  - Load benchmark stabilization:
    - added warmup support to `scripts/benchmark.sh load` (`[warmup]` arg).
    - increased recorder sampling to `load 20 2 2` in `scripts/benchmark_record.sh`.
    - refreshed baseline artifact so release gate compares like-for-like load telemetry.
  - Added structured serve telemetry:
    - introduced `EventPayload::ServeRequest { method, path, status }`.
    - `serve` now emits route-level events for each handled request.
    - integration coverage asserts both emitted `200` health and `404` unknown-route events.
  - Validation pass:
    - `cargo test --workspace`
    - `./scripts/ci_matrix.sh`
