# Report Schema And Stability

Court Jester verify reports are intended to be machine-consumable. This document defines the current stability contract for `schema_version: 2`.

## Top-Level Contract

Full and minimal verify reports both expose these top-level keys:

- `schema_version`
- `overall_ok`
- `summary`
- `stages`
- `report_path` on direct CLI output
- `meta` on persisted reports written via `--output-dir`

Within a given `schema_version`, these keys are stable. Breaking changes to their meaning or shape require a schema bump.

## Stage Contract

Stage objects use this stable shape:

- `name`
- `ok`
- `duration_ms`
- `detail` when the stage has structured detail
- `error` when the stage failed or has an advisory warning

Current stage names in schema v2 are:

- `parse`
- `complexity`
- `lint`
- `coverage`
- `portability`
- `execute`
- `test`

Stage names are append-only within a schema version. Existing names will not be repurposed silently. If a future change needs to remove or fundamentally redefine a stage, the report schema must bump.

## Full vs Minimal

`--report-level full` keeps the complete stage detail payload, including raw parse output, stderr, and detailed fuzz artifacts.

`--report-level minimal` keeps the fields intended for CI and dashboards:

- top-level pass/fail and summary
- stage names, outcomes, and durations
- execute finding counts and failure lists
- complexity violations
- coverage counts
- portability reason, imports, and fix hint

If a field exists only in `full`, consumers should treat it as debug-only and not build hard CI dependencies on it.

## Execute Severity Contract

Execute findings currently use these categories:

- `crash`
- `property_violation`
- `no_inputs_reached`

Execute findings may also include an optional `classification` field when Court Jester can refine the result without changing the base severity. Current classifications include:

- `type_signature_wider_than_usage`

`no_inputs_reached` is diagnostic-only by default. It is reported in stage detail and summary counts, but it does not fail the execute stage unless a future gate explicitly chooses to do so.

`--execute-gate` controls which execute severities fail the run:

- `all`: fail on crash and property-violation findings
- `crash`: fail only on crash findings
- `none`: never fail on execute findings

The selected gate is recorded in the execute stage detail.

## Coverage Status Contract

Coverage detail reports per-function statuses such as:

- `fuzzed`
- `skipped_no_fuzzable_surface`
- `skipped_unsupported_type`
- `skipped_internal_helper`
- `skipped_method`
- `skipped_nested`
- `skipped_private_name`
- `skipped_diff_filtered`
- `blocked_module_load`

`skipped_no_fuzzable_surface` is used for zero-argument functions where Court Jester cannot derive a meaningful parameter surface or stable return contract to exercise.

## Complexity Contract

Complexity threshold reports include:

- `threshold`
- `metric`
- `violations`
- `suppressed_violations`
- `checked_functions`
- `diff_scoped`

`metric` is explicit so consumers know whether the gate used `cyclomatic` or `cognitive` complexity.

## Suppressions

When `--suppressions-file` is used, suppressed findings remain visible in the report:

- execute stage:
  - `suppressed_fuzz_failures`
  - `suppressed_finding_counts`
- complexity stage:
  - `suppressed_violations`
- portability stage:
  - `suppressed: true`

The suppression file path is echoed back as `suppression_source` where relevant.

## Seed Inputs

When auto-seeding is enabled, coverage and execute detail can include:

- `seed_input_count`
- `seeded_functions`
- `seed_sources`

Court Jester seeds fuzzing from:

- simple literal call sites in the source file
- explicit test files provided to `verify`
- conventional nearby test files when `--no-auto-seed` is not set

## Schema Bump Rules

These changes require a new `schema_version`:

- removing or renaming an existing top-level key
- removing or renaming an existing stage
- changing the type of an existing stable field
- changing the meaning of `overall_ok` or a stable severity class

These changes do not require a schema bump:

- adding a new stage
- adding new fields to `detail`
- adding new summary counters
- adding new optional execute or portability reason strings
