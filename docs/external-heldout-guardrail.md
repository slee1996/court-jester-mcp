# External Held-Out Guardrail

This is the non-Terminal-Bench guardrail for synth work. It uses upstream-derived repo tasks rather than QuixBugs-style single-function fixtures.

The full matrix lane is still useful for end-to-end product scoring, but it is too slow and test-heavy for this specific synth guardrail. This guardrail intentionally uses direct no-test-file verify so it measures generated input domains and inferred properties.

## Task Sets

Current fresh gate:

- `bench/task_sets/external-heldout-synth-guardrail-v4.json`
- 4 tasks
- Mix: TypeScript local enums, imported enums, local `as const` tuple aliases, and imported `as const` tuple aliases.
- Purpose: measure whether synth walks source-derived closed input domains beyond direct literal annotations.

Saturated regression gates:

- `bench/task_sets/external-heldout-synth-guardrail-v3.json`
- 4 tasks
- Mix: Python `typing.Literal` params, nested Python `Literal` collection elements, TypeScript literal unions, and TypeScript object fields with literal domains.
- Status: saturated by the current synth stack. Keep it as a regression check.

- `bench/task_sets/external-heldout-synth-guardrail-v2.json`
- 8 tasks
- Mix: Express fresh-spec clone tasks plus unseen `qs` parse/stringify variants.
- Status: saturated by the current synth stack. Keep it as a regression check.

- `bench/task_sets/external-heldout-synth-guardrail.json`
- 9 tasks
- Mix: requests-style cookie rendering, packaging version/specifier behavior, node-semver max-satisfying behavior, `qs` deep collections, and lodash slice behavior.
- Status: saturated by the current synth stack. Keep it as a regression check.

v2 robustness note:

- `ts-express-fresh-response-headers-v2` was originally excluded because recursive framework aliases caused `court-jester verify` to stack-overflow before JSON.
- The TypeScript synth path now bounds recursive alias/object expansion, so this task is back in the current v2 gate.

## Commands

Current v4 buggy recall lane:

```sh
python3 -m bench.autoresearch_signature_contracts \
  --court-jester /Users/spencerlee/court-jester-mcp/target/release/court-jester \
  --task-set external-heldout-synth-guardrail-v4 \
  --output bench/results/autoresearch/heldout-external-v4/direct-buggy \
  --limit 20 \
  --report-level minimal \
  --timeout-seconds 25
```

Current v4 fixed-code false-positive lane:

```sh
python3 -m bench.autoresearch_signature_contracts \
  --court-jester /Users/spencerlee/court-jester-mcp/target/release/court-jester \
  --task-set external-heldout-synth-guardrail-v4 \
  --output bench/results/autoresearch/heldout-external-v4/direct-fixed \
  --limit 20 \
  --report-level minimal \
  --timeout-seconds 25 \
  --use-task-gold-patches
```

## Latest Result

Current v4 buggy direct synth:

- Ledger: `bench/results/autoresearch/heldout-external-v4/release-buggy/run-1777143099725452000/ledger.json`
- Result: 4 true positives, 0 misses, 0 false positives, 0 timeouts, 0 unscored
- True-positive source: typed input crashes reached through source-derived closed domains.
- Lift delta from baseline: before enum/const-tuple analysis, this gate had 2 true positives, 2 misses, and the fixed lane had 2 false positives caused by invalid `null` inputs for unresolved const-tuple aliases.
- Synth delta: TypeScript enum declarations are recorded as literal-union aliases; `typeof CONST_TUPLE[number]` aliases are rewritten from `as const` arrays, including imported type context.

Current v4 fixed gold-patch control:

- Ledger: `bench/results/autoresearch/heldout-external-v4/release-fixed/run-1777143099725596000/ledger.json`
- Result: 4 true negatives, 0 false positives, 0 timeouts, 0 unscored

Current v4 miss set:

- None. v4 is saturated and should become a regression check after the next fresh external gate is added.

Current v3 buggy direct synth:

- Ledger: `bench/results/autoresearch/heldout-external-v3/direct-buggy/run-1777142136669045000/ledger.json`
- Result: 4 true positives, 0 misses, 0 false positives, 0 timeouts, 0 unscored
- True-positive source: typed input crashes reached through signature-derived literal domains.
- Lift delta from baseline: before literal-domain synthesis, this gate had 1 true positive, 3 misses, and the fixed lane had 1 false positive caused by invalid `None`/`{}` domain generation.
- Synth delta: Python `Literal[...]` values and nested literal collection elements now shape fuzz inputs; TypeScript literal unions and literal object fields now generate declared branch values; closed literal-domain objects no longer receive broad `{}` object edge cases.

Current v3 fixed gold-patch control:

- Ledger: `bench/results/autoresearch/heldout-external-v3/direct-fixed/run-1777142136689521000/ledger.json`
- Result: 4 true negatives, 0 false positives, 0 timeouts, 0 unscored

Current v3 miss set:

- None.

Current v2 buggy direct synth:

- Ledger: `bench/results/autoresearch/heldout-external-v2/direct-buggy/run-1777142136693803000/ledger.json`
- Result: 8 true positives, 0 misses, 0 false positives, 0 timeouts, 0 unscored
- True-positive source: 5 mapping/query serializer semantics, 1 request metadata semantics, 1 response helper semantics, and 1 static file middleware semantics.
- Robustness delta: framework files that previously stack-overflowed now score cleanly because recursive TypeScript alias expansion is bounded.
- Context-synth delta: `WORKMAP.md`/`UPSTREAM_NOTES.md` hints now attach nested query, request metadata, response helper, and static middleware semantics to the right implementation files.

Current v2 fixed gold-patch control:

- Ledger: `bench/results/autoresearch/heldout-external-v2/direct-fixed/run-1777142136694726000/ledger.json`
- Result: 8 true negatives, 0 false positives, 0 timeouts, 0 unscored

Current v2 miss set:

- None.

Historical v1 buggy direct synth:

- Ledger: `bench/results/autoresearch/heldout-external/direct-buggy/run-1777142147331575000/ledger.json`
- Result: 9 true positives, 0 misses, 0 false positives, 0 timeouts
- Delta from the original direct run in this lane: +7 true positives from context-enabled `qs` parse/stringify semantics, exact-standard `sameValueZero`, context-enabled PEP 440 semantics, and cookie quote semantics from cookie source context.

Historical v1 fixed gold-patch control:

- Ledger: `bench/results/autoresearch/heldout-external/direct-fixed/run-1777142147335781000/ledger.json`
- Result: 9 true negatives, 0 false positives, 0 timeouts

## Rule

A Terminal-Bench-driven synth lift is not a win unless this gate stays sane:

1. Buggy recall should improve on the current fresh-gate miss set without degrading saturated gates.
2. Fixed-code false positives should remain zero, measured as gold-patched runs blocked by Court Jester verify.
3. Report the held-out result separately from Terminal-Bench. Do not blend the scores.
4. Do not claim a new synth win from v1, v2, v3, or v4 alone; saturated gates are regression checks.
5. Add a v5 fresh external gate before the next product-lift claim.
