# SWE-bench Lite Plan

Date: 2026-04-10

## Goal

Validate Court Jester on a small external benchmark slice that is closer to real repository bugs than the current micro-fixture suite.

The question is not:

- can Court Jester solve all of SWE-bench?

The question is:

- does Court Jester improve final task success on a curated SWE-bench-style slice without introducing false positives that block known-good patches?

## Recommendation

Start with a curated `10-20` task Python-only pilot.

Do not start with:

- full SWE-bench
- mixed-language repo setup chaos
- networked dependency installation during benchmark runs

Instead:

- vendor a local fixture repo per selected instance
- use the real upstream-style failing tests as the judge
- use Court Jester only as the gate and repair aid
- replay the gold patch to measure false positives separately

## Why This Is Worth Doing

The current internal benchmark already answers:

- does Court Jester help on curated Python and TypeScript tasks?

SWE-bench-lite would answer the stronger external question:

- does Court Jester help on real repository bugs with real repo tests?

That is much better evidence for private beta readiness than adding more internal micro tasks alone.

## Pilot Selection Criteria

Pick instances that are:

- Python-only
- localized to `1-4` edited files
- reproducible from a vendored repo fixture
- judged by a narrow test target, not the whole repository
- not dominated by network setup, service containers, or huge build steps

Good candidates:

- parsing bugs
- formatting/serialization bugs
- nullish/defaulting bugs
- path or config handling bugs
- cross-file helper/consumer contract bugs

Bad candidates for the pilot:

- tasks requiring databases, browsers, or network services
- tasks with very broad edit surfaces
- tasks where the authoritative test target takes minutes
- tasks whose gold patch edits dozens of files

## Benchmark Shape

For each selected SWE-bench-lite instance, run three evaluations:

1. Agent utility
- compare `baseline` vs `repair-loop`
- judge with the real repo test target

2. False-positive control
- apply the gold patch
- run `required-final`
- verify that Court Jester does not incorrectly block the correct fix

3. Infra accounting
- separate provider failures from code-quality failures
- record setup failures distinctly from judge failures

## Recommended Task Format

The current task manifest shape is close, but not quite enough for a real SWE-bench-lite workflow.

Current implementation status:

- implemented now:
  - `setup_commands`
  - `setup_cache_key`
  - `gold_patch_path`
  - `gold_changed_files`
  - `upstream_benchmark`
  - `upstream_instance_id`
  - `instance_notes`
  - `python -m bench.run_matrix --use-task-gold-patches`
  - first checked-in pilot task:
    - `py-swebench-lite-cookiejar-quoted`
    - task set: `swebench-lite-pilot`
    - known-good control set: `swebench-lite-known-good`
- still future work:
  - broader summarizer slices for setup-specific failures

Current pilot validation:

- `python -m bench.run_matrix --tasks py-swebench-lite-cookiejar-quoted --models noop --policies baseline --output-dir /tmp/court-jester-swebench-lite-baseline-smoke-v3`
  - result: `0/1` success, failing as intended on the visible upstream-style cookie-header regression
- `python -m bench.run_matrix --task-set swebench-lite-known-good --models noop --policies required-final --use-task-gold-patches --output-dir /tmp/court-jester-swebench-lite-known-good-smoke-v5`
  - result: `1/1` success, with public checks, hidden checks, and `verify` all passing on the task gold patch

### Current fields we can keep

- `id`
- `title`
- `repo_fixture`
- `prompt`
- `language`
- `bucket`
- `verify_paths`
- `verify_test_path`
- `public_check_commands`
- `hidden_check_command`
- `expected_files`
- `tags`
- `family`
- `bug_class`
- `bug_surface`
- `difficulty`
- `uses_project_dir`
- `cross_file`

### Proposed new fields

- `upstream_benchmark`
  - example: `SWE-bench_Lite`
- `upstream_instance_id`
  - original benchmark case identifier
- `setup_commands`
  - commands to prepare the vendored repo fixture before provider/edit/judge steps
- `setup_cache_key`
  - stable key for caching prepared workspaces across repeats
- `gold_patch_path`
  - local patch file for known-good replay
- `gold_changed_files`
  - expected edited files from the gold patch
- `judge_check_commands`
  - optional broader authoritative test target when `public_check_commands` is only a visible subset
- `instance_notes`
  - short provenance and adaptation notes

### Example manifest

```json
{
  "id": "swebench-lite-py-requests-cookiejar-001",
  "title": "Requests cookiejar preserves quoted values",
  "repo_fixture": "swebench_lite_py_requests_cookiejar_001",
  "prompt": "Fix cookie handling so quoted cookie values round-trip correctly for the affected request path.",
  "language": "python",
  "bucket": "swebench_lite",
  "verify_paths": [
    "requests/cookies.py"
  ],
  "verify_test_path": "tests/court_jester_public_verify.py",
  "public_check_commands": [
    [
      "python",
      "-m",
      "pytest",
      "tests/test_cookies.py",
      "-k",
      "quoted_cookie_value"
    ]
  ],
  "hidden_check_command": [
    "python",
    "-m",
    "pytest",
    "tests/test_cookies.py"
  ],
  "expected_files": [
    "requests/cookies.py"
  ],
  "family": "swebench_lite",
  "bug_class": "contract_drift",
  "bug_surface": "cross_file",
  "difficulty": "medium",
  "uses_project_dir": true,
  "cross_file": true,
  "tags": [
    "python",
    "swebench_lite",
    "requests",
    "cookies",
    "upstream_derived"
  ],
  "upstream_benchmark": "SWE-bench_Lite",
  "upstream_instance_id": "requests__requests-12345",
  "setup_commands": [
    [
      "python",
      "-m",
      "pip",
      "install",
      "-e",
      ".[test]"
    ]
  ],
  "setup_cache_key": "requests-cookiejar-py311-v1",
  "gold_patch_path": "gold/requests__requests-12345.patch",
  "gold_changed_files": [
    "requests/cookies.py"
  ],
  "instance_notes": "Vendored from SWE-bench Lite. Visible check is a narrow failing-test subset; hidden check is the broader file-level regression target."
}
```

