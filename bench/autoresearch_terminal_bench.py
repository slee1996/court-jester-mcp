from __future__ import annotations

import argparse
import ast
import json
import os
import re
import shutil
import subprocess
import time
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_DATASET = REPO_ROOT / "tmp" / "terminal-bench-datasets" / "quixbugs"
DEFAULT_OUTPUT = REPO_ROOT / "bench" / "results" / "autoresearch" / "terminal-bench-quixbugs"
HEREDOC_RE = re.compile(
    r"cat\s*>\s*/app/(?P<name>[^\s]+)\s*<<\s*'(?P<tag>[^']+)'\n"
    r"(?P<body>.*?)(?:\n(?P=tag)\n)",
    re.S,
)


@dataclass(slots=True)
class TaskPaths:
    task_id: str
    task_dir: Path
    source_file: Path
    fixed_body: str
    json_cases: list[Any] | None


@dataclass(slots=True)
class TaskSliceInfo:
    task_id: str
    function_name: str | None
    arity: int | None
    annotated_params: int
    has_return_annotation: bool
    has_json_cases: bool
    json_case_count: int
    input_shapes: list[str]
    expected_shapes: list[str]
    slices: list[str]


def primary_python_file(task_dir: Path) -> Path | None:
    files = [path for path in task_dir.glob("*.py") if not path.name.startswith("fixed_")]
    return files[0] if len(files) == 1 else None


def solution_body(task_dir: Path) -> str | None:
    solution = task_dir / "solution.sh"
    if not solution.exists():
        return None
    match = HEREDOC_RE.search(solution.read_text())
    return match.group("body") if match else None


def load_json_cases(task_dir: Path, stem: str) -> list[Any] | None:
    path = task_dir / "tests" / f"{stem}.json"
    if not path.exists():
        return None
    rows = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if line:
            rows.append(json.loads(line))
    return rows or None


def discover_tasks(dataset: Path) -> list[TaskPaths]:
    tasks: list[TaskPaths] = []
    for task_dir in sorted(dataset.glob("quixbugs-python-*")):
        source_file = primary_python_file(task_dir)
        fixed_body = solution_body(task_dir)
        if source_file is None or fixed_body is None:
            continue
        tasks.append(
            TaskPaths(
                task_id=task_dir.name,
                task_dir=task_dir,
                source_file=source_file,
                fixed_body=fixed_body,
                json_cases=load_json_cases(task_dir, source_file.stem),
            )
        )
    return tasks


def first_function_signature(source_file: Path) -> tuple[str | None, int | None, int, bool]:
    try:
        tree = ast.parse(source_file.read_text())
    except SyntaxError:
        return None, None, 0, False
    for node in tree.body:
        if isinstance(node, ast.FunctionDef):
            params = [
                arg
                for arg in [*node.args.posonlyargs, *node.args.args, *node.args.kwonlyargs]
                if arg.arg not in {"self", "cls"}
            ]
            return (
                node.name,
                len(params),
                sum(1 for arg in params if arg.annotation is not None),
                node.returns is not None,
            )
    return None, None, 0, False


def value_shape(value: Any) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, (int, float)):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        if not value:
            return "empty_list"
        child_shapes = {value_shape(item) for item in value}
        if any(shape.endswith("list") or "list" in shape for shape in child_shapes):
            return "nested_list"
        if len(child_shapes) == 1:
            return f"{next(iter(child_shapes))}_list"
        return "mixed_list"
    if isinstance(value, dict):
        return "object"
    return type(value).__name__


def json_case_shapes(cases: list[Any] | None) -> tuple[list[str], list[str]]:
    if not cases:
        return [], []
    input_shapes: list[str] = []
    expected_shapes: list[str] = []
    for row in cases:
        if not isinstance(row, list) or len(row) != 2:
            continue
        inputs, expected = row
        if isinstance(inputs, list):
            input_shapes.extend(value_shape(item) for item in inputs)
        else:
            input_shapes.append(value_shape(inputs))
        expected_shapes.append(value_shape(expected))
    return sorted(set(input_shapes)), sorted(set(expected_shapes))


