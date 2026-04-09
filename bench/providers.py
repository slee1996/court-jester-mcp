from __future__ import annotations

import json
import os
import re
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path

from .common import BENCH_ROOT, ModelManifest, TaskManifest


@dataclass(slots=True)
class ProviderResult:
    changed_files: list[str] = field(default_factory=list)
    transcript: list[str] = field(default_factory=list)
    unsupported: bool = False
    unsupported_reason: str | None = None
    failed: bool = False
    failure_reason: str | None = None
    exit_code: int | None = None
    parsed_summary: dict[str, object] | None = None


class Provider:
    def __init__(self, model: ModelManifest) -> None:
        self.model = model
        self.supports_repair = False

    def apply(
        self,
        workspace: Path,
        task: TaskManifest,
        *,
        feedback: str | None = None,
        attempt: int = 0,
        history: list[dict[str, object]] | None = None,
    ) -> ProviderResult:
        raise NotImplementedError

    def critique(
        self,
        workspace: Path,
        task: TaskManifest,
        *,
        feedback: str,
        promoted_repros: list[str] | None = None,
        history: list[dict[str, object]] | None = None,
    ) -> str | None:
        return None


def strip_thinking(text: str) -> str:
    cleaned = re.sub(r"<think>[\s\S]*?(</think>\s*|<\|im_end\|>)", "", text)
    cleaned = re.sub(r"<think>[\s\S]*$", "", cleaned)
    cleaned = re.sub(r"<\|im_end\|>", "", cleaned)
    return cleaned.strip()


def parse_json_object(text: str) -> dict[str, object] | None:
    stripped = text.strip()
    candidates = [stripped]
    if "{" in stripped and "}" in stripped:
        candidates.append(stripped[stripped.find("{") : stripped.rfind("}") + 1].strip())
        decoder = json.JSONDecoder()
        for start_index, ch in enumerate(stripped):
            if ch != "{":
                continue
            try:
                parsed, end_index = decoder.raw_decode(stripped[start_index:])
            except json.JSONDecodeError:
                continue
            tail = stripped[start_index + end_index :].strip()
            if tail:
                continue
            if isinstance(parsed, dict):
                return parsed
        last_open = stripped.rfind("{")
        if last_open != -1:
            candidates.append(stripped[last_open:].strip())
    for candidate in candidates:
        if not candidate:
            continue
        try:
            parsed = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        if isinstance(parsed, dict):
            return parsed
    return None


def safe_relative_path(path: str) -> str | None:
    candidate = Path(path)
    if candidate.is_absolute():
        return None
    parts = candidate.parts
    if any(part == ".." for part in parts):
        return None
    return candidate.as_posix()


def stable_subprocess_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    env = dict(os.environ)
    env.update(
        {
            "OTEL_SDK_DISABLED": "true",
            "OTEL_TRACES_EXPORTER": "none",
            "OTEL_METRICS_EXPORTER": "none",
        }
    )
    if extra:
        env.update(extra)
    return env


class NoopProvider(Provider):
    def apply(
        self,
        workspace: Path,
        task: TaskManifest,
        *,
        feedback: str | None = None,
        attempt: int = 0,
        history: list[dict[str, object]] | None = None,
    ) -> ProviderResult:
        return ProviderResult(
            changed_files=[],
            transcript=[f"noop provider left workspace unchanged for task {task.id} on attempt {attempt}."],
        )


class ReplayProvider(Provider):
    def apply(
        self,
        workspace: Path,
        task: TaskManifest,
        *,
        feedback: str | None = None,
        attempt: int = 0,
        history: list[dict[str, object]] | None = None,
    ) -> ProviderResult:
        changed_files: list[str] = []
        transcript: list[str] = []
        for edit in self.model.replay_edits:
            src = BENCH_ROOT.parent / edit.content_path
            if not src.exists():
                raise FileNotFoundError(f"Replay content file not found: {src}")
            dst = workspace / edit.path
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_text(src.read_text())
            changed_files.append(edit.path)
            transcript.append(f"replay wrote {edit.path} from {edit.content_path}")
        return ProviderResult(
            changed_files=changed_files,
            transcript=transcript,
            parsed_summary={
                "status": "completed",
                "summary": "Applied deterministic replay edits.",
                "files_changed": changed_files,
            },
        )


