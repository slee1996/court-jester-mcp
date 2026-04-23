# Changelog

This changelog tracks user-visible verifier semantics, report-shape changes, and release-level benchmark or documentation updates. If a release can change findings or stage outcomes without any change in the target repo, it should be called out here.

## Unreleased

## 0.1.15 - 2026-04-23

### Operator Ergonomics

- Added `verify --summary human` for a fast CLI summary over the structured report.
- Lint runner and infrastructure failures are now separated from ordinary Ruff/Biome findings instead of inflating `lint_issues`.
- Added source-level `court-jester-ignore complexity` support so complexity suppressions can live next to the code they justify.
- Added explicit declarative execute properties with `court-jester-properties ...`, including checks such as `sorted`, `permutation`, `nonnegative`, `clamped`, `nonempty_string`, `symmetric`, and `antisymmetric`.

### Callable Surface Expansion

- Exported object-literal methods and zero-argument exported class methods can now be surfaced and invoked as first-class callable APIs.
- Factory-returned methods are now explicit in coverage output via `fuzzed_via_factory` instead of remaining an implicit side effect of factory exercise.
- Added explicit support for Zustand-style container surfaces such as `create(... => ({ ... }))` and curried `create<T>()(... )` patterns. Surfaced methods are reported with stable names like `useStore.method`.

### CI Workflow

- Added a first-party `court-jester ci` subcommand for changed-file PR workflows.
- `court-jester ci` reuses the existing verify report schema and gate semantics, scopes to changed Python/TypeScript files from `git diff`, and supports `human`, `github`, and `json` output.

## 0.1.14 - 2026-04-22

### TypeScript Fuzz Coverage

- Added TypeScript fuzz-generator support for generic collection types including `Set<T>`, `ReadonlySet<T>`, `Map<K, V>`, `ReadonlyMap<K, V>`, and `ReadonlyArray<T>`.
- Functions that use supported collection generics such as `Set<string>` are no longer skipped as `unsupported or unresolved TypeScript types`.

## 0.1.13 - 2026-04-22

### Authoritative Test Runners

- Fixed Bun-backed authoritative TypeScript tests to invoke `bun test <file>` instead of Bun script mode.
- `--test-runner bun` and `--test-runner auto` now correctly run `bun:test` suites under Bun's test runner once Bun is selected.

## 0.1.12 - 2026-04-22

### Packaging

- Added the missing `Cargo.lock` update for the `0.1.11` package version.
- `cargo build --release --locked` now succeeds again for release builds.

## 0.1.11 - 2026-04-22

### Authoritative Test Runners

- Added `--test-runner auto|node|bun|repo-native` for authoritative test execution.
- TypeScript authoritative tests in `auto` mode now prefer Bun whenever the test imports `bun:test`, even if the repo is not otherwise marked Bun-native.

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
- TypeScript authoritative `--test-file` runs remained Node-only in `0.1.10`; this was addressed in `0.1.11` and corrected for Bun test-runner invocation in `0.1.13`.

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
