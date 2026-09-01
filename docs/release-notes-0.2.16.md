# Court Jester 0.2.16

Release date: 2026-08-30

Court Jester 0.2.16 promotes bounded test-quality analysis to the stable `--test-quality [N]` interface. The feature is advisory: it exposes missing behavioral observations without changing verifier verdict, strength, process exit, or CI gate results.

## Behavior-sensitivity mutants

- Court Jester plans conservative comparison-boundary, condition-negation, and boolean mutants, then runs them through the target's authoritative tests.
- A mutant is `killed` or `survived` only when the authoritative run contains evidence for the exact mutated surface. Other outcomes remain distinct: `invalid`, `blocked`, and `no_coverage`.
- Reports deliberately contain no score, percentage, grade, or quality gate. A survivor is a concrete advisory observation, not a release verdict.

## Direct verify and CI

- Direct `verify --test-quality [N]` requires exactly one `--test-file`.
- Changed-file `ci --test-quality [N]` accepts repeatable `--test-file` arguments, with at most one Python entrypoint and at most one TypeScript/TSX entrypoint. CI selects the entrypoint that matches each target language.
- `N` defaults to 8 and accepts 1 through 32. In CI it is one global per-command cap, allocated deterministically across changed files and their required surfaces rather than reset per file. Planning may explicitly underfill the cap when candidates or authoritative tests are unavailable.
- Baseline authoritative-test semantics are unchanged. `--tests-only` remains unsupported in CI.

## Coupling and report semantics

Target-aware private access, private spies, and source-introspection findings are reported separately from mutant outcomes. Access to unrelated targets is not attributed to the file under review. Every coupling finding carries the normalized authoritative `test_source_file`, making its test-side provenance explicit.

The report stage remains `test_quality` with `mode: advisory` and `experimental: false`. Full reports retain the requested mutant cap, baseline eligibility, outcome counts, per-mutant evidence, coupling findings, and planning or coupling errors. CI JSON keeps per-file stages and derives its aggregate summary from them; human and GitHub summaries show planned, killed, survived, unjudged, and coupling counts without a score.

## Limitations

Test-quality analysis requires authoritative tests and intentionally covers only bounded, supported mutation surfaces. Unsupported candidates, unavailable matching test entrypoints, blocked runs, invalid mutants, and uncovered surfaces are reported rather than guessed. The analysis does not replace baseline verification and cannot make an otherwise passing or failing command change verdict.

## Verification

The repository expects the focused integration and release-contract checks before the canonical full release gate:

```text
cargo test --locked --test verify_test -- --test-threads=1
python3 -m unittest discover -s tests -p 'release_test.py'
just release-check
```

`just release-check` is the complete local pre-tag gate. It validates release metadata, formatting, Clippy, locked Rust integration and benchmark/release-contract tests, an optimized build, a sample CLI smoke, package staging, the fuzz-effectiveness lane, and the held-out dry run. GitHub Actions does not invoke or repeat that complete local recipe: the release workflow first calls the reusable Ubuntu quality workflow, then independently validates release metadata and builds, packages, checksums, and uploads the four platform archives.

## Distribution

Court Jester remains distributed through GitHub Releases only. Version 0.2.16 is not published to crates.io; tagging and publication are separate release-maintainer actions and are not performed as part of this metadata preparation.
