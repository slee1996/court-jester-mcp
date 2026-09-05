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

Generated Python and TypeScript campaigns preserve admission evidence for planned rows validated against closed parameter domains. An exception from the initial target call on such a row is a target exception, including custom exception classes; it is not silently discarded because of its exception name. Predicate-derived seeds explore branches, including rejection branches, and their provenance alone never establishes admission. Input mutation does not inherit admission evidence, and shrinking an admitted exception must retain an admitted input. Open-ended type annotations alone do not establish an exception-free domain. These admission checks are not a general proof of a function's full contract.

Direct Python and TypeScript exceptions not classified by the current target-exception rules are retained as low-confidence observations with `input_classification: unknown`. Strict inferred-oracle gating does not turn these observations into proven failures. Without resolving authoritative evidence, verification is inconclusive and recommends adding a contract or test; an independently admitted target failure still gates. Arguments outside a finite declared domain can be classified as rejected. These domain checks derive from the function signature, even when no seed rows exist, and use bound argument slots rather than raw signature positions. TypeScript `undefined` is a distinct expression-valued literal, not JSON `null`; explicit unions and optional parameter domains retain it during planning and rejection checks.

An observed unknown exception closes its invocation with `unclassified_exception`, contributing to `unknown_completed`, not valid-invocation or oracle-check credit. Direct and factory campaigns do not infer input validity from exception classes or engine-like diagnostic messages. The direct classifier requires admission evidence or a generated property failure observed during evaluation. Factory action sequences currently lack a lifecycle admission contract: their exceptions remain low-confidence/unknown observations, can be replayed within the supported sequence contract, and cannot be exported as passing-regression expectations even with `--accept-inferred`. This is not a complete general exception-contract model; zero-argument calls and broader application state still need an explicit expectation model.

Direct Python and TypeScript property violations carry a generated runtime type rather than being identified by copied diagnostic wording. Campaign classification and property replay require that evidence; target exceptions from initial or repeat calls do not become property violations solely because their text or constructor name matches. Python property authority is attached to the actual failed oracle, not the presence of an unrelated property declaration. Independent TypeScript semantic campaigns are not suppressed by the direct fuzz campaign's verdict. Their inferred findings remain advisory by default.

Optional native-engine exception records use `observed_call` provenance. Version-2 records may gain input admission only from an unambiguous planned surface with closed parameter domains and successfully bound, matching snapshot values. Open domains, missing JSON values, legacy records, and unsupported variadic bindings remain unknown/low-confidence. Native stage counts distinguish admitted gating failures from unknown observations; an engine finding count alone does not prove a bug. No minimized case is claimed. Version-2 native records preserve pre-invocation argument expressions for the native decoder's supported runtime types, with optional faithful JSON values. Their persisted replay invokes current source with the recorded binding and compares exception type and full message (or exact supported JavaScript primitive throws). Successful invocation supplies `check_passed: true`, not input-admission proof. Legacy records and unsupported thrown values abstain. Unknown-input observations cannot be exported even with inference acceptance. Raw native-stage IDs are diagnostic identifiers; the aggregate execute findings contain canonical report IDs. Native minimization and broader admission coverage remain unfinished capabilities, not implied by an engine's coverage-guided search.

TypeScript primitive thrown values replay using exact value comparison, preserving `undefined`, `null`, `NaN`, negative zero, strings, booleans, numbers, and bigint. Unsupported runtime-only thrown objects, symbols, and functions produce inconclusive replay, not a false `not_reproduced` result. This is observation replay, not proof that an unknown-input exception is a product defect.

Direct Python and TypeScript exception replay requires the complete exception message, not only its prefix before a colon. Direct shrinking uses the same failure-identity rule. Generated property failures retain their typed oracle identity so changing diagnostic values while shrinking does not invent a different oracle. TypeScript unknown exceptions raised during repeated property calls persist the evaluator as well as the target invocation; replay does not replace them with a single initial call. Primitive identities distinguish negative zero from zero and preserve full strings. Runtime-only thrown values have no persistent identity and explicitly abstain; two unavailable identities cannot justify a preserved shrink.

