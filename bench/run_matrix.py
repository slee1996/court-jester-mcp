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
import sys
from typing import Any

from .common import (
    ARTIFACT_SCHEMA_VERSION,
    VERIFY_SCHEMA_VERSION_REQUIRED,
    BENCH_ROOT,
    ModelManifest,
    PolicyManifest,
    TaskManifest,
    load_manifest_dir,
    load_model,
    load_policy,
    load_task,
    load_task_set,
    canonical_json,
    suite_lock_digest,
    suite_lock_projection,
    sha256_bytes,
    ArtifactVersionError,
)
from .providers import terminate_active_provider_processes
from .runner import run_single


MAX_DOCTOR_REPORT_AGE_SECONDS = 60 * 60


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
    parser.add_argument("--verify-runtime-profile", choices=["local-trusted", "isolated"], default="local-trusted")
    parser.add_argument("--doctor-report", type=Path)
    parser.add_argument("--python-docker-image", default="python:3.12-slim")
    parser.add_argument("--typescript-docker-image", default="node:24-bookworm-slim")
    parser.add_argument("--verify-memory-mb", type=int, default=512)
    parser.add_argument("--verify-network", choices=["deny", "allow"], default="deny")
    parser.add_argument("--write-heldout-lock", type=Path)
    parser.add_argument("--enforce-heldout-lock", action="store_true")
    parser.add_argument("--shadow-records", type=Path)
    parser.add_argument("--summary-json", type=Path)
    parser.add_argument("--baseline-policy", default="baseline")
    parser.add_argument("--candidate-policy", default="repair-loop-verify-only")
    parser.add_argument("--bootstrap-samples", type=int, default=10000)
    parser.add_argument("--known-good-summary", action="append", default=[])
    parser.add_argument("--gate-policy", choices=["none", "private-beta-default", "strict-heldout"], default="none")
    parser.add_argument("--fail-on-gate", action="store_true")
    parser.add_argument("--evidence-bundle", action="store_true")
    parser.add_argument("--evidence-redaction", choices=["none", "transcripts", "all-text"], default="transcripts")
    parser.add_argument("--strict-evidence", action="store_true")
    return parser.parse_args()


def pick(items: list[object], wanted: set[str]) -> list[object]:
    if not wanted:
        return items
    return [item for item in items if getattr(item, "id") in wanted]


def hidden_seed_for(task_id: str, model_id: str, repeat_index: int) -> str:
    return hashlib.sha256(f"{task_id}::{model_id}::repeat::{repeat_index}".encode("utf-8")).hexdigest()


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
                                "hidden_seed": hidden_seed_for(task.id, model.id, repeat_index),
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
                                "hidden_seed": hidden_seed_for(task.id, model.id, repeat_index),
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
                            "hidden_seed": hidden_seed_for(task.id, model.id, repeat_index),
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


def _passed_doctor_check(
    checks: list[dict[str, Any]],
    name: str,
    language: str | None,
) -> dict[str, Any] | None:
    matches = [
        check
        for check in checks
        if check.get("name") == name and check.get("language") == language
    ]
    if len(matches) != 1 or matches[0].get("status") != "passed":
        return None
    return matches[0]


