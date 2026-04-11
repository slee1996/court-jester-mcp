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
    "timeout_seconds": 420
  }
}
```

## Current Provider Support

Implemented now:

- `noop`: makes no changes
- `replay`: copies deterministic file contents into the workspace
- `codex_cli`: runs `codex exec` against the copied fixture
- `claude_cli`: runs `claude -p` against the copied fixture
- `openai_compat_chat`: posts to an OpenAI-compatible `/chat/completions` endpoint

Present in manifests but not implemented in `provider_from_manifest` yet:

- `openai_responses`

The harness can now validate policy flow, result logging, `court-jester` CLI integration, hidden/public evaluator wiring, local agent CLI execution, and OpenAI-compatible HTTP adapters such as Actual.

## Local Smoke Test

Dry-run the matrix:

```bash
python -m bench.run_matrix --dry-run
```

Replay task-level gold patches instead of asking a provider to edit the workspace:

```bash
python -m bench.run_matrix \
  --task-set known-good-corpus \
  --models noop \
  --policies required-final \
  --use-task-gold-patches \
  --output-dir bench/results/dev
```

This is intended for known-good control runs on upstream-derived tasks. The runner will apply `gold_patch_path`, then run `verify`, public checks, and hidden checks normally.

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

Summarize results:

```bash
python -m bench.summarize_runs bench/results/dev
```

## What To Add Next

1. Expand `swebench-lite-pilot` beyond the first Python cookie-header instance.
2. Add broader known-good coverage beyond the current small corpus.
3. Add summarizer cuts for `setup_error` and `gold_patch_apply_error`.
