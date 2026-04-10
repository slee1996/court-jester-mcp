#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import select
import signal
import subprocess
import sys
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent

# Bundled fixture used by --verify-sample. Kept in one place so
# `scripts/smoke_mcp.py --verify-sample` stays a hands-off end-to-end check
# that doesn't require the caller to remember any paths.
SAMPLE_FIXTURE = {
    "verify_file": REPO_ROOT / "bench/repos/mini_py_service/profile.py",
    "language": "python",
    "project_dir": REPO_ROOT / "bench/repos/mini_py_service",
    "test_file": REPO_ROOT
    / "bench/repos/mini_py_service/tests/court_jester_public_verify.py",
}


class McpSmokeClient:
    def __init__(self, command: list[str]) -> None:
        self.command = command
        self.proc: subprocess.Popen[str] | None = None
        self._next_id = 1

    def __enter__(self) -> "McpSmokeClient":
        self.start()
        return self

    def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
        self.close()

    def start(self) -> None:
        self.proc = subprocess.Popen(
            self.command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            cwd=REPO_ROOT,
            start_new_session=True,
        )
        response = self.request(
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "court-jester-smoke", "version": "0.1"},
            },
        )
        result = response.get("result", {})
        server_info = result.get("serverInfo", {})
        name = server_info.get("name", "<unknown>")
        version = server_info.get("version", "<unknown>")
        print(f"Connected to {name} {version}")
        self.notify("notifications/initialized", {})

    def close(self) -> None:
        if self.proc is None:
            return
        proc = self.proc
        if proc.stdin is not None:
            try:
                proc.stdin.close()
            except OSError:
                pass
        if proc.poll() is None:
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except (OSError, ProcessLookupError):
                pass
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            if proc.poll() is None:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except (OSError, ProcessLookupError):
                    pass
            proc.wait(timeout=2)
        for handle in (proc.stdout, proc.stderr):
            if handle is None:
                continue
            try:
                handle.close()
            except OSError:
                pass
        self.proc = None

    def request(
        self,
        method: str,
        params: dict[str, Any],
        timeout_seconds: float = 30.0,
    ) -> dict[str, Any]:
        if self.proc is None or self.proc.stdin is None or self.proc.stdout is None:
            raise RuntimeError("court-jester process is not running")
        request_id = self._next_id
        self._next_id += 1
        message = {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        }
        self.proc.stdin.write(json.dumps(message) + "\n")
        self.proc.stdin.flush()
        readable, _, _ = select.select([self.proc.stdout], [], [], timeout_seconds)
        if not readable:
            raise TimeoutError(f"Timed out waiting for {method}")
        line = self.proc.stdout.readline()
        if not line:
            raise RuntimeError(f"court-jester closed the connection during {method}")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(f"court-jester {method} error: {response['error']}")
        return response

    def notify(self, method: str, params: dict[str, Any]) -> None:
        if self.proc is None or self.proc.stdin is None:
            raise RuntimeError("court-jester process is not running")
        message = {"jsonrpc": "2.0", "method": method, "params": params}
        self.proc.stdin.write(json.dumps(message) + "\n")
        self.proc.stdin.flush()

    def list_tools(self) -> list[str]:
        response = self.request("tools/list", {})
        tools = response["result"].get("tools", [])
        return [tool["name"] for tool in tools]

    def call_tool(self, name: str, arguments: dict[str, Any]) -> Any:
        response = self.request(
            "tools/call",
            {"name": name, "arguments": arguments},
            timeout_seconds=120.0,
        )
        content = response["result"].get("content", [])
        if not content:
            return response["result"]
        text = content[0].get("text", "")
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return text


def resolve_command(args: argparse.Namespace) -> list[str]:
    if args.binary:
        return [str(Path(args.binary).expanduser().resolve())]
    profile = "debug" if args.debug else "release"
    binary = REPO_ROOT / "target" / profile / "court-jester-mcp"
    if binary.exists():
        return [str(binary)]
    raise FileNotFoundError(
        f"Could not find {binary}. Build it first with `cargo build --{profile}`."
    )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Smoke-test the Court Jester MCP server over stdio."
    )
    profile = parser.add_mutually_exclusive_group()
    profile.add_argument(
        "--release",
        action="store_true",
        help="Use target/release/court-jester-mcp (default).",
    )
    profile.add_argument(
        "--debug",
        action="store_true",
        help="Use target/debug/court-jester-mcp.",
    )
    parser.add_argument(
        "--binary",
        help="Use an explicit binary path instead of target/{release,debug}/court-jester-mcp.",
    )
    parser.add_argument(
        "--verify-file",
        help="Optional source file to verify after the MCP handshake succeeds.",
    )
    parser.add_argument(
        "--language",
        choices=["python", "typescript"],
        help="Language for --verify-file.",
    )
    parser.add_argument(
        "--project-dir",
        help="Optional project directory for import and dependency resolution.",
    )
    parser.add_argument(
        "--test-file",
        help="Optional explicit test file to include in the verify call.",
    )
    parser.add_argument(
        "--verify-sample",
        action="store_true",
        help=(
            "Run a full verify call against the bundled mini_py_service fixture. "
            "Overrides --verify-file/--language/--project-dir/--test-file."
        ),
    )
    args = parser.parse_args()

    if args.verify_sample:
        args.verify_file = str(SAMPLE_FIXTURE["verify_file"])
        args.language = SAMPLE_FIXTURE["language"]
        args.project_dir = str(SAMPLE_FIXTURE["project_dir"])
        args.test_file = str(SAMPLE_FIXTURE["test_file"])

    try:
        command = resolve_command(args)
        with McpSmokeClient(command) as client:
            tools = client.list_tools()
            print("Tools:")
            for tool_name in tools:
                print(f"- {tool_name}")

            if not args.verify_file:
                return 0

            if not args.language:
                raise ValueError("--language is required when --verify-file is set")

            verify_args: dict[str, Any] = {
                "file_path": str(Path(args.verify_file).expanduser().resolve()),
                "language": args.language,
            }
            if args.project_dir:
                verify_args["project_dir"] = str(
                    Path(args.project_dir).expanduser().resolve()
                )
            if args.test_file:
                verify_args["test_file_path"] = str(
                    Path(args.test_file).expanduser().resolve()
                )

            print("\nverify result:")
            result = client.call_tool("verify", verify_args)
            print(json.dumps(result, indent=2))
        return 0
    except Exception as exc:
        print(f"Smoke test failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
