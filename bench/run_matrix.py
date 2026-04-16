from __future__ import annotations

import argparse
import hashlib
import json
import random
import time
from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from threading import Lock
from typing import Any

from .common import (
    BENCH_ROOT,
    ModelManifest,
    PolicyManifest,
    TaskManifest,
    load_manifest_dir,
    load_model,
    load_policy,
    load_task,
    load_task_set,
)
from .providers import terminate_active_provider_processes
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
        "--schedule",
        choices=["task-major", "blocked-random", "fully-random"],
        default="blocked-random",
        help=(
            "Execution order for the matrix. "
            "'blocked-random' keeps the same task+repeat cells together while randomizing order to reduce drift bias."
        ),
    )
    parser.add_argument(
        "--shuffle-seed",
        type=int,
        default=0,
        help="Deterministic seed used by randomized schedules.",
    )
    parser.add_argument(
        "--use-task-gold-patches",
        action="store_true",
        help="Apply task-level gold patches instead of asking a provider to edit the fixture.",
    )
    parser.add_argument(
        "--parallel-by-provider",
        action="store_true",
        help=(
            "Run one serial queue per provider concurrently. "
            "Cells for the same provider keep their relative order; different providers run in parallel."
        ),
    )
    parser.add_argument("--dry-run", action="store_true", help="Expand the matrix without executing it.")
    return parser.parse_args()


def pick(items: list[object], wanted: set[str]) -> list[object]:
    if not wanted:
        return items
    return [item for item in items if getattr(item, "id") in wanted]


def hidden_seed_for(task_id: str, repeat_index: int) -> str:
    return hashlib.sha256(f"{task_id}::repeat::{repeat_index}".encode("utf-8")).hexdigest()


def build_run_plan(
    tasks: list[TaskManifest],
    models: list[ModelManifest],
    policies: list[PolicyManifest],
    *,
    repeats: int,
    schedule: str,
    shuffle_seed: int,
) -> list[dict[str, Any]]:
    if schedule == "task-major":
        plan: list[dict[str, Any]] = []
        for task in tasks:
            for model in models:
                for policy in policies:
                    for repeat_index in range(repeats):
                        plan.append(
                            {
                                "task": task,
                                "model": model,
                                "policy": policy,
                                "repeat_index": repeat_index,
                                "hidden_seed": hidden_seed_for(task.id, repeat_index),
                            }
                        )
        return plan

    rng = random.Random(shuffle_seed)
    if schedule == "fully-random":
        plan = []
        for task in tasks:
            for model in models:
                for policy in policies:
                    for repeat_index in range(repeats):
                        plan.append(
                            {
                                "task": task,
                                "model": model,
                                "policy": policy,
                                "repeat_index": repeat_index,
                                "hidden_seed": hidden_seed_for(task.id, repeat_index),
                            }
                        )
        rng.shuffle(plan)
        return plan

    blocks: list[list[dict[str, Any]]] = []
    for task in tasks:
        for repeat_index in range(repeats):
            block: list[dict[str, Any]] = []
            for model in models:
                for policy in policies:
                    block.append(
                        {
                            "task": task,
                            "model": model,
                            "policy": policy,
                            "repeat_index": repeat_index,
                            "hidden_seed": hidden_seed_for(task.id, repeat_index),
                        }
                    )
            rng.shuffle(block)
            blocks.append(block)
    rng.shuffle(blocks)
    return [cell for block in blocks for cell in block]


def partition_plan_by_provider(plan: list[dict[str, Any]]) -> OrderedDict[str, list[dict[str, Any]]]:
    queues: OrderedDict[str, list[dict[str, Any]]] = OrderedDict()
    for cell in plan:
        provider_id = cell["model"].provider
        queues.setdefault(provider_id, []).append(cell)
    return queues


def execute_cell(
    cell: dict[str, Any],
    *,
    output_dir: Path,
    dry_run: bool,
    repeats: int,
    use_task_gold_patches: bool,
) -> tuple[bool, str]:
    task = cell["task"]
    model = cell["model"]
    policy = cell["policy"]
    repeat_index = int(cell["repeat_index"])
    result = run_single(
        task,
        model,
        policy,
        output_dir,
        dry_run=dry_run,
        repeat_index=repeat_index,
        repeat_count=repeats,
        hidden_seed=str(cell["hidden_seed"]),
        use_task_gold_patches=use_task_gold_patches,
    )
    status = result["status"]
    success = result.get("success", False)
    line = (
        f"[{status}] task={task.id} model={model.id} "
        f"policy={policy.id} repeat={repeat_index + 1}/{repeats} success={success}"
    )
    return success, line


