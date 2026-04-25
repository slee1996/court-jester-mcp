from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

from .common import BENCH_ROOT, REPO_ROOT, TaskManifest, load_task, load_task_set


DEFAULT_OUTPUT = BENCH_ROOT / "results" / "autoresearch" / "signature-contracts"
DEFAULT_TASK_LIMIT = 40


def task_paths(
    task_ids: list[str] | None,
    task_set: str | None,
    expected_failure_kind: str | None,
) -> list[Path]:
    selected_task_ids = list(task_ids or [])
    if task_set:
        selected = load_task_set(BENCH_ROOT / "task_sets" / f"{task_set}.json")
        selected_task_ids.extend(selected.task_ids)
    if selected_task_ids:
        seen = set()
        paths = []
        for task_id in selected_task_ids:
            if task_id in seen:
                continue
            seen.add(task_id)
            paths.append(BENCH_ROOT / "tasks" / f"{task_id}.json")
        return paths
    paths = []
    for path in sorted((BENCH_ROOT / "tasks").glob("*.json")):
        data = json.loads(path.read_text())
        expected = data.get("expected_verify_outcome")
        if expected not in {"fail", "pass"}:
            continue
        if (
            expected_failure_kind
            and expected == "fail"
            and expected_failure_kind not in data.get("expected_verify_failure_kinds", [])
        ):
            continue
        paths.append(path)
    return paths


def run_json_command(command: list[str], *, cwd: Path, timeout_seconds: float) -> dict[str, Any]:
    started = time.time()
    try:
        proc = subprocess.run(
            command,
            cwd=cwd,
            text=True,
            capture_output=True,
            timeout=timeout_seconds,
            env={**os.environ, "COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS": "5"},
        )
    except subprocess.TimeoutExpired as exc:
        return {
            "timed_out": True,
            "duration_ms": int((time.time() - started) * 1000),
            "exit_code": None,
            "stdout": exc.stdout or "",
            "stderr": exc.stderr or str(exc),
        }

    try:
        parsed = json.loads(proc.stdout)
    except json.JSONDecodeError:
        parsed = None
    return {
        "timed_out": False,
        "duration_ms": int((time.time() - started) * 1000),
        "exit_code": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
        "json": parsed,
    }


def stage_map(report: dict[str, Any] | None) -> dict[str, dict[str, Any]]:
    if not isinstance(report, dict):
        return {}
    return {
        stage.get("name"): stage
        for stage in report.get("stages", [])
        if isinstance(stage, dict) and stage.get("name")
    }


def first_execute_failure(report: dict[str, Any] | None) -> dict[str, Any] | None:
    execute = stage_map(report).get("execute")
    if not execute:
        return None
    failures = (execute.get("detail") or {}).get("fuzz_failures") or []
    if not failures:
        return None
    first = failures[0]
    if not isinstance(first, dict):
        return None
    return {
        "function": first.get("function"),
        "input": first.get("input"),
        "error_type": first.get("error_type"),
        "message": first.get("message"),
        "severity": first.get("severity"),
    }


def evidence_kind(failure: dict[str, Any] | None, report: dict[str, Any] | None) -> str:
    if not failure:
        stages = stage_map(report)
        failed = [name for name, stage in stages.items() if stage.get("ok") is False]
        return failed[0] if failed else "none"

    text = " ".join(
        str(failure.get(key) or "")
        for key in ["function", "input", "error_type", "message", "severity"]
    ).lower()
    if (
        "query semantics" in text
        or "query parse semantics" in text
        or "nullish string leak" in text
    ):
        return "mapping_serializer_semantics"
    if "samevaluezero" in text or "same value zero" in text:
        return "same_value_zero_semantics"
    if "pep 440" in text or "pep440" in text:
        return "pep440_semantics"
    if "cookie value quote" in text or "cookie header quote" in text or "cookie header quoting" in text:
        return "cookie_quote_semantics"
    if "http request metadata" in text:
        return "http_request_metadata_semantics"
    if "http response helpers" in text:
        return "http_response_helper_semantics"
    if "http static file middleware" in text:
        return "http_static_file_semantics"
    if "semver" in text or "prerelease" in text or "caret" in text:
        return "domain_semver_signature"
    if "blank string output" in text:
        return "structured_nonempty_string"
    if "comparator" in text or "antisymmetry" in text:
        return "comparator_signature"
    if "inconsistent" in text:
        return "determinism_signature"
    if failure.get("severity") == "crash":
        return "typed_input_crash"
    if failure.get("severity") == "property_violation":
        return "generic_property"
    return "unknown"


