from __future__ import annotations

import json
import os
import select
import subprocess
from pathlib import Path
from typing import Any

from .common import REPO_ROOT


class CourtJesterClient:
    def __init__(self, binary_path: Path | None = None) -> None:
        self.binary_path = binary_path or Path(
            os.getenv(
                "CJ_MCP_BINARY",
                str(REPO_ROOT / "target" / "release" / "court-jester-mcp"),
            )
        )
        self.proc: subprocess.Popen[str] | None = None
        self._next_id = 1
        self.last_error_context: dict[str, Any] = {}

    def __enter__(self) -> "CourtJesterClient":
        self.start()
        return self

    def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
        self.close()

    def start(self) -> None:
        if not self.binary_path.exists():
            raise FileNotFoundError(
                f"court-jester binary not found at {self.binary_path}. Build it before running the benchmark."
            )
        self.proc = subprocess.Popen(
            [str(self.binary_path)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            start_new_session=True,
        )
        self._request(
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "court-jester-bench", "version": "0.1"},
            },
        )
        self._notify("notifications/initialized", {})

    def close(self) -> None:
        if self.proc is None:
            return
        proc = self.proc
        if self.proc.stdin is not None:
            try:
                self.proc.stdin.close()
            except OSError:
                pass
        if proc.poll() is None:
            try:
                os.killpg(proc.pid, subprocess.signal.SIGTERM)
            except (OSError, ProcessLookupError):
                pass
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            if proc.poll() is None:
                try:
                    os.killpg(proc.pid, subprocess.signal.SIGKILL)
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

    def restart(self) -> None:
        self.close()
        self.start()

    def list_tools(self) -> dict[str, Any]:
        return self._request("tools/list", {})

    def call_tool(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        response = self._request(
            "tools/call",
            {"name": name, "arguments": arguments},
            timeout_seconds=120.0,
        )
        result = response["result"]
        content = result.get("content", [])
        if content and content[0].get("type") == "text":
            text = content[0].get("text", "")
            try:
                result["parsed"] = json.loads(text)
            except json.JSONDecodeError:
                result["parsed"] = text
        return response

    def _request(
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
            context = self._capture_process_context()
            self.last_error_context = context
            if context.get("return_code") is not None:
                raise RuntimeError(
                    f"court-jester request {method} lost the process while waiting: "
                    f"rc={context.get('return_code')} stderr={context.get('stderr_tail') or '<empty>'}"
                )
            raise TimeoutError(
                f"court-jester request {method} timed out after {timeout_seconds:.1f}s "
                f"(pid={context.get('pid')})"
            )
        line = self.proc.stdout.readline()
        if not line:
            context = self._capture_process_context()
            self.last_error_context = context
            raise RuntimeError(
                f"court-jester closed the connection during {method}: "
                f"rc={context.get('return_code')} stderr={context.get('stderr_tail') or '<empty>'}"
            )
        response = json.loads(line)
        if "error" in response:
            self.last_error_context = self._capture_process_context()
            raise RuntimeError(f"court-jester {method} error: {response['error']}")
        return response

    def _notify(self, method: str, params: dict[str, Any]) -> None:
        if self.proc is None or self.proc.stdin is None:
            raise RuntimeError("court-jester process is not running")
        message = {"jsonrpc": "2.0", "method": method, "params": params}
        self.proc.stdin.write(json.dumps(message) + "\n")
        self.proc.stdin.flush()

    def _capture_process_context(self) -> dict[str, Any]:
        if self.proc is None:
            return {"pid": None, "return_code": None, "stderr_tail": ""}
        return {
            "pid": self.proc.pid,
            "return_code": self.proc.poll(),
            "stderr_tail": self._read_available_stderr(),
        }

    def _read_available_stderr(self) -> str:
        if self.proc is None or self.proc.stderr is None:
            return ""
        chunks: list[str] = []
        while True:
            readable, _, _ = select.select([self.proc.stderr], [], [], 0)
            if not readable:
                break
            try:
                chunk = os.read(self.proc.stderr.fileno(), 4096)
            except OSError:
                break
            if not chunk:
                break
            chunks.append(chunk.decode("utf-8", errors="replace"))
            if sum(len(item) for item in chunks) >= 8192:
                break
        text = "".join(chunks).strip()
        if len(text) > 1000:
            return text[-1000:]
        return text
