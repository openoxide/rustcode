# Release Packaging Checklist

## Versioning
1. Confirm target version in workspace metadata and release notes draft.
2. Ensure changelog entries map to release scope (`feat/fix/perf/breaking`).
3. Rebuild benchmark artifact and assertions before tagging.
4. Release PRs should use `.github/PULL_REQUEST_TEMPLATE/release.md`.

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
- Bundle automation:
  - script: `./scripts/build_release_bundle.sh <version> [output_dir]`
  - workflow: `.github/workflows/release-candidate-bundle.yml` (`workflow_dispatch`)

## Tagging and Notes
1. Finalize `docs/RELEASE_NOTES_DRAFT.md` into release notes.
2. Create annotated tag: `v<version>`.
3. Include benchmark artifact reference (`benchmarks/latest.json`) in release metadata.
4. Preferred automation:
   - dry-run: `./scripts/release_with_gh.sh <version> --dry-run`
   - execute: `./scripts/release_with_gh.sh <version>`
   - auto-draft notes from commit history: `./scripts/release_with_gh.sh <version> --auto-notes`
   - signed tags are required by default; use `--allow-unsigned-tag` only when signing is unavailable.
5. Release PR checklist gate:
   - workflow: `.github/workflows/release-pr-checklist.yml`
   - validator: `./scripts/check_release_pr_checklist.sh`
6. Release PR benchmark drift comment:
   - workflow: `.github/workflows/release-pr-benchmark-report.yml`
   - posts baseline-vs-candidate compare output on release PRs.
7. Nightly drift monitor:
   - workflow: `.github/workflows/nightly-benchmark-drift.yml`
   - confirm no open "Nightly benchmark drift alert" issue before tag cut.
8. Release notes drafting helper:
   - script: `./scripts/draft_release_notes.sh <version> [from_ref] [to_ref] [output_file]`

## Post-Release
1. Snapshot benchmark artifact:
   - `./scripts/benchmark_snapshot.sh benchmarks/latest.json benchmarks/history`
2. Rotate old snapshots via retention policy.
3. Refresh release baseline (for next release branch comparisons):
   - `./scripts/refresh_release_baseline.sh --from-latest`
4. Advance `PLAN.md` next-action queue for following cycle.