def run_serial_plan(
    plan: list[dict[str, Any]],
    *,
    output_dir: Path,
    dry_run: bool,
    repeats: int,
    use_task_gold_patches: bool,
) -> tuple[int, int]:
    total = 0
    successes = 0
    for cell in plan:
        total += 1
        success, line = execute_cell(
            cell,
            output_dir=output_dir,
            dry_run=dry_run,
            repeats=repeats,
            use_task_gold_patches=use_task_gold_patches,
        )
        if success:
            successes += 1
        print(line)
    return total, successes


def run_parallel_provider_plan(
    plan: list[dict[str, Any]],
    *,
    output_dir: Path,
    dry_run: bool,
    repeats: int,
    use_task_gold_patches: bool,
) -> tuple[int, int]:
    provider_queues = partition_plan_by_provider(plan)
    if len(provider_queues) <= 1:
        return run_serial_plan(
            plan,
            output_dir=output_dir,
            dry_run=dry_run,
            repeats=repeats,
            use_task_gold_patches=use_task_gold_patches,
        )

    print_lock = Lock()

    def worker(cells: list[dict[str, Any]]) -> int:
        local_successes = 0
        for cell in cells:
            success, line = execute_cell(
                cell,
                output_dir=output_dir,
                dry_run=dry_run,
                repeats=repeats,
                use_task_gold_patches=use_task_gold_patches,
            )
            if success:
                local_successes += 1
            with print_lock:
                print(line)
        return local_successes

    successes = 0
    with ThreadPoolExecutor(max_workers=len(provider_queues)) as executor:
        futures = [executor.submit(worker, cells) for cells in provider_queues.values()]
        try:
            for future in futures:
                successes += future.result()
        except KeyboardInterrupt:
            executor.shutdown(wait=False, cancel_futures=True)
            terminate_active_provider_processes()
            raise
    return len(plan), successes


def main() -> int:
    args = parse_args()
    tasks = load_manifest_dir(BENCH_ROOT / "tasks", load_task)
    models = load_manifest_dir(BENCH_ROOT / "models", load_model)
    policies = load_manifest_dir(BENCH_ROOT / "policies", load_policy)
    task_sets = load_manifest_dir(BENCH_ROOT / "task_sets", load_task_set) if (BENCH_ROOT / "task_sets").exists() else []

    requested_tasks = set(filter(None, args.tasks.split(",")))
    selected_task_set = None
    if args.task_set:
        matched = [item for item in task_sets if item.id == args.task_set]
        if not matched:
            available = ", ".join(item.id for item in task_sets) or "<none>"
            raise SystemExit(f"Unknown task set '{args.task_set}'. Available: {available}")
        selected_task_set = matched[0]
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
    plan = build_run_plan(
        selected_tasks,
        selected_models,
        selected_policies,
        repeats=repeats,
        schedule=args.schedule,
        shuffle_seed=args.shuffle_seed,
    )
    matrix_metadata = {
        "created_at_epoch_ms": int(time.time() * 1000),
        "task_ids": [task.id for task in selected_tasks],
        "model_ids": [model.id for model in selected_models],
        "policy_ids": [policy.id for policy in selected_policies],
        "task_set_id": args.task_set or None,
        "task_set_title": selected_task_set.title if selected_task_set else None,
        "task_set_goal": selected_task_set.goal if selected_task_set else None,
        "task_set_suite_kind": selected_task_set.suite_kind if selected_task_set else None,
        "repeats": repeats,
        "schedule": args.schedule,
        "shuffle_seed": args.shuffle_seed,
        "dry_run": args.dry_run,
        "use_task_gold_patches": args.use_task_gold_patches,
        "parallel_by_provider": args.parallel_by_provider,
        "provider_ids": list(partition_plan_by_provider(plan).keys()),
        "expected_total": len(plan),
    }
    (output_dir / "matrix.json").write_text(json.dumps(matrix_metadata, indent=2, sort_keys=True) + "\n")

    try:
        if args.parallel_by_provider:
            total, successes = run_parallel_provider_plan(
                plan,
                output_dir=output_dir,
                dry_run=args.dry_run,
                repeats=repeats,
                use_task_gold_patches=args.use_task_gold_patches,
            )
        else:
            total, successes = run_serial_plan(
                plan,
                output_dir=output_dir,
                dry_run=args.dry_run,
                repeats=repeats,
                use_task_gold_patches=args.use_task_gold_patches,
            )
    except KeyboardInterrupt:
        terminate_active_provider_processes()
        raise

    print(f"matrix complete: {total} runs, {successes} succeeded")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
