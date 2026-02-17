## Release Summary
- Release version:
- Scope:
- Risks:

## Release Checklist
- [ ] `./scripts/ci_matrix.sh` passed on this branch.
- [ ] `./scripts/benchmark_release_gate.sh benchmarks/release-baseline.json benchmarks/latest.json` passed.
- [ ] `./scripts/release_with_gh.sh <version> --dry-run` reviewed.
- [ ] Release notes are ready (`docs/RELEASE_NOTES_DRAFT.md` updated or finalized).
- [ ] Baseline handling decided (`./scripts/refresh_release_baseline.sh --from-latest` if needed post-release).
