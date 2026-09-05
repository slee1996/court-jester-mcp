# Implementation map

Court Jester has two independent entrypoints: the Rust CLI verifies source code, and the Python benchmark harness measures agent repair outcomes. A refactor should preserve the public command, report, and import contracts at these boundaries.

## CLI

`src/main.rs` starts Tokio and calls `cli::run`. The private `src/cli/` modules are binary implementation details, not additions to the public Rust library.

| Module | Owns |
| --- | --- |
| `mod.rs` | Help, command dispatch, output selection, and process exit codes |
| `args.rs` | Parsed configuration, flag validation, and source/base input validation |
| `config.rs` | Versioned repository defaults, bounded discovery, config-relative paths, and CLI precedence |
| `environment.rs` | Scoped CLI overrides for verification timeouts and optional fuzz engines |
| `ci.rs` | Changed-file selection, baseline materialization, global mutation allocation, and CI output |
| `doctor.rs` | Runtime and isolated-profile readiness checks |

Shared source/workspace resolution remains in `src/lib.rs`; all commands use the same execution-context contract. The report and plan types remain in `src/types.rs`.

## Verification

`src/tools/verify.rs` coordinates analysis, project adapters, input planning, generated execution, authoritative tests, and optional test-quality campaigns. Supporting modules own independent evidence operations:

| Module under `src/tools/verify/` | Owns |
| --- | --- |
| `decisions.rs` | Diagnostic precedence, evidence strength, coverage accounting, and verdict construction |
| `reporting.rs` | Full/minimal JSON, human summaries, advisory test-quality counts, and persisted report writing |
| `report_text.rs` | Secret redaction and bounded diagnostic text |
| `corpus.rs` | Saved fuzz inputs, bounded merging, and conversion into planned arguments |
| `provenance.rs` | SHA-256 source and embedded-tree hashes shared by verification and replay |
| `replay.rs` | Loading reports, validating embedded evidence, and executing persisted repros |
| `regression.rs` | Opt-in regression export, acceptance checks, portable source binding, and no-overwrite bundle creation |

`verify/regression/python.py` and `verify/regression/node.mjs` are compile-time test-wrapper assets. Exported tests invoke Court Jester and require positive replay-check completion; they are not independent reimplementations of the recorded oracle. Keep protocol, CLI exit codes, and wrapper assertions in sync.

Public functions and types remain available through `court_jester::tools::verify`, using explicit re-exports. Callers should not depend on the internal file layout. Replay uses the same differential invocation helpers as initial verification; changing those helpers requires testing both paths.

Parsing and callable discovery remain in `analyze.rs`; input-domain planning is in `domain.rs`; mutation planning and coupling analysis are in `test_quality.rs`; diff parsing and lint adapters retain their existing modules.

## Execution and generated code

`src/tools/sandbox.rs` owns runtime selection, project loading, instrumentation, Docker execution, and test-runner adapters. Its supporting modules are:

- `sandbox/events.rs`: the versioned generated-harness event protocol, including lifecycle ordering and bounds.
- `sandbox/process.rs`: subprocess launch, process-group memory monitoring, timeout handling, output collection, and termination diagnostics.

Public sandbox APIs and event constants remain re-exported through `court_jester::tools::sandbox`.

`src/tools/synthesize.rs` renders the per-surface harness and optional native fuzz adapters. Shared Python and TypeScript preludes/epilogues live in `synthesize/python/` and `synthesize/typescript/`. They are compile-time `include_str!` assets, so the installed CLI is still a single binary. These files are generated-program fragments, not standalone applications; preserve their whitespace and protocol output when editing them.

## Benchmark

`bench/run_matrix.py` schedules experiments. `bench/runner.py` owns agent calls, workspace setup, retries, evaluation, and artifact production. Supporting modules are:

- `bench/reporting.py`: schema validation and interpretation of verifier evidence.
- `bench/feedback.py`: repair messages, failure summaries, and repro-assertion formatting.
- `bench/results.py`: shared command and workspace-setup result records.

Existing imports from `bench.runner` continue to work through explicit imports. The extracted modules do not import the runner, avoiding a circular dependency. Provider invocation and workspace mutations remain in the runner; feedback formatting can be exercised without launching an agent.

`bench/summarize_runs.py` aggregates completed experiments; it is distinct from `bench/reporting.py`, which interprets individual verifier reports. Providers, CLI transport, evidence bundles, fixtures, evaluators, and task/model/policy manifests retain their existing responsibilities.

## Verification workflow

Use the canonical commands in `justfile`. For refactors, run Rust compilation, formatting, integration tests and CLI smoke checks, plus benchmark unit tests and a matrix dry-run. Use a fresh `--output-dir` for benchmark validation when the default results directory contains historical artifacts with older schemas.

Process lifecycle changes also require timeout/memory or concurrent workload checks. Report or provenance changes require persisted-repro tests, including replay after the source workspace has been removed. Baseline failures must remain visible and be reproduced against the original source before claiming compatibility.
