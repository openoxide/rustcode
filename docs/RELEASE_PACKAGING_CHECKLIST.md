# Release Packaging Checklist

## Versioning
1. Confirm target version in workspace metadata and release notes draft.
2. Ensure changelog entries map to release scope (`feat/fix/perf/breaking`).
3. Rebuild benchmark artifact and assertions before tagging.

## Validation Gate
1. `cargo check --workspace`
2. `cargo test --workspace`
3. `./scripts/benchmark_record.sh benchmarks/latest.json`
4. `./scripts/benchmark_assert.sh benchmarks/latest.json`
5. `./scripts/benchmark_release_gate.sh benchmarks/release-baseline.json benchmarks/latest.json`

## Artifact Naming
- Binary artifact base: `rustcode-cli`
- Suggested release bundle naming:
  - `rustcode-v<version>-<target>.tar.gz`
  - `rustcode-v<version>-<target>.sha256`

## Tagging and Notes
1. Finalize `docs/RELEASE_NOTES_DRAFT.md` into release notes.
2. Create annotated tag: `v<version>`.
3. Include benchmark artifact reference (`benchmarks/latest.json`) in release metadata.

## Post-Release
1. Snapshot benchmark artifact:
   - `./scripts/benchmark_snapshot.sh benchmarks/latest.json benchmarks/history`
2. Rotate old snapshots via retention policy.
3. Advance `PLAN.md` next-action queue for following cycle.
