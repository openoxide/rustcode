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
- Status: Analysis complete, implementation not started

## Milestones

1. Workspace Bootstrap
- Status: pending
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

2. Command Surface and Routing
- Status: pending
- Deliverables:
  - Strict clap parser
  - Command router with typed command enum
  - No implicit positional ambiguity
- Validation:
  - `cargo test -p rustcode-cli`
  - invalid-command snapshots

3. Layered Config System
- Status: pending
- Deliverables:
  - Deterministic config precedence
  - Explicit trust model for project-local config
  - Schema validation + actionable errors
- Validation:
  - malformed config fixtures
  - precedence tests

4. Event Protocol and Renderers
- Status: pending
- Deliverables:
  - Domain event types (turn/item/tool/error)
  - Human renderer
  - JSONL renderer
- Validation:
  - snapshot tests for both output modes
  - `jq`-valid JSONL stream checks

5. Engine Skeleton
- Status: pending
- Deliverables:
  - Async execution loop
  - Cancellation propagation
  - Structured error domains
- Validation:
  - interrupt integration tests
  - no-panic policy in expected failure paths

6. Tool Runtime v1
- Status: pending
- Deliverables:
  - Core tools (`list`, `read`, `write`, `edit`, `exec`)
  - Permission policy hooks
  - File boundary checks
- Validation:
  - unit + integration tests
  - invalid input and boundary tests

7. Plugin Boundary v1
- Status: pending
- Deliverables:
  - Trait-based plugin API
  - Registration lifecycle
  - Event subscription hooks
- Validation:
  - fixture plugin crate tests

8. TUI Integration
- Status: pending
- Deliverables:
  - Event-consumer-only TUI
  - Dedicated UI loop
  - Ctrl+C and resize behavior
- Validation:
  - TUI smoke tests
  - manual interaction checklist

9. Hardening and Benchmarks
- Status: pending
- Deliverables:
  - startup benchmark
  - memory benchmark
  - concurrency/load tests
- Validation targets:
  - warm startup under `50ms` for trivial non-interactive command
  - stable memory in long-run idle sample

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
1. Create workspace skeleton and crate manifests.
2. Implement CLI parse + command enum + invalid command snapshots.
3. Implement config loader skeleton with deterministic layer ordering.

## Update Log
- 2026-02-17:
  - Created initial execution plan from measured comparative audit.
  - Marked all build milestones pending until implementation kickoff.
