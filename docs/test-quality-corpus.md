# Advisory test-quality classification corpus

Run `just test-quality-corpus`, or:

```sh
cargo build --locked --release --bin court-jester --example test_quality_validation
python3 -m bench.test_quality_corpus --binary target/release/court-jester \
  --validation-binary target/release/examples/test_quality_validation \
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

## Separate invalid-candidate tier

Invalid mutants are not interchangeable with blocked or skipped campaigns. The normal planner already filters candidates that fail application or syntax validation, so the ten runtime fixtures do not claim to exercise an emitted `invalid` outcome.

The `test_quality_validation` helper calls the same pre-execution validator as production verification. For each language it checks a valid planned edit, stale source text, an out-of-range edit, a split UTF-8 boundary, invalid syntax, and a renamed required function. These 12 fault-injection/control rows execute no mutants and remain separate from runtime counts. Production invalid observations now retain a typed `validation_kind` alongside their reason.

The checker requires the complete unique matrix, exact classifications, no execution, and matching verifier/helper hashes. The helper also records compiled source digests. Build both executables together as shown above: binary hashes identify artifacts, but do not independently prove that two executables were compiled from the same checkout. CI uses a single build invocation. `classification_evidence_complete` is true only when both tiers pass and the verifier remains unchanged. Omitting `--validation-binary` allows an explicitly incomplete runtime-only run; canonical quality/release gates always supply it. This bounded corpus does not prove general mutation adequacy or product usefulness.
