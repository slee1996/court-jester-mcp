from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


BENCH_ROOT = Path(__file__).resolve().parent
REPO_ROOT = BENCH_ROOT.parent
ARTIFACT_SCHEMA_VERSION = 1
VERIFY_SCHEMA_VERSION_REQUIRED = 3

class ArtifactVersionError(ValueError):
    """Raised when benchmark artifacts are missing or have mixed schema versions."""

def canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")

def sha256_bytes(value: bytes) -> str:
    import hashlib
    return hashlib.sha256(value).hexdigest()

def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())

def validate_artifact_metadata(payload: dict[str, Any], *, allow_legacy: bool = False) -> None:
    artifact = payload.get("artifact_schema_version")
    verify = payload.get("verify_schema_version_required")
    if artifact != ARTIFACT_SCHEMA_VERSION or verify != VERIFY_SCHEMA_VERSION_REQUIRED:
        if allow_legacy:
            return
        raise ArtifactVersionError(f"artifact schema mismatch: artifact_schema_version={artifact!r}, verify_schema_version_required={verify!r}")

def iter_closure_files(root: Path) -> list[Path]:
    ignored = {".git", ".venv", "__pycache__", "node_modules", ".pytest_cache", ".mypy_cache"}
    if not root.exists():
        return []
    files: list[Path] = []
    for path in root.rglob("*"):
        rel = path.relative_to(root)
        if not path.is_file() or any(part.startswith(".") for part in rel.parts) or any(part in ignored for part in rel.parts):
            continue
        files.append(path)
    return sorted(files, key=lambda p: p.relative_to(root).as_posix())

def suite_lock_projection(task_set: "TaskSetManifest", tasks: list["TaskManifest"]) -> tuple[dict[str, Any], list[dict[str, str]]]:
    closure: list[dict[str, str]] = []
    for task in sorted(tasks, key=lambda t: t.id):
        manifest_path = BENCH_ROOT / "tasks" / f"{task.id}.json"
        if manifest_path.exists():
            closure.append({"path": manifest_path.relative_to(REPO_ROOT).as_posix(), "sha256": sha256_file(manifest_path)})
        fixture = BENCH_ROOT / "repos" / task.repo_fixture
        for path in iter_closure_files(fixture):
            closure.append({"path": path.relative_to(REPO_ROOT).as_posix(), "sha256": sha256_file(path)})
        declared = list(task.verify_paths) + list(task.expected_files)
        if task.verify_test_path:
            declared.append(task.verify_test_path)
        if task.gold_patch_path:
            declared.append(task.gold_patch_path)
        for raw in declared:
            path = (fixture / raw) if not Path(raw).is_absolute() else Path(raw)
            if path.is_file():
                try:
                    rel = path.relative_to(REPO_ROOT).as_posix()
                except ValueError:
                    rel = f"external:{path.as_posix()}"
                entry = {"path": rel, "sha256": sha256_file(path)}
                if entry not in closure:
                    closure.append(entry)
    closure.sort(key=lambda item: item["path"])
    projection = {"task_set": {"id": task_set.id, "title": task_set.title, "task_ids": sorted(task_set.task_ids), "goal": task_set.goal, "suite_kind": task_set.suite_kind, "immutable": task_set.immutable, "gate_role": task_set.gate_role, "lock_version": task_set.lock_version}, "tasks": [task.id for task in sorted(tasks, key=lambda t: t.id)], "closure": closure}
    return projection, closure

def suite_lock_digest(task_set: "TaskSetManifest", tasks: list["TaskManifest"]) -> str:
    projection, _ = suite_lock_projection(task_set, tasks)
    return sha256_bytes(canonical_json(projection))


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
    verify_tests_only: bool = False
    setup_commands: list[list[str]] = field(default_factory=list)
    setup_cache_key: str | None = None
    public_check_commands: list[list[str]] = field(default_factory=list)
    judge_check_commands: list[list[str]] = field(default_factory=list)
    hidden_check_command: list[str] | None = None
    provider_timeout_seconds: int | None = None
    provider_idle_timeout_seconds: int | None = None
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
    public_only_repair: bool = False
    blind_retry_without_verify: bool = False
    repair_feedback_style: str = "detailed"
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
    suite_kind: str | None = None
    immutable: bool = False
    gate_role: str | None = None
    lock_version: int = 1
    locked_suite_sha256: str | None = None

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
