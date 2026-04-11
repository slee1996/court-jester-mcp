from __future__ import annotations

import argparse
import hashlib
from pathlib import Path

from .common import (
    BENCH_ROOT,
    load_manifest_dir,
    load_model,
    load_policy,
    load_task,
    load_task_set,
)
from .runner import run_single


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run court-jester benchmark matrix.")
    parser.add_argument("--tasks", default="", help="Comma-separated task ids.")
    parser.add_argument("--task-set", default="", help="Task set id from bench/task_sets.")
    parser.add_argument("--models", default="", help="Comma-separated model ids.")
    parser.add_argument("--policies", default="", help="Comma-separated policy ids.")
    parser.add_argument(
        "--output-dir",
        default=str(BENCH_ROOT / "results" / "dev"),
        help="Directory for benchmark run artifacts.",
    )
    parser.add_argument(
        "--repeats",
        type=int,
        default=1,
        help="Number of repeated runs for each task/model/policy combination.",
    )
    parser.add_argument(
        "--use-task-gold-patches",
        action="store_true",
        help="Apply task-level gold patches instead of asking a provider to edit the fixture.",
    )
    parser.add_argument("--dry-run", action="store_true", help="Expand the matrix without executing it.")
    return parser.parse_args()


def pick(items: list[object], wanted: set[str]) -> list[object]:
    if not wanted:
        return items
    return [item for item in items if getattr(item, "id") in wanted]


def main() -> int:
    args = parse_args()
    tasks = load_manifest_dir(BENCH_ROOT / "tasks", load_task)
    models = load_manifest_dir(BENCH_ROOT / "models", load_model)
    policies = load_manifest_dir(BENCH_ROOT / "policies", load_policy)
    task_sets = load_manifest_dir(BENCH_ROOT / "task_sets", load_task_set) if (BENCH_ROOT / "task_sets").exists() else []

    requested_tasks = set(filter(None, args.tasks.split(",")))
    if args.task_set:
        matched = [item for item in task_sets if item.id == args.task_set]
        if not matched:
            available = ", ".join(item.id for item in task_sets) or "<none>"
            raise SystemExit(f"Unknown task set '{args.task_set}'. Available: {available}")
        requested_tasks.update(matched[0].task_ids)
    selected_tasks = pick(tasks, requested_tasks)
    requested_models = set(filter(None, args.models.split(",")))
    if requested_models:
        selected_models = pick(models, requested_models)
    else:
        selected_models = [model for model in models if getattr(model, "enabled_by_default", False)]
    selected_policies = pick(policies, set(filter(None, args.policies.split(","))))
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    repeats = max(args.repeats, 1)

    total = 0
    successes = 0
    for task in selected_tasks:
        paired_hidden_seeds = [
            hashlib.sha256(f"{task.id}::repeat::{repeat_index}".encode("utf-8")).hexdigest()
            for repeat_index in range(repeats)
        ]
        for model in selected_models:
            for policy in selected_policies:
                for repeat_index in range(repeats):
                    total += 1
                    result = run_single(
                        task,
                        model,
                        policy,
                        output_dir,
                        dry_run=args.dry_run,
                        repeat_index=repeat_index,
                        repeat_count=repeats,
                        hidden_seed=paired_hidden_seeds[repeat_index],
                        use_task_gold_patches=args.use_task_gold_patches,
                    )
                    status = result["status"]
                    success = result.get("success", False)
                    if success:
                        successes += 1
                    print(
                        f"[{status}] task={task.id} model={model.id} "
                        f"policy={policy.id} repeat={repeat_index + 1}/{repeats} success={success}"
                    )

    print(f"matrix complete: {total} runs, {successes} succeeded")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
