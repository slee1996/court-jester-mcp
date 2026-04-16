from __future__ import annotations

import json
import os
import shutil
import stat
import sys
import textwrap
from collections import Counter
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterator


TRACEABLE_COMMANDS = [
    "bash",
    "sh",
    "zsh",
    "cat",
    "rg",
    "grep",
    "fd",
    "find",
    "ls",
    "sed",
    "head",
    "tail",
    "wc",
    "sort",
    "cut",
    "jq",
    "git",
    "python",
    "python3",
    "node",
    "npm",
    "npx",
    "yarn",
    "pnpm",
    "bun",
    "cargo",
    "pytest",
    "uv",
    "pip",
    "make",
    "cp",
    "mv",
    "rm",
    "mkdir",
    "touch",
    "diff",
    "patch",
]


@dataclass(slots=True)
class AgentTraceSetup:
    trace_dir: str
    events_path: str
    summary_path: str
    shim_dir: str
    env_updates: dict[str, str]
    wrapped_commands: list[str]


def _wrapper_source() -> str:
    python = sys.executable
    return textwrap.dedent(
        f"""\
        #!{python}
        import json
        import os
        import shlex
        import shutil
        import sys
        import time
        from pathlib import Path

        SEARCH_COMMANDS = {{"rg", "grep", "fd", "find"}}
        READ_COMMANDS = {{"cat", "sed", "head", "tail", "ls", "wc", "sort", "cut", "jq"}}
        SHELL_COMMANDS = {{"bash", "sh", "zsh"}}
        VCS_COMMANDS = {{"git"}}
        TEST_OR_BUILD_COMMANDS = {{"cargo", "pytest", "npm", "npx", "yarn", "pnpm", "bun", "make", "uv", "pip"}}
        EDIT_COMMANDS = {{"python", "python3", "node", "cp", "mv", "rm", "mkdir", "touch", "diff", "patch"}}


        def infer_category(command: str) -> str:
            if command in SHELL_COMMANDS:
                return "shell"
            if command in SEARCH_COMMANDS:
                return "search"
            if command in READ_COMMANDS:
                return "read"
            if command in VCS_COMMANDS:
                return "vcs"
            if command in TEST_OR_BUILD_COMMANDS:
                return "test_or_build"
            if command in EDIT_COMMANDS:
                return "edit_or_script"
            return "command"


        def extract_path_hints(args: list[str]) -> list[str]:
            hints: list[str] = []
            for token in args:
                if not token or token.startswith("-"):
                    continue
                if token in {{"." , ".."}}:
                    continue
                if "/" in token or "." in Path(token).name:
                    hints.append(token)
            return hints[:12]


        def shell_command(args: list[str]) -> str | None:
            for index, token in enumerate(args):
                if token in {{"-c", "-lc", "-ic"}} and index + 1 < len(args):
                    return args[index + 1]
            return None


        def append_event(event: dict[str, object]) -> None:
            trace_file = os.environ.get("CJ_AGENT_TRACE_FILE")
            if not trace_file:
                return
            Path(trace_file).parent.mkdir(parents=True, exist_ok=True)
            with open(trace_file, "a", encoding="utf-8") as handle:
                handle.write(json.dumps(event, sort_keys=True) + "\\n")


        def main() -> int:
            invoked_as = os.path.basename(sys.argv[0])
            manifest_path = os.environ.get("CJ_AGENT_TRACE_MANIFEST")
            original_path = os.environ.get("CJ_AGENT_TRACE_ORIG_PATH", os.environ.get("PATH", ""))
            real_commands = {{}}
            if manifest_path and Path(manifest_path).exists():
                with open(manifest_path, encoding="utf-8") as handle:
                    real_commands = json.load(handle)
            real_command = real_commands.get(invoked_as) or shutil.which(invoked_as, path=original_path)
            event = {{
                "timestamp_ms": int(time.time() * 1000),
                "event": "command",
                "command": invoked_as,
                "argv": sys.argv[1:],
                "command_text": " ".join([invoked_as, *[shlex.quote(arg) for arg in sys.argv[1:]]]).strip(),
                "cwd": os.getcwd(),
                "pid": os.getpid(),
                "ppid": os.getppid(),
                "category": infer_category(invoked_as),
                "path_hints": extract_path_hints(sys.argv[1:]),
            }}
            shell_text = shell_command(sys.argv[1:])
            if shell_text:
                event["shell_command"] = shell_text
            append_event(event)
            if not real_command:
                sys.stderr.write(f"court-jester trace wrapper could not resolve real command: {{invoked_as}}\\n")
                return 127
            os.execv(real_command, [real_command, *sys.argv[1:]])
            return 0


        if __name__ == "__main__":
            raise SystemExit(main())
        """
    )


