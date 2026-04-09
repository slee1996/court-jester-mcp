# Benchmark Gap Audit: March 26, 2026

This audit covers the four highest-signal tasks currently used for Court Jester product comparison:

1. `py-primary-plan-code-cross-file`
2. `py-query-string-canonicalization`
3. `ts-primary-plan-code-cross-file`
4. `ts-semver-max-stable-cross-file`

## Goal

Make the benchmark answer product questions about Court Jester itself:

1. Which bug classes does verify catch?
2. Which bug classes does repair-loop improve?
3. Where are task evaluators weaker than Court Jester?

## Audit outcome

### 1. `py-primary-plan-code-cross-file`

Status: evaluator and verifier are reasonably aligned.

Why:

1. public checks cover missing plans, empty plans, blank-first-entry, and null-first-entry behavior
2. hidden checks reinforce the same first-usable semantics
3. verify uses the same public-style file test and reliably surfaces the relevant failure

Primary bug class:

1. `cross_file_contract`

Secondary tags:

1. `nullish_handling`
2. `first_usable_selection`

### 2. `py-query-string-canonicalization`

Status: this task had a real evaluator gap and has now been tightened.

Original problem:

1. Court Jester caught real bugs involving nullish and non-scalar value leakage
2. the hidden evaluator only checked:
   - `None` inside flat lists
   - blank scalar values
   - accented scalar normalization
3. that allowed bad patches to pass hidden checks while still failing verify fuzzing

Fix applied:

1. hidden evaluator now also checks:
   - nested dict/nullish leakage
   - non-scalar list member filtering

Primary bug class:

1. `non_scalar_input_leak`

Secondary tags:

1. `typed_edge_case`
2. `nullish_handling`
3. `semantic_normalization`

Interpretation rule:

1. if verify still fails while public and hidden pass, that is now more likely to indicate a genuinely stronger verifier rather than a weak evaluator

### 3. `ts-primary-plan-code-cross-file`

Status: evaluator and verifier are reasonably aligned.

Why:

1. public checks cover missing plans, empty plans, blank-first-entry, and null-first-entry behavior
2. hidden checks reinforce the same first-usable semantics
3. verify attaches the public-style TS test to the main path and surfaces the right counterexamples

Primary bug class:

1. `cross_file_contract`

Secondary tags:

1. `nullish_handling`
2. `first_usable_selection`

### 4. `ts-semver-max-stable-cross-file`

Status: evaluator coverage is already stronger than the public checks and appropriate for hidden semantic verification.

Why:

1. public checks cover obvious stable-version behavior
2. hidden checks add:
   - prerelease filtering
   - `v` prefix normalization
   - build metadata stripping
   - randomized stable-version comparisons
3. this task still stresses model cross-file reasoning more than evaluator weakness

Primary bug class:

1. `semantic_normalization`

Secondary tags:

1. `cross_file_contract`

## Changes made from this audit

1. added explicit bug-class metadata to the four task manifests
2. changed the old `verify_false_positive` label to `verify_stronger_than_eval`
3. strengthened `py-query-string-canonicalization` hidden evaluation
4. added bug-class grouping to `bench.summarize_runs`

## Current benchmark guidance

Use bug classes, not individual models, as the primary unit of product analysis:

1. `cross_file_contract`
2. `non_scalar_input_leak`
3. `semantic_normalization`

Then ask:

1. does verify catch the bug class?
2. does repair-loop improve success on the bug class?
3. does Court Jester block bad patches even when the model cannot repair them?