class CliAgentProvider(Provider):
    def __init__(self, model: ModelManifest) -> None:
        super().__init__(model)
        self.supports_repair = True

    def apply(
        self,
        workspace: Path,
        task: TaskManifest,
        *,
        feedback: str | None = None,
        attempt: int = 0,
        history: list[dict[str, object]] | None = None,
    ) -> ProviderResult:
        raise NotImplementedError

    def _build_prompt(
        self,
        task: TaskManifest,
        feedback: str | None,
        attempt: int,
        history: list[dict[str, object]] | None,
    ) -> str:
        allowed_files = ", ".join(task.expected_files) if task.expected_files else "any file in the workspace"
        prompt = [
            "You are participating in a benchmark for agentic code generation.",
            f"Task id: {task.id}",
            f"Task: {task.prompt}",
            f"Allowed files: {allowed_files}",
            "Constraints:",
            "- Work only inside the current workspace.",
            "- Prefer minimal changes.",
            "- Run local checks if useful.",
            "- Return JSON that matches the required schema after you finish editing.",
            "- Do not include markdown fences in the final JSON response.",
        ]
        if history:
            prompt.extend(["", "Attempt history:"])
            for item in history:
                prompt.extend(render_attempt_history_entry(item))
        if feedback:
            prompt.extend(
                [
                    "",
                    f"Repair attempt {attempt}. The previous solution failed benchmark evaluation.",
                    "Use this feedback to repair the workspace:",
                    feedback,
                    "",
                    "Repair requirements:",
                    "- Treat every concrete failing repro as authoritative.",
                    "- Your patch must change behavior on the cited failing repros.",
                    "- Before finishing, check whether your new code would return the expected result on each cited repro.",
                    "- Do not claim the code is already correct if the cited repro would still fail.",
                    "- If you cannot produce a patch that fixes the cited repros, return blocked instead of completed.",
                ]
            )
        return "\n".join(prompt)

    def _parse_structured_output(self, raw: str) -> dict[str, object] | None:
        text = raw.strip()
        if not text:
            return None
        candidates = [text]
        if "{" in text and "}" in text:
            candidates.append(text[text.find("{") : text.rfind("}") + 1])
        for candidate in candidates:
            try:
                parsed = json.loads(candidate)
            except json.JSONDecodeError:
                continue
            if isinstance(parsed, dict):
                if "structured_output" in parsed and isinstance(parsed["structured_output"], dict):
                    return parsed["structured_output"]
                if "result" in parsed and isinstance(parsed["result"], dict):
                    return parsed["result"]
                return parsed
        return None

    def _timeout_seconds(self) -> int:
        value = self.model.metadata.get("timeout_seconds", 180)
        try:
            return int(value)
        except (TypeError, ValueError):
            return 180

    def _agent_env(self) -> dict[str, str]:
        return stable_subprocess_env()

    def _extract_failure_reason(
        self,
        *,
        completed: subprocess.CompletedProcess[str],
        parsed: dict[str, object] | None,
    ) -> str | None:
        stderr_text = completed.stderr.strip()
        if stderr_text:
            return stderr_text
        if isinstance(parsed, dict):
            result = parsed.get("result")
            if isinstance(result, str) and result.strip():
                return result.strip()
            summary = parsed.get("summary")
            if isinstance(summary, str) and summary.strip():
                return summary.strip()
        stdout_text = completed.stdout.strip()
        return stdout_text or None

    def _finalize_cli_result(
        self,
        *,
        completed: subprocess.CompletedProcess[str],
        raw_output: str,
    ) -> ProviderResult:
        parsed = self._parse_structured_output(raw_output)
        failure_reason = None
        failed = completed.returncode != 0
        if failed:
            failure_reason = self._extract_failure_reason(completed=completed, parsed=parsed)
        return ProviderResult(
            transcript=[completed.stdout, completed.stderr],
            exit_code=completed.returncode,
            failed=failed,
            failure_reason=failure_reason,
            parsed_summary=parsed,
            changed_files=list(parsed.get("files_changed", [])) if isinstance(parsed, dict) else [],
        )