Direct Python campaign, minimization, and persisted replay share the generated signature-aware invocation and selected property evaluator. Repeated and transformed calls preserve keyword-only binding. Replay checks the execution phase and failure identity; a different exception message of the same class does not by itself reproduce the recorded observation. Replay and minimization do not add campaign oracle credit. Python executable input expressions preserve nonfinite floats and supported built-in containers; runtime-only, cyclic, or excessively deep inputs produce inconclusive replay instead of claiming a successful reproduction. Stateful/factory and paired-function replay retain separate limitations.

A run in which a surface rejects every generated input has an inconclusive execute stage, even when its harness exits successfully. A pass still requires the repository's ordinary tests and review before shipping.

Inferred Python and TypeScript encode/decode-style pairs now persist the actual two-call round trip and its comparison, not an executable call to the pair's display label. Replay preserves the input before encoder mutation and distinguishes an encoder exception from a decoder exception. Python keyword-only parameters retain their binding. Pair mismatches remain low-confidence inferred observations; exceptions carry unknown input classification and are not promoted by strict inferred gating. Reproduction of a guessed round-trip property does not make it an authoritative API requirement. The shared evaluators preserve the supported language-specific comparison behavior; arbitrary runtime-object and asynchronous composition remain outside this contract.

The built-in Python query-string, PEP440, and cookie semantic campaigns share one observer between campaign and replay. They retain the original case arguments and compared observation, preserve keyword-only binding, and distinguish invocation, projection, and comparison failures. Exceptions require the same phase, class name, and complete message. Comparison errors remain recorded observations instead of escaping the harness. Their `oracle.expected` string contains JSON-encoded expected data; executable inputs and expectations use the bounded expression renderer. Python findings without an explicit invocation replay contract abstain rather than guessing a target call from display text. These findings remain low-confidence inferred evidence under the existing gate policy. `not_reproduced` means that particular stored observation did not recur, not that every behavior of the edited implementation is correct; run verification and repository tests again.

TypeScript query serialization/parsing, semver comparison/caret, SameValueZero, feature-flag, defaults, HTTP request/response helper, and static-middleware campaigns use the same per-case observation during campaign and replay. Original inputs survive target mutation, and `oracle.expected` contains JSON-encoded expected data. Replay distinguishes invocation, projection, and comparison errors, preserves exception identity, and scopes helper definitions separately from the target module. HTTP response projections additionally identify method steps, so the same exception moving between repeated method calls does not reproduce the original observation. Runtime-only thrown values explicitly abstain. These observations remain low-confidence inferred evidence.

The static-middleware observation reruns the factory with the project's `static` root, invokes its returned handler with a fresh GET request and response fixture, and compares response completion, fall-through, and body. It distinguishes factory failures from handler failures. The existing known-file expectation requires `static/hello.txt` containing `hello world` followed by a newline; replay does not create or replace project files. This synchronous fixture is not a general asynchronous middleware or arbitrary factory/action replay contract.

Deterministic generated fixtures can provide an input-construction recipe. Campaign and replay each construct fresh arguments from that recipe, preserving behavior such as inherited properties and Map-valued HTTP settings without a JSON round trip. Recipe-backed argument expressions are executable; their optional `json_value` is omitted to prevent lossy corpus reuse. Replay constructs the complete argument vector together. For HTTP decorators, it reruns the target to install methods on the fresh object, then executes the stored observation callback against that object. This supports the bundled defaults and HTTP fixtures, not arbitrary serialization or cloning of user runtime objects or captured closures.

TypeScript related-call observations use `repro.kind: semantic_case`. Each entry in `repro.arguments` represents one complete argument vector, in invocation order. For feature-flag null fallback checks, replay reruns both the default and null-input calls and compares their boolean results; it does not freeze the original default. An exception must recur at the same call index as well as match phase and failure identity. Explicit-false checks remain ordinary single-call observations.

Python generated factory sequences also use `semantic_case`, with one structured argument containing `factory: {args, kwargs}` and ordered `actions`. Each action retains its name, positional and keyword inputs, and whether resolution produced a callable. Replay reconstructs the factory once and repeats the recorded actions through the failure. It requires the same resolution/invocation phase, action index, exception class name, and complete message; changed callable availability invalidates the sequence. The clipped case label is display-only. Unsupported runtime-only inputs or input-construction failures abstain from replay.