def classify(expected: str | None, actual_failed: bool | None, timed_out: bool) -> str:
    if timed_out:
        return "timeout"
    if expected == "fail" and actual_failed is True:
        return "true_positive"
    if expected == "fail" and actual_failed is False:
        return "miss"
    if expected == "pass" and actual_failed is False:
        return "true_negative"
    if expected == "pass" and actual_failed is True:
        return "false_positive"
    return "unscored"


def materialize_task(
    task: TaskManifest,
    run_dir: Path,
    *,
    use_task_gold_patches: bool,
) -> tuple[Path | None, dict[str, Any], dict[str, Any] | None]:
    fixture = BENCH_ROOT / "repos" / task.repo_fixture
    workspace = (run_dir / "workspaces" / task.id).resolve()
    if workspace.exists():
        shutil.rmtree(workspace)
    if not fixture.exists():
        return (
            None,
            {"success": False, "failure_reason": f"missing fixture: {fixture}"},
            None,
        )
    shutil.copytree(fixture, workspace)
    setup_dir = run_dir / "setup" / task.id
    setup_dir.mkdir(parents=True, exist_ok=True)
    started = time.time()
    commands = []
    for idx, raw_command in enumerate(task.setup_commands):
        command = [
            part.replace("{workspace}", str(workspace)).replace("{bench_root}", str(BENCH_ROOT))
            for part in raw_command
        ]
        proc = subprocess.run(command, cwd=workspace, text=True, capture_output=True)
        (setup_dir / f"setup_{idx}.stdout.txt").write_text(proc.stdout)
        (setup_dir / f"setup_{idx}.stderr.txt").write_text(proc.stderr)
        commands.append(
            {
                "argv": command,
                "exit_code": proc.returncode,
                "stdout_path": str(setup_dir / f"setup_{idx}.stdout.txt"),
                "stderr_path": str(setup_dir / f"setup_{idx}.stderr.txt"),
            }
        )
        if proc.returncode != 0:
            return (
                workspace,
                {
                    "success": False,
                    "cache_hit": False,
                    "duration_ms": int((time.time() - started) * 1000),
                    "commands": commands,
                    "failure_reason": proc.stderr.strip() or proc.stdout.strip(),
                },
                None,
            )
    setup = {
        "success": True,
        "cache_hit": False,
        "duration_ms": int((time.time() - started) * 1000),
        "commands": commands,
        "failure_reason": None,
    }
    gold_patch = apply_task_gold_patch(task, workspace, run_dir) if use_task_gold_patches else None
    return workspace, setup, gold_patch