def task_slice_info(task: TaskPaths) -> TaskSliceInfo:
    function_name, arity, annotated_params, has_return_annotation = first_function_signature(
        task.source_file
    )
    input_shapes, expected_shapes = json_case_shapes(task.json_cases)
    source_text = task.source_file.read_text()
    combined_text = f"{source_text}\n{task.fixed_body}"
    slices = ["all_python"]
    if task.json_cases:
        slices.append("json_oracle")
    else:
        slices.extend(["no_json_oracle", "object_or_graph_fixture"])
    if arity is not None:
        slices.append(f"arity_{arity}")
        if arity <= 1:
            slices.append("low_arity")
        else:
            slices.append("multi_arg")
    if annotated_params or has_return_annotation:
        slices.append("typed_or_annotated")
    else:
        slices.append("untyped_signature")
    if set(input_shapes).issubset({"number", "bool"}) and input_shapes:
        slices.append("primitive_numeric_inputs")
    if any("list" in shape for shape in input_shapes):
        slices.append("collection_inputs")
    if any(shape == "nested_list" for shape in input_shapes + expected_shapes):
        slices.append("nested_collection")
    if "yield" in combined_text:
        slices.append("generator_like")
    if any(shape in {"number", "bool"} for shape in expected_shapes):
        slices.append("scalar_expected")
    if any("list" in shape for shape in expected_shapes):
        slices.append("collection_expected")
    if any(keyword in (function_name or task.task_id) for keyword in ["sort", "search", "find"]):
        slices.append("order_or_search_name")
    return TaskSliceInfo(
        task_id=task.task_id,
        function_name=function_name,
        arity=arity,
        annotated_params=annotated_params,
        has_return_annotation=has_return_annotation,
        has_json_cases=bool(task.json_cases),
        json_case_count=len(task.json_cases or []),
        input_shapes=input_shapes,
        expected_shapes=expected_shapes,
        slices=sorted(set(slices)),
    )


def build_slice_infos(tasks: list[TaskPaths]) -> dict[str, TaskSliceInfo]:
    return {task.task_id: task_slice_info(task) for task in tasks}


def slice_members(slice_infos: dict[str, TaskSliceInfo]) -> dict[str, list[str]]:
    members: dict[str, list[str]] = defaultdict(list)
    for info in slice_infos.values():
        for name in info.slices:
            members[name].append(info.task_id)
    return {name: sorted(task_ids) for name, task_ids in sorted(members.items())}


def slice_counts(
    rows: list[dict[str, Any]],
    slice_infos: dict[str, TaskSliceInfo],
) -> dict[str, dict[str, int]]:
    by_slice: dict[str, Counter[str]] = defaultdict(Counter)
    for row in rows:
        task_id = row.get("task_id")
        info = slice_infos.get(task_id)
        if info is None:
            continue
        classification = row.get("classification", "unknown")
        for name in info.slices:
            by_slice[name][classification] += 1
    return {name: dict(counts) for name, counts in sorted(by_slice.items())}


