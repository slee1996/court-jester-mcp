# Court Jester Benchmark Harness

This benchmark scaffold measures whether `court-jester` improves agentic coding outcomes across:

- task buckets
- model tiers
- tool-gating policies

The harness is intentionally separate from the Rust CLI. `court-jester` stays focused on verification, while the benchmark layer orchestrates:

- task fixtures
- model/provider adapters
- policy-controlled `court-jester` calls
- public and hidden evaluators
- result aggregation

## Recent Writeups

Benchmark writeups:

- `docs/benchmark-2026-04-10.md`
- `docs/benchmark-2026-03-26.md`
- `docs/archive/benchmark-2026-03-18.md`
- `docs/archive/benchmark-and-fuzzing-2026-03-20.md`
- `docs/archive/sprint-2026-03-18.md`
- `docs/README.md`

Release-positioning and current release bar:

- `docs/release-readiness-private-beta.md`

Current headline run:

- `core-current` task set
- `39` tasks
- models: `codex-default`, `claude-default`
- policies: `baseline`, `repair-loop-verify-only`
- result: `71 / 78` baseline -> `76 / 78` verify-only repair loop
- repair triggers: verify-only, with `0` public-trigger and `0` hidden-trigger repairs

Next causal-control run:

- compare `baseline`, `public-repair-1`, `repair-loop-verify-only`, `public-repair-2`, and `repair-loop-verify-only-2`
- match public-tests-only repair against verify-guided repair at one extra attempt and two extra attempts
- keep hidden evaluation final-score only so the comparator is realistic instead of oracle-gated

Large-scale product-clone roadmap:

- [bench/express-full-clone-plan.md](/Users/spencerlee/court-jester-mcp/bench/express-full-clone-plan.md)
- [bench/express-testfile-gauntlet-v1-plan.md](/Users/spencerlee/court-jester-mcp/bench/express-testfile-gauntlet-v1-plan.md)
- the near-term goal is an Express-derived gauntlet that rolls cleanly into `express-full-clone-alpha`, not another disconnected toy slice

## Benchmark Guardrails

The benchmark is intentionally split into different suite roles. Do not treat every task set as a headline product benchmark.

- `core-current`
  - `suite_kind: headline_curated`
  - main product-utility suite
- `public-repair-proving-ground`
  - `suite_kind: mechanism_public_repair`
  - mechanism suite for testing whether public-test repair loops actually fire
- `known-good-corpus`
  - `suite_kind: false_positive_control`
  - local false-positive control on already-correct implementations
- `swebench-lite-known-good`
  - `suite_kind: external_false_positive_control`
  - upstream-derived false-positive control via gold patches
- `swebench-lite-pilot`
  - `suite_kind: external_held_out_pilot`
  - held-out external slice that is not built from Court Jester-shaped fixture tasks
- `library-slices`
  - `suite_kind: library_spec_slice`
  - bounded library/spec-conformance slice
- `express-clone-alpha-pilot`
  - `suite_kind: framework_clone_alpha`
  - first shared-repo Express clone lane with seeded framework regressions across routing, error flow, request, response, and wrapper semantics
- `express-clone-alpha-monolith`
  - `suite_kind: framework_clone_monolith`
  - first large-task shared-repo Express lane aimed directly at the question "does Court Jester help agents repair a broad library/framework clone more successfully than baseline generation alone?"
- `express-clone-alpha-fresh-spec`
  - `suite_kind: framework_clone_fresh_spec`
  - one-shot fresh-repo Express build from a broad visible public spec
- `express-clone-alpha-fresh-chunks-v4`
  - `suite_kind: framework_clone_fresh_spec_chunks`
  - current tuned fresh-repo Express chunk suite with tests-only verify and narrower router/static slices
- `express-clone-alpha-fresh-ladder`
  - `suite_kind: framework_clone_fresh_spec_ladder`
  - the fresh-repo progression from chunked subsystem builds to the broad monolith final

Recommended usage:

- use `core-current` for the headline CLI utility number
- use `public-repair-proving-ground` for public-vs-verify mechanism questions
- use `known-good-corpus` and `swebench-lite-known-good` for false-positive control
- use `swebench-lite-pilot` as the first external held-out sanity slice
- use `express-clone-alpha-pilot` when you want a product-shaped framework benchmark rather than independent micro-fixtures
- use `express-clone-alpha-monolith` when you want the closest current benchmark to the real product thesis: broad shared-library repair, not isolated surface patches
- use `express-clone-alpha-fresh-chunks-v4` when you want the current tuned suite for fresh-repo library construction
- use `express-clone-alpha-fresh-ladder` when you want both the tuned chunk tasks and the broad fresh-spec final in one suite