def apply_task_gold_patch(
    task: TaskManifest,
    workspace: Path,
    run_dir: Path,
) -> dict[str, Any]:
    patch_dir = run_dir / "gold_patch" / task.id
    patch_dir.mkdir(parents=True, exist_ok=True)
    if not task.gold_patch_path:
        return {
            "success": False,
            "failure_reason": "Task gold patch mode requested but task.gold_patch_path is not set",
            "command": None,
        }
    patch_path = workspace / task.gold_patch_path
    if not patch_path.exists():
        return {
            "success": False,
            "failure_reason": f"Task gold patch not found: {patch_path}",
            "command": None,
        }
    command = ["patch", "-p1", "-i", str(patch_path.resolve())]
    started = time.time()
    proc = subprocess.run(command, cwd=workspace, text=True, capture_output=True)
    stdout_path = patch_dir / "gold_patch.stdout.txt"
    stderr_path = patch_dir / "gold_patch.stderr.txt"
    stdout_path.write_text(proc.stdout)
    stderr_path.write_text(proc.stderr)
    return {
        "success": proc.returncode == 0,
        "duration_ms": int((time.time() - started) * 1000),
        "failure_reason": None
        if proc.returncode == 0
        else proc.stderr.strip() or proc.stdout.strip() or "task gold patch apply failed",
        "command": {
            "argv": command,
            "exit_code": proc.returncode,
            "stdout_path": str(stdout_path),
            "stderr_path": str(stderr_path),
        },
    }


def verify_task(
    task: TaskManifest,
    workspace: Path,
    court_jester: Path,
    *,
    report_level: str,
    timeout_seconds: float,
) -> dict[str, Any]:
    path_results = []
    for relative in task.verify_paths:
        source_path = workspace / relative
        command = [
            str(court_jester),
            "verify",
            "--file",
            str(source_path),
            "--language",
            task.language,
            "--project-dir",
            str(workspace),
            "--report-level",
            report_level,
        ]
        result = run_json_command(command, cwd=workspace, timeout_seconds=timeout_seconds)
        report = result.get("json")
        failed = None
        if isinstance(report, dict) and isinstance(report.get("overall_ok"), bool):
            failed = not report["overall_ok"]
        failure = first_execute_failure(report)
        path_results.append(
            {
                "path": relative,
                "failed": failed,
                "duration_ms": result["duration_ms"],
                "timed_out": result["timed_out"],
                "exit_code": result["exit_code"],
                "summary": report.get("summary") if isinstance(report, dict) else None,
                "stages": {
                    name: stage.get("ok")
                    for name, stage in stage_map(report).items()
                },
                "failure": failure,
                "evidence_kind": "timeout"
                if result["timed_out"]
                else evidence_kind(failure, report),
                "stderr": result.get("stderr", "")[-1200:],
            }
        )
    failed_values = [row["failed"] for row in path_results if row["failed"] is not None]
    actual_failed = any(failed_values) if failed_values else None
    primary_failure = next((row for row in path_results if row["failure"]), None)
    timed_out = any(row["timed_out"] for row in path_results)
    return {
        "actual_failed": actual_failed,
        "timed_out": timed_out,
        "paths": path_results,
        "primary_evidence_kind": (
            "timeout"
            if timed_out
            else primary_failure["evidence_kind"]
            if primary_failure
            else "none"
        ),
    }


def summarize(rows: list[dict[str, Any]]) -> dict[str, Any]:
    counts = Counter(row["classification"] for row in rows)
    by_evidence: dict[str, Counter[str]] = defaultdict(Counter)
    by_bucket: dict[str, Counter[str]] = defaultdict(Counter)
    for row in rows:
        by_evidence[row["primary_evidence_kind"]][row["classification"]] += 1
        by_bucket[row["bucket"]][row["classification"]] += 1

    return {
        "counts": dict(counts),
        "by_evidence_kind": {
            key: dict(value) for key, value in sorted(by_evidence.items())
        },
        "by_bucket": {key: dict(value) for key, value in sorted(by_bucket.items())},
        "misses": [
            {
                "task_id": row["task_id"],
                "bucket": row["bucket"],
                "expected_failure_kinds": row["expected_verify_failure_kinds"],
            }
            for row in rows
            if row["classification"] == "miss"
        ][:20],
        "false_positives": [
            {
                "task_id": row["task_id"],
                "bucket": row["bucket"],
                "evidence_kind": row["primary_evidence_kind"],
                "failure": next(
                    (path["failure"] for path in row["verify"]["paths"] if path["failure"]),
                    None,
                ),
            }
            for row in rows
            if row["classification"] == "false_positive"
        ][:20],
    }