def prepare_agent_trace(trace_dir: Path) -> AgentTraceSetup:
    trace_dir.mkdir(parents=True, exist_ok=True)
    shim_dir = trace_dir / "shim"
    shim_dir.mkdir(parents=True, exist_ok=True)
    wrapper_path = shim_dir / "_cj_trace_exec.py"
    wrapper_path.write_text(_wrapper_source())
    wrapper_path.chmod(wrapper_path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    original_path = os.environ.get("PATH", "")
    command_map: dict[str, str] = {}
    for command in TRACEABLE_COMMANDS:
        resolved = shutil.which(command, path=original_path)
        if not resolved:
            continue
        command_map[command] = resolved
        shim_path = shim_dir / command
        if shim_path.exists() or shim_path.is_symlink():
            shim_path.unlink()
        try:
            shim_path.symlink_to(wrapper_path.name)
        except OSError:
            shim_path.write_text(
                (
                    f"#!{sys.executable}\n"
                    "import os, sys\n"
                    f"os.execv({json.dumps(str(wrapper_path))}, "
                    f"[{json.dumps(command)}, *sys.argv[1:]])\n"
                )
            )
            shim_path.chmod(shim_path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    manifest_path = trace_dir / "commands.json"
    manifest_path.write_text(json.dumps(command_map, indent=2, sort_keys=True) + "\n")
    events_path = trace_dir / "events.jsonl"
    summary_path = trace_dir / "summary.json"
    traced_path = f"{shim_dir}{os.pathsep}{original_path}"
    bash_env_path = trace_dir / "bash_env.sh"
    bash_env_path.write_text(f"export PATH={json.dumps(traced_path)}\n")
    zdotdir = trace_dir / "zdotdir"
    zdotdir.mkdir(parents=True, exist_ok=True)
    (zdotdir / ".zshenv").write_text(f"export PATH={json.dumps(traced_path)}\n")

    env_updates = {
        "CJ_AGENT_TRACE_DIR": str(trace_dir),
        "CJ_AGENT_TRACE_FILE": str(events_path),
        "CJ_AGENT_TRACE_MANIFEST": str(manifest_path),
        "CJ_AGENT_TRACE_ORIG_PATH": original_path,
        "PATH": traced_path,
        "BASH_ENV": str(bash_env_path),
        "ENV": str(bash_env_path),
        "ZDOTDIR": str(zdotdir),
    }
    return AgentTraceSetup(
        trace_dir=str(trace_dir),
        events_path=str(events_path),
        summary_path=str(summary_path),
        shim_dir=str(shim_dir),
        env_updates=env_updates,
        wrapped_commands=sorted(command_map),
    )


def summarize_agent_trace(trace_dir: Path, *, tail_limit: int = 20) -> dict[str, Any]:
    events_path = trace_dir / "events.jsonl"
    summary_path = trace_dir / "summary.json"
    events: list[dict[str, Any]] = []
    if events_path.exists():
        for line in events_path.read_text().splitlines():
            text = line.strip()
            if not text:
                continue
            try:
                parsed = json.loads(text)
            except json.JSONDecodeError:
                continue
            if isinstance(parsed, dict):
                events.append(parsed)

    command_counts = Counter(str(item.get("command", "")) for item in events if item.get("command"))
    category_counts = Counter(str(item.get("category", "")) for item in events if item.get("category"))
    tail = [
        {
            "timestamp_ms": item.get("timestamp_ms"),
            "command": item.get("command"),
            "category": item.get("category"),
            "argv": item.get("argv"),
            "path_hints": item.get("path_hints"),
            "shell_command": item.get("shell_command"),
            "cwd": item.get("cwd"),
        }
        for item in events[-tail_limit:]
    ]
    summary = {
        "trace_dir": str(trace_dir),
        "events_path": str(events_path),
        "event_count": len(events),
        "commands": dict(command_counts),
        "categories": dict(category_counts),
        "tail": tail,
    }
    summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    return summary


@contextmanager
def temporary_environment(updates: dict[str, str]) -> Iterator[None]:
    previous: dict[str, str | None] = {}
    for key, value in updates.items():
        previous[key] = os.environ.get(key)
        os.environ[key] = value
    try:
        yield
    finally:
        for key, value in previous.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value
