from __future__ import annotations

import argparse
import json
import random
import statistics
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from ..common import BENCH_ROOT
from ..mcp_client import CourtJesterClient


@dataclass(slots=True)
class StressRequestResult:
    worker_id: int
    request_index: int
    tool: str
    language: str
    duration_ms: int
    ok: bool
    error_kind: str | None
    error_message: str | None
    process_pid: int | None
    process_return_code: int | None
    stderr_tail: str | None
    overall_ok: bool | None
    parsed_exit_code: int | None
    parsed_timed_out: bool | None
    parsed_memory_error: bool | None
    parsed_stderr_summary: str | None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run Court Jester stress scenarios.")
    parser.add_argument("--scenario", required=True, help="Scenario id from bench/stress/scenarios.")
    parser.add_argument(
        "--output-dir",
        default=str(BENCH_ROOT / "results" / "stress"),
        help="Directory for stress run artifacts.",
    )
    return parser.parse_args()


def load_scenario(scenario_id: str) -> dict[str, Any]:
    path = BENCH_ROOT / "stress" / "scenarios" / f"{scenario_id}.json"
    if not path.exists():
        raise SystemExit(f"Unknown stress scenario '{scenario_id}': {path} does not exist")
    return json.loads(path.read_text())


def percentile(values: list[int], ratio: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, int(round((len(ordered) - 1) * ratio))))
    return float(ordered[index])


def build_arguments(tool: str, payload: dict[str, Any]) -> dict[str, Any]:
    arguments = {
        "language": payload["language"],
        "code": payload["code"],
    }
    if tool == "verify":
        arguments["output_dir"] = payload.get(
            "output_dir",
            str(BENCH_ROOT / "results" / "stress" / "_verify_reports"),
        )
        for key in (
            "complexity_threshold",
            "project_dir",
            "diff",
            "test_code",
            "test_file_path",
            "file_path",
        ):
            if key in payload:
                arguments[key] = payload[key]
    if tool == "execute":
        arguments["timeout_seconds"] = payload.get("timeout_seconds", 10.0)
        arguments["memory_mb"] = payload.get("memory_mb", 128)
        if "project_dir" in payload:
            arguments["project_dir"] = payload["project_dir"]
        if "file_path" in payload:
            arguments["file_path"] = payload["file_path"]
    if tool == "lint":
        if "file_path" in payload:
            arguments["file_path"] = payload["file_path"]
    return arguments


def choose_tool(request_mix: list[dict[str, Any]]) -> str:
    tools = [item["tool"] for item in request_mix]
    weights = [int(item.get("weight", 1)) for item in request_mix]
    return random.choices(tools, weights=weights, k=1)[0]


def classify_error(exc: Exception) -> str:
    text = str(exc).lower()
    if "timed out" in text:
        return "timeout"
    if "closed the connection" in text:
        return "connection_closed"
    return "error"


def summarize_text(value: str | None, limit: int = 240) -> str | None:
    if not value:
        return None
    for raw_line in value.splitlines():
        line = raw_line.strip()
        if line:
            return line[:limit]
    return None


def worker_run(
    worker_id: int,
    scenario: dict[str, Any],
    results: list[StressRequestResult],
    lock: threading.Lock,
) -> None:
    mode = scenario.get("mode", "per_agent_server")
    if mode != "per_agent_server":
        raise RuntimeError(
            f"Scenario mode '{mode}' is not implemented yet. Use 'per_agent_server' for now."
        )

    payloads = scenario["payloads"]
    request_mix = scenario["request_mix"]
    client = CourtJesterClient()
    client.start()
    try:
        for request_index in range(int(scenario["requests_per_worker"])):
            payload = payloads[request_index % len(payloads)]
            tool = choose_tool(request_mix)
            arguments = build_arguments(tool, payload)
            started = time.time()
            ok = False
            overall_ok = None
            error_kind = None
            error_message = None
            process_pid = None
            process_return_code = None
            stderr_tail = None
            parsed_exit_code = None
            parsed_timed_out = None
            parsed_memory_error = None
            parsed_stderr_summary = None
            try:
                response = client.call_tool(tool, arguments)
                parsed = response.get("result", {}).get("parsed")
                if isinstance(parsed, dict) and "overall_ok" in parsed:
                    overall_ok = bool(parsed.get("overall_ok"))
                if isinstance(parsed, dict):
                    if "exit_code" in parsed:
                        parsed_exit_code = parsed.get("exit_code")
                    if "timed_out" in parsed:
                        parsed_timed_out = bool(parsed.get("timed_out"))
                    if "memory_error" in parsed:
                        parsed_memory_error = bool(parsed.get("memory_error"))
                    parsed_stderr_summary = summarize_text(parsed.get("stderr"))
                ok = True
            except Exception as exc:
                error_kind = classify_error(exc)
                error_message = str(exc)
                context = dict(client.last_error_context)
                process_pid = context.get("pid")
                process_return_code = context.get("return_code")
                stderr_tail = context.get("stderr_tail")
                if error_kind in {"connection_closed", "timeout", "error"}:
                    client.restart()
            duration_ms = int((time.time() - started) * 1000)
            with lock:
                results.append(
                    StressRequestResult(
                        worker_id=worker_id,
                        request_index=request_index,
                        tool=tool,
                        language=payload["language"],
                        duration_ms=duration_ms,
                        ok=ok,
                        error_kind=error_kind,
                        error_message=error_message,
                        process_pid=process_pid,
                        process_return_code=process_return_code,
                        stderr_tail=stderr_tail,
                        overall_ok=overall_ok,
                        parsed_exit_code=parsed_exit_code,
                        parsed_timed_out=parsed_timed_out,
                        parsed_memory_error=parsed_memory_error,
                        parsed_stderr_summary=parsed_stderr_summary,
                    )
                )
    finally:
        client.close()


