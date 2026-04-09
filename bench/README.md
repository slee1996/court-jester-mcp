# Court Jester Benchmark Harness

This benchmark scaffold measures whether `court-jester` improves agentic coding outcomes across:

- task buckets
- model tiers
- tool-gating policies

The harness is intentionally separate from the Rust server. `court-jester` stays focused on verification, while the benchmark layer orchestrates:

- task fixtures
- model/provider adapters
- policy-controlled `court-jester` calls
- public and hidden evaluators
- result aggregation

## Recent Writeups

Benchmark writeups:

- `docs/benchmark-2026-03-26.md`
- `docs/archive/benchmark-2026-03-18.md`
- `docs/archive/benchmark-and-fuzzing-2026-03-20.md`
- `docs/archive/sprint-2026-03-18.md`
- `docs/README.md`

Release-positioning and current release bar:

- `docs/release-readiness-private-beta.md`

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
  "verify_test_path": "tests/court_jester_public_verify.py",
  "public_check_commands": [["python", "tests/public_checks.py"]],
  "hidden_check_command": [
    "python",
    "{bench_root}/evaluators/profile_hidden.py",
    "{workspace}"
  ],
  "expected_files": ["profile.py"],
  "tags": ["null_handling", "hidden_edge_case"]
}
```

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

The harness can now validate policy flow, result logging, `court-jester` MCP integration, hidden/public evaluator wiring, local agent CLI execution, and OpenAI-compatible HTTP adapters such as Actual.

## Local Smoke Test

Dry-run the matrix:

```bash
python -m bench.run_matrix --dry-run
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

Summarize results:

```bash
python -m bench.summarize_runs bench/results/dev
```

## What To Add Next

1. Expand the task suite from 9 to 20-50 tasks.
2. Add slower TypeScript fixture repos to keep pressure on execute-stage stability.
3. Track transcript-level deltas: when `court-jester` changed the patch, and whether that improved hidden-check pass rate.
4. Add named Codex/Claude manifests for more model-tier comparisons beyond the current defaults.
