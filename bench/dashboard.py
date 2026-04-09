"""Court Jester Bench Dashboard — live run watcher.

Usage:
    python -m bench.dashboard [--port 8777] [--results-dir bench/results/dev]
"""
from __future__ import annotations

import argparse
import json
import os
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
    for path in sorted(results_dir.glob("*/result.json")):
        try:
            rows.append(json.loads(path.read_text()))
        except (json.JSONDecodeError, OSError):
            continue
    return rows


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
