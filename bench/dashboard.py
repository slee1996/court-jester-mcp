"""Court Jester Bench Dashboard — live run watcher.

Usage:
    python -m bench.dashboard [--port 8777] [--results-dir bench/results/dev]
"""
from __future__ import annotations

import argparse
import json
from http.server import HTTPServer, SimpleHTTPRequestHandler
from pathlib import Path
from urllib.parse import parse_qs, urlparse

BENCH_ROOT = Path(__file__).resolve().parent


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Court Jester bench dashboard server.")
    parser.add_argument("--port", type=int, default=8777)
    parser.add_argument(
        "--results-dir",
        default=str(BENCH_ROOT / "results" / "dev"),
        help="Directory containing run result dirs.",
    )
    return parser.parse_args()


def load_results(results_dir: Path) -> list[dict]:
    rows = []
    if not results_dir.exists():
        return rows
    for run_dir in sorted(path for path in results_dir.iterdir() if path.is_dir()):
        result_path = run_dir / "result.json"
        run_path = run_dir / "run.json"
        target_path = result_path if result_path.exists() else run_path if run_path.exists() else None
        if target_path is None:
            continue
        try:
            row = json.loads(target_path.read_text())
        except (json.JSONDecodeError, OSError):
            continue
        if target_path == run_path and "status" not in row:
            row["status"] = "running"
        rows.append(row)
    return rows


def load_matrix_metadata(results_dir: Path) -> dict:
    matrix_path = results_dir / "matrix.json"
    if not matrix_path.exists():
        return {}
    try:
        data = json.loads(matrix_path.read_text())
    except (json.JSONDecodeError, OSError):
        return {}
    return data if isinstance(data, dict) else {}


def load_status(results_dir: Path) -> dict:
    rows = load_results(results_dir)
    run_dirs = [path for path in results_dir.iterdir() if path.is_dir()] if results_dir.exists() else []
    completed = sum(1 for row in rows if row.get("status") != "running")
    running = sum(1 for row in rows if row.get("status") == "running")
    bare_running = 0
    for run_dir in run_dirs:
        if (run_dir / "result.json").exists():
            continue
        if not (run_dir / "run.json").exists():
            bare_running += 1
    running += bare_running
    metadata = load_matrix_metadata(results_dir)
    expected_total = metadata.get("expected_total")
    if not isinstance(expected_total, int):
        expected_total = None
    return {
        "results_dir": str(results_dir),
        "completed_runs": completed,
        "running_runs": running,
        "observed_runs": max(len(rows), completed + running),
        "expected_total": expected_total,
        "metadata": metadata,
    }


def load_tasks(bench_root: Path) -> list[dict]:
    tasks = []
    tasks_dir = bench_root / "tasks"
    if not tasks_dir.exists():
        return tasks
    for path in sorted(tasks_dir.glob("*.json")):
        try:
            tasks.append(json.loads(path.read_text()))
        except (json.JSONDecodeError, OSError):
            continue
    return tasks


def load_policies(bench_root: Path) -> list[dict]:
    policies = []
    policies_dir = bench_root / "policies"
    if not policies_dir.exists():
        return policies
    for path in sorted(policies_dir.glob("*.json")):
        try:
            policies.append(json.loads(path.read_text()))
        except (json.JSONDecodeError, OSError):
            continue
    return policies


def load_models(bench_root: Path) -> list[dict]:
    models = []
    models_dir = bench_root / "models"
    if not models_dir.exists():
        return models
    for path in sorted(models_dir.glob("*.json")):
        try:
            models.append(json.loads(path.read_text()))
        except (json.JSONDecodeError, OSError):
            continue
    return models


class DashboardHandler(SimpleHTTPRequestHandler):
    results_dir: Path = Path(".")

    def do_GET(self) -> None:
        parsed = urlparse(self.path)
        if parsed.path == "/api/runs":
            self._json_response(load_results(self.results_dir))
        elif parsed.path == "/api/status":
            self._json_response(load_status(self.results_dir))
        elif parsed.path == "/api/manifests":
            data = {
                "tasks": load_tasks(BENCH_ROOT),
                "policies": load_policies(BENCH_ROOT),
                "models": load_models(BENCH_ROOT),
            }
            self._json_response(data)
        elif parsed.path == "/api/run":
            params = parse_qs(parsed.query)
            run_id = params.get("id", [None])[0]
            if run_id:
                result_path = self.results_dir / run_id / "result.json"
                if result_path.exists():
                    self._json_response(json.loads(result_path.read_text()))
                else:
                    self._json_response({"error": "not found"}, status=404)
            else:
                self._json_response({"error": "missing id"}, status=400)
        elif parsed.path == "/" or parsed.path == "/index.html":
            html_path = BENCH_ROOT / "dashboard.html"
            content = html_path.read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(content)))
            self.end_headers()
            self.wfile.write(content)
        else:
            self.send_response(404)
            self.end_headers()

    def _json_response(self, data: object, status: int = 200) -> None:
        body = json.dumps(data).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        # Suppress per-request logging noise
        pass


def main() -> int:
    args = parse_args()
    results_dir = Path(args.results_dir).resolve()
    DashboardHandler.results_dir = results_dir

    server = HTTPServer(("127.0.0.1", args.port), DashboardHandler)
    print(f"Court Jester Dashboard → http://127.0.0.1:{args.port}")
    print(f"Watching: {results_dir}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
