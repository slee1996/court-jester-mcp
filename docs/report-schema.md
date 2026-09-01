# Report Schema And Stability

Court Jester reports are machine-consumable contracts. The active report schema is **v3** (`schema_version: 3`). A consumer MUST branch on the typed `verdict` and `strength` fields; a missing, unknown, or older schema is not a successful verification.

## Top-level report

Full and minimal `verify` reports contain:

- `schema_version`: integer `3`.
- `verdict`: `pass`, `fail`, or `inconclusive`.
- `strength`: `none`, `parse_only`, `static_checked`, `runtime_smoke`, `property_checked`, or `authoritative_tests`.
- `summary`: aggregate function, finding, lint, complexity, and coverage counts.
- `stages`: ordered stage records.
- `report_path`: present on direct output when a report was persisted.
- persisted reports additionally contain `meta` (`source_file`, `language`, `timestamp`, and `duration_ms`).

The v3 contract is a clean cutover: consumers must not infer a verdict from a legacy boolean field or compatibility alias.

### Verdict semantics

Verdict precedence is fail > inconclusive > pass:

- `fail`: syntax/complexity gate failure, a gated finding, or an authoritative test failure.
- `inconclusive`: no valid behavioral evidence, a required coverage gap, unsupported or blocked module loading, rejected/all-invalid inputs, timeout or resource kill, or an infrastructure condition that prevents a trustworthy check.
- `pass`: the applicable gates completed, required surfaces were behaviorally checked, and no gated finding or test failure occurred.

An advisory finding or advisory lint/portability warning remains visible without changing a pass to a fail. `--inferred-oracle-gate fail` promotes low-confidence inferred semantic findings to the gate; the default is `advisory`.

Strength describes evidence, not the verdict: a parse failure can be `parse_only`; completed static checks can be `static_checked`; a valid invocation without an evaluated oracle is `runtime_smoke`; an evaluated runtime/type/declared/generic oracle is `property_checked`; and a completed authoritative test is `authoritative_tests`.

## Stage contract

Each stage has this shape:

```json
{
  "name": "execute",
  "status": "passed",
  "duration_ms": 42,
  "detail": {},
  "message": "optional human-readable context"
}
```

`detail` and `message` are optional. `status` is one of `passed`, `failed`, `inconclusive`, `advisory`, or `skipped`. Current stage names are:

- `parse`
- `complexity`
- `lint`
- `coverage`
- `portability`
- `execute`
- `test`
- `test_quality` (present when the stable advisory opt-in is requested)

Lint is advisory by design. Portability is advisory after a successful repository-native fallback and inconclusive when loading prevents behavioral execution.

### Stable advisory test-quality detail

`--test-quality [N]` adds the non-gating `test_quality` stage after authoritative baseline-test and coverage eligibility are established. Direct `verify` requires exactly one `--test-file`. In `ci`, `--test-file` is repeatable with at most one Python and one TypeScript/TSX entrypoint; the matching entrypoint is selected for each target language. The default budget is 8 and the valid range is 1 through 32.

Full stage detail has these stable fields:

```json
{
  "experimental": false,
  "mode": "advisory",
  "max_mutants": 8,
  "baseline_eligible": true,
  "counts": {
    "planned": 4,
    "killed": 3,
    "survived": 1,
    "invalid": 0,
    "blocked": 0,
    "no_coverage": 0
  },
  "mutants": [],
  "coupling_findings": [],
  "planning_error": null,
  "coupling_error": null
}
```

- `experimental` remains in schema v3 and is always `false` for the stable feature.
- `mode` is always `advisory`.
- `max_mutants` is the direct campaign cap or, in a CI per-file stage, that file's deterministic share of the global cap. The CI aggregate exposes the configured global cap separately.
- `baseline_eligible` states whether the original authoritative test passed, entered every required surface, and used supported instrumentation.
- `counts` contains bounded `planned`, `killed`, `survived`, `invalid`, `blocked`, and `no_coverage` observations.
- `mutants` contains per-candidate mutation identity and location, operator, original and replacement source, behavioral witness, outcome, exact target-entry evidence, test status, process diagnostics, duration, and bounded failure excerpt.
- `coupling_findings` contains only target-resolved `private_target_access`, `private_target_import`, `private_target_spy`, and `target_source_introspection` evidence. Each finding includes the normalized authoritative `test_source_file`; its line, column, symbol, evidence, and message are attributable to that test file. This additive provenance field is authoritative even when the same target is analyzed from multiple test entrypoints. Unrelated private-looking members, blanket mock usage, and text matches without target binding are outside this contract. Coupling evidence is independent of mutation outcomes.
- `planning_error` and `coupling_error` are nullable diagnostics. They never become a kill or a verifier failure.