class CodexCliProvider(CliAgentProvider):
    def apply(
        self,
        workspace: Path,
        task: TaskManifest,
        *,
        feedback: str | None = None,
        attempt: int = 0,
        history: list[dict[str, object]] | None = None,
    ) -> ProviderResult:
        workspace = workspace.resolve()
        prompt = self._build_prompt(task, feedback, attempt, history)
        schema_path = BENCH_ROOT / "schemas" / "agent_summary.json"
        command = [
            "codex",
            "exec",
            "--skip-git-repo-check",
            "--ephemeral",
            "--full-auto",
            "--color",
            "never",
            "--cd",
            str(workspace),
            "--output-schema",
            str(schema_path),
        ]
        if self.model.model:
            command.extend(["--model", self.model.model])
        try:
            completed = subprocess.run(
                command + [prompt],
                cwd=workspace,
                capture_output=True,
                text=True,
                timeout=self._timeout_seconds(),
                env=self._agent_env(),
            )
        except subprocess.TimeoutExpired as exc:
            return ProviderResult(
                transcript=[exc.stdout or "", exc.stderr or ""],
                failed=True,
                failure_reason=f"codex_cli timed out after {self._timeout_seconds()} seconds",
            )
        raw = completed.stdout
        return self._finalize_cli_result(completed=completed, raw_output=raw)


class ClaudeCliProvider(CliAgentProvider):
    def apply(
        self,
        workspace: Path,
        task: TaskManifest,
        *,
        feedback: str | None = None,
        attempt: int = 0,
        history: list[dict[str, object]] | None = None,
    ) -> ProviderResult:
        workspace = workspace.resolve()
        schema_obj = json.loads((BENCH_ROOT / "schemas" / "agent_summary.json").read_text())
        schema = json.dumps(schema_obj, separators=(",", ":"))
        prompt = self._build_prompt(task, feedback, attempt, history)
        command = [
            "claude",
            "-p",
            "--output-format",
            "json",
            "--permission-mode",
            "bypassPermissions",
            "--dangerously-skip-permissions",
            "--tools=default",
            f"--add-dir={workspace}",
            "--json-schema",
            schema,
        ]
        if self.model.model:
            command.extend(["--model", self.model.model])
        try:
            completed = subprocess.run(
                command + [prompt],
                cwd=workspace,
                capture_output=True,
                text=True,
                timeout=self._timeout_seconds(),
                env=self._agent_env(),
            )
        except subprocess.TimeoutExpired as exc:
            return ProviderResult(
                transcript=[exc.stdout or "", exc.stderr or ""],
                failed=True,
                failure_reason=f"claude_cli timed out after {self._timeout_seconds()} seconds",
            )
        return self._finalize_cli_result(completed=completed, raw_output=completed.stdout)