TypeScript factory sequences retain one executable case expression containing the factory argument vector and ordered action records (`action`, `args`, and `callable`). Inputs are rendered before the target can mutate them; optional JSON values are omitted. Campaign and replay share action resolution, and replay preserves the receiver via `apply`. Factory, resolution, and action phases are distinct; exceptions require the same action index and full identity, including exact primitive values. Unclassified factory exceptions remain low-confidence, unknown-input observations. Runtime-only throws and inputs that cannot be represented faithfully yield inconclusive replay. This bounded input transport does not serialize arbitrary prototypes, descriptors, shared references, or closures. Factory input-admission and lifecycle accounting, and the broader replay/export workflow, remain separate unfinished work.

For Python values outside JSON's representable domain (including strings with lone Unicode surrogates), the optional `json_value` is unavailable rather than invalid JSON. Such rows are excluded from the JSON corpus, and diagnostic text escapes unsupported characters so reporting itself does not crash. The separate Python expression remains the repro representation for these values.

Python named classes retain nominal identity as an `instance` domain with a name and fields. They are not structural JSON objects and do not acquire closed-domain admission from their fields. Direct `TypedDict` declarations retain structural object domains. JSON input validation checks nested collection elements and required object fields; incompatible saved Python rows are skipped before synthesis. This does not establish support for every constructor shape or make runtime instances replayable.

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
- `configuration` (invalid library-supplied configuration prevents verification)

Invalid suppression data produces an inconclusive `configuration` stage before source/test execution. Its typed diagnostic has domain `environment`, kind `invalid_configuration`, component `configuration`, and impact `blocking`; `configuration_kind` identifies `suppressions`. CLI callers receive a usage error instead, using the same validation schema.

Lint is advisory by design. Portability is advisory after a successful repository-native fallback and inconclusive when loading prevents behavioral execution.

Docker control-plane failures (including create/start/wait/state/log collection and unconfirmed cleanup) remain blocking infrastructure evidence, not target exceptions. Container completion requires successful state inspection with an explicit stopped state, exit code, and OOM flag. Managed output collection shares the process deadline even when the parent has exited and descendants still hold the pipes; bytes already captured are retained on timeout.

Managed host commands own their process group, output collectors, and memory monitor for the execution future's lifetime. Dropping/cancelling that future terminates its group and aborts its auxiliary tasks; ordinary completion also terminates remaining background members of that group. A cancelled future does not produce a successful execution report.

An isolated lifecycle worker owns its container and tool-generated workspace, guard, and resolver leases through teardown. Caller cancellation signals that worker instead of dropping an in-flight create/start request. Those control commands have a ten-second ceiling within the remaining launch budget; after they return, cancellation prevents further normal work and triggers bounded exact-name removal. Wait/state/log operations are interruptible, but cleanup itself is not cancelled by the caller. Cleanup failures name the container on stderr and, when a caller remains, in the blocking execution result.

SIGINT/SIGTERM cancel the CLI command and allow up to 20 seconds for owned workers to finish before exit 130/143. No successful verification report is fabricated for an interrupted command. Library callers can keep their runtime alive with `sandbox::wait_for_docker_cleanup`; its boolean means workers finished, not that an unavailable daemon confirmed every removal. Runtime destruction, SIGKILL, host failure, or a nonresponsive daemon can still prevent confirmed removal; these are not claimed as successful cleanup.

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
- `reached_direct`
- `reached_via_factory`
- `reached_via_authoritative_test`
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

Direct generated coverage additionally requires a matched completed invocation classified valid, with outcome `passed` or `target_exception`. Entry without that evidence is `reached_direct`, not `checked_direct`. Rejected, invalid, unknown, and interrupted invocations do not supply this credit. Python interpreter shutdown leaves an unfinished invocation open rather than reporting it as a rejection.

Execute detail's `harness_events.surfaces` maps exact surface identifiers to `started`, `completed`, `valid_completed`, `rejected`, `invalid_completed`, and `unknown_completed` counts. Duplicate invocation identities are protocol errors. `valid_invocations` counts valid completed invocation records; `functions_with_valid_invocations` retains the legacy function-outcome count used by `summary.fuzz_pass`. Textual `FUZZ` summaries cannot increase the invocation count. These counters reflect the harness's classifications; they do not independently establish the soundness of generated input admission or prove an application-specific property.

