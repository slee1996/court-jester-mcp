# CI Adoption Guide

Court Jester is a typed, confidence-calibrated gate for changed Python and TypeScript code. CI MUST branch on the report `verdict`; it MUST NOT reconstruct a verdict from stage names or legacy boolean fields.

## Recommended pull-request command

The first-party changed-file wrapper is the shortest path:

```bash
court-jester ci \
  --base origin/main \
  --gate parse,lint,coverage,portability,execute,test \
  --report github \
  --report-level minimal
```

The default gates are `parse,lint,coverage,portability,execute,test`; `--gate all` also includes the optional complexity gate. Aggregation is fail > inconclusive > pass. A file with no changed exported/invocable surface is still subject to the repository's selected coverage behavior; use `--coverage-gate none` only when best-effort coverage is intentional.

Exit codes for `ci` (and `verify`) are:

- `0`: pass;
- `1`: fail (a selected gate failed);
- `2`: usage or setup failed before a report was available;
- `3`: a report was produced but is inconclusive (for example, required coverage, module loading, timeout, or missing valid behavioral evidence).

An inconclusive result is not a pass. The normal action is to inspect the environment or add a contract/authoritative test, not to blindly retry.

## Direct verify equivalent

```bash
court-jester verify \
  --file src/example.ts \
  --language typescript \
  --project-dir . \
  --diff-file /tmp/pr.diff \
  --report-level minimal \
  --execute-gate crash \
  --coverage-gate changed-exports \
  --suppressions-file .court-jester-ignore.json \
  --output-dir .court-jester/reports
```

`--execute-gate all|crash|none` chooses which execute findings are gating. `no_inputs_reached` remains diagnostic and does not become a pass merely because the execute gate is `none`. `--inferred-oracle-gate advisory|fail` controls low-confidence name/context semantic findings; the default is advisory.

## Reading a result

A v3 report contains:

- `verdict`: `pass`, `fail`, or `inconclusive`;
- `strength`: `none`, `parse_only`, `static_checked`, `runtime_smoke`, `property_checked`, or `authoritative_tests`;
- typed stages with status `passed`, `failed`, `inconclusive`, `advisory`, or `skipped`;
- `summary.coverage` and typed per-surface invocation evidence;
- typed findings with oracle provenance, confidence, structured arguments, minimization, and replay snippets.

A reach event through a caller or factory does not count as behaviorally checked. In `--tests-only`, the authoritative test process must emit coverage for every required selected surface or the report is inconclusive. Differential differences are advisory unless an authoritative fixture, test, or declared contract proves the candidate wrong.

Use `--summary repair-json` when an agent needs only the repair-loop view. It emits `recommended_action` as `repair`, `inspect_environment`, `add_contract_or_test`, or `none`, plus the primary finding and coverage summary.

## Runtime profiles and readiness

The default `--runtime-profile local-trusted` executes in a host subprocess and is not a security boundary. For CI isolation, use Docker:

```bash
court-jester doctor --language all --runtime-profile isolated --summary json > .court-jester/doctor.json
court-jester ci \
  --base origin/main \
  --runtime-profile isolated \
  --python-docker-image python:3.12-slim \
  --typescript-docker-image node:24-bookworm-slim \
  --report github
```

`isolated` uses no network, read-only source/project mounts and root filesystem, bounded resources, and deterministic cleanup. Docker, daemon, image, or module-loading failures are inconclusive; Court Jester never pulls an image or silently falls back to local execution. Image overrides are valid only with `isolated`. Limits must be finite and greater than zero.

Run `doctor` before a release or benchmark lane. It reports the selected profile, runtime/image checks, and optional linter checks with schema v3 typed statuses. `doctor` exits `0` for readiness pass, `1` for failed readiness, and `2` for usage.

For local readiness, use `court-jester doctor --language typescript --project-dir . --file src/index.ts --timeout-seconds 10`. It probes the runtime selected by the shared execution context (including project-local tools and TSX mode), and resolves Ruff/Biome with the same precedence as lint. A broken selected tool does not silently fall back to a different installation. Linter readiness requires a successful, nonempty version response; missing, broken, or timed-out optional linters are advisory. Local execution is trusted host execution, not network isolation. Ordinary doctor checks do not import the target. Add one selected/configured test file and `--probe-entrypoint` to check project imports and test completion; add `--runtime-profile isolated` to execute that probe through the normal Docker test runner. Isolated project path checks and image smoke alone do not prove entrypoint readiness. See [entrypoint readiness](repository-config.md#opt-in-entrypoint-readiness).

## Authoritative tests, suppression, and reports

Add `--test-file <PATH>` to run a caller-supplied authoritative stage. `--tests-only` makes that test the sole behavioral gate. Choose `--test-runner auto|node|bun|repo-native` for TypeScript.

Suppression rules remain visible but non-gating:

```json
{
  "rules": [
    {
      "path": "src/hotel-cache.ts",
      "stage": "execute",
      "function": "jsonResponse",
      "severity": "crash",
      "error_type": "RangeError"
    }
  ]
}
```

`path` suffix-matches the verified file. `stage` scopes to `execute`, `complexity`, or `portability`; optional `function`, `severity`, `error_type`, and `reason` narrow the rule. Suppressed findings remain in `findings`/`suppressed_findings` and cannot become the repair summary's primary finding.

Use `--report-level full` for debugging and `--report-level minimal` for CI artifacts. Persist a report with `--output-dir` to receive replay commands for each finding. Replay a persisted finding with:

```bash
court-jester replay \
  --report .court-jester/reports/report.json \
  --finding '<stage>:<symbol>:<ordinal>'
```

Replay returns `0` when the stored expectation is reproduced, `1` when it is not reproduced, `2` for an invalid report/id/argument, and `3` for an inconclusive dependency, runtime, or protocol condition. Differential replay may require `--dependency-project-dir` matching the recorded dependency contract.

## Rollout pattern

1. Start with `--report-level minimal`, default `changed-exports` coverage, and `--execute-gate crash` if existing code is noisy.
2. Treat every `inconclusive` as a coverage/environment signal; add an authoritative test or repository domain evidence rather than weakening the global verdict.
3. Review structured repros and use `repair-json` in agent prompts.
4. Add `execute-gate all` and `--inferred-oracle-gate fail` only after reviewing advisory findings.
5. Persist reports and replay representative findings before making the gate required.

The reusable quality workflow runs the focused Rust integration targets, benchmark unit modules, release CLI smoke, and a locked dry-run. Release packaging depends on that same quality gate.


## Benchmark evidence in CI

Benchmark lanes are separate from the per-file CI verdict. Non-dry `bench.run_matrix` runs require a passing schema-v3 `doctor` report for the selected `--verify-runtime-profile`; they persist artifact v1 metadata, resolved runtime/image IDs, and the doctor digest. Use `--summary-json` with `--baseline-policy`/`--candidate-policy`, then choose `--gate-policy none|private-beta-default|strict-heldout` and `--fail-on-gate` when the gate should affect CI. `--evidence-bundle --evidence-redaction transcripts --strict-evidence` creates a portable checked bundle. `--shadow-records` is opt-in observation only and never changes run success.