def run_cj(
    *,
    court_jester: Path,
    file_path: Path,
    project_dir: Path,
    test_file: Path | None = None,
    tests_only: bool = False,
    timeout_seconds: float = 30.0,
) -> dict[str, Any]:
    env = os.environ.copy()
    env.setdefault("COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS", "5")
    command = [
        str(court_jester),
        "verify",
        "--file",
        str(file_path),
        "--language",
        "python",
        "--project-dir",
        str(project_dir),
        "--report-level",
        "minimal",
    ]
    if test_file is not None:
        command.extend(["--test-file", str(test_file)])
    if tests_only:
        command.append("--tests-only")

    started = time.time()
    try:
        proc = subprocess.run(
            command,
            text=True,
            capture_output=True,
            timeout=timeout_seconds,
            env=env,
        )
    except subprocess.TimeoutExpired as exc:
        return {
            "command_timeout": True,
            "duration_ms": int((time.time() - started) * 1000),
            "overall_ok": None,
            "exit_code": None,
            "stderr": str(exc),
            "stage_status": {},
            "test_error": None,
            "first_execute_failure": None,
        }

    try:
        report = json.loads(proc.stdout)
    except json.JSONDecodeError:
        report = {"stdout": proc.stdout, "stderr": proc.stderr}

    stages = report.get("stages", []) if isinstance(report, dict) else []
    stage_status = {
        stage.get("name"): stage.get("ok")
        for stage in stages
        if isinstance(stage, dict)
    }
    test_error = None
    first_execute_failure = None
    for stage in stages:
        if not isinstance(stage, dict):
            continue
        if stage.get("name") == "test":
            test_error = stage.get("error")
        if stage.get("name") == "execute":
            failures = (stage.get("detail") or {}).get("fuzz_failures") or []
            if failures:
                first = failures[0]
                first_execute_failure = {
                    "function": first.get("function"),
                    "input": first.get("input"),
                    "error_type": first.get("error_type"),
                    "message": first.get("message"),
                    "severity": first.get("severity"),
                }
            elif not stage.get("ok"):
                first_execute_failure = {"error": stage.get("error")}

    return {
        "command_timeout": False,
        "duration_ms": int((time.time() - started) * 1000),
        "overall_ok": report.get("overall_ok") if isinstance(report, dict) else None,
        "exit_code": proc.returncode,
        "stderr": proc.stderr,
        "summary": report.get("summary") if isinstance(report, dict) else None,
        "stage_status": stage_status,
        "test_error": test_error,
        "first_execute_failure": first_execute_failure,
    }


def classify(original: dict[str, Any], fixed: dict[str, Any]) -> str:
    original_ok = original.get("overall_ok")
    fixed_ok = fixed.get("overall_ok")
    if original_ok is False and fixed_ok is True:
        return "clean_true_positive"
    if original_ok is False and fixed_ok is False:
        return "fixed_still_fails"
    if original_ok is True and fixed_ok is True:
        return "miss_buggy_passes"
    if original_ok is True and fixed_ok is False:
        return "fixed_regression_or_noise"
    return "infra_or_timeout"


def copy_pair(task: TaskPaths, work_root: Path, label: str) -> tuple[Path, Path]:
    original = work_root / f"{task.task_id}-{label}-original"
    fixed = work_root / f"{task.task_id}-{label}-fixed"
    for path in [original, fixed]:
        if path.exists():
            shutil.rmtree(path)
        shutil.copytree(task.task_dir, path)
    (fixed / task.source_file.name).write_text(task.fixed_body)
    return original, fixed


def write_test_file(workspace: Path, stem: str, cases: list[Any], *, normalized: bool) -> Path:
    path = workspace / "cj_verify_cases.py"
    if normalized:
        source = f"""\
from collections.abc import Generator
from math import isclose
from {stem} import {stem}

CASES = {repr(cases)}

def _norm(value):
    if isinstance(value, Generator):
        value = list(value)
    if isinstance(value, tuple):
        return [_norm(item) for item in value]
    if isinstance(value, list):
        return [_norm(item) for item in value]
    return value

def _eq(actual, expected):
    actual = _norm(actual)
    expected = _norm(expected)
    if isinstance(actual, float) and isinstance(expected, float):
        return isclose(actual, expected, rel_tol=1e-6, abs_tol=1e-6)
    return actual == expected

for input_data, expected in CASES:
    actual = {stem}(*input_data)
    assert _eq(actual, expected), f"input={{input_data!r}} expected={{expected!r}} actual={{actual!r}}"
"""
    else:
        source = f"""\
from {stem} import {stem}

CASES = {repr(cases)}
for input_data, expected in CASES:
    actual = {stem}(*input_data)
    assert actual == expected, f"input={{input_data!r}} expected={{expected!r}} actual={{actual!r}}"
"""
    path.write_text(source)
    return path