Outcome meanings are deliberately asymmetric:

- `killed` requires an eligible baseline, a valid mutant, exact entry into the mutated public surface, and authoritative-test failure. A target-originated assertion failure or exception may be a kill; infrastructure failure may not.
- `survived` requires the same eligibility, validity, and exact entry, followed by authoritative-test success. It identifies one reached behavioral distinction the test did not detect, not a globally weak test.
- `invalid` means the mutation could not be applied, parsed, or validated while preserving the required callable surface.
- `blocked` means runner, instrumentation, timeout, memory, sandbox, or other non-target infrastructure prevented judgment.
- `no_coverage` means the valid mutant ran without exact entry into its mutated surface.

Only `killed` and `survived` are judged outcomes. `invalid`, `blocked`, and `no_coverage` are unjudged and must never be folded into kills or treated as negative test results. Coupling findings remain separate because mutation sensitivity cannot establish implementation independence.

The stage status is informational. `passed` means every planned mutant was killed with no coupling, unjudged outcome, mutation-planning diagnostic, or baseline infrastructure blocker. `advisory` means at least one survivor, coupling finding, or unjudged outcome exists, or mutation planning produced a diagnostic, or a non-target authoritative baseline infrastructure blocker prevented the campaign; `advisory` remains reachable even when every mutant count and the coupling count are zero. `skipped` is reserved for a blocker-free inability to run a campaign with no coupling finding, including no eligible candidates, no share of the global CI budget, or no matching authoritative test entrypoint. None of these statuses changes top-level `verdict`, `strength`, process exit status, or CI gates. The schema deliberately provides no quality score, percentage, grade, threshold, or synthetic pass/fail field.

## Coverage contract

The default `--coverage-gate changed-exports` requires every changed exported/invocable surface when a diff is supplied, and every exported/invocable surface otherwise. If a file has no exports, selected top-level callables are the fallback. `--coverage-gate none` disables per-surface enforcement but does not manufacture a pass when there is zero valid behavioral evidence and no authoritative test.

Coverage entries use typed statuses:

- `checked_direct`
- `reached_via_factory`
- `checked_via_factory`
- `checked_via_caller`
- `checked_via_authoritative_test`
- `skipped_no_fuzzable_surface`
- `skipped_unsupported_type`
- `skipped_internal_helper`
- `skipped_method`
- `skipped_nested`
- `skipped_private_name`
- `skipped_diff_filtered`
- `blocked_module_load`

Every entry also has `required`, `invocation_path`, and an optional `reason`. A factory/caller reach event alone is not behavioral checking. `CoverageSummary` reports `required`, `behaviorally_checked`, `reached_only`, `no_inputs_reached`, `skipped`, and `blocked`.

In `--tests-only`, the supplied test process is the sole behavioral gate: each required surface must emit its matching entry event. A passing test that does not cover all required surfaces is inconclusive.

## Findings and repros

Execute detail contains typed `findings`, `suppressed_findings`, and `findings_summary`; it does not contain the obsolete `fuzz_failures` arrays. A finding includes:

- `id`, `severity`, `confidence`, `category`, `location`, `oracle`, and `input_classification`;
- a structured `repro` with kind, arguments, minimized expressions, snippet, expectation, and (when persisted) replay command;
- `minimization` with `status`, `attempts`, `original`, and an optional reconfirmed `minimized` case;
- optional `error_type`, `classification`, `suggestion`, and `suppressed`.

Enums are serialized in snake case. Severities are `crash`, `property_violation`, `behavioral_regression`, and `infrastructure`; confidences are `authoritative`, `high`, `medium`, and `low`; categories are `exception`, `property`, `test`, `differential`, and `infrastructure`. Oracle kinds are `authoritative_test`, `runtime_contract`, `type_contract`, `declared_property`, `seed_regression`, `differential`, `generic_property`, and `inferred_semantic`.

Every finding has a machine-verifiable replay snippet. The snippet emits exactly one `__COURT_JESTER_REPLAY_JSON__` sentinel followed by `{reproduced,severity,oracle_kind,category}`. A persisted report gets `court-jester replay --report <report_path> --finding <id>` in the repro; direct stdout reports keep `command: null`.

Differential repros embed base/candidate local source closures and a dependency contract. A base/candidate behavior difference is advisory unless an authoritative fixture, test, or declared contract proves the candidate side wrong. Replay requires matching third-party/runtime dependencies; missing or mismatched dependencies are inconclusive.

## Full, minimal, and repair output

`--report-level full` retains complete stage detail, structured findings, repros, and execution diagnostics. `--report-level minimal` keeps the verdict/strength, stage statuses and durations, actionable findings/repros, summary counts, and coverage summary while omitting verbose parse and harness detail. For `test_quality`, both levels retain the stable detail fields listed above, including per-mutant evidence, coupling findings, and nullable planning/coupling errors. Consumers MUST use values, not field presence, to make gate decisions.