def run(args: argparse.Namespace) -> Path:
    started_ms = int(time.time() * 1000)
    run_dir = args.output.resolve() / f"run-{time.time_ns()}"
    run_dir.mkdir(parents=True, exist_ok=True)

    paths = task_paths(args.task_id, args.task_set, args.expected_failure_kind)
    if args.limit:
        paths = paths[: args.limit]

    rows = []
    for path in paths:
        task = load_task(path)
        workspace, setup, gold_patch = materialize_task(
            task,
            run_dir,
            use_task_gold_patches=args.use_task_gold_patches,
        )
        if workspace is None or not setup["success"]:
            rows.append(
                {
                    "task_id": task.id,
                    "bucket": task.bucket,
                    "expected_verify_outcome": task.expected_verify_outcome,
                    "expected_verify_failure_kinds": task.expected_verify_failure_kinds,
                    "classification": "setup_error",
                    "primary_evidence_kind": "setup",
                    "setup": setup,
                }
            )
            continue
        if gold_patch is not None and not gold_patch["success"]:
            rows.append(
                {
                    "task_id": task.id,
                    "bucket": task.bucket,
                    "expected_verify_outcome": "pass"
                    if args.use_task_gold_patches
                    else task.expected_verify_outcome,
                    "expected_verify_failure_kinds": []
                    if args.use_task_gold_patches
                    else task.expected_verify_failure_kinds,
                    "classification": "gold_patch_error",
                    "primary_evidence_kind": "gold_patch",
                    "setup": setup,
                    "gold_patch": gold_patch,
                }
            )
            continue

        verify = verify_task(
            task,
            workspace,
            args.court_jester,
            report_level=args.report_level,
            timeout_seconds=args.timeout_seconds,
        )
        expected_verify_outcome = "pass" if args.use_task_gold_patches else task.expected_verify_outcome
        expected_verify_failure_kinds = (
            [] if args.use_task_gold_patches else task.expected_verify_failure_kinds
        )
        classification = classify(
            expected_verify_outcome,
            verify["actual_failed"],
            verify["timed_out"],
        )
        rows.append(
            {
                "task_id": task.id,
                "title": task.title,
                "language": task.language,
                "bucket": task.bucket,
                "expected_verify_outcome": expected_verify_outcome,
                "expected_verify_failure_kinds": expected_verify_failure_kinds,
                "classification": classification,
                "primary_evidence_kind": verify["primary_evidence_kind"],
                "setup": setup,
                "gold_patch": gold_patch,
                "verify": verify,
            }
        )

    ledger = {
        "started_at_epoch_ms": started_ms,
        "court_jester": str(args.court_jester),
        "task_count": len(rows),
        "mode": "fixed_gold_patch_no_test_file"
        if args.use_task_gold_patches
        else "signature_only_no_test_file",
        "task_set": args.task_set,
        "summary": summarize(rows),
        "rows": rows,
    }
    (run_dir / "ledger.json").write_text(json.dumps(ledger, indent=2, sort_keys=True))
    print(json.dumps(ledger["summary"], indent=2, sort_keys=True))
    return run_dir


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Autoresearch CJ's signature/context-derived verification without test files."
    )
    parser.add_argument(
        "--court-jester",
        type=Path,
        default=REPO_ROOT / "target" / "release" / "court-jester",
    )
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--task-id", action="append")
    parser.add_argument("--task-set")
    parser.add_argument("--expected-failure-kind")
    parser.add_argument("--limit", type=int, default=DEFAULT_TASK_LIMIT)
    parser.add_argument("--report-level", choices=["minimal", "full"], default="minimal")
    parser.add_argument("--timeout-seconds", type=float, default=25.0)
    parser.add_argument("--use-task-gold-patches", action="store_true")
    args = parser.parse_args()
    run(args)


if __name__ == "__main__":
    main()
