# Court Jester 0.2.0

Release date: 2026-07-15

Court Jester 0.2.0 is the confidence-contract and central-engine overhaul. It replaces ambiguous boolean success with explicit evidence accounting, introduces a repository-derived verification-plan engine, makes findings independently replayable, adds differential and isolated execution, and turns the benchmark and release paths into versioned, auditable contracts.

This is a breaking report-contract release for automation built against 0.1.x.

## Release scope

Compared with `v0.1.16`, this release contains:

- 78 changed files;
- 14,839 insertions and 3,498 deletions across product code, tests, benchmarks, release automation, documentation, and publication assets;
- 5,960 insertions and 1,232 deletions across the six central engine files alone;
- a 3,251-line change to `src/tools/verify.rs`;
- a new 880-line `src/tools/domain.rs` planning engine;
- major rewrites of analysis, synthesis, sandbox, report types, and the agent-facing CLI;
- 328 passing Rust unit and integration tests, 65 benchmark-harness tests, and 2 release-contract tests in the final local candidate.

Full source comparison: [`v0.1.16...v0.2.0`](https://github.com/slee1996/court-jester-mcp/compare/v0.1.16...v0.2.0)

## Full change log

### 1. Report schema v3 and confidence-calibrated verdicts

- Replaced the legacy `overall_ok` report contract with top-level `schema_version: 3`, `verdict`, and `strength`.
- Added typed verdicts: `pass`, `fail`, and `inconclusive`.
- Added typed evidence strengths: `none`, `parse_only`, `static_checked`, `runtime_smoke`, `property_checked`, and `authoritative_tests`.
- Added typed stage statuses: `passed`, `failed`, `inconclusive`, `advisory`, and `skipped`.
- Defined verdict precedence as fail > inconclusive > pass.
- Made missing behavioral evidence, required coverage gaps, rejected-only inputs, module-loading failures, runtime setup failures, timeouts, resource kills, and unsupported required surfaces inconclusive instead of silently successful.
- Kept lint advisory by design and made portability advisory only after successful repository-native fallback; a portability condition that blocks behavior remains inconclusive.
- Added aggregate `VerificationEvidence`, `CoverageSummary`, and `FindingsSummary` records.
- Removed legacy compatibility booleans and obsolete `fuzz_failures` report arrays from the active contract.
- Updated full, minimal, persisted, human, JSON, GitHub, and repair-loop output paths to use the same typed semantics.
- Added stable v3 exit behavior: `0` pass, `1` fail, `2` pre-report usage/setup error, and `3` inconclusive.

### 2. New repository-derived domain and verification-plan engine

- Added a first-class domain intermediate representation covering any, boolean, integer, float, string, bytes, literal, nullable, union, array, tuple, set, map, object, and opaque domains.
- Added a domain-source vocabulary for type annotations, TypeScript enums, TypeScript const tuples, imported types, observed calls, JSON fixtures, default values, and validation guards. The current planner emits concrete provenance for type annotations/defaults, observed calls, and JSON fixtures while resolved enum/tuple/import structure is represented in the derived domain.
- Added explicit surface specifications with stable IDs, source locations, export/invocation state, and parameter lists.
- Added per-parameter domain records with closed/open classification and evidence sources.
- Added planned positional and named arguments instead of treating every input as an unstructured expression list.
- Added valid, invalid, and unknown input classification so rejected inputs cannot be mistaken for behavioral proof.
- Added caller examples, fixture examples, inferred properties, contract specifications, and typed confidence/provenance.
- Added execution units that bind a surface, invocation path, target, source file, inputs, and contracts into an auditable plan.
- Added deterministic Cartesian exhaustion for closed literal products.
- Added recursive-alias guards and opaque-domain handling so unresolved or recursive types do not fabricate valid object inputs or overflow planning.
- Replaced removed business-field value pools with repository-derived evidence and generic type-safe boundaries.
- Exposed the verification plan in report detail so consumers can inspect what was selected, why it was selected, and how it was exercised.

### 3. Analyzer, type resolution, and callable-surface expansion

- Added cross-file call-edge collection for Python and TypeScript.
- Added lexical binding resolution for TypeScript callable aliases and shorthand properties.
- Improved detection of object-returning callbacks, returned object methods, factory callables, and nested expression forms.
- Kept data fields out of callable coverage even when they appear next to callable shorthand properties.
- Improved exported arrow-function, block-bodied arrow, export-list, and default-export handling.
- Improved exported object-literal method, zero-argument class method, factory-returned method, Zustand-style container, and curried container discovery.
- Added typed invocation targets for direct calls, exported callers, factory callables, and authoritative tests.
- Scoped imported type resolution to referenced names while retaining required transitive dependencies.
- Expanded sibling, parent-relative Python, deep TypeScript, enum, const-tuple, object-alias, and non-object-alias resolution.
- Preserved recursive imported aliases without inventing unsupported concrete shapes.
- Improved Python and TypeScript complexity accounting, including nested Python functions, `match` cases, TypeScript switches, `for of`, and logical operators.
- Continued honoring source-level complexity directives while reporting cyclomatic/cognitive breakdowns at function granularity.

### 4. Strict surface coverage and runtime proof

- Added `--coverage-gate changed-exports|none`; `changed-exports` is the default.
- Required every changed exported or otherwise invocable surface when a diff is supplied, and every exported/invocable surface otherwise.
- Added top-level callable fallback for files without exports.
- Added typed invocation paths for direct, factory, caller, and authoritative-test execution.
- Added distinct coverage states for direct checks, factory reach, factory checks, caller checks, authoritative-test checks, unsupported types, internal helpers, methods, nested/private symbols, diff filtering, no fuzzable surface, and blocked module loading.
- Separated runtime reach from behavioral checking: merely entering a factory or caller no longer proves the returned/target surface was evaluated.
- Added source instrumentation overlays and per-surface runtime entry events.
- Applied runtime-confirmed coverage back to the synthesized plan instead of relying on static discovery alone.
- Made `--tests-only` use same-process instrumentation from the authoritative test runner; a passing test with partial required-surface reach is inconclusive.
- Prevented `--coverage-gate none` and `--execute-gate none` from manufacturing a pass when no valid behavioral evidence exists.
- Added exact required, behaviorally checked, reached-only, no-input, skipped, and blocked coverage totals.

### 5. Input generation, contracts, and oracle policy

- Reworked synthesis to consume the verification plan and repository-derived domains.
- Added stable handling for Python keyword-only arguments in normal and differential execution.
- Preserved closed Python `Literal[...]`, nested literal collections, TypeScript literal unions, literal object fields, enums, and `as const` tuple-derived aliases.
- Added repository call-site seeds, nearby test literals, object-literal calls, defaults, JSON fixture rows, and validation evidence as typed input sources.
- Added structural fixture inference without treating a single fixture row or incidental exact output as an authoritative oracle.
- Kept unsupported/unresolved parameters visible as coverage evidence instead of generating arbitrary values and claiming success.
- Added explicit oracle kinds for authoritative tests, runtime contracts, type contracts, declared properties, seed regressions, differential comparisons, generic properties, and inferred semantics.
- Added oracle provenance and confidence to every finding.
- Added `--inferred-oracle-gate advisory|fail`; low-confidence name/context inference is advisory by default.
- Preserved source directives and explicit declared properties as stronger evidence than name-based guesses.
- Tightened false-positive controls around idempotency, boundedness, nonnegativity, symmetry, nonempty strings, structured identifiers, and comparator-like names.
- Expanded or revalidated semantic probes for query-string encoding/parsing, Unicode and blank values, PEP 440 versions/specifiers, cookie quoting, request metadata, response helpers, static-file middleware, explicit-false feature flags, semver prereleases and caret ranges, null/inherited defaults, and SameValueZero behavior.
- Kept context-specific semantics gated by actual repository evidence rather than broad business-name heuristics.

### 6. Typed findings, bounded minimization, and repair output

- Replaced loosely structured execution failures with `VerificationFinding` records.
- Added stable finding IDs, typed severity, confidence, category, source location, input classification, oracle information, and suppression state.
- Added finding severities for crashes, property violations, behavioral regressions, and infrastructure conditions.
- Added categories for exceptions, properties, authoritative tests, differential findings, and infrastructure.
- Added structured repro cases with positional/named arguments, minimized expressions, snippets, expectations, and persisted replay commands.
- Added bounded minimization with status, attempt count, original case, and reconfirmed minimized case.
- Added deterministic replay sentinels containing reproduced state, severity, oracle kind, and category.
- Added truncation for oversized inputs and messages so one pathological value cannot dominate a report.
- Added repair prioritization that favors authoritative findings, valid-input crashes, declared/type contracts, and reproducible regressions over low-confidence inference.
- Added `--summary repair-json` with `recommended_action`: `repair`, `inspect_environment`, `add_contract_or_test`, or `none`.
- Prevented suppressed findings from becoming the primary repair recommendation.

### 7. Persisted replay

- Added the `court-jester replay` subcommand.
- Added `--report <PATH>` and `--finding <ID>` selection for persisted schema-v3 findings.
- Added optional `--dependency-project-dir` for dependency-sensitive replay.
- Added replay outcomes `reproduced`, `not_reproduced`, and `inconclusive` with distinct exit codes.
- Made persisted minimal and full reports loadable by the same replay engine.
- Embedded source closure and structured expectations so ordinary findings remain replayable after temporary harnesses disappear.
- Added protocol validation for replay sentinels and typed inconclusive results for missing dependencies, runtime mismatches, or malformed payloads.

### 8. Differential verification

- Added `--base-file` plus `--base-project-dir` as a required pair for candidate/base comparison.
- Added complete read-only baseline-tree handling rather than comparing a source file in isolation.
- Added compatible-surface matching and explicit disabled reasons for missing or incompatible baseline surfaces.
- Added normalized behavior snapshots covering success, returned values, output, exceptions, and binding failures.
- Added typed differential findings and repros with embedded base/candidate source closures.
- Added dependency contracts and tree digests for replay integrity.
- Added named binding for Python keyword-only differential parameters.
- Disabled false regressions for unstable address-bearing representations and incompatible callable surfaces.
- Kept unproven base/candidate differences advisory; a difference gates only when an authoritative fixture, declared contract, or test establishes which side is correct.
- Made full and minimal differential reports replayable after the original source projects are removed.

### 9. Runtime execution and sandbox overhaul

- Added `--runtime-profile local-trusted|isolated`; `local-trusted` remains the default.
- Added Docker-isolated Python and TypeScript execution with no network, read-only mounts/root filesystem, bounded CPU/memory/processes/file size, and deterministic cleanup.
- Added default isolated images `python:3.12-slim` and `node:24-bookworm-slim`.
- Added `--python-docker-image` and `--typescript-docker-image` overrides, valid only in isolated mode.
- Added Docker daemon and image-ID readiness checks.
- Made daemon, image, mount, module-load, timeout, and resource failures structured and inconclusive; isolated execution never silently falls back to the host.
- Validated finite positive timeout and memory values and incompatible profile/image combinations.
- Preserved resource limits for project-aware execution and counted TypeScript child processes toward memory enforcement.
- Executed original Python/TypeScript source files when the verified code matches disk, preserving module identity and relative imports.
- Improved Python package and relative-import behavior.
- Improved TypeScript source execution for type-only imports/re-exports, extensionless relative imports, repo-local packages, project directories, and original-file identity.
- Preferred the Node transform path for ordinary TypeScript execution, retried with the loader where type-only resolution requires it, and retained Bun fallback for Bun-native extensionless imports.
- Kept execution cleanup deterministic across generated files, overlays, processes, and Docker containers.

### 10. Authoritative tests and lint/runtime integration

- Added language- and runner-specific authoritative-test overlays.
- Preserved original Python test module identity and relative imports when test code matches disk.
- Improved TypeScript test/source scoping for tests with and without direct imports.
- Retained `auto|node|bun|repo-native` selection and Bun-test detection while recording the actual runtime path.
- Added same-process coverage instrumentation to authoritative tests.
- Kept lint runner failures separate from ordinary Ruff/Biome diagnostics and summary issue counts.
- Improved project-local Ruff/Biome discovery, config paths, virtual paths, source-file materialization, and actionable runner errors.
- Added macOS Gatekeeper/quarantine guidance for signal-killed sibling linters without misclassifying that platform-specific behavior on Linux.
- Kept anonymous inline-snippet unused-variable noise advisory/filtered where it does not represent target behavior.

### 11. CLI, doctor, and CI behavior

- Added `court-jester doctor` for Python, TypeScript, linter, Docker, and selected-profile readiness.
- Added schema-v3 doctor reports with typed checks, runtime profile, verdict, and human/JSON summaries.
- Added `court-jester replay` to the top-level CLI and help output.
- Added runtime-profile and Docker-image flags to verify, execute, replay, doctor, and CI paths where applicable.
- Added base-tree differential flags and validation.
- Added `repair-json` summary selection.
- Added coverage and inferred-oracle gates.
- Upgraded `court-jester ci` to aggregate typed per-file verdicts with fail > inconclusive > pass.
- Added/selectively enforced parse, complexity, lint, coverage, portability, execute, and test gates.
- Preserved human, GitHub annotation, and JSON CI output while making each consume the v3 contract.
- Added revision-archive support for CI base-tree verification.
- Improved validation and actionable errors for missing flag pairs, invalid profiles/images/limits, fused flags, replay arguments, and unsupported gate values.

### 12. Benchmark artifact contract and evidence pipeline

- Added immutable benchmark `artifact_schema_version: 1` across matrix, run, result, summary, and evidence artifacts.
- Added `verify_schema_version_required: 3` so benchmark lanes cannot silently mix report generations.
- Added schema-v3 doctor prerequisites for non-dry benchmark runs.
- Persisted resolved runtime profiles, image/runtime identities, and doctor digests.
- Added explicit abstentions for missing, mixed, malformed, or incompatible artifacts instead of counting them as successes or semantic failures.
- Separated operational/provider errors from semantic task outcomes.
- Added hidden-seed digests and paired-cell eligibility rules.
- Added paired lift, confidence intervals, and exact McNemar statistics when the paired evidence supports them.
- Added gate policies `none`, `private-beta-default`, and `strict-heldout`, with explicit ineligibility when required cells or metrics are missing.
- Added held-out-lock enforcement to prevent accidental evaluation drift.
- Added portable evidence bundles with relative paths, manifests, SHA-256 digests, verification, and configurable transcript redaction.
- Added strict-evidence mode for release-grade bundle validation.
- Added opt-in shadow JSONL records with `blocking_mode: shadow`; shadow observation never alters task success.
- Added artifact-aware benchmark tests for evidence integrity, redaction, gate eligibility, pairing, abstentions, and summary calculations.
- Updated benchmark dashboards, runbooks, methodology, runner behavior, and stress/autoresearch integrations to consume the new contract.

### 13. Packaging, installation, and release integrity

- Renamed the Cargo package identity from `court-jester-mcp` to `court-jester` while retaining the historical GitHub repository URL.
- Bumped the package and lockfile version from `0.1.16` to `0.2.0`.
- Added package description, repository, readme, keywords, and categories metadata; this release is distributed through GitHub Releases, not crates.io.
- Added the `tar` dependency for baseline-tree archive handling.
- Added a reusable pull-request/release quality workflow using Rust 1.86.0, Python 3.12, and Bun 1.3.14.
- Added hosted formatting, Clippy, Rust integration, benchmark-unit, release-contract, optimized-build, CLI-smoke, and held-out dry-run gates.
- Made the release workflow depend on the same quality gate before any platform build.
- Added exact tag/Cargo/lockfile/changelog/release-note/workflow/installer contract validation.
- Added mismatched-tag rejection tests.
- Continued building official archives for macOS Arm64, macOS AMD64, Linux Arm64, and Linux AMD64.
- Changed publication to build artifacts first and publish once after every platform succeeds.
- Added a matching SHA-256 file for every archive and verified every checksum before release creation.
- Made the release use the versioned notes file and require an existing verified tag.
- Updated `install.sh` to download and validate the published checksum with `shasum` or `sha256sum` and refuse unverified installation.
- Kept public archives limited to the `court-jester` binary; Ruff and Biome remain project/operator dependencies.
- Added `just release-check` as the canonical full local release gate.
- Added a GitHub-only release runbook and explicit post-publication asset verification procedure.

### 14. Documentation and publication assets

- Rewrote the report-schema documentation for the clean v3 cutover, typed stages, coverage, findings, replay, runtime profiles, CI behavior, and benchmark boundary.
- Expanded the CI adoption guide with verdict handling, rollout strategy, isolation, replay, suppressions, and artifact evidence.
- Added a complete architecture diagram covering analysis, planning, synthesis, runtime, differential verification, reports, replay, and benchmark evidence.
- Added release operations documentation, signature-contract research notes, and a Terminal-Bench stress plan.
- Updated the benchmark methodology, big-run runbook, private-beta readiness material, overview, README, system flow, and documentation index.
- Added a long-form product/blog write-up and updated the research paper/manuscript package for the confidence-contract architecture.
- Added the public product site, blog page, visual scorecard, hero imagery, and release promo asset.
- Removed the previously tracked local governed-task manifest from the public repository and ignored generated governance, benchmark-result, paper-build, deck-build, cache, and temporary artifacts.

### 15. Hosted release corrections included in the tag

- Limited the macOS Gatekeeper signal-kill integration test to macOS so Linux correctly exercises the general runner-failure path.
- Added pinned Bun 1.3.14 setup to the hosted quality job because a required Bun runtime regression test is part of the release gate.
- Re-ran the complete hosted quality gate and all four platform builds after both corrections.
- Published `v0.2.0` from commit `76a1f95147c0a3f752cc5c84215f3a5984c86e1a` only after the corrected workflow passed.

## Breaking changes and migration

1. Require `schema_version: 3` in every active report consumer.
2. Branch on `verdict`; do not reconstruct success from stage names and do not read the removed `overall_ok` boolean.
3. Treat `inconclusive` as missing or blocked evidence, not a pass and not a confirmed semantic failure.
4. Consume `strength` independently from `verdict`; strength describes what evidence ran, not whether the target is correct.
5. Update exit-code handling:
   - `0`: passing verdict or successful replay;
   - `1`: failed verification or replay mismatch;
   - `2`: CLI usage/setup failure before a report;
   - `3`: inconclusive verification/replay.
6. Replace consumers of `fuzz_failures` with typed `findings`, `suppressed_findings`, and `findings_summary`.
7. Use typed coverage states, `required`, `invocation_path`, and `summary.coverage`; do not equate static discovery or caller/factory reach with a behavioral check.
8. Update agent prompts to repair `fail`, inspect the environment or add evidence for `inconclusive`, and ship only `pass`.
9. Review low-confidence inferred semantic findings before enabling `--inferred-oracle-gate fail`; the default remains advisory.
10. Pass both `--base-file` and `--base-project-dir` when enabling differential verification.
11. Regenerate benchmark artifacts under artifact schema `1`; do not mix legacy artifacts into an active release gate.
12. If automation assumed the old Cargo package name, update it to `court-jester`. Distribution remains GitHub-only.

Historical schema-v2 reports and pre-artifact-v1 benchmark outputs remain historical evidence. Court Jester does not silently rewrite them into the new contracts.

## New and materially changed command surface

```text
court-jester replay --report <PATH> --finding <ID>
court-jester doctor --language python|typescript|all

--coverage-gate changed-exports|none
--inferred-oracle-gate advisory|fail
--runtime-profile local-trusted|isolated
--python-docker-image <IMAGE>
--typescript-docker-image <IMAGE>
--base-file <PATH>
--base-project-dir <PATH>
--summary repair-json
--dependency-project-dir <PATH>
```

Existing `verify`, `ci`, `analyze`, `lint`, and `execute` commands remain available. `ci` now uses the typed v3 verdict and coverage contract.

## Install

Court Jester currently ships through GitHub Releases:

```bash
curl -fsSL https://raw.githubusercontent.com/slee1996/court-jester-mcp/main/install.sh | sh
```

The installer selects the current macOS/Linux architecture, downloads the archive and matching SHA-256 file, verifies the checksum, and installs `court-jester` to `~/.local/bin`.

Published assets:

- `court-jester-v0.2.0-darwin-arm64.tar.gz`
- `court-jester-v0.2.0-darwin-amd64.tar.gz`
- `court-jester-v0.2.0-linux-arm64.tar.gz`
- `court-jester-v0.2.0-linux-amd64.tar.gz`
- one matching `.sha256` file for each archive.

## Validation

The final local release candidate passed:

- `cargo fmt --all -- --check`;
- `cargo clippy --locked --all-targets -- -D warnings`;
- 328 Rust unit and integration tests;
- 65 benchmark-harness unit tests;
- 2 release-contract tests, including mismatched-tag rejection;
- locked optimized build and CLI smoke against `court-jester 0.2.0`;
- real verify smoke against the bundled failing fixture;
- GitHub archive staging with optional linters excluded;
- locked `swebench-lite-pilot` dry-run with held-out-lock enforcement.

Hosted workflow [`29468582406`](https://github.com/slee1996/court-jester-mcp/actions/runs/29468582406) then passed:

- the complete reusable quality job;
- macOS Arm64 build and packaging;
- macOS AMD64 build and packaging;
- Linux Arm64 build and packaging;
- Linux AMD64 build and packaging;
- aggregate checksum verification;
- single-job GitHub Release publication.

After publication, all eight assets were downloaded independently. Every checksum matched, every archive contained only `court-jester`, and the downloaded macOS Arm64 binary returned `court-jester 0.2.0` and valid help output.

## Known boundaries

- Python and TypeScript are the supported source languages.
- `local-trusted` executes target code on the host and is not a security boundary.
- `isolated` requires a ready Docker daemon and pre-available selected images; Court Jester does not silently pull images or fall back to host execution.
- Ruff, Biome, Node.js, and optionally Bun are resolved from the target project or operator environment and are not bundled.
- Low-confidence inferred semantics remain advisory unless explicitly promoted.
- Differential verification is evidence, not an authoritative oracle, unless paired with a declared contract, fixture, or authoritative test.
- Third-party dependencies required by a replay must match the recorded dependency contract or replay is inconclusive.
- Court Jester remains an experimental verifier for agent repair loops, not a general CI system or secure hidden-judge replacement.
