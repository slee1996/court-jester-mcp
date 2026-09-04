"""Shared command and workspace-setup result records."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(slots=True)
class CommandResult:
    argv: list[str]
    exit_code: int
    duration_ms: int
    stdout_path: str
    stderr_path: str


@dataclass(slots=True)
class WorkspaceSetupResult:
    success: bool
    cache_hit: bool
    duration_ms: int
    commands: list[CommandResult]
    cache_dir: str | None = None
    failure_reason: str | None = None