Harness protocol 2 adds `oracle_evaluated` records with the active `surface_id`, `iteration`, nonempty `oracle_id`, and boolean `passed`. Per-surface `passed_oracles` and `failed_oracles` count observed checks from valid completed units; their sum supplies execute `evaluated_oracles` and property strength. Return annotations alone no longer supply that count. Rejected, invalid, unknown, and unfinished units receive no check credit. Minimization and replay do not add to campaign counts. A passed unit cannot contain a failed check. The decoder accepts legacy protocol-1 streams without check records, rejects check records labeled version 1, and rejects mixed-version streams.

Direct generated Python assertions and TypeScript property evaluations emit these records. Semantic-template and factory/caller paths without matched check records do not acquire property strength from their annotations. Input admission and authoritative-test coverage have separate evidence requirements; these check counters do not certify them.

In `--tests-only`, the supplied test process is the sole behavioral gate: each required surface must emit its matching entry event. A passing test that does not cover all required surfaces is inconclusive.

## Findings and repros

Executable argument expressions and replay snippets are not display-truncated. Diagnostic messages and human previews remain bounded. This preserves long supported data arguments; it does not imply every runtime object can be serialized or every inferred property has a complete replay implementation.

Direct TypeScript property findings persist the campaign's evaluator and required helper definitions in their replay snippets. Minimization repeats that evaluator, and replay requires the same failure identity after the initial invocation completes. An initial target exception with copied property diagnostic text does not prove that the property reproduced. This does not extend to every Python, stateful, or semantic-template replay path.

TypeScript repro JSON and saved corpus values use tagged transport values: `{"type":"undefined"}` and `{"type":"number","value":...}`, where the number value is one of `"NaN"`, `"Infinity"`, `"-Infinity"`, or `"-0"`. Ordinary objects matching reserved tag shapes are escaped in `{"type":"object","value":...}` envelopes. Tags apply recursively to nested data. Corpus reuse decodes these transport values into executable expressions; ordinary JSON domain inputs are not globally reinterpreted as tags. Object keys remain data, including `__proto__`. Malformed escape envelopes and corpus rows exceeding the decoding depth bound are skipped without preventing subsequent valid rows from running.

Execute detail contains typed `findings`, `suppressed_findings`, and `findings_summary`; it does not contain the obsolete `fuzz_failures` arrays. A finding includes:

- `id`, `severity`, `confidence`, `category`, `location`, `oracle`, and `input_classification`;
- a structured `repro` with kind, arguments, minimized expressions, snippet, expectation, and (when persisted) replay command;
- `minimization` with `status`, `attempts`, `original`, and an optional reconfirmed `minimized` case;
- optional `error_type`, `classification`, `suggestion`, and `suppressed`.

Enums are serialized in snake case. Severities are `crash`, `property_violation`, `behavioral_regression`, and `infrastructure`; confidences are `authoritative`, `high`, `medium`, and `low`; categories are `exception`, `property`, `test`, `differential`, and `infrastructure`. Oracle kinds are `authoritative_test`, `runtime_contract`, `type_contract`, `declared_property`, `seed_regression`, `differential`, `generic_property`, and `inferred_semantic`.

Findings carry a replay snippet. Supported replay paths emit exactly one `__COURT_JESTER_REPLAY_JSON__` sentinel followed by `{reproduced,severity,oracle_kind,category}`; explicitly unsupported runtime values abstain instead. A snippet's presence alone does not prove complete replay support for the remaining semantic and stateful paths noted above. A persisted report gets `court-jester replay --report <report_path> --finding <id>` in the repro; direct stdout reports keep `command: null`.

Replay reports optionally include `check_passed`. New supported direct, property, semantic, paired, and factory snippets emit this boolean alongside the reproduction payload. `true` requires normal completion of the recorded invocation/check (and the entire recorded factory trace); another exception or a changed/missing action is not success. `not_reproduced` alone still means only that the original failure did not recur. Older snippets omit positive-check evidence. Replay accepts a payload only after a successful, non-timeout, non-memory-failed process; malformed booleans or contradictory `reproduced: true, check_passed: true` abstain.

