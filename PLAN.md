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
- Status: Implementation active (milestone-driven with test+commit loop)

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
- Status: in_progress
- Deliverables:
  - Event-consumer-only TUI
  - Dedicated UI loop
  - Ctrl+C and resize behavior
- Validation:
  - TUI smoke tests
  - manual interaction checklist
  - Pass (partial): event-consumer TUI loop wired in CLI path (`rustcode-cli tui`) with integration + unit tests (2026-02-17)

9. Hardening and Benchmarks
- Status: in_progress
- Deliverables:
  - startup benchmark
  - memory benchmark
  - concurrency/load tests
- Validation targets:
  - warm startup under `50ms` for trivial non-interactive command
  - stable memory in long-run idle sample
  - Pass (partial): benchmark harness script added (`scripts/benchmark.sh`) with startup and memory modes (2026-02-17)
  - Note: sandbox restricts detailed `/usr/bin/time -l` metrics; fallback timing output used in restricted runs.

10. Documentation and Release Prep
- Status: pending
- Deliverables:
  - architecture doc
  - operator/developer docs
  - changelog policy
- Validation:
  - docs build + command examples verified

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
1. Complete TUI signal/resize handling contracts and smoke checklist.
2. Run benchmark harness outside sandbox-restricted timing mode to capture RSS baselines.
3. Add structured UI event abstractions (resize/shutdown) for automatable TUI tests.

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
