# Changelog

This changelog tracks user-visible verifier semantics, report-shape changes, and release-level benchmark or documentation updates. If a release can change findings or stage outcomes without any change in the target repo, it should be called out here.

## Unreleased

## 0.2.5 - 2026-08-15

Publication retry for the correctness backlog after concurrent child-process integration tests produced a second CI-only false failure. See [docs/release-notes-0.2.5.md](docs/release-notes-0.2.5.md) for the complete change list and validation evidence.

### Release validation

- Rust integration targets now run one test at a time in both canonical local recipes and CI, preventing resource-sensitive Node sandbox tests from changing product verdicts under scheduler contention.

## 0.2.4 - 2026-08-15

Publication retry for the 0.2.3 correctness backlog after the first tag exposed a CI-only test flake. See [docs/release-notes-0.2.4.md](docs/release-notes-0.2.4.md) for the complete change list and validation evidence.

### Release validation

- Human-summary formatting now uses a deterministic report fixture instead of depending on concurrent Node fuzz execution, eliminating the CI-only false failure without reducing formatter coverage.


## 0.2.3 - 2026-08-15

Runtime, synthesis, and reporting correctness release for issues #1, #9, and #15 through #26. See [docs/release-notes-0.2.3.md](docs/release-notes-0.2.3.md) for the complete change list and validation evidence.

### Runtime and project context

- Repository TypeScript tests run through their native Vitest/Jest/Bun coordinator, including aliased or global launcher layouts, bounded workers, mixed log/JSON output, and preserved network/process guards.
- Isolated doctor artifacts retain Docker Desktop-accessible lexical paths while canonical containment continues to reject symlink escapes.
- Temporary TypeScript harnesses preserve `tsconfig`/Nuxt alias semantics and classify missing framework auto-import context as inconclusive instead of target-code crashes.
- Generated and replayed Python harnesses import targets under a non-main module identity, so guarded CLI/server entry points do not execute during verification.

### Synthesis and findings

- TypeScript synthesis respects array element types, default initializers, imported non-null object aliases, alias recursion limits, and supported nullable object unions.
- Implicit consistency checks are disabled for real nondeterministic/stateful functions while remaining active for pure transforms, shadowed locals, and semantically equal callable containers.
- JavaScript failure reports preserve `undefined`, `NaN`, positive/negative infinity, and ordinary tag-shaped objects without lossy JSON collisions.

### Diagnostics and stage semantics

- Safe unshadowed `Object.prototype.hasOwnProperty.call` usage no longer triggers `noPrototypeBuiltins`, while unsafe nested, target-owned, and shadowed calls remain findings.
- Signal-terminated Ruff processes produce actionable environment diagnostics with executable, arguments, target, working directory, and safe rerun guidance.
- Verification always emits an explicit execute stage when execution cannot run, and MCP failures retain stable messages and diagnostic context rather than returning message-less protocol errors.

## 0.2.2 - 2026-08-14

Runtime correctness release for the three defects confirmed after 0.2.1. See [docs/release-notes-0.2.2.md](docs/release-notes-0.2.2.md) for the complete change list and validation evidence.

### Authoritative test runners

- Bun tests now use the project test runner without incomplete JUnit reporter arguments, while preserving sandbox-blocker precedence and the public `bun_junit` adapter value.
- Automatic TypeScript test selection recognizes Vitest imports and configured Vitest packages, including globals-enabled TSX suites.
- Vitest coordinators retain their bounded worker pool while test workers enforce network and process denial.

### Isolated execution and dependencies

- Canonical Docker path mapping fixes isolated doctor smoke checks on macOS without weakening symlink containment.
- Generated Node and TSX harnesses resolve target, workspace, scoped, self-reference, and pnpm-linked dependencies while preserving native nearest-package precedence.


## 0.2.1 - 2026-08-01

Post-0.2.0 correctness and execution-hardening release. See [docs/release-notes-0.2.1.md](docs/release-notes-0.2.1.md) for the complete change list and validation evidence.

### Verification context and execution

- Project-aware source, test, dependency, and package-root resolution now remains consistent across analysis, synthesis, lint, authoritative tests, differential runs, and replay.
- Python and TypeScript/TSX harness launches preserve project semantics while reporting parse, module-load, timeout, memory, process, and network failures as typed diagnostics.
- Generated harnesses and candidate artifacts stay inside owned temporary roots; local and isolated execution enforce the selected network and resource policies without mutating the target repository.

### Synthesis and provenance

- Rest and keyword-variadic arguments preserve their call shape through planning, binding, execution, minimization, persistence, and replay.
- Callable and service-like defaults receive deterministic no-I/O substitutes only when their declared return shape is synthesizable; unsafe defaults are skipped with typed reasons.
- Seed, fixture, observed-call, generated-input, and safe-substitute provenance is retained in unit events, plans, findings, and benchmark metadata.

### Compatibility

- The schema-v3 report contract remains compatible with 0.2.0 consumers; this release adds typed diagnostic detail without restoring legacy success fields.

## 0.2.0 - 2026-07-15

Full categorized change log, migration guide, command additions, validation evidence, and source statistics: [docs/release-notes-0.2.0.md](docs/release-notes-0.2.0.md).

### Report and verification contract

- Verification reports now use schema `3` with typed `pass`, `fail`, and `inconclusive` verdicts, evidence strength, stage statuses, strict changed-export coverage, typed invocation paths, and provenance-rich findings/repros. A coverage or runtime evidence gap is inconclusive rather than a silent pass.
- Added `--coverage-gate changed-exports|none`, `--inferred-oracle-gate advisory|fail`, `--runtime-profile local-trusted|isolated`, Docker image overrides, `doctor`, `repair-json`, differential base-tree inputs, and persisted finding replay. Low-confidence inferred and unproven differential findings remain advisory by default.
- Added artifact-v1 benchmark metadata: schema-v3 doctor prerequisites, abstention-aware observations and paired statistics, held-out locks, portable redaction-aware evidence bundles, release gate policies, and opt-in non-blocking shadow records.
- Reports and benchmark artifacts from earlier releases remain historical evidence; they are not silently converted to the active v3/artifact-v1 contracts.

## 0.1.16 - 2026-04-25

### Domain-Aware Synthesis

- TypeScript and Python fuzz synthesis now derives closed input domains from literal type surfaces instead of falling back to arbitrary values.
- Python `typing.Literal[...]` annotations, including nested literal collection elements, now generate only declared values.
- TypeScript literal unions and object fields with literal domains now generate declared branch values.
- TypeScript enum declarations are recorded as literal-union aliases, including imported enum type context.
- TypeScript `typeof CONST_TUPLE[number]` aliases are rewritten from `as const` arrays, including imported type context.

### False-Positive Control

- Closed literal-domain object inputs no longer receive broad `{}` object edge cases that are outside the declared shape.
- Fresh external guardrails `v3` and `v4` both reach full buggy recall with zero fixed-code false positives, while saturated `v1` and `v2` remain clean.

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