class OpenAICompatProvider(Provider):
    def __init__(self, model: ModelManifest) -> None:
        super().__init__(model)
        self.supports_repair = True

    def apply(
        self,
        workspace: Path,
        task: TaskManifest,
        *,
        feedback: str | None = None,
        attempt: int = 0,
        history: list[dict[str, object]] | None = None,
    ) -> ProviderResult:
        api_key_env = str(self.model.metadata.get("api_key_env", "ACTUAL_API_KEY"))
        api_key = os.getenv(api_key_env)
        if not api_key:
            return ProviderResult(
                failed=True,
                failure_reason=f"Missing API key env var: {api_key_env}",
            )

        base_url = str(
            os.getenv("ACTUAL_API_BASE_URL")
            or self.model.metadata.get("base_url")
            or "https://api.actual.inc/v1"
        ).rstrip("/")
        model_name = str(os.getenv("ACTUAL_API_MODEL") or self.model.model or "")
        if not model_name:
            return ProviderResult(
                failed=True,
                failure_reason="No model configured for openai-compatible provider",
            )

        allowed_files = task.expected_files or task.verify_paths
        prompt = self._build_prompt(workspace, task, allowed_files, feedback, attempt, history)
        system_prompt = (
            "You are a code-editing benchmark model. "
            "Return strict JSON only, with no markdown fences or prose outside JSON. "
            "Use this schema: "
            '{"status":"completed|blocked","summary":"string","files":[{"path":"relative/path","content":"full file content"}]}. '
            "Only include files you want to replace fully. Do not invent paths outside the allowed files."
        )
        body = {
            "model": model_name,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": prompt},
            ],
            "max_tokens": int(self.model.metadata.get("max_tokens", 4000)),
            "temperature": float(self.model.metadata.get("temperature", 0.2)),
            "stream": False,
        }

        parsed = None
        raw_text = ""
        for request_attempt in range(2):
            request_body = body
            if request_attempt == 1:
                request_body = dict(body)
                request_body["temperature"] = 0.0
                request_body["messages"] = list(body["messages"]) + [
                    {
                        "role": "user",
                        "content": (
                            "Your previous response was rejected because it was not valid JSON. "
                            "Return valid JSON only, exactly matching the schema, with no think tags, "
                            "no prose, and no markdown."
                        ),
                    }
                ]
            last_error: Exception | None = None
            max_transport_attempts = 3
            for transport_attempt in range(max_transport_attempts):
                try:
                    raw_text = self._request_chat_completion(base_url, api_key, request_body)
                    last_error = None
                    break
                except Exception as exc:
                    last_error = exc
                    if transport_attempt < max_transport_attempts - 1 and self._is_retryable_request_error(exc):
                        delay_seconds = self._retry_delay_seconds(exc, transport_attempt)
                        if delay_seconds > 0:
                            time.sleep(delay_seconds)
                        continue
                    return ProviderResult(
                        failed=True,
                        failure_reason=str(exc),
                    )
            if last_error is not None:
                return ProviderResult(
                    failed=True,
                    failure_reason=str(last_error),
                )
            parsed = parse_json_object(strip_thinking(raw_text))
            if parsed:
                break
        if not parsed:
            return ProviderResult(
                failed=True,
                failure_reason="Model response was not valid JSON",
                transcript=[raw_text],
            )

        status = str(parsed.get("status", "completed"))
        summary = str(parsed.get("summary", "")).strip()
        files_value = parsed.get("files", [])
        if not isinstance(files_value, list):
            return ProviderResult(
                failed=True,
                failure_reason="Model response field 'files' must be a list",
                transcript=[raw_text],
            )

        changed_files: list[str] = []
        allowed_set = set(allowed_files)
        for item in files_value:
            if not isinstance(item, dict):
                return ProviderResult(
                    failed=True,
                    failure_reason="Model response file entries must be objects",
                    transcript=[raw_text],
                )
            raw_path = item.get("path")
            content = item.get("content")
            if not isinstance(raw_path, str) or not isinstance(content, str):
                return ProviderResult(
                    failed=True,
                    failure_reason="Model response file entries require string path and content",
                    transcript=[raw_text],
                )
            safe_path = safe_relative_path(raw_path)
            if not safe_path:
                return ProviderResult(
                    failed=True,
                    failure_reason=f"Unsafe file path from model: {raw_path}",
                    transcript=[raw_text],
                )
            if allowed_set and safe_path not in allowed_set:
                return ProviderResult(
                    failed=True,
                    failure_reason=f"Model edited disallowed file: {safe_path}",
                    transcript=[raw_text],
                )
            dst = workspace / safe_path
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_text(content)
            changed_files.append(safe_path)

        parsed_summary = {
            "status": status,
            "summary": summary or ("Blocked" if status == "blocked" else "Completed API edit response."),
            "files_changed": changed_files,
        }
        failed = status == "blocked"
        failure_reason = summary if failed else None
        return ProviderResult(
            changed_files=changed_files,
            transcript=[raw_text],
            failed=failed,
            failure_reason=failure_reason,
            parsed_summary=parsed_summary,
        )

    def critique(
        self,
        workspace: Path,
        task: TaskManifest,
        *,
        feedback: str,
        promoted_repros: list[str] | None = None,
        history: list[dict[str, object]] | None = None,
    ) -> str | None:
        api_key_env = str(self.model.metadata.get("api_key_env", "ACTUAL_API_KEY"))
        api_key = os.getenv(api_key_env)
        if not api_key:
            return None

        base_url = str(
            os.getenv("ACTUAL_API_BASE_URL")
            or self.model.metadata.get("base_url")
            or "https://api.actual.inc/v1"
        ).rstrip("/")
        model_name = str(os.getenv("ACTUAL_API_MODEL") or self.model.model or "")
        if not model_name:
            return None

        prompt = self._build_critic_prompt(
            workspace=workspace,
            task=task,
            feedback=feedback,
            promoted_repros=promoted_repros or [],
            history=history or [],
        )
        body = {
            "model": model_name,
            "messages": [
                {
                    "role": "system",
                    "content": (
                        "You are a code repair critic. "
                        "Return strict JSON only using this schema: "
                        '{"status":"completed","summary":"string","advice":["string"]}. '
                        "Do not propose markdown, file contents, or code blocks."
                    ),
                },
                {"role": "user", "content": prompt},
            ],
            "max_tokens": 1200,
            "temperature": 0.0,
            "stream": False,
        }

        raw_text = self._request_chat_completion(base_url, api_key, body)
        parsed = parse_json_object(strip_thinking(raw_text))
        if not parsed:
            return None
        advice = parsed.get("advice")
        summary = str(parsed.get("summary") or "").strip()
        lines: list[str] = []
        if summary:
            lines.append(f"Critic summary: {summary}")
        if isinstance(advice, list):
            for item in advice[:5]:
                if isinstance(item, str) and item.strip():
                    lines.append(f"- {item.strip()}")
        return "\n".join(lines).strip() or None

    def _timeout_seconds(self) -> int:
        value = self.model.metadata.get("timeout_seconds", 180)
        try:
            return int(value)
        except (TypeError, ValueError):
            return 180

    def _is_retryable_request_error(self, exc: Exception) -> bool:
        message = str(exc).lower()
        if "curl: (28)" in message or "timed out" in message:
            return True
        if "curl: (6)" in message or "could not resolve host" in message:
            return True
        for code in ("408", "409", "425", "429", "500", "502", "503", "504"):
            if f"http {code}" in message:
                return True
        return False

    def _retry_delay_seconds(self, exc: Exception, attempt_index: int) -> float:
        message = str(exc).lower()
        if "http 503" in message and ("retry shortly" in message or "currently busy" in message):
            return [2.0, 5.0][min(attempt_index, 1)]
        if "http 429" in message:
            return [1.0, 3.0][min(attempt_index, 1)]
        return [0.5, 1.5][min(attempt_index, 1)]

    def _build_prompt(
        self,
        workspace: Path,
        task: TaskManifest,
        allowed_files: list[str],
        feedback: str | None,
        attempt: int,
        history: list[dict[str, object]] | None,
    ) -> str:
        lines = [
            "You are participating in a benchmark for code generation.",
            f"Task id: {task.id}",
            f"Task: {task.prompt}",
            f"Attempt: {attempt}",
            "Allowed files:",
        ]
        for path in allowed_files:
            lines.append(f"- {path}")
        if history:
            lines.extend(["", "Attempt history:"])
            for item in history:
                lines.extend(render_attempt_history_entry(item))
        if feedback:
            lines.extend(
                [
                    "",
                    "Repair feedback from the previous attempt:",
                    feedback,
                    "",
                    "Repair requirements:",
                    "- Treat every concrete failing repro as authoritative.",
                    "- Your patch must change behavior on the cited failing repros.",
                    "- Before finishing, check whether your new code would return the expected result on each cited repro.",
                    "- Do not claim the code is already correct if the cited repro would still fail.",
                    "- If you cannot produce a patch that fixes the cited repros, return blocked instead of completed.",
                ]
            )
        lines.extend(
            [
                "",
                "Current file contents:",
            ]
        )
        for path in allowed_files:
            file_path = workspace / path
            if file_path.exists():
                content = file_path.read_text()
                lines.extend(
                    [
                        f"FILE: {path}",
                        "```",
                        content,
                        "```",
                    ]
                )
            else:
                lines.extend(
                    [
                        f"FILE: {path}",
                        "```",
                        "<missing>",
                        "```",
                    ]
                )
        lines.extend(
            [
                "",
                "Return JSON only.",
                "If you can solve the task, set status to completed and include full replacement contents for any changed files.",
                "If you cannot solve it, set status to blocked and explain why in summary.",
            ]
        )
        return "\n".join(lines)

    def _build_critic_prompt(
        self,
        *,
        workspace: Path,
        task: TaskManifest,
        feedback: str,
        promoted_repros: list[str],
        history: list[dict[str, object]],
    ) -> str:
        allowed_files = task.expected_files or task.verify_paths
        lines = [
            "You are helping another model repair a benchmark task.",
            f"Task id: {task.id}",
            f"Task: {task.prompt}",
            "Provide brief, concrete repair advice only.",
            "Focus on the minimum behavioral change required to satisfy the failing repros.",
        ]
        if history:
            lines.extend(["", "Attempt history:"])
            for item in history:
                lines.extend(render_attempt_history_entry(item))
        lines.extend(["", "Evaluation feedback:", feedback])
        if promoted_repros:
            lines.append("")
            lines.append("Promoted repros:")
            for repro in promoted_repros:
                lines.append(f"- {repro}")
        lines.extend(["", "Current file contents:"])
        for path in allowed_files:
            file_path = workspace / path
            if file_path.exists():
                lines.extend(
                    [
                        f"FILE: {path}",
                        "```",
                        file_path.read_text(),
                        "```",
                    ]
                )
        lines.extend(
            [
                "",
                "Return JSON only with a short summary and a small list of actionable advice items.",
                "Do not return code. Do not restate the full task. Do not claim success.",
            ]
        )
        return "\n".join(lines)

    def _request_chat_completion(self, base_url: str, api_key: str, body: dict[str, object]) -> str:
        timeout_seconds = self._timeout_seconds()
        payload_json = json.dumps(body)
        completed = subprocess.run(
            [
                "curl",
                "-sS",
                "--retry",
                "2",
                "--retry-delay",
                "1",
                "--retry-all-errors",
                "--max-time",
                str(timeout_seconds),
                "-X",
                "POST",
                f"{base_url}/chat/completions",
                "-H",
                f"Authorization: Bearer {api_key}",
                "-H",
                "Content-Type: application/json",
                "-d",
                payload_json,
                "-w",
                "\n__HTTP_STATUS__:%{http_code}\n",
            ],
            capture_output=True,
            text=True,
            env=stable_subprocess_env(),
        )
        if completed.returncode != 0:
            raise RuntimeError(
                f"openai_compat curl failed: {completed.stderr.strip() or completed.stdout.strip()}"
            )

        marker = "\n__HTTP_STATUS__:"
        if marker not in completed.stdout:
            raise RuntimeError(f"Unexpected curl response: {completed.stdout[:500]}")
        raw_body, status_part = completed.stdout.rsplit(marker, 1)
        status_text = status_part.strip().splitlines()[0]
        try:
            status_code = int(status_text)
        except ValueError as exc:
            raise RuntimeError(f"Unexpected HTTP status from curl: {status_text}") from exc
        if status_code >= 400:
            raise RuntimeError(f"openai_compat HTTP {status_code}: {raw_body.strip()}")
        try:
            payload = json.loads(raw_body)
        except json.JSONDecodeError as exc:
            raise RuntimeError(f"Unexpected chat completion response: {raw_body[:500]}") from exc

        try:
            return str(payload["choices"][0]["message"]["content"])
        except Exception as exc:
            raise RuntimeError(f"Unexpected chat completion response: {payload}") from exc