def run_generic_iteration(
    tasks: list[TaskPaths],
    run_dir: Path,
    court_jester: Path,
    slice_infos: dict[str, TaskSliceInfo],
) -> dict[str, Any]:
    rows = []
    work_root = run_dir / "workspaces"
    work_root.mkdir(parents=True, exist_ok=True)
    for task in tasks:
        original_result = run_cj(
            court_jester=court_jester,
            file_path=task.source_file,
            project_dir=task.task_dir,
            timeout_seconds=20.0,
        )
        original, fixed = copy_pair(task, work_root, "generic")
        fixed_result = run_cj(
            court_jester=court_jester,
            file_path=fixed / task.source_file.name,
            project_dir=fixed,
            timeout_seconds=20.0,
        )
        rows.append(
            {
                "task_id": task.task_id,
                "classification": classify(original_result, fixed_result),
                "original": original_result,
                "fixed": fixed_result,
            }
        )
    return summarize_iteration("generic_fuzz", rows, slice_infos)


def run_tests_only_iteration(
    tasks: list[TaskPaths],
    run_dir: Path,
    court_jester: Path,
    slice_infos: dict[str, TaskSliceInfo],
    *,
    normalized: bool,
) -> dict[str, Any]:
    rows = []
    work_root = run_dir / "workspaces"
    work_root.mkdir(parents=True, exist_ok=True)
    for task in tasks:
        if not task.json_cases:
            rows.append({"task_id": task.task_id, "classification": "skipped_no_json_cases"})
            continue
        original, fixed = copy_pair(task, work_root, "normalized" if normalized else "raw")
        original_test = write_test_file(original, task.source_file.stem, task.json_cases, normalized=normalized)
        fixed_test = write_test_file(fixed, task.source_file.stem, task.json_cases, normalized=normalized)
        original_result = run_cj(
            court_jester=court_jester,
            file_path=original / task.source_file.name,
            project_dir=original,
            test_file=original_test,
            tests_only=True,
            timeout_seconds=35.0,
        )
        fixed_result = run_cj(
            court_jester=court_jester,
            file_path=fixed / task.source_file.name,
            project_dir=fixed,
            test_file=fixed_test,
            tests_only=True,
            timeout_seconds=35.0,
        )
        rows.append(
            {
                "task_id": task.task_id,
                "classification": classify(original_result, fixed_result),
                "original": original_result,
                "fixed": fixed_result,
            }
        )
    return summarize_iteration(
        "tests_only_normalized" if normalized else "tests_only_raw",
        rows,
        slice_infos,
    )


def summarize_iteration(
    name: str,
    rows: list[dict[str, Any]],
    slice_infos: dict[str, TaskSliceInfo],
) -> dict[str, Any]:
    counts: dict[str, int] = {}
    for row in rows:
        key = row.get("classification", "unknown")
        counts[key] = counts.get(key, 0) + 1
    return {
        "name": name,
        "counts": counts,
        "slice_counts": slice_counts(rows, slice_infos),
        "rows": rows,
    }


