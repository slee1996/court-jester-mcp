# Terminal-Bench Slice Process

Terminal-Bench is now part of the Court Jester autoresearch loop as a sliced stress gate. The first integrated dataset is the Python QuixBugs subset from Terminal-Bench.

## Command

```sh
python3 -m bench.autoresearch_terminal_bench --limit 40
```

Latest run:

- Ledger: `bench/results/autoresearch/terminal-bench-quixbugs/run-1777142162216006000/ledger.json`
- Slices: `bench/results/autoresearch/terminal-bench-quixbugs/run-1777142162216006000/slices.json`

## Slices

The loop now writes per-task slice metadata and per-iteration slice counts. Current slices include:

- `primitive_numeric_inputs`
- `collection_inputs`
- `nested_collection`
- `generator_like`
- `json_oracle`
- `no_json_oracle`
- `object_or_graph_fixture`
- `order_or_search_name`
- `arity_1`, `arity_2`, `arity_3`
- `low_arity`, `multi_arg`
- `scalar_expected`, `collection_expected`
- `typed_or_annotated`, `untyped_signature`

## Latest Signal

Generic fuzz is now a useful precision gate after domain-shaped seed synthesis:

- Original baseline: 40 fixed-still-fails out of 40
- Current run after JSON fixture input discovery, seed-shaped fuzzing, and conservative structural fixture inference: 13 clean true positives, 24 miss-buggy-passes, 3 fixed-still-fails

Raw oracle tests are useful but have representation noise:

- `tests_only_raw`: 25 clean true positives, 6 fixed-still-fails, 9 skipped no-JSON

Normalized oracle tests are the best Terminal-Bench quality gate:

- `tests_only_normalized`: 28 clean true positives, 3 fixed-still-fails, 9 skipped no-JSON

Important normalized slice results:

- `primitive_numeric_inputs`: 8 clean true positives, 1 fixed-still-fails
- `collection_inputs`: 16 clean true positives, 1 fixed-still-fails
- `nested_collection`: 5 clean true positives, 1 fixed-still-fails
- `generator_like`: 2 clean true positives
- `arity_1`: 15 clean true positives, 4 skipped no-JSON
- `arity_2`: 11 clean true positives, 3 fixed-still-fails, 4 skipped no-JSON
- `no_json_oracle` / `object_or_graph_fixture`: 9 skipped no-JSON

Important generic-fuzz slice results:

- `json_oracle`: 13 clean true positives, 15 miss-buggy-passes, 3 fixed-still-fails
- `primitive_numeric_inputs`: 4 clean true positives, 5 miss-buggy-passes
- `collection_inputs`: 9 clean true positives, 6 miss-buggy-passes, 2 fixed-still-fails
- `nested_collection`: 2 clean true positives, 3 miss-buggy-passes, 1 fixed-still-fails
- `generator_like`: 1 clean true positive, 1 miss-buggy-passes
- `no_json_oracle` / `object_or_graph_fixture`: 9 miss-buggy-passes

## Anti-Overfit Guardrails

Terminal-Bench should propose synth hypotheses, not define the product target. A CJ lift should clear these gates before it counts as a real win:

1. Structural properties inferred from fixtures need repeated nontrivial support. A single example can seed input shape, but it cannot create a hard correctness property.
2. Fixture outputs can only become general properties such as sorted, permutation, nonnegative, or palindrome. They must not become exact-output oracles in generic fuzz mode.
3. Every Terminal-Bench-driven change needs a non-Terminal-Bench known-good pass. Current control run: `bench/results/autoresearch/signature-contracts/anti-overfit-support-threshold/run-1777129248524948000/ledger.json`, with 15 true positives, 8 true negatives, and 0 false positives.
4. Run the external held-out synth guardrail in `docs/external-heldout-guardrail.md`. It uses upstream-derived repo tasks, not QuixBugs-shaped fixtures.
5. Report slice deltas separately. A lift that only improves one Terminal-Bench slice while increasing fixed-still-fails or false positives elsewhere is suspect.

## How To Use This

For synth work, treat generic Terminal-Bench fuzz as a precision and domain-inference stress test. Fixed-still-fails means generated inputs are outside the task contract or the harness is asserting the wrong property. Miss-buggy-passes is acceptable when CJ has input-domain evidence but no correctness oracle or high-confidence invariant.

Use normalized Terminal-Bench slices as regression checks for fixture importers and representation normalization. The no-JSON/object-graph slice is the next useful frontier because it currently has no importer path.

The process should be:

1. Run repo signature-contract autoresearch for CJ-native product signal.
2. Run Terminal-Bench normalized slices for external benchmark stress.
3. Compare slice counts, not just aggregate pass/fail.
4. Investigate any slice where fixed-still-fails increases.