Execution order also matters. The matrix runner now defaults to `--schedule blocked-random`, which randomizes block order while keeping the same task/repeat cells together. This reduces policy drift caused by provider noise and long serial runs. Use `--schedule task-major` only when you explicitly want the old deterministic nested-loop order for debugging.

If you want faster wall-clock without turning the harness into a provider stress test, use `--parallel-by-provider`. That runs one serial queue per provider concurrently, so `codex_cli` cells stay ordered relative to each other, `claude_cli` cells stay ordered relative to each other, and the two providers can make progress at the same time.

Current Express alpha status:

- shared slice-task fixture repo: `bench/repos/express_clone_alpha`
- dedicated monolith fixture repo: `bench/repos/express_clone_alpha_monolith`
- dedicated fresh-spec fixture repo: `bench/repos/express_clone_alpha_fresh_spec`
- chunked fresh-spec fixture repos:
  - `bench/repos/express_clone_alpha_fresh_router_dispatch`
  - `bench/repos/express_clone_alpha_fresh_urlencoded`
  - `bench/repos/express_clone_alpha_fresh_request_meta_v2`
  - `bench/repos/express_clone_alpha_fresh_response_headers_v2`
  - `bench/repos/express_clone_alpha_fresh_static_file_v2`
- seeded pilot task set: `bench/task_sets/express-clone-alpha-pilot.json`
- seeded monolith task set: `bench/task_sets/express-clone-alpha-monolith.json`
- fresh chunked task set: `bench/task_sets/express-clone-alpha-fresh-chunks-v4.json`
- fresh ladder task set: `bench/task_sets/express-clone-alpha-fresh-ladder.json`
- seeded bug smoke with `noop` baseline: `21 / 21` tasks fail as expected
- seeded monolith bug smoke with `noop` baseline: `0 / 1` success, failing publicly on the first attempt as intended
- clean clone public/verify local test files pass when run directly
- clean clone hidden suites pass through `bench/evaluators/ts_workspace_test.py`
- clean monolith public suite passes directly, hidden suite passes through `bench/evaluators/ts_workspace_test.py`, and the verifier suite is materialized from verify assets only during the Court Jester call
- hidden Express tests now live under `bench/hidden_assets/express_clone_alpha/tests`, not in the copied fixture workspace
- monolith verifier tests now live under `bench/verify_assets/express_clone_alpha_monolith/tests`, not in the copied fixture workspace
- each fresh chunk fixture exposes only `tests/harness.ts` plus one `tests/public_spec.ts`; verifier and hidden suites live outside the workspace under `bench/verify_assets/...` and `bench/hidden_assets/...`
- framework fresh-spec slices can also set `verify_tests_only: true`, which makes Court Jester skip generic execute fuzz and judge the candidate only by the authoritative verify test file; this avoids helper-fuzz false positives on repo-shaped framework tasks
- the alpha fixture now includes `express.json`, `express.urlencoded`, `express.text`, `express.raw`, `req.get`, `req.protocol`, `res.sendStatus`, `res.location`, `res.links`, and `res.vary` in addition to router, app, request, and response helpers
- full `required-final` known-good control is not yet trustworthy on this repo shape because Court Jester currently reports `verify_stronger_than_eval` false positives on the clone aggregate

## Layout

- `bench/tasks/`: task manifests
- `bench/models/`: model manifests
- `bench/policies/`: policy manifests
- `bench/repos/`: local fixture repos copied into temp workspaces
- `bench/replays/`: deterministic replay edits for local smoke tests
- `bench/evaluators/`: hidden evaluator scripts
- `bench/results/`: run artifacts and summaries
- `bench/run_matrix.py`: matrix runner
- `bench/summarize_runs.py`: result summarizer
- `bench/dashboard.py`: live local dashboard server

Each run directory now records more than the final outcome:

- `run.json`: in-flight metadata for live dashboard progress
- `result.json`: final outcome plus per-attempt provider/evaluator metadata
- `patch.diff`: final workspace diff vs the original copied fixture
- `attempt_<n>.diff`: per-attempt workspace diff so you can reconstruct patch chronology
- `agent_trace/attempt_<n>/events.jsonl`: best-effort command trace for CLI agents
- `agent_trace/attempt_<n>/summary.json`: compact command/category summary for that attempt

The CLI trace is intentionally best-effort. It captures shell/tool commands reached through the benchmark's shimmed `PATH` and is useful for reconstructing searches, file reads, and command flow, but it is not a perfect internal reasoning trace.