def compact_iteration(iteration: dict[str, Any]) -> dict[str, Any]:
    return {
        "name": iteration["name"],
        "counts": iteration["counts"],
        "slice_counts": iteration.get("slice_counts", {}),
        "sample_failures": [
            {
                "task_id": row.get("task_id"),
                "classification": row.get("classification"),
                "original_failure": (row.get("original") or {}).get("first_execute_failure")
                or (row.get("original") or {}).get("test_error"),
                "fixed_failure": (row.get("fixed") or {}).get("first_execute_failure")
                or (row.get("fixed") or {}).get("test_error"),
            }
            for row in iteration["rows"]
            if row.get("classification") in {"fixed_still_fails", "fixed_regression_or_noise"}
        ][:10],
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Autoresearch loop for Terminal-Bench/QuixBugs Court Jester stress testing."
    )
    parser.add_argument("--dataset", type=Path, default=DEFAULT_DATASET)
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--court-jester", type=Path, default=REPO_ROOT / "target" / "release" / "court-jester")
    parser.add_argument("--limit", type=int, default=0)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    started = int(time.time() * 1000)
    run_dir = args.output_dir.resolve() / f"run-{time.time_ns()}"
    run_dir.mkdir(parents=True, exist_ok=True)
    tasks = discover_tasks(args.dataset)
    if args.limit > 0:
        tasks = tasks[: args.limit]
    slice_infos = build_slice_infos(tasks)
    slices = slice_members(slice_infos)
    (run_dir / "slices.json").write_text(
        json.dumps(
            {
                "tasks": {
                    task_id: {
                        "function_name": info.function_name,
                        "arity": info.arity,
                        "annotated_params": info.annotated_params,
                        "has_return_annotation": info.has_return_annotation,
                        "has_json_cases": info.has_json_cases,
                        "json_case_count": info.json_case_count,
                        "input_shapes": info.input_shapes,
                        "expected_shapes": info.expected_shapes,
                        "slices": info.slices,
                    }
                    for task_id, info in sorted(slice_infos.items())
                },
                "slices": slices,
            },
            indent=2,
        )
    )

    ledger: dict[str, Any] = {
        "started_at_epoch_ms": started,
        "dataset": str(args.dataset),
        "court_jester": str(args.court_jester),
        "task_count": len(tasks),
        "slice_count": len(slices),
        "slices_path": str(run_dir / "slices.json"),
        "iterations": [],
        "product_lift_notes": [],
    }

    iteration_plan = [
        (
            "generic_fuzz",
            lambda: run_generic_iteration(
                tasks, run_dir / "generic_fuzz", args.court_jester, slice_infos
            ),
        ),
        (
            "tests_only_raw",
            lambda: run_tests_only_iteration(
                tasks,
                run_dir / "tests_only_raw",
                args.court_jester,
                slice_infos,
                normalized=False,
            ),
        ),
        (
            "tests_only_normalized",
            lambda: run_tests_only_iteration(
                tasks,
                run_dir / "tests_only_normalized",
                args.court_jester,
                slice_infos,
                normalized=True,
            ),
        ),
    ]

    for name, thunk in iteration_plan:
        print(f"running iteration: {name}", flush=True)
        iteration = thunk()
        (run_dir / f"{name}.json").write_text(json.dumps(iteration, indent=2))
        compact = compact_iteration(iteration)
        ledger["iterations"].append(compact)
        (run_dir / "ledger.json").write_text(json.dumps(ledger, indent=2))
        print(json.dumps({"name": name, "counts": iteration["counts"]}, indent=2), flush=True)

    ledger["product_lift_notes"] = [
        "Generic fuzz has low precision on untyped algorithm tasks when oracle-fixed code still fails.",
        "Slice counts separate primitive numeric, collection, nested collection, generator-like, and no-JSON graph/object tasks so Terminal-Bench can be used as a recurring synth stress gate.",
        "Authoritative tests-only mode is a cleaner integration point for external benchmark fixtures.",
        "Normalizing generator outputs, tuple/list structures, and float tolerances should reduce fixed-code false positives.",
        "Tasks without simple JSON cases need fixture-aware importers rather than arbitrary fuzzing.",
    ]
    ledger["ended_at_epoch_ms"] = int(time.time() * 1000)
    (run_dir / "ledger.json").write_text(json.dumps(ledger, indent=2))
    print(f"wrote {run_dir / 'ledger.json'}", flush=True)


if __name__ == "__main__":
    main()