## Fixture Layout

Use the same `bench/repos/` model as the current harness.

Recommended structure:

```text
bench/repos/swebench_lite_py_requests_cookiejar_001/
  requests/
  tests/
  pyproject.toml
  setup.cfg
  gold/
    requests__requests-12345.patch
  SWEBENCH_NOTES.md
```

Rules:

- the fixture must already be locally runnable
- no network should be required during benchmark execution
- any heavy setup should be converted into vendored local state where practical
- if setup caching is used, setup commands should write their important outputs inside the copied workspace so cache restores are meaningful

## Judge Strategy

Do not use the full repo test suite as the first pilot judge unless it is already cheap.

Use a two-level judge:

1. `public_check_commands`
- narrow visible failing test target
- used for agent-facing repair pressure

2. `hidden_check_command`
- slightly broader authoritative regression target
- same file/module/test class when possible

This keeps the benchmark aligned with the current harness design while still being upstream-rooted.

## False-Positive Validation

Gold-patch replay is mandatory for SWE-bench-lite.

The current repo already has replay providers, but they are model-manifest scoped, not task scoped. That is not enough for per-instance gold patches.

### Proposed runner support

Add task-level gold replay support:

- `gold_patch_path`
- `gold_changed_files`

Then add one of:

1. new provider type: `task_gold_patch`
- runner reads the patch from the task and applies it directly

or

2. runner flag: `--use-task-gold-patches`
- bypass provider generation entirely and apply the task gold patch before evaluation

The second option is cleaner for false-positive control because it is explicitly an evaluation mode, not a fake model.

## Exact Runner Changes

These are the minimum changes I would actually make.

### 1. Extend `TaskManifest`

In [bench/common.py](../bench/common.py):

- add `upstream_benchmark: str | None = None`
- add `upstream_instance_id: str | None = None`
- add `setup_commands: list[list[str]] = field(default_factory=list)`
- add `setup_cache_key: str | None = None`
- add `gold_patch_path: str | None = None`
- add `gold_changed_files: list[str] = field(default_factory=list)`
- add `instance_notes: str | None = None`

### 2. Add fixture setup support

In [bench/runner.py](../bench/runner.py):

- run `setup_commands` after copying the fixture and before the pre-edit snapshot
- record `setup_ms`
- record setup stdout/stderr artifacts
- fail as `setup_error` if setup does not complete

### 3. Add setup caching

Without caching, SWE-bench-lite repeats will be too slow.

Add:

- prepared-workspace cache root under `/tmp/court-jester-setup-cache`
- key by `setup_cache_key`
- if present, copy the prepared workspace into the run dir instead of rerunning setup

### 4. Add task-level gold replay mode

Add a runner path that:

- reads `gold_patch_path`
- applies it to the copied fixture
- skips the provider step
- still runs `verify`, `public_check_commands`, and `hidden_check_command`

This is the cleanest way to measure false positives on known-good upstream fixes.

### 5. Add new failure categories

In [bench/runner.py](../bench/runner.py), keep provider failures separate and add:

- `setup_error`
- `gold_patch_apply_error`

This matters because SWE-bench-lite can otherwise be dominated by repo-prep noise.

### 6. Extend summarization slices

In the summarizer, add grouping by:

- `upstream_benchmark`
- `family`
- `setup_error`

We want to answer:

- did Court Jester help on SWE-bench-lite specifically?
- did setup noise dominate outcomes?

## Recommended Task Sets

Start with two task sets.

### 1. `swebench-lite-pilot`

- `10-20` curated Python tasks
- utility benchmark
- models: `claude-default`, `codex-default`
- policies: `baseline`, `repair-loop`

### 2. `swebench-lite-known-good`

- same instances
- gold-patch replay only
- policy equivalent to `required-final`
- goal: false-positive measurement

## Recommended Commands

Utility:

```bash
python -m bench.run_matrix \
  --task-set swebench-lite-pilot \
  --models claude-default,codex-default \
  --policies baseline,repair-loop \
  --repeats 3 \
  --output-dir /tmp/court-jester-swebench-lite
```

Known-good control after adding task-level gold replay:

```bash
python -m bench.run_matrix \
  --task-set swebench-lite-known-good \
  --models noop \
  --policies required-final \
  --use-task-gold-patches \
  --repeats 3 \
  --output-dir /tmp/court-jester-swebench-lite-known-good
```

## Success Criteria

I would treat SWE-bench-lite as validating Court Jester if all of these are true:

- `repair-loop` beats `baseline` on the pilot set
- known-good gold patches are rarely or never blocked
- setup errors are not the dominant failure mode
- provider failures are a minority of runs, not the headline result

## What Not To Do

Do not call it “SWE-bench support” after the first pilot.

Call it what it is:

- a curated SWE-bench-lite external validation slice

That is already a meaningful step forward and a much more honest one.