CLI tracing is now opt-in. Set `CJ_AGENT_TRACE=1` when launching `bench.run_matrix` if you want per-attempt command traces. The default is off because trace shims add real process and CPU overhead, especially on long CLI startup paths.

For CLI benchmarks, evaluator isolation is mandatory. If a hidden or verifier-only test file is copied into the workspace, the benchmark is contaminated because the agent can read it. The Express clone lanes now avoid that by keeping hidden suites under `bench/hidden_assets/...` and monolith verifier suites under `bench/verify_assets/...`; the harness materializes those files only during evaluation and removes them afterward.

`result.json` timings now also include trace-cost telemetry:

- `agent_trace_setup_ms`
- `agent_trace_summary_ms`
- `agent_trace_event_count`
- `agent_trace_event_overhead_estimate_ms`
- `agent_trace_overhead_estimate_ms`

The estimate is intentionally simple: fixed setup/summary time plus a per-event shell-wrapper cost estimate. It is useful for comparing policies and providers, not for precise per-command profiling.

## Manifest Shapes

Task manifest:

```json
{
  "id": "py-semantic-profile-empty-name",
  "title": "Empty display name hidden regression",
  "repo_fixture": "mini_py_service",
  "prompt": "Fix the bug where empty names crash normalization.",
  "language": "python",
  "bucket": "semantic_bug",
  "verify_paths": ["profile.py"],
  "setup_commands": [],
  "setup_cache_key": null,
  "verify_test_path": "tests/court_jester_public_verify.py",
  "public_check_commands": [["python", "tests/public_checks.py"]],
  "hidden_check_command": [
    "python",
    "{bench_root}/evaluators/profile_hidden.py",
    "{workspace}"
  ],
  "gold_patch_path": null,
  "gold_changed_files": [],
  "expected_files": ["profile.py"],
  "upstream_benchmark": null,
  "upstream_instance_id": null,
  "instance_notes": null,
  "tags": ["null_handling", "hidden_edge_case"]
}
```

Additional task fields now supported for external-repo style benchmarks:

- `setup_commands`: prepare the copied fixture before provider or judge steps
- `setup_cache_key`: reuse prepared workspaces across repeats
- `verify_tests_only`: run verify in authoritative-test-only mode and skip execute-stage fuzz
- `gold_patch_path`: task-local patch file for known-good replay mode
- `gold_changed_files`: optional changed-file hint for gold patch mode
- `upstream_benchmark`, `upstream_instance_id`, `instance_notes`: provenance metadata

`setup_cache_key` caches the prepared workspace tree, so setup commands should materialize anything important inside the copied workspace itself. Do not rely on mutable global virtualenv or system state if you want cache hits to be meaningful.

Policy manifest:

```json
{
  "id": "required-final",
  "title": "Required verify gate",
  "description": "Always run court-jester verify before finalizing.",
  "court_jester_mode": "required",
  "required_tools": ["verify"],
  "block_on_failed_verify": true,
  "max_repair_rounds": 0
}
```

Additional policy fields now supported:

- `verify_only_repair`: allow repair only after a failed Court Jester verify
- `public_only_repair`: allow repair only after a failed public check and keep hidden checks final-score only
- `blind_retry_without_verify`: reserve extra attempts without calling Court Jester or feeding evaluator failures back between attempts
- `repair_feedback_style`: `detailed`, `generic`, or `none`
- `promote_verify_repros`: materialize verify repros into a temporary test file when the task supports it
- `replay_attempt_history`: include earlier attempt summaries in the next provider prompt

Model manifest:

```json
{
  "id": "claude-default",
  "title": "Claude Code default model",
  "provider": "claude_cli",
  "enabled_by_default": false,
  "metadata": {
    "timeout_seconds": 1000
  }
}
```

## Current Provider Support

Implemented now:

- `noop`: makes no changes
- `replay`: copies deterministic file contents into the workspace
- `codex_cli`: runs `codex exec` against the copied fixture with user MCP servers and Codex plugins disabled, so benchmark runs do not drift because of local connector or marketplace state
- `claude_cli`: runs `claude -p` against the copied fixture with user settings excluded and slash-command/bootstrap extras disabled, so benchmark runs do not drift because of local Claude plugins or editor integrations
- `openai_compat_chat`: posts to an OpenAI-compatible `/chat/completions` endpoint

Present in manifests but not implemented in `provider_from_manifest` yet:

- `openai_responses`

The harness can now validate policy flow, result logging, `court-jester` CLI integration, hidden/public evaluator wiring, local agent CLI execution, and OpenAI-compatible HTTP adapters such as Actual.

## Local Smoke Test