`--summary repair-json` emits only:

```json
{
  "schema_version": 3,
  "verdict": "inconclusive",
  "strength": "static_checked",
  "recommended_action": "add_contract_or_test",
  "primary_finding": null,
  "findings": [],
  "coverage": {}
}
```

`recommended_action` is exactly `repair` for `fail`, `inspect_environment` for infrastructure inconclusive, `add_contract_or_test` for coverage/no-input inconclusive, and `none` for `pass`. Suppressed findings cannot become `primary_finding`.

## CI and exit codes

`court-jester ci` defaults to `parse,lint,coverage,portability,execute,test`; `--gate` selects a comma-separated subset or `all`. Aggregate and per-file results use typed verdicts, with fail taking precedence over inconclusive, then pass. CI JSON uses the same typed contract as `verify`. When test quality is requested, every file report retains its own `test_quality` stage and the CI object adds this exact aggregate derived from those stages:

```json
{
  "test_quality": {
    "max_mutants": 8,
    "planned": 8,
    "killed": 6,
    "survived": 1,
    "invalid": 0,
    "blocked": 1,
    "no_coverage": 0,
    "unjudged": 1,
    "coupling": 2
  }
}
```

`max_mutants` is the configured global per-command budget. `planned`, `killed`, `survived`, `invalid`, `blocked`, `no_coverage`, and `coupling` are sums of the corresponding per-file evidence; `unjudged` is the derived sum `invalid + blocked + no_coverage`. The separate unjudged outcome totals are required in machine-readable JSON so consumers can distinguish mutation validity, infrastructure, and coverage problems. Candidate allocation is deterministic across files and required surfaces, aggregate `planned` never exceeds `max_mutants`, and valid underfilling remains explicit when candidates or matching authoritative tests are unavailable. Human and GitHub summaries may collapse `invalid`, `blocked`, and `no_coverage` into a single `unjudged` count and render planned/killed/survived/unjudged/coupling without a score. The quality aggregate and per-file quality stages are always advisory and cannot affect CI verdict or selected gates.

For `verify` and `ci`:

- `0`: pass;
- `1`: fail;
- `2`: usage/setup error before a report exists;
- `3`: inconclusive report.

`execute`, `doctor`, and `replay` preserve their own command contracts. `doctor` returns a schema-v3 readiness report and exits `0` for pass, `1` for failed readiness, and `2` for usage. `replay` returns `0` for `reproduced`, `1` for `not_reproduced`, `2` for invalid report/id/arguments, and `3` for inconclusive runtime/dependency/protocol conditions.

## Runtime profiles

`--runtime-profile local-trusted` (the default) runs the existing host subprocess path and is not a security boundary. `--runtime-profile isolated` uses Docker with the selected language image (`python:3.12-slim` or `node:24-bookworm-slim` by default), no network, read-only mounts/root filesystem, bounded CPU/memory/processes, and cleanup on every path. `--python-docker-image` and `--typescript-docker-image` are valid only with `isolated`; timeout and memory values must be finite/positive. Docker/image/daemon failures are structured inconclusive results and never fall back to local execution.

`court-jester doctor --language python|typescript|all` checks the selected runtime profile, images, and optional linters. Its JSON includes `schema_version`, `verdict`, `runtime_profile`, and typed `checks`.

## Stability and migration

Schema v3 is a clean cutover. Consumers MUST reject any report whose `schema_version` is not `3`; they MUST NOT infer a verdict from legacy boolean fields or from stage names. Adding optional detail fields or counters is compatible. Removing/renaming a stable v3 field, changing an enum, or changing the meaning of a verdict/strength requires a future schema bump.

Benchmark artifacts from earlier releases remain historical evidence only. They are not silently converted into v3 reports or artifact-v1 benchmark inputs.

## Benchmark artifact boundary

The Python benchmark has its own immutable `artifact_schema_version: 1`; every `matrix.json`, `run.json`, `result.json`, summary, and evidence manifest also records `verify_schema_version_required: 3`. Missing, mixed, or mismatched versions are abstentions and are excluded from semantic gates unless an explicit legacy mode labels them as historical. Evidence bundles are checksummed and redaction-aware. Optional shadow JSONL records carry `blocking_mode: shadow` and never alter task success. Gate decisions are emitted separately as `none`, `private-beta-default`, or `strict-heldout` and are ineligible when required cells or metrics are missing.

This section defines the format of artifacts generated or supplied to the general benchmark harness. It does not claim that a test-quality-specific matrix, classification oracle, result set, or evidence bundle is committed, and none of those artifacts gates the advisory 0.2.16 feature.
