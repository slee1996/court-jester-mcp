# Court Jester 0.1.7

Date: 2026-04-17

## Summary

This release strengthens TypeScript verifier coverage for Lodash-style `defaults` behavior, closes the last known mutation-recall miss in the seeded verify lane, and validates that both `claude-default` and `codex-default` can repair the object-shard mutation cleanly under the real repair loop.

## Highlights

- The TypeScript verifier now treats exported `defaults`-style APIs as a first-class semantic target.
  - Generic-target `defaults` functions are now fuzzable in the TypeScript harness.
  - The verifier checks the three object-merge behaviors that mattered in the surfaced miss:
    - preserve `null` target values
    - fill `undefined` target values
    - include inherited enumerable source keys
- This closes the `ts-lodash-object-slice-1-mutation` verifier gap.
  - The prior miss was a public-check failure that verify did not catch.
  - The new verifier catches that bug locally and still leaves the known-good object shard alone.
- Provider-backed repair behavior is now validated on that shard.
  - Both `claude-default` and `codex-default` successfully repaired the seeded Lodash object mutation under `repair-loop-verify-only`.

## Validation

Validated for this release:

- targeted Rust verifier coverage:
  - `cargo test --test synthesize_test --test verify_test`
  - `cargo test --test analyze_test`
- repeated local mutation/control validation:
  - `python3 -m bench.run_matrix --tasks ts-lodash-object-slice-1-mutation,ts-lodash-object-slice-1-known-good --models noop --policies advisory --repeats 5 --output-dir /tmp/cj-defaults-object-v2`
  - result: known-good `5 / 5`, mutation verify-caught `5 / 5`
- repeated full mutation-lane validation:
  - `python3 -m bench.run_matrix --task-set verify-mutation-seeds-v1 --models noop --policies advisory --repeats 2 --output-dir /tmp/cj-verify-mutation-seeds-v3`
  - result: verifier recall `14 / 14`, precision `1.00`
- provider-backed targeted matrix:
  - `python3 -m bench.run_matrix --tasks ts-lodash-object-slice-1-mutation,ts-lodash-object-slice-1-known-good --models claude-default,codex-default --policies repair-loop-verify-only --repeats 2 --schedule blocked-random --shuffle-seed 7 --output-dir /tmp/cj-defaults-provider-v1`
  - result: `8 / 8` successes
  - mutation shard: `claude-default 2 / 2`, `codex-default 2 / 2`
  - known-good shard: `claude-default 2 / 2`, `codex-default 2 / 2`

## Known Limits

- The benchmark harness still has a bookkeeping quirk on this provider slice.
  - Some per-run `run.json` files remained marked `status=running` even though the corresponding `result.json` files were present and the summarized matrix was complete.
  - This does not affect the scored result data, but it should be fixed separately in the harness.
- The new `defaults` semantic coverage is intentionally narrow.
  - It targets the surfaced Lodash-style object-default behavior rather than trying to generalize all object-merging APIs at once.
