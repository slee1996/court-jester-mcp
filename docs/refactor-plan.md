# Repository refactor

Objective: refactor the repository around explicit responsibilities while preserving the current CLI, library API, report schema, runtime behavior, and benchmark contracts.

This is a single-owner, staged refactor. Existing worktree changes are the baseline, not changes to discard. The unrelated completed release manifest in `tasks.json` remains untouched.

## Acceptance

- CLI dispatch/options, verifier orchestration, runtime execution, generated harnesses, and benchmark orchestration have focused implementation boundaries.
- Existing public Rust paths and Python entrypoints remain compatible; no flags, report fields, gates, or fixture expectations change as part of extraction.
- Extracted modules use explicit dependencies and the narrowest practical visibility; no textual `include!` splitting or blanket parent imports.
- Maintained architecture documentation describes the resulting layout.
- Rust formatting, compilation, integration tests, CLI smoke checks, Python benchmark unit tests, and matrix dry-run verify the changed surfaces. Existing failures must be established against the baseline and documented, not hidden.

## Milestones

1. Establish baseline and extract independent protocol, persistence, and report-text utilities. **Done.**
2. Separate verifier reporting, evidence/coverage decisions, and replay from pipeline orchestration. **Done.**
3. Separate CLI configuration/CI and sandbox runtime responsibilities; move generated harness assets behind their existing renderers where appropriate. **Done.**
4. Separate benchmark reporting/feedback responsibilities from experiment execution. **Done.**
5. Update architecture documentation and complete cross-repository validation and final diff review. **Done; baseline failures remain explicitly recorded below.**

## Evidence and progress

- Initial inspection: existing edits in 11 tracked files; verifier ~10k lines, sandbox ~8k, synthesis ~6k, CLI ~2.8k, benchmark runner ~2.5k.
- Initial compile invocation found system `rustc` 1.83 despite selecting Cargo by absolute path. All Rust commands must prepend the user's `.cargo/bin` to PATH so the pinned toolchain is used.
- Baseline `cargo check --locked --all-targets`: passed with the pinned toolchain.
- Baseline library tests: 89 passed, two existing failures in `isolated_runtime_temporary_directories_use_docker_shared_home` and `isolated_standalone_temporary_directories_use_docker_shared_home`. Both assert the temporary directory's parent. Investigate execution-environment permissions before final validation.
- Extracted event protocol into `sandbox/events.rs`, corpus persistence into `verify/corpus.rs`, and redaction/bounding into `verify/report_text.rs`. Public sandbox paths are preserved through explicit re-exports; redaction unit tests moved with their implementation.
- Extracted JSON/human report views, test-quality summaries, and report persistence into `verify/reporting.rs`. Existing public functions and `TestQualitySummary` remain available through `tools::verify`.
- Moved four Python/TypeScript prelude and epilogue literals to `synthesize/python/` and `synthesize/typescript/`, loaded using `include_str!`. Compared each extracted file against its original literal content: exact matches, including leading/trailing newlines.
- Focused test command: `cargo test --locked --lib --test schema_test --test diff_test --test file_path_test --test execution_context_test -- --test-threads=1 --skip isolated_runtime_temporary_directories_use_docker_shared_home --skip isolated_standalone_temporary_directories_use_docker_shared_home`. Result: 89 library + 22 integration tests passed; two previously failing library tests explicitly excluded from this diagnostic run, not counted as passes.
- `cargo test --locked --test verify_test -- --test-threads=1`: 174 passed, seven failed. Reconstructed the original working-tree sources in a separate temporary baseline and ran the same suite: identical passing count and failing tests (`context_notes_can_enable_pep440_semantics`, `cookie_file_context_can_enable_cookie_quote_semantics`, `factory_returned_methods_appear_in_coverage`, `python_factory_action_sequence_finds_second_step_crash`, `rejected_only_fuzz_run_is_not_counted_as_pass_in_report_summary`, `typescript_factory_action_sequence_finds_second_step_crash`, `value_error_is_treated_as_a_crash`).
- `cargo test --locked --test synthesize_test --test schema_test -- --test-threads=1`: schema tests passed; synthesis 111 passed, two failed. Original-source baseline synthesis run reproduced both failures (`factory_campaigns_generate_repeated_stateful_action_sequences`, `python_set_members_use_semantic_consistency_equality`).
- Formatted the changed Rust modules only, preserving unrelated worktree formatting. `git diff --check` passed.
- Extracted `verify/decisions.rs` and `verify/replay.rs`, preserving their public facade. Added shared `verify/provenance.rs`: replaced handwritten SHA-256 with the existing `sha2` dependency and removed duplicate embedded-tree hashing. Empty, short, and million-byte SHA-256 vectors pass, as do five persisted-replay integration tests.
- Extracted CLI argument validation, scoped environment overrides, CI orchestration, and readiness checks into `src/cli/`. `src/main.rs` now only initializes Tokio and calls the CLI. All 17 existing CLI tests pass.
- Extracted subprocess launch, memory monitoring, timeout handling, and termination evidence into `sandbox/process.rs`. All 49 sandbox integration tests pass.
- Extracted benchmark report interpretation, repair-feedback formatting, and shared result records into `bench/reporting.py`, `bench/feedback.py`, and `bench/results.py`. Existing runner imports remain available; the new modules do not import the runner. All 77 benchmark unit tests pass before and after extraction.
- Added `docs/code-map.md` and linked it from the README and maintained architecture documents. Historical experiment and release documents are unchanged.
- Applied canonical Rust formatting and resolved two existing lint-only issues: an unnecessary explicit lifetime in source-depth analysis and a needless borrow in a report test.

