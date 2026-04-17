# Court Jester 0.1.6

Date: 2026-04-17

## Summary

This release closes the last known `core-current` regression on `main`, strengthens semver-caret verification, and adds a first dedicated mutation-recall benchmark lane for measuring verifier bug-detection coverage directly.

## Highlights

- The semver-caret verifier now covers stable-base same-core prerelease cases.
  - This closes the missed case behind the prior `ts-semver-caret-prerelease` regression.
  - The merged-main `core-current × claude-default × repair-loop-verify-only × repeats=2` validation slice now finishes cleanly.
- A new verify-mutation benchmark lane is in the repo.
  - [`bench/task_sets/verify-mutation-seeds-v1.json`](../bench/task_sets/verify-mutation-seeds-v1.json) seeds real existing repo bugs into known-good base fixtures.
  - [`bench/materialize_mutation.py`](../bench/materialize_mutation.py) materializes those mutations during task setup instead of inventing synthetic one-off fixtures by hand.
  - [`bench/summarize_runs.py`](../bench/summarize_runs.py) now reports verify classifier metrics such as recall, specificity, and expected-failure-kind hit rate.
- New fixed-base micro fixtures are included for query-string, feature-flag, semver-compare, and semver-caret mutation seeding.
  - These are benchmark assets, not product runtime changes.

## Validation

Validated for this release:

- targeted Rust verifier coverage:
  - `cargo test --test synthesize_test --test verify_test`
- Python benchmark harness unit tests:
  - `python3 -m unittest bench.test_summarize_runs bench.test_materialize_mutation bench.test_run_matrix`
- local mutation-recall sanity run:
  - `python3 -m bench.run_matrix --task-set verify-mutation-seeds-v1 --models noop --policies advisory --output-dir /tmp/cj-verify-mutation-seeds-v1`
- merged-main headline validation slice:
  - `python3 -m bench.run_matrix --task-set core-current --models claude-default --policies repair-loop-verify-only --repeats 2 --schedule blocked-random --shuffle-seed 7 --output-dir bench/results/matrix/2026-04-17-main-merge-core-current-claude-candidate-v2`
  - result: `78 / 78` successes

## Known Limits

- The new mutation lane is useful but not yet a release gate.
  - Current local `noop + advisory` mutation recall is `6 / 7`.
  - The surfaced miss is `ts-lodash-object-slice-1-mutation`, which should be the first follow-up verifier target after this release.
- The current strong validation slice is still Claude-only.
  - A broader cross-model release bar should still include more runs on `codex-default` and additional control suites.
