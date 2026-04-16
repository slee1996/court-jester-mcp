from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path
from typing import Any

from .common import REPO_ROOT


TOOL_NAMES = ("analyze", "lint", "execute", "verify")


class CourtJesterClient:
    def __init__(self, binary_path: Path | None = None) -> None:
        self.binary_path = binary_path or Path(
            os.getenv(
                "CJ_BINARY",
                str(REPO_ROOT / "target" / "release" / "court-jester"),
            )
        )
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

    def close(self) -> None:
        return None

    def restart(self) -> None:
        self.start()

    def list_tools(self) -> dict[str, Any]:
        return {"result": {"tools": [{"name": name} for name in TOOL_NAMES]}}

    def call_tool(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        if name not in TOOL_NAMES:
            raise ValueError(f"unsupported tool '{name}'")
        self.start()

        with tempfile.TemporaryDirectory(prefix="court-jester-cli-") as temp_dir_str:
            temp_dir = Path(temp_dir_str)
            command = self._build_command(name, arguments, temp_dir)
            cwd = self._resolve_cwd(arguments, command)
            try:
                completed = subprocess.run(
                    command,
                    cwd=cwd,
                    capture_output=True,
                    text=True,
                    timeout=120.0,
                    check=False,
                )
            except subprocess.TimeoutExpired as exc:
                self.last_error_context = {
                    "pid": None,
                    "return_code": None,
                    "stderr_tail": self._tail_text(exc.stderr or ""),
                }
                raise TimeoutError(
                    f"court-jester {name} timed out after 120.0s"
                ) from exc

        parsed = self._parse_stdout(completed.stdout)
        self.last_error_context = {
            "pid": None,
            "return_code": completed.returncode,
            "stderr_tail": self._tail_text(completed.stderr),
        }
        if parsed is None:
            stderr_tail = self.last_error_context["stderr_tail"] or "<empty>"
            stdout_tail = self._tail_text(completed.stdout) or "<empty>"
            raise RuntimeError(
                f"court-jester {name} exited rc={completed.returncode} "
                f"stderr={stderr_tail} stdout={stdout_tail}"
            )
        return {
            "result": {
                "parsed": parsed,
                "stdout": completed.stdout,
                "stderr": completed.stderr,
                "exit_code": completed.returncode,
            }
        }

    def _build_command(
        self,
        tool: str,
        arguments: dict[str, Any],
        temp_dir: Path,
    ) -> list[str]:
        language = arguments.get("language")
        if language not in {"python", "typescript"}:
            raise ValueError(f"unsupported or missing language: {language!r}")

        command = [str(self.binary_path), tool]
        source_path, virtual_file_path = self._resolve_source_file(arguments, temp_dir, language)
        command.extend(["--file", str(source_path), "--language", language])

        project_dir = arguments.get("project_dir")
        if project_dir:
            command.extend(["--project-dir", str(project_dir)])
        config_path = arguments.get("config_path")
        if config_path:
            command.extend(["--config-path", str(config_path)])
        if virtual_file_path:
            command.extend(["--virtual-file-path", virtual_file_path])

        if tool in {"analyze", "verify"}:
            complexity_threshold = arguments.get("complexity_threshold")
            if complexity_threshold is not None:
                command.extend(["--complexity-threshold", str(complexity_threshold)])
            diff_path = self._materialize_optional_text(
                temp_dir,
                arguments.get("diff"),
                "changes.diff",
            )
            if diff_path is not None:
                command.extend(["--diff-file", str(diff_path)])

        if tool == "verify":
            test_path = self._resolve_optional_file(
                temp_dir,
                language,
                arguments.get("test_code"),
                arguments.get("test_file_path"),
                default_name="verify_test",
            )
            if test_path is not None:
                command.extend(["--test-file", str(test_path)])
            if arguments.get("tests_only"):
                command.append("--tests-only")
            output_dir = arguments.get("output_dir")
            if output_dir:
                command.extend(["--output-dir", str(output_dir)])

        if tool == "execute":
            timeout_seconds = arguments.get("timeout_seconds")
            if timeout_seconds is not None:
                command.extend(["--timeout-seconds", str(timeout_seconds)])
            memory_mb = arguments.get("memory_mb")
            if memory_mb is not None:
                command.extend(["--memory-mb", str(memory_mb)])

        return command

    def _resolve_source_file(
        self,
        arguments: dict[str, Any],
        temp_dir: Path,
        language: str,
    ) -> tuple[Path, str | None]:
        code = arguments.get("code")
        file_path = arguments.get("file_path")
        explicit_virtual_path = arguments.get("virtual_file_path")
        if code is not None:
            hinted_path = explicit_virtual_path or file_path
            temp_file = self._materialize_source(temp_dir, hinted_path, language, code)
            return temp_file, explicit_virtual_path or file_path
        if file_path:
            return Path(file_path).expanduser().resolve(), explicit_virtual_path
        raise ValueError("tool call requires either code or file_path")

    def _resolve_optional_file(
        self,
        temp_dir: Path,
        language: str,
        inline_text: str | None,
        file_path: str | None,
        *,
        default_name: str,
    ) -> Path | None:
        if inline_text is not None:
            return self._materialize_source(temp_dir, None, language, inline_text, default_name)
        if file_path:
            return Path(file_path).expanduser().resolve()
        return None

    def _materialize_source(
        self,
        temp_dir: Path,
        hinted_path: str | None,
        language: str,
        content: str,
        default_stem: str = "source",
    ) -> Path:
        suffix = ".py" if language == "python" else ".ts"
        if hinted_path:
            hinted = Path(hinted_path)
            relative = Path(*hinted.parts[1:]) if hinted.is_absolute() else hinted
            target = temp_dir / relative
            if target.suffix != suffix:
                target = target.with_suffix(suffix)
        else:
            target = temp_dir / f"{default_stem}{suffix}"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
        return target.resolve()

    def _materialize_optional_text(
        self,
        temp_dir: Path,
        content: str | None,
        filename: str,
    ) -> Path | None:
        if not content:
            return None
        target = temp_dir / filename
        target.write_text(content)
        return target.resolve()

    def _resolve_cwd(self, arguments: dict[str, Any], command: list[str]) -> Path:
        if arguments.get("project_dir"):
            return Path(arguments["project_dir"]).expanduser().resolve()
        file_index = command.index("--file") + 1
        return Path(command[file_index]).resolve().parent

    def _parse_stdout(self, stdout: str) -> Any | None:
        text = stdout.strip()
        if not text:
            return None
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return None

    def _tail_text(self, value: str, limit: int = 1000) -> str:
        value = value.strip()
        if len(value) > limit:
            return value[-limit:]
        return value