class UnsupportedProvider(Provider):
    def apply(
        self,
        workspace: Path,
        task: TaskManifest,
        *,
        feedback: str | None = None,
        attempt: int = 0,
        history: list[dict[str, object]] | None = None,
    ) -> ProviderResult:
        return ProviderResult(
            unsupported=True,
            unsupported_reason=(
                f"Provider '{self.model.provider}' is declared for model '{self.model.id}' "
                "but no adapter is implemented yet."
            ),
        )


def render_attempt_history_entry(item: dict[str, object]) -> list[str]:
    attempt = item.get("attempt")
    changed_files = item.get("changed_files") or []
    summary = str(item.get("summary") or "").strip()
    feedback = str(item.get("feedback") or "").strip()
    promoted_repros = item.get("promoted_repros") if isinstance(item.get("promoted_repros"), list) else []
    lines = [f"- Previous attempt {attempt}"]
    if summary:
        lines.append(f"  Model summary: {summary}")
    if changed_files:
        lines.append(f"  Changed files: {', '.join(str(v) for v in changed_files)}")
    if promoted_repros:
        lines.append("  Promoted failing repros:")
        for repro in promoted_repros:
            lines.append(f"    - {repro}")
    if feedback:
        lines.append("  Evaluation feedback:")
        for feedback_line in feedback.splitlines():
            lines.append(f"    {feedback_line}")
    return lines


def provider_from_manifest(model: ModelManifest) -> Provider:
    if model.provider == "noop":
        return NoopProvider(model)
    if model.provider == "replay":
        return ReplayProvider(model)
    if model.provider == "codex_cli":
        return CodexCliProvider(model)
    if model.provider == "claude_cli":
        return ClaudeCliProvider(model)
    if model.provider == "openai_compat_chat":
        return OpenAICompatProvider(model)
    return UnsupportedProvider(model)