def validate_doctor_report(
    report: Any,
    *,
    runtime_profile: str,
    selected_languages: set[str],
    python_docker_image: str,
    typescript_docker_image: str,
) -> None:
    if not isinstance(report, dict):
        raise ValueError("doctor report must be a JSON object")
    if type(report.get("schema_version")) is not int or report["schema_version"] != VERIFY_SCHEMA_VERSION_REQUIRED:
        raise ValueError(f"doctor report schema_version must be exactly {VERIFY_SCHEMA_VERSION_REQUIRED}")
    if report.get("verdict") != "pass":
        raise ValueError("doctor report verdict must be pass")
    if report.get("runtime_profile") != runtime_profile:
        raise ValueError("doctor report runtime_profile does not match the selected profile")
    checks = report.get("checks")
    if not isinstance(checks, list) or not checks or not all(isinstance(check, dict) for check in checks):
        raise ValueError("doctor report must contain structured readiness checks")
    if any(check.get("status") in {"failed", "inconclusive"} for check in checks):
        raise ValueError("doctor report pass verdict contradicts a failed readiness check")

    if runtime_profile == "local-trusted":
        for language in sorted(selected_languages):
            runtime = _passed_doctor_check(checks, "runtime", language)
            detail = runtime.get("detail") if runtime else None
            version = detail.get("version") if isinstance(detail, dict) else None
            if not isinstance(version, str) or not version.strip():
                raise ValueError(f"doctor report lacks passed runtime readiness for {language}")
            if language == "typescript":
                major_text = version.strip().lstrip("v").split(".", 1)[0]
                if not major_text.isdigit() or int(major_text) < 24:
                    raise ValueError("doctor report requires Node.js >=24 readiness")
        return

    daemon = _passed_doctor_check(checks, "docker_daemon", None)
    daemon_detail = daemon.get("detail") if daemon else None
    if not isinstance(daemon_detail, dict) or daemon_detail.get("network") != "none" or daemon_detail.get("read_only") is not True:
        raise ValueError("doctor report lacks isolated Docker daemon readiness")
    selected_images = {
        "python": python_docker_image,
        "typescript": typescript_docker_image,
    }
    for language in sorted(selected_languages):
        image = selected_images[language]
        image_check = _passed_doctor_check(checks, "docker_image", language)
        image_detail = image_check.get("detail") if image_check else None
        if (
            not isinstance(image_detail, dict)
            or image_detail.get("image") != image
            or not isinstance(image_detail.get("id"), str)
            or not image_detail["id"].strip()
        ):
            raise ValueError(f"doctor report lacks selected image readiness for {language}")
        smoke = _passed_doctor_check(checks, "runtime_smoke", language)
        smoke_detail = smoke.get("detail") if smoke else None
        if (
            not isinstance(smoke_detail, dict)
            or smoke_detail.get("image") != image
            or smoke_detail.get("network") != "none"
            or smoke_detail.get("read_only") is not True
            or type(smoke_detail.get("memory_mb")) is not int
            or smoke_detail["memory_mb"] <= 0
        ):
            raise ValueError(f"doctor report lacks isolated runtime smoke readiness for {language}")