Differential replay defaults to the original embedded baseline and candidate trees, preserving historical reproduction even after the checkout changes. Explicit `replay --candidate-project-dir <root>` instead compares the embedded baseline with the current candidate entry at the recorded project-relative path and its current imports. This option requires a differential finding; it is never silently ignored for ordinary replay. Missing/out-of-project source entries, incompatible signatures, binding failures, and unsupported execution evidence abstain. Recorded dependency lockfiles must still match supplied dependency and live-candidate roots. Live comparison supplies `check_passed: true` only for matching, successfully observed non-exception snapshots; it is agreement for the recorded input, not proof of general correctness or input admission. The execution payload identifies `candidate_mode`, entry path, entry-source digest, and baseline-tree digest. The entry digest is not a dependency-tree digest: live replay is not an atomic or hermetic checkout snapshot. Historical comparison continues to omit positive-check evidence.

`replay --export-regression <new-directory> --dependency-project-dir <root>` writes a selected-evidence report, a standard-library Python/Node test wrapper, a README, and an artifact-v1 `regression.json` manifest. Export replays first and requires a supported, conclusive check result (which may currently be failing). Inferred expectations require `--accept-inferred`; the manifest records acceptance separately from original confidence. The report retains original tool/candidate provenance and recomputes the selected-finding summary; it is not a new verification run. The test requires Court Jester and passes only for `not_reproduced` plus `check_passed: true` and the matching CLI exit code. Source paths are relative to the project, and an explicit dependency project wins over the caller's directory for relative replay sources. Export never overwrites an existing directory; its manifest is written last so incomplete bundles fail closed. Unknown/invalid inputs, suppressed findings, unsupported/legacy snippets, and historical-only differential replay are not exportable. Successful export returns `0` and adds `regression_export` to the replay response; export validation failures return `2` without claiming a successful test.

Differential export additionally requires explicit `--candidate-project-dir` selecting the same canonical root as `--dependency-project-dir`, plus `--accept-inferred` for the baseline-comparison expectation. Before writing, the exporter requires structured live comparison evidence bound to that entry and the report's baseline digest; historical, timed-out, or contradictory evidence cannot authorize a bundle. Its manifest sets `replay_mode: differential_live`, and the wrapper passes its relocated project root as the live candidate on every run. Ordinary bundles use `current_source` (also the default for older manifests); unknown modes fail. Differential bundles retain both historical trees as provenance, execute only the baseline against current candidate code, and include `BASELINE.md` explaining the accepted expectation. Agreement for one input is not a general specification or evidence that baseline behavior is correct.

Differential input admission uses the same closed-domain, exact-surface, argument-binding rule as native findings. Deterministic exploration of an open string/number/container domain is not admission, even if both invocations return or produce differing observations. Those differences remain visible as unknown-input findings. Generated differential repro values retain their available JSON argument evidence. Replay independently rechecks admission against both analyzed source trees and includes `input_classification` in its comparison payload; a historical report's `valid` label alone is insufficient. Export and newly generated differential test wrappers require this fresh classification to be `valid`. Older findings/bundles without argument evidence must be reverified and re-exported; inference acceptance cannot bypass missing input admission. Positive comparison completion remains separate from admission.

Direct property replay adds `required_oracle` and `passed_oracles` to its sentinel payload. The required ID comes from the actual failing check, not the property declaration or diagnostic text; successful checks are collected from evaluator callbacks. A normal evaluator return is insufficient when a changed return shape skips the required check. TypeScript minimization must preserve that exact failed check, including subchecks such as clamp bounds versus passthrough. Comparator numeric validation also emits check evidence. Legacy declared/generic/type-property snippets without this witness retain their reproduction outcome but supply no `check_passed` result, even if they claimed normal completion. Malformed witnesses or a positive claim with only unrelated passed checks are inconclusive. Regenerate those findings from the failing revision and export again; existing reports are not rewritten or silently upgraded.

Differential repros embed base/candidate local source closures and a dependency contract. A base/candidate behavior difference is advisory unless an authoritative fixture, test, or declared contract proves the candidate side wrong. Replay requires matching third-party/runtime dependencies; missing or mismatched dependencies are inconclusive. Differential replay does not yet provide the live-candidate positive-check contract required by regression export.

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
