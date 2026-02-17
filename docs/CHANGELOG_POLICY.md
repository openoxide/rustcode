# Changelog Policy

## Scope
Every user-visible behavior change must be captured in changelog entries before release.

## Entry Format
- `type`: `feat`, `fix`, `perf`, `docs`, `refactor`, `breaking`
- `area`: crate or subsystem (`cli`, `engine`, `config`, `tui`, `plugins`, etc.)
- `summary`: one-sentence externally visible change
- `validation`: command(s) used to verify behavior

## Release Rules
- `breaking` entries require migration notes.
- Performance claims require benchmark evidence reference.
- Security-sensitive changes require explicit risk note.
- No release tag without passing `cargo test --workspace`.

## Source of Truth
- Keep incremental updates in `PLAN.md`.
- Aggregate release notes from commit history + validated plan entries.
