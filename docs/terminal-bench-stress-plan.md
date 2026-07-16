# Terminal-Bench Stress Plan

Date: 2026-04-24

## Goal

Use Terminal-Bench as an external stress lane for Court Jester.

Court Jester's current benchmark harness is strongest for Python and TypeScript
library repair loops. Terminal-Bench is broader: terminal-native tasks with real
setup, filesystems, containers, tests, and multi-step agent workflows. That makes
it useful as a pressure test for where Court Jester helps, where it is irrelevant,
and where it creates false confidence or false positives.

This should be treated as an external stress benchmark, not as a replacement for
the curated `bench/` suites.

## What We Want To Learn

1. Does adding `court-jester verify` to agent workflows improve pass rate on
   Terminal-Bench tasks that contain Python or TypeScript code edits?
2. How often is Court Jester inapplicable because the task is mostly shell,
   infrastructure, data, ML, services, or non-Python/TypeScript work?
3. How often does Court Jester block correct solutions on repo-shaped tasks?
4. How often does Court Jester catch a real bug before the Terminal-Bench tests
   do?
5. What failure categories show the next product work: better test-stage
   integration, better project detection, broader language support, fewer
   verifier-shaped false positives, or better repair feedback?

## Current Product Hypothesis

Court Jester should not be expected to improve every Terminal-Bench task.

The expected useful subset is:

- Python package or script repair tasks.
- TypeScript or Node package repair tasks.
- Tasks with local unit-testable functions or modules.
- Tasks where the agent edits parser, serializer, normalizer, validator,
  config, CLI, or library code.
- Tasks where public tests are incomplete and hidden tests exercise edge cases.

The expected weak subset is:

- Shell-only tasks.
- System administration tasks.
- Long-running service setup tasks.
- ML training or data processing tasks where correctness is not localized to a
  Python/TypeScript callable surface.
- Tasks where success depends on external binaries, networking, Docker
  orchestration, notebooks, generated artifacts, or full end-to-end behavior.

## Evaluation Modes

Run the same selected Terminal-Bench tasks under matched policies:

| Mode | Agent Instruction | Court Jester Role |
| --- | --- | --- |
| `baseline` | Solve the task normally. | None. |
| `advisory` | Solve normally, then run Court Jester on changed Python/TS files if applicable. | Report only. |
| `required-final` | Before final answer, run Court Jester on changed Python/TS files. | Failed verify blocks final. |
| `repair-loop` | If Court Jester fails, feed the compact repro back to the agent for one repair. | Repair trigger. |
| `tests-only` | If Terminal-Bench exposes task tests that can be run against edited Python/TS modules, run them through Court Jester's authoritative test stage. | Test-stage wrapper, no fuzz. |

Primary comparison should be `baseline` vs `repair-loop`. `required-final`
is useful as a recall signal, but it can punish the agent without allowing
recovery.

## Task Selection

Start with a small tagged subset rather than the full benchmark.

Selection criteria:

- Task has Python or TypeScript files in the workspace.
- Expected solution edits at least one `.py` or `.ts` file.
- Terminal-Bench tests are deterministic and run within a reasonable local
  timeout.
- Docker image setup is not the dominant task cost for the first pilot.
- The task instruction does not require secrets or external network access.

Pilot size:

- 10 tasks for smoke.
- 25 tasks for first comparative run.
- 50+ tasks only after adapter stability is proven.

Suggested buckets:

- `cj_applicable_python`
- `cj_applicable_typescript`
- `cj_inapplicable_terminal`
- `cj_false_positive_control`
- `cj_infra_heavy`

## Adapter Design

Add a thin external-runner adapter rather than folding Terminal-Bench tasks into
`bench/tasks/` immediately.

Proposed files:

```text
bench/external/terminal_bench/
  README.md
  select_tasks.py
  run_terminal_bench_matrix.py
  collect_results.py
  cj_agent_wrapper.md
```

Adapter responsibilities:

1. Install or locate the Terminal-Bench CLI.
2. Select task ids and record exact benchmark version/commit.
3. Run each task with a baseline agent instruction.
4. Run the same task with a Court Jester-aware agent instruction.
5. Capture changed files after each agent attempt.
6. For changed `.py` and `.ts` files, run:

   ```bash
   court-jester verify --file <path> --language <python|typescript> --project-dir <workspace>
   ```

7. Feed compact verify failures back to the agent only in repair policies.
8. Preserve Terminal-Bench's own pass/fail result as the final judge.
9. Write normalized result JSON that can be summarized alongside `bench/`
   results without pretending it is the same suite.

## Result Schema Additions

Each Terminal-Bench run should record:

- `terminal_bench_version`
- `terminal_bench_task_id`
- `terminal_bench_category`
- `terminal_bench_result`
- `terminal_bench_test_stdout_path`
- `terminal_bench_test_stderr_path`
- `court_jester_applicable`
- `court_jester_changed_files_checked`
- `court_jester_verify_failures`
- `court_jester_repair_attempts`
- `court_jester_blocked_final`
- `failure_category`

Suggested `failure_category` values:

- `success`
- `terminal_bench_fail_cj_not_applicable`
- `terminal_bench_fail_cj_passed`
- `verify_caught_then_repaired`
- `verify_caught_not_repaired`
- `verify_false_positive`
- `verify_infra_error`
- `agent_error`
- `terminal_bench_infra_error`

## Metrics

Headline metrics:

- Terminal-Bench pass rate by policy.
- Pass-rate delta on Court-Jester-applicable tasks.
- Pass-rate delta on all selected tasks.
- Court Jester applicability rate.
- Verify failure rate.
- Repair success after verify failure.
- False-positive rate on known-good or already-passing tasks.
- Median added wall-clock time per task.

Diagnostic metrics:

- Failures by stage: parse, lint runner, coverage, portability, execute, test.
- Execute failure kinds: crash vs property violation.
- Module-load blocked rate.
- No-inputs-reached rate.
- Python vs TypeScript split.
- Tasks where Terminal-Bench failed despite Court Jester passing.

## Guardrails

- Do not headline full Terminal-Bench score as a Court Jester product score.
  Most Terminal-Bench tasks are broader than Court Jester's target surface.
- Always report applicability. A pass-rate delta without applicability is
  misleading.
- Keep Terminal-Bench tests as the final judge.
- Track exact Terminal-Bench version, task list, agent, model, and policy.
- Keep Docker/container setup failures separate from product failures.
- Do not feed hidden test output to the agent unless the selected Terminal-Bench
  harness policy normally allows it.
- Treat `required-final` failures as recall evidence unless a repair attempt is
  allowed.

## First Pilot

1. Pin a Terminal-Bench version or commit.
2. Select 10 Python/TypeScript-heavy tasks.
3. Run `baseline` and `advisory` once to measure applicability and overhead.
4. Run `baseline` and `repair-loop` with one repeat.
5. Manually audit every case where:
   - Court Jester failed and Terminal-Bench passed.
   - Court Jester passed and Terminal-Bench failed.
   - Court Jester changed the repair trajectory.
6. Promote stable scripts into `bench/external/terminal_bench/`.

## Success Bar

The pilot is useful if it produces one of these outcomes:

- A measurable repair-loop lift on the Court-Jester-applicable subset.
- A clear false-positive cluster with concrete fixes.
- A clear inapplicability map that sharpens product positioning.
- A set of real Terminal-Bench failures that point to next verifier features.

The pilot is not useful if it only reports an aggregate Terminal-Bench score
without explaining when Court Jester actually ran and what signal it provided.