def main() -> int:
    args = parse_args()
    scenario = load_scenario(args.scenario)
    output_root = Path(args.output_dir)
    run_dir = output_root / scenario["id"] / time.strftime("%Y%m%dT%H%M%S")
    run_dir.mkdir(parents=True, exist_ok=True)

    results: list[StressRequestResult] = []
    lock = threading.Lock()
    threads: list[threading.Thread] = []
    started = time.time()

    for worker_id in range(int(scenario["concurrency"])):
        thread = threading.Thread(
            target=worker_run,
            args=(worker_id, scenario, results, lock),
            daemon=True,
        )
        threads.append(thread)
        thread.start()

    for thread in threads:
        thread.join()

    total_duration_ms = int((time.time() - started) * 1000)
    durations = [item.duration_ms for item in results]
    successes = sum(1 for item in results if item.ok)
    error_counts: dict[str, int] = {}
    tool_counts: dict[str, int] = {}
    process_exit_counts: dict[str, int] = {}
    parsed_timeout_count = 0
    parsed_memory_error_count = 0
    for item in results:
        tool_counts[item.tool] = tool_counts.get(item.tool, 0) + 1
        if item.error_kind:
            error_counts[item.error_kind] = error_counts.get(item.error_kind, 0) + 1
        if item.process_return_code is not None:
            key = str(item.process_return_code)
            process_exit_counts[key] = process_exit_counts.get(key, 0) + 1
        if item.parsed_timed_out:
            parsed_timeout_count += 1
        if item.parsed_memory_error:
            parsed_memory_error_count += 1

    summary = {
        "scenario_id": scenario["id"],
        "title": scenario.get("title"),
        "mode": scenario.get("mode"),
        "concurrency": scenario.get("concurrency"),
        "requests_per_worker": scenario.get("requests_per_worker"),
        "total_requests": len(results),
        "successes": successes,
        "success_rate": (successes / len(results)) if results else 0.0,
        "wall_clock_ms": total_duration_ms,
        "latency_ms": {
            "p50": percentile(durations, 0.50),
            "p95": percentile(durations, 0.95),
            "p99": percentile(durations, 0.99),
            "avg": statistics.mean(durations) if durations else 0.0,
        },
        "tool_counts": tool_counts,
        "error_counts": error_counts,
        "process_exit_counts": process_exit_counts,
        "parsed_execute_outcomes": {
            "timed_out_count": parsed_timeout_count,
            "memory_error_count": parsed_memory_error_count,
        },
    }

    (run_dir / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    with (run_dir / "requests.ndjson").open("w") as handle:
        for item in results:
            handle.write(
                json.dumps(
                    {
                        "worker_id": item.worker_id,
                        "request_index": item.request_index,
                        "tool": item.tool,
                        "language": item.language,
                        "duration_ms": item.duration_ms,
                        "ok": item.ok,
                        "error_kind": item.error_kind,
                        "error_message": item.error_message,
                        "process_pid": item.process_pid,
                        "process_return_code": item.process_return_code,
                        "stderr_tail": item.stderr_tail,
                        "overall_ok": item.overall_ok,
                        "parsed_exit_code": item.parsed_exit_code,
                        "parsed_timed_out": item.parsed_timed_out,
                        "parsed_memory_error": item.parsed_memory_error,
                        "parsed_stderr_summary": item.parsed_stderr_summary,
                    },
                    sort_keys=True,
                )
                + "\n"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