Dry-run the matrix:

```bash
python -m bench.run_matrix --dry-run
```

Run one serial queue per provider in parallel:

```bash
python -m bench.run_matrix \
  --task-set core-current \
  --models codex-default,claude-default \
  --policies baseline,repair-loop-verify-only \
  --parallel-by-provider \
  --output-dir /tmp/court-jester-core-parallel
```

Replay task-level gold patches instead of asking a provider to edit the workspace:

```bash
python -m bench.run_matrix \
  --task-set swebench-lite-known-good \
  --models noop \
  --policies required-final \
  --use-task-gold-patches \
  --output-dir bench/results/dev
```

This is intended for known-good control runs on upstream-derived tasks. The runner will apply `gold_patch_path`, then run `verify`, public checks, and hidden checks normally.

Run the local already-correct control corpus without gold patches:

```bash
python -m bench.run_matrix \
  --task-set known-good-corpus \
  --models noop \
  --policies required-final \
  --output-dir bench/results/dev
```

Run the sample task with the local replay model:

```bash
python -m bench.run_matrix \
  --tasks py-semantic-profile-empty-name \
  --models replay-fixed-profile,noop \
  --policies baseline,required-final \
  --output-dir bench/results/dev
```

Run the same task against local Codex and Claude Code CLIs:

```bash
python -m bench.run_matrix \
  --tasks py-semantic-profile-empty-name \
  --models codex-default,claude-default \
  --policies required-final \
  --output-dir bench/results/dev
```

Run the current strict utility benchmark:

```bash
python -m bench.run_matrix \
  --task-set core-current \
  --models codex-default,claude-default \
  --policies baseline,repair-loop-verify-only \
  --output-dir /tmp/court-jester-core-cli-verify-only-rerun
```

Run the blind-retry ablation against the strict verify-only loop:

```bash
python -m bench.run_matrix \
  --task-set core-current \
  --models codex-default,claude-default \
  --policies baseline,retry-once-no-verify,repair-loop-verify-only \
  --output-dir /tmp/court-jester-core-cli-blind-retry-ablation
```

Run the multishot blind-retry comparison against the two-round verify loop:

```bash
python -m bench.run_matrix \
  --task-set core-current \
  --models codex-default,claude-default \
  --policies baseline,retry-twice-no-verify,repair-loop-2 \
  --output-dir /tmp/court-jester-core-cli-blind-retry-multishot
```

Run the full attempt-budget matrix:

```bash
python -m bench.run_matrix \
  --task-set core-current \
  --models codex-default,claude-default \
  --policies baseline,retry-once-no-verify,repair-loop-verify-only,retry-twice-no-verify,repair-loop-2 \
  --output-dir /tmp/court-jester-core-cli-attempt-budget-ablation
```

Run the public-tests-only repair comparison against verify-only repair:

```bash
python -m bench.run_matrix \
  --task-set core-current \
  --models codex-default,claude-default \
  --policies baseline,public-repair-1,repair-loop-verify-only,public-repair-2,repair-loop-verify-only-2 \
  --output-dir /tmp/court-jester-core-cli-public-repair-ablation
```

Run the public-repair mechanism suite where public repairs are expected to fire:

```bash
python -m bench.run_matrix \
  --task-set public-repair-proving-ground \
  --models codex-default,claude-default \
  --policies baseline,public-repair-1,repair-loop-verify-only,public-repair-2,repair-loop-verify-only-2 \
  --output-dir /tmp/court-jester-public-repair-proving-ground
```

Summarize results:

```bash
python -m bench.summarize_runs bench/results/dev
```

Watch a run live in the local dashboard:

```bash
python -m bench.dashboard \
  --port 8777 \
  --results-dir /tmp/court-jester-core-cli-blind-retry-ablation
```

Then open `http://127.0.0.1:8777`.

The summarizer now reports:

- `verify_triggered_repairs` and `verify_recovery_rate`
- `successes_per_hour` and `minutes_per_success`
- `product_successes_per_hour` and `product_minutes_per_success`
- `avg_hidden_eval_ms`, `avg_setup_ms`, and `avg_harness_overhead_ms`
- baseline-relative lift tables, including `marginal_minutes_per_saved_task`
- baseline-relative lift tables for product-loop time, including `marginal_product_minutes_per_saved_task`
- grouped cuts by `bucket`, `bug_class`, and `task_id`

## What To Add Next

1. Expand `swebench-lite-pilot` beyond the first Python cookie-header instance.
2. Add broader known-good coverage beyond the current small local and external controls.
3. Add summarizer cuts for `setup_error` and `gold_patch_apply_error`.
