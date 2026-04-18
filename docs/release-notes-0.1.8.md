# Court Jester 0.1.8

Date: 2026-04-18

## Summary

This release is about two things:

- a materially stronger false-positive story
- a stronger repeated benchmark after tightening the verifier against false positives first

Since `0.1.7`, Court Jester expanded its known-good and upstream replay controls, fixed analyzer/verifier gaps that were causing real false positives, and finished a larger repeated `core-current` run that still shows a strong verify-only lift.

## Highlights

- False-positive controls are now much broader and currently clean.
  - Local known-good corpus: `80 / 80`
  - External upstream replay lane: `190 / 190`
  - Combined false-positive gauntlet: `270 / 270`
- The current headline utility benchmark is now the repeated April 18 package, not the older single-repeat strict run.
  - `core-current`, `39` tasks, `claude-default` and `codex-default`
  - `209 / 234` baseline -> `232 / 234` with `repair-loop-verify-only`
  - `89.3% -> 99.1%`
  - `+23` saved tasks
- Repair attribution remains clean.
  - `31` repair rounds were triggered by Court Jester `verify`
  - `0` repair rounds were triggered by public or hidden evaluator feedback
  - `29` verify-triggered repair rounds ended in final success
- The docs now include a dedicated benchmark methodology writeup and a fuller April 18 benchmark report.

## Product Changes Since 0.1.7

- TypeScript export-surface detection is stronger in the analyzer.
  - The analyzer now recognizes more exported TS function surfaces instead of falling back to helper fuzz on internal functions.
- Malformed URI handling no longer looks like a crash in verifier feedback.
  - Invalid URI inputs are treated as invalid-input rejections rather than verifier crashes.
- The broader known-good and replay lanes are now first-class benchmark assets.
  - Added upstream-derived gold replay patches across packaging, semver, lodash, `qs`, and fresh Express slices.
  - Expanded local known-good tasks for query-string, semver, feature-flag, and related semantic families.

## Benchmark Package

Validated for this release:

- local known-good control:
  - `python3 -m bench.run_matrix --task-set known-good-corpus --models noop --policies required-final --repeats 10 --schedule blocked-random --output-dir bench/results/matrix/2026-04-18-known-good-corpus-r10`
  - result: `80 / 80`
- external replay false-positive control:
  - `python3 -m bench.run_matrix --task-set external-known-good-replay --models noop --policies required-final --repeats 10 --schedule blocked-random --use-task-gold-patches --output-dir /tmp/cj-external-known-good-replay-r10-v2`
  - result: `190 / 190`
- repeated strict utility benchmark:
  - `python3 -m bench.run_matrix --task-set core-current --models claude-default,codex-default --policies baseline,repair-loop-verify-only --repeats 3 --schedule blocked-random --shuffle-seed 7 --output-dir bench/results/matrix/2026-04-18-core-current-r3-v2`
  - result: `209 / 234` baseline -> `232 / 234` verify-only repair loop
- targeted Rust coverage for the false-positive fixes:
  - `cargo test --test analyze_test --test verify_test`

## Docs

New and updated benchmark docs:

- [benchmark-2026-04-18.md](benchmark-2026-04-18.md)
- [benchmark-methodology.md](benchmark-methodology.md)
- [release-readiness-private-beta.md](release-readiness-private-beta.md)

## Known Limits

- The April 18 benchmark package is strong, but it still does not prove broad readiness on arbitrary external repos.
- The current repeated `core-current` rerun still finishes with two verify-only non-successes total:
  - one `verify_caught_hidden_bug`
  - one `hidden_semantic_miss`
- The next honest benchmark step is still broader repo-shaped utility, not just more micro-fixture repetition.
