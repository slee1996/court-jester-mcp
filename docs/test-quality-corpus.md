# Advisory test-quality classification corpus

Run `just test-quality-corpus`, or:

```sh
python3 -m bench.test_quality_corpus --binary target/release/court-jester \
  --output target/test-quality-corpus.json
```

The runtime manifest is `bench/test_quality_cases.json`. Each case creates a fresh temporary project and runs the actual CLI with one bounded mutant and a clean authoritative baseline. Python and Node/TypeScript each exercise five contracts:

| Contract | Independent fixture expectation |
| --- | --- |
| Killed | A boundary assertion fails after a comparison mutation; the mutated function was entered. |
| Reached survivor | The test calls the boundary but does not assert its result; the entered mutant survives. |
| Blocked | Only the mutant attempts a sandbox-denied child process; this is infrastructure/policy evidence, not a kill. |
| No coverage | A fixture-owned campaign-lifetime marker allows the baseline call but skips the later mutant call; success without entry is not survival. |
| Coupling | A test reads a target-private constant and also kills the boundary mutant; coupling remains a separate advisory observation. |

Expected outcomes are declared before execution. The checker requires clean baseline eligibility, exact per-mutant entry evidence, matching outcome/count conservation, and independent coupling classification. It rejects nonzero baseline exits, scores/grades/percentages, planning abstentions, and mismatched counters. All ten cases must match for the runtime artifact to pass. This is a deterministic classification regression corpus, not a mutation score, general test-adequacy estimate, or claim about real-world repair usefulness.

Artifacts include the exact binary and manifest SHA-256 digests, per-case reports/errors and duration, and a post-run binary-identity check. `--output` refuses to overwrite an existing file. The quality workflow (also reused by release) runs the corpus and uploads its evidence. Hosted success must be checked separately from local results.

The corpus exposed Node TAP moving target-entry records and child diagnostics into `# ...` comment envelopes. Decoding now follows the selected test adapter: raw JSON remains raw, Node TAP comments are decoded for Node, and non-target policy/resource blockers take precedence over a generic failed TAP test. The recorded process streams remain unchanged.

## Remaining invalid-candidate tier

Invalid mutants are not interchangeable with blocked or skipped campaigns. The normal planner already filters candidates that fail application or syntax validation, so the ten runtime fixtures do not claim to exercise an emitted `invalid` outcome. A separate, explicitly fault-injected validation-boundary tier is still required to prove invalid-candidate handling for both languages. The broader P7 corpus acceptance remains unfinished until that tier has reproducible evidence; these ten passing cases do not substitute for it.