def execute_cell(
    cell: dict[str, Any],
    *,
    output_dir: Path,
    dry_run: bool,
    repeats: int,
    use_task_gold_patches: bool,
    verify_runtime_profile: str = "local-trusted",
    python_docker_image: str = "python:3.12-slim",
    typescript_docker_image: str = "node:24-bookworm-slim",
    verify_memory_mb: int = 512,
    verify_network: str = "deny",
    doctor_report: dict[str, Any] | None = None,
    shadow_records: Path | None = None,
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
        verify_runtime_profile=verify_runtime_profile,
        python_docker_image=python_docker_image,
        typescript_docker_image=typescript_docker_image,
        verify_memory_mb=verify_memory_mb,
        verify_network=verify_network,
        doctor_report=doctor_report,
        shadow_records=shadow_records,
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
    verify_runtime_profile: str = "local-trusted",
    python_docker_image: str = "python:3.12-slim",
    typescript_docker_image: str = "node:24-bookworm-slim",
    verify_memory_mb: int = 512,
    verify_network: str = "deny",
    doctor_report: dict[str, Any] | None = None,
    shadow_records: Path | None = None,
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
            verify_runtime_profile=verify_runtime_profile,
            python_docker_image=python_docker_image,
            typescript_docker_image=typescript_docker_image,
            verify_memory_mb=verify_memory_mb,
            verify_network=verify_network,
            doctor_report=doctor_report,
            shadow_records=shadow_records,
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
    verify_runtime_profile: str = "local-trusted",
    python_docker_image: str = "python:3.12-slim",
    typescript_docker_image: str = "node:24-bookworm-slim",
    verify_memory_mb: int = 512,
    verify_network: str = "deny",
    doctor_report: dict[str, Any] | None = None,
    shadow_records: Path | None = None,
) -> tuple[int, int]:
    provider_queues = partition_plan_by_provider(plan)
    if len(provider_queues) <= 1:
        return run_serial_plan(
            plan,
            output_dir=output_dir,
            dry_run=dry_run,
            repeats=repeats,
            use_task_gold_patches=use_task_gold_patches,
            verify_runtime_profile=verify_runtime_profile,
            python_docker_image=python_docker_image,
            typescript_docker_image=typescript_docker_image,
            verify_memory_mb=verify_memory_mb,
            verify_network=verify_network,
            doctor_report=doctor_report,
            shadow_records=shadow_records,
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
                verify_runtime_profile=verify_runtime_profile,
                python_docker_image=python_docker_image,
                typescript_docker_image=typescript_docker_image,
                verify_memory_mb=verify_memory_mb,
                verify_network=verify_network,
                doctor_report=doctor_report,
                shadow_records=shadow_records,
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
    if args.verify_memory_mb <= 0:
        raise SystemExit("--verify-memory-mb must be greater than zero")
    if args.verify_runtime_profile == "isolated" and args.verify_network == "allow":
        raise SystemExit("--verify-network allow is incompatible with isolated verification")
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
    if not selected_tasks:
        raise SystemExit("matrix selection contains no tasks")
    if not selected_models:
        raise SystemExit("matrix selection contains no models")
    if not selected_policies:
        raise SystemExit("matrix selection contains no policies")
    doctor_payload: dict[str, Any] | None = None
    if not args.dry_run:
        if not args.doctor_report:
            raise SystemExit("--doctor-report is required for non-dry runs")
        try:
            doctor_age = time.time() - args.doctor_report.stat().st_mtime
            if doctor_age < -300 or doctor_age > MAX_DOCTOR_REPORT_AGE_SECONDS:
                raise ValueError("doctor report is stale or has an invalid modification time")
            doctor_payload = json.loads(args.doctor_report.read_text())
            validate_doctor_report(
                doctor_payload,
                runtime_profile=args.verify_runtime_profile,
                selected_languages={task.language for task in selected_tasks},
                python_docker_image=args.python_docker_image,
                typescript_docker_image=args.typescript_docker_image,
            )
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            raise SystemExit(f"invalid doctor report: {exc}") from exc
    elif args.doctor_report:
        try:
            doctor_payload = json.loads(args.doctor_report.read_text())
        except (OSError, json.JSONDecodeError) as exc:
            raise SystemExit(f"invalid doctor report: {exc}") from exc
    lock_digest = None
    lock_closure: list[dict[str, str]] = []
    if selected_task_set:
        lock_digest = suite_lock_digest(selected_task_set, selected_tasks)
        _, lock_closure = suite_lock_projection(selected_task_set, selected_tasks)
        if args.write_heldout_lock:
            if not args.dry_run:
                raise SystemExit("--write-heldout-lock is dry-run-only")
            args.write_heldout_lock.write_text(json.dumps({"lock_version": selected_task_set.lock_version, "task_set_id": selected_task_set.id, "locked_suite_sha256": lock_digest, "closure": lock_closure}, indent=2, sort_keys=True) + "\n")
        if args.enforce_heldout_lock and selected_task_set.locked_suite_sha256 != lock_digest:
            raise SystemExit("held-out suite lock mismatch")
    if args.shadow_records and not args.shadow_records.parent.exists():
        raise SystemExit(f"shadow records parent does not exist: {args.shadow_records.parent}")
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    summary_path = args.summary_json or output_dir / "summary.json"
    gate_payload: dict[str, Any] = {
        "policy": args.gate_policy,
        "eligible": False,
        "passed": False,
        "failures": ["summary_not_computed"],
        "metrics": {},
    }
    summary_failed = False
    interrupted = False
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
        "artifact_schema_version": ARTIFACT_SCHEMA_VERSION,
        "verify_schema_version_required": VERIFY_SCHEMA_VERSION_REQUIRED,
        "verify_runtime_profile": args.verify_runtime_profile,
        "runtime_images": {"python": args.python_docker_image, "typescript": args.typescript_docker_image},
        "verify_memory_mb": args.verify_memory_mb,
        "verify_network": args.verify_network,
        "verification_policy": {
            "network_policy": args.verify_network,
            "runtime_profile": args.verify_runtime_profile,
            "memory_mb": args.verify_memory_mb,
            "runtime_images": {"python": args.python_docker_image, "typescript": args.typescript_docker_image},
            "typed_cause_precedence": [
                "target_code:gating",
                "blocking_diagnostic",
                "inconclusive",
                "advisory",
            ],
        },
        "doctor_report_sha256": sha256_bytes(args.doctor_report.read_bytes()) if args.doctor_report and args.doctor_report.exists() else None,
        "expected_suite_digest": lock_digest,
        "observed_suite_digest": lock_digest,
        "heldout_closure": lock_closure,
        "heldout_lock_enforced": args.enforce_heldout_lock,
        "summary_json": str(summary_path),
        "gate_policy": args.gate_policy,
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

    total = 0
    successes = 0
    try:
        common_kwargs = {
            "verify_runtime_profile": args.verify_runtime_profile,
            "python_docker_image": args.python_docker_image,
            "typescript_docker_image": args.typescript_docker_image,
            "verify_memory_mb": args.verify_memory_mb,
            "verify_network": args.verify_network,
            "doctor_report": doctor_payload,
            "shadow_records": args.shadow_records,
        }
        if args.parallel_by_provider:
            total, successes = run_parallel_provider_plan(plan, output_dir=output_dir, dry_run=args.dry_run, repeats=repeats, use_task_gold_patches=args.use_task_gold_patches, **common_kwargs)
        else:
            total, successes = run_serial_plan(plan, output_dir=output_dir, dry_run=args.dry_run, repeats=repeats, use_task_gold_patches=args.use_task_gold_patches, **common_kwargs)
    except KeyboardInterrupt:
        terminate_active_provider_processes()
        interrupted = True
    try:
        from .summarize_runs import build_summary, evaluate_gate

        summary_payload = build_summary(output_dir, args.baseline_policy, args.candidate_policy, args.bootstrap_samples)
        if not isinstance(summary_payload, dict):
            raise TypeError("build_summary did not return an object")
        summary_payload.setdefault("artifact_schema_version", ARTIFACT_SCHEMA_VERSION)
        summary_payload.setdefault("verify_schema_version_required", VERIFY_SCHEMA_VERSION_REQUIRED)
        known_good = [json.loads(Path(path).read_text()) for path in args.known_good_summary]
        gate_payload = evaluate_gate(
            summary_payload,
            policy=args.gate_policy,
            known_good_summaries=known_good,
        )
        summary_payload["gate"] = gate_payload
        summary_path.parent.mkdir(parents=True, exist_ok=True)
        summary_path.write_text(json.dumps(summary_payload, indent=2, sort_keys=True) + "\n")
    except Exception as exc:
        summary_failed = True
        gate_payload = {
            "policy": args.gate_policy,
            "eligible": False,
            "passed": False,
            "failures": [f"summary_error:{type(exc).__name__}:{exc}"],
            "metrics": {},
        }
        failure_summary = {
            "artifact_schema_version": ARTIFACT_SCHEMA_VERSION,
            "verify_schema_version_required": VERIFY_SCHEMA_VERSION_REQUIRED,
            "gate": gate_payload,
        }
        try:
            summary_path.parent.mkdir(parents=True, exist_ok=True)
            summary_path.write_text(json.dumps(failure_summary, indent=2, sort_keys=True) + "\n")
        except OSError as write_exc:
            print(f"failed to write benchmark summary: {write_exc}", file=sys.stderr)
        print(f"failed to summarize benchmark matrix: {exc}", file=sys.stderr)
    if args.evidence_bundle:
        try:
            from .evidence import build_evidence_bundle
            build_evidence_bundle(output_dir, output_dir / "evidence", redaction=args.evidence_redaction, strict=args.strict_evidence)
        except Exception:
            if args.strict_evidence:
                raise
    if summary_failed:
        return 1
    if interrupted:
        return 130
    if args.fail_on_gate and (not gate_payload.get("eligible") or not gate_payload.get("passed")):
        return 1

    print(f"matrix complete: {total} runs, {successes} succeeded")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
