# TUI Smoke Checklist

Date: 2026-02-17
Scope: `rustcode tui`

## Preconditions
- Build complete: `cargo build -p rustcode`
- Run from a workspace directory with no blocking config errors.

## Checklist
1. Launch path
- Command: `./target/debug/rustcode tui`
- Expected: process exits cleanly after consuming emitted events.

2. Ctrl+C cancellation
- Command: `./target/debug/rustcode tui`
- Action: press `Ctrl+C` during startup.
- Expected: clean exit; no panic; no partial UTF-8 escape output.

3. Resize behavior
- Command: `./target/debug/rustcode tui`
- Action: resize terminal rapidly.
- Expected: no panic, no malformed output, stable process state.

4. JSON mode coexistence
- Command: `./target/debug/rustcode --json tui`
- Expected: TUI path takes precedence and does not emit CLI JSON renderer output.

5. Repeated launch stability
- Run `tui` command 20+ times.
- Expected: consistent startup/exit behavior, no accumulated stderr warnings.

## Current Status
- Automated coverage exists for routing to TUI path and clean exit.
- Ctrl+C and resize require richer interactive TUI loop semantics before full automation.
