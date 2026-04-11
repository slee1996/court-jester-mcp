from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


BENCH_ROOT = Path(__file__).resolve().parent
REPO_ROOT = BENCH_ROOT.parent


@dataclass(slots=True)
class TaskManifest:
    id: str
    title: str
    repo_fixture: str
    prompt: str
    language: str
    bucket: str
    verify_paths: list[str]
    verify_test_path: str | None = None
    setup_commands: list[list[str]] = field(default_factory=list)
    setup_cache_key: str | None = None
    public_check_commands: list[list[str]] = field(default_factory=list)
    judge_check_commands: list[list[str]] = field(default_factory=list)
    hidden_check_command: list[str] | None = None
    gold_patch_path: str | None = None
    gold_changed_files: list[str] = field(default_factory=list)
    expected_files: list[str] = field(default_factory=list)
    tags: list[str] = field(default_factory=list)
    family: str | None = None
    bug_class: str | None = None
    bug_surface: str | None = None
    difficulty: str | None = None
    seeded_variant_of: str | None = None
    golden_patch_description: str | None = None
    expected_verify_outcome: str | None = None
    expected_verify_failure_kinds: list[str] = field(default_factory=list)
    expected_hidden_failure_without_fix: bool | None = None
    expected_public_failure_without_fix: bool | None = None
    uses_project_dir: bool | None = None
    uses_relative_imports: bool | None = None
    cross_file: bool | None = None
    upstream_benchmark: str | None = None
    upstream_instance_id: str | None = None
    instance_notes: str | None = None


@dataclass(slots=True)
class PolicyManifest:
    id: str
    title: str
    description: str
    court_jester_mode: str
    required_tools: list[str] = field(default_factory=list)
    block_on_failed_verify: bool = False
    max_repair_rounds: int = 0
    verify_only_repair: bool = False
    promote_verify_repros: bool = False
    replay_attempt_history: bool = False
    critic_model_id: str | None = None
    structured_first_party_feedback: bool = False


@dataclass(slots=True)
class ReplayEdit:
    path: str
    content_path: str


@dataclass(slots=True)
class ModelManifest:
    id: str
    title: str
    provider: str
    model: str | None = None
    reasoning_effort: str | None = None
    enabled_by_default: bool = True
    replay_edits: list[ReplayEdit] = field(default_factory=list)
    metadata: dict[str, Any] = field(default_factory=dict)


@dataclass(slots=True)
class TaskSetManifest:
    id: str
    title: str
    task_ids: list[str]
    goal: str | None = None


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def load_task(path: Path) -> TaskManifest:
    data = read_json(path)
    return TaskManifest(**data)


def load_policy(path: Path) -> PolicyManifest:
    data = read_json(path)
    return PolicyManifest(**data)


def load_model(path: Path) -> ModelManifest:
    data = read_json(path)
    edits = [ReplayEdit(**edit) for edit in data.pop("replay_edits", [])]
    metadata = data.pop("metadata", {})
    return ModelManifest(replay_edits=edits, metadata=metadata, **data)


def load_task_set(path: Path) -> TaskSetManifest:
    data = read_json(path)
    return TaskSetManifest(**data)


def load_manifest_dir(path: Path, loader: Any) -> list[Any]:
    manifests = []
    for item in sorted(path.glob("*.json")):
        manifests.append(loader(item))
    return manifests


def slugify(value: str) -> str:
    chars: list[str] = []
    for ch in value.lower():
        if ch.isalnum():
            chars.append(ch)
        else:
            chars.append("-")
    slug = "".join(chars).strip("-")
    while "--" in slug:
        slug = slug.replace("--", "-")
    return slug