## Final validation

Commands below use the pinned toolchain on PATH. This is a behavior-preserving refactor; suite failures are not described as passes merely because they predate it.

| Acceptance | Evidence |
| --- | --- |
| Focused implementation boundaries | Source audit of `src/cli/`, `verify/`, `sandbox/`, harness assets, and benchmark modules; ownership documented in `code-map.md` |
| Public compatibility | Existing CLI tests, Rust integration callers, report/replay tests, and benchmark runner imports compile and execute; report schemas, flags, and task manifests were not changed |
| Explicit dependencies | New production modules use explicit imports and narrow internal visibility; no `include!` or blanket parent imports |
| Source/provenance preservation | Exact equality of all four extracted harness assets; standard SHA-256 vectors; five replay tests pass, including removed-source-workspace repros |
| Rust gates | `cargo fmt --all -- --check`, `cargo check --locked --all-targets`, and `cargo clippy --locked --all-targets -- -D warnings` pass |
| Full Rust behavior comparison | `cargo test --locked --no-fail-fast -- --test-threads=1`: 565 passed, 11 failed. All 11 failures reproduce in original-source baseline suites: seven verifier, two synthesis, and two special-value tests |
| Runtime environment | Both Docker-shared temporary-directory tests pass when run outside the filesystem sandbox; the initial two failures were environmental. Final full-suite run used that access and all 92 library tests passed |
| Benchmark contracts | `python3 -m unittest bench.test_run_matrix bench.test_runner bench.test_summarize_runs bench.test_agent_trace bench.test_evidence bench.test_materialize_mutation bench.test_fuzz_effectiveness`: 77 passed |
| Matrix | `python3 -m bench.run_matrix --dry-run --output-dir <fresh-directory>`: all 6,171 planned runs validated. The exact default dry-run was also attempted; its summarizer rejected historical incompatible-schema artifacts in the existing results directory |
| Distribution | `cargo build --locked --release --bin court-jester`, release metadata validation for `v0.2.16`, and both release-contract tests pass |
| CLI smoke | `python3 scripts/smoke_cli.py --binary target/release/court-jester` passes. Extended TSX/harness-argument checks pass when invoked directly. `--verify-sample` fails identically for original and refactored binaries because the sample receives `pass` instead of the smoke test's expected `fail` |
| Concurrency/lifecycle | `mixed_verify` stress: 40/40 requests completed with four workers and no harness errors. Direct valid-source timeout probe: eight requests across four workers all returned `timed_out` with typed timeout termination |
| Final hygiene | `git diff --check` passes; existing user changes remain; no commits, releases, or external publication performed |

The additional original-source failures are `typescript_findings_losslessly_serialize_and_reproduce_special_values` and `typescript_special_value_tags_do_not_collide_with_ordinary_objects` in `issue24_test`.

The checked-in timeout/memory stress scenarios contain literal backslash-n sequences, so their successful transport runs are not accepted as evidence of resource-limit behavior. The direct timeout probe used actual newlines, while the sandbox integration suite exercises child-process memory accounting. Fixing those pre-existing fixtures and the baseline semantic failures is separate behavioral work.

Implementation and compatibility validation are complete. The remaining known suite and sample-smoke failures are pre-existing and were preserved rather than changing verification semantics during a structural refactor.
