# Changelog

This changelog tracks user-visible verifier semantics, report-shape changes, and release-level benchmark or documentation updates. If a release can change findings or stage outcomes without any change in the target repo, it should be called out here.

## Unreleased

## 0.1.10 - 2026-04-22

### Report And CI Contract

- Added `schema_version: 2` to verify reports.
- Added `--report-level full|minimal` so CI can keep only pass/fail, findings, stage outcomes, and summary counts.
- Added `--execute-gate all|crash|none` so teams can fail only on crash findings when needed.
- Added `--suppressions-file <PATH>` with JSON suppression rules for known findings. Suppressed execute and complexity findings remain visible in report output.
- Added `--complexity-metric cyclomatic|cognitive` so complexity thresholds can gate on either metric explicitly.
- The install script now prints a Biome follow-up when no sibling or `PATH` Biome is available.

### Finding Semantics

- `no_inputs_reached` is now diagnostic by default instead of failing the whole execute stage.
- TypeScript portability reports now expose machine-readable `reason`, `failing_imports`, and `fix_hint` fields alongside raw stderr.
- Zero-argument functions with no meaningful parameter surface can now report `skipped_no_fuzzable_surface` instead of overstating coverage.
- Execute failures can now carry `classification: "type_signature_wider_than_usage"` when static literal call sites suggest the type surface is wider than observed usage.
- Verify can auto-seed fuzzing from simple literal call sites in the source file and nearby conventional test files. Use `--no-auto-seed` to disable that path.
- Fused flag/value CLI mistakes now get a split-argument hint instead of a bare unknown-flag error.
- TypeScript authoritative `--test-file` runs remain Node-only; Bun-native `bun:test` files are not yet supported directly.

## 0.1.9 - 2026-04-22

### Highlights

- Coverage reporting became explicit instead of implied.
- TypeScript portability failures were split from behavioral execute failures.
- TypeScript generators expanded to cover `Headers`, `Request`, `Response`, and `URLSearchParams`.
- Zero-argument entropy helpers stopped producing fake determinism failures.
- `--profile security` was simplified to a complexity threshold of `20` unless explicitly overridden.

### Why Findings Changed

- Some files that previously looked green now show explicit `coverage` and `portability` stages.
- Bun-native repos can now fall back to the repo runtime for behavior checks while still surfacing strict-Node portability warnings.

Reference: [docs/release-notes-0.1.9.md](docs/release-notes-0.1.9.md)

## 0.1.8 - 2026-04-18

### Highlights

- Broadened the false-positive control package and replay suite.
- Tightened analyzer and verifier behavior around exported TypeScript surfaces.
- Published the repeated `core-current` benchmark package showing verify-only repair lift on the curated suite.

### Why Findings Changed

- Known-good and replay controls were expanded, so confidence in a green result improved materially.
- The benchmark package and release notes shifted from a single headline claim to a fuller causal-control story.

Reference: [docs/release-notes-0.1.8.md](docs/release-notes-0.1.8.md)

## 0.1.7 - 2026-04-17

### Highlights

- Stabilized the early false-positive controls.
- Strengthened benchmark writeups and release-readiness framing.

### Why Findings Changed

- This was still pre-coverage-stage and pre-portability-stage Court Jester. A green run in `0.1.7` carried much less accounting than the same green run in later releases.

Reference: [docs/release-notes-0.1.7.md](docs/release-notes-0.1.7.md)
