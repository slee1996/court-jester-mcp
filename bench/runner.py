from __future__ import annotations

import difflib
import hashlib
import json
import os
import re
import shutil
import subprocess
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

from .cli_client import CourtJesterClient
from .common import BENCH_ROOT, REPO_ROOT, ModelManifest, PolicyManifest, TaskManifest, load_model, slugify
from .providers import ProviderResult, provider_from_manifest


PROVIDER_RETRYABLE_KINDS = {"capacity_busy", "internal_server_error", "transport_error"}


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


def select_repair_trigger_source(
    *,
    policy: PolicyManifest,
    verify_failed: bool,
    public_ok: bool,
    hidden_checks_ran: bool,
    hidden_ok: bool,
) -> str | None:
    if verify_failed:
        return "verify"
    if policy.verify_only_repair:
        return None
    if not public_ok:
        return "public"
    if hidden_checks_ran and not hidden_ok:
        return "hidden"
    return None


def run_single(
    task: TaskManifest,
    model: ModelManifest,
    policy: PolicyManifest,
    output_root: Path,
    dry_run: bool = False,
    repeat_index: int = 0,
    repeat_count: int = 1,
    hidden_seed: str | None = None,
    use_task_gold_patches: bool = False,
) -> dict[str, Any]:
    started_at_ms = int(time.time() * 1000)
    run_id = (
        f"{slugify(task.id)}__{slugify(model.id)}__{slugify(policy.id)}"
        f"__r{repeat_index + 1:02d}of{repeat_count:02d}__{int(time.time() * 1000)}"
    )
    run_dir = output_root / run_id
    workspace = run_dir / "workspace"
    run_dir.mkdir(parents=True, exist_ok=True)
    fixture = BENCH_ROOT / "repos" / task.repo_fixture
    if not fixture.exists():
        raise FileNotFoundError(f"Task fixture not found: {fixture}")
    shutil.copytree(fixture, workspace)

    result: dict[str, Any] = {
        "run_id": run_id,
        "task_id": task.id,
        "model_id": model.id,
        "policy_id": policy.id,
        "bucket": task.bucket,
        "task_metadata": {
            "family": task.family,
            "bug_class": task.bug_class,
            "bug_surface": task.bug_surface,
            "difficulty": task.difficulty,
            "seeded_variant_of": task.seeded_variant_of,
            "golden_patch_description": task.golden_patch_description,
            "expected_verify_outcome": task.expected_verify_outcome,
            "expected_verify_failure_kinds": task.expected_verify_failure_kinds,
            "expected_hidden_failure_without_fix": task.expected_hidden_failure_without_fix,
            "expected_public_failure_without_fix": task.expected_public_failure_without_fix,
            "uses_project_dir": task.uses_project_dir,
            "uses_relative_imports": task.uses_relative_imports,
            "cross_file": task.cross_file,
            "tags": task.tags,
            "upstream_benchmark": task.upstream_benchmark,
            "upstream_instance_id": task.upstream_instance_id,
            "instance_notes": task.instance_notes,
        },
        "repeat_index": repeat_index,
        "repeat_ordinal": repeat_index + 1,
        "repeat_count": repeat_count,
        "dry_run": dry_run,
        "task_gold_patch_mode": use_task_gold_patches,
        "timestamps": {"started_at_epoch_ms": int(time.time() * 1000)},
        "provider": {
            "id": model.provider,
            "model": model.model,
            "reasoning_effort": model.reasoning_effort,
        },
        "tool_usage": {
            "analyze_calls": 0,
            "lint_calls": 0,
            "execute_calls": 0,
            "verify_calls": 0,
        },
        "timings": {
            "setup_ms": 0,
            "provider_apply_ms": 0,
            "court_jester_total_ms": 0,
            "public_checks_ms": 0,
            "hidden_checks_ms": 0,
            "end_to_end_ms": 0,
        },
    }

    if dry_run:
        result["status"] = "dry_run"
        write_json(run_dir / "result.json", result)
        return result

    hidden_seed = hidden_seed or hashlib.sha256(
        f"{task.id}::{model.id}::{policy.id}::{repeat_index}".encode("utf-8")
    ).hexdigest()
    (run_dir / "hidden_seed.txt").write_text(hidden_seed + "\n")

    setup_result = prepare_workspace_for_run(task, workspace, run_dir)
    result["setup"] = serialize_setup_result(setup_result)
    result["timings"]["setup_ms"] = setup_result.duration_ms
    if not setup_result.success:
        result["status"] = "setup_error"
        result["success"] = False
        result["failure_category"] = "setup_error"
        result["failure_details"] = {
            "failure_reason": setup_result.failure_reason,
            "cache_hit": setup_result.cache_hit,
            "cache_dir": setup_result.cache_dir,
        }
        finalize_result(result)
        write_json(run_dir / "result.json", result)
        return result

    before = snapshot_tree(workspace)
    provider = provider_from_manifest(model)
    attempts: list[dict[str, Any]] = []
    provider_result = None
    court_jester_results: list[dict[str, Any]] = []
    final_public_results: list[CommandResult] = []
    final_public_ok = True
    final_hidden_results: list[CommandResult] = []
    final_hidden_ok = True
    final_hidden_checks_ran = False
    final_hidden_checks_sampled = False
    hidden_checks_requested = bool(task.hidden_check_command)
    feedback: str | None = None
    promoted_verify_test_path: Path | None = None
    attempt_history: list[dict[str, object]] = []
    critic_feedbacks: list[str] = []
    max_attempts = 1 if use_task_gold_patches else 1 + max(policy.max_repair_rounds, 0)
    for attempt in range(max_attempts):
        provider_retry_records: list[dict[str, Any]] = []
        provider_apply_ms = 0
        provider_result = None
        provider_call_count = 0
        gold_patch_command: CommandResult | None = None
        if use_task_gold_patches:
            provider_started = time.time()
            provider_result, gold_patch_command = apply_task_gold_patch(task, workspace, run_dir, attempt)
            provider_apply_ms = int((time.time() - provider_started) * 1000)
            provider_call_count = 1
        else:
            max_provider_retries = provider_retry_limit() if supports_provider_retry(model) else 0
            backup_dir = run_dir / f".provider_backup_attempt_{attempt}"
            while True:
                if max_provider_retries > 0 and provider_call_count <= max_provider_retries:
                    snapshot_workspace_for_retry(workspace, backup_dir)
                provider_started = time.time()
                provider_candidate = provider.apply(
                    workspace,
                    task,
                    feedback=feedback,
                    attempt=attempt,
                    history=attempt_history if policy.replay_attempt_history else None,
                )
                provider_apply_ms += int((time.time() - provider_started) * 1000)
                provider_call_count += 1
                provider_error_kind = (
                    classify_provider_failure(provider_candidate) if provider_candidate.failed else None
                )
                if (
                    provider_candidate.failed
                    and provider_error_kind is not None
                    and should_retry_provider_failure(provider_error_kind)
                    and len(provider_retry_records) < max_provider_retries
                ):
                    restore_workspace_from_retry_snapshot(backup_dir, workspace)
                    delay_seconds = provider_retry_delay_seconds(provider_error_kind, len(provider_retry_records))
                    provider_retry_records.append(
                        {
                            "retry_index": len(provider_retry_records),
                            "provider_error_kind": provider_error_kind,
                            "failure_reason": provider_candidate.failure_reason,
                            "delay_seconds": delay_seconds,
                        }
                    )
                    if delay_seconds > 0:
                        time.sleep(delay_seconds)
                    continue
                provider_result = provider_candidate
                break
        result["timings"]["provider_apply_ms"] += provider_apply_ms
        attempt_record: dict[str, Any] = {
            "attempt": attempt,
            "provider_apply_ms": provider_apply_ms,
            "provider_call_count": provider_call_count,
            "provider_retries": provider_retry_records,
            "provider_result": {
                "changed_files": provider_result.changed_files,
                "transcript": provider_result.transcript,
                "unsupported": provider_result.unsupported,
                "unsupported_reason": provider_result.unsupported_reason,
                "failed": provider_result.failed,
                "failure_reason": provider_result.failure_reason,
                "exit_code": provider_result.exit_code,
                "parsed_summary": provider_result.parsed_summary,
            },
        }
        if gold_patch_command is not None:
            attempt_record["gold_patch_apply"] = asdict(gold_patch_command)
        attempts.append(attempt_record)
        if provider_result.unsupported:
            break
        if provider_result.failed:
            break

        attempt_cj_results: list[dict[str, Any]] = []
        verify_failed = False
        attempt_cj_total_ms = 0
        if policy.court_jester_mode != "none":
            verify_output_dir = run_dir / "court_jester"
            verify_output_dir.mkdir(parents=True, exist_ok=True)
            with CourtJesterClient() as client:
                for verify_index, rel_path in enumerate(task.verify_paths):
                    arguments: dict[str, Any] = {
                        "file_path": str(workspace / rel_path),
                        "language": task.language,
                        "output_dir": str(verify_output_dir),
                    }
                    # Public-file tests generally import the task's main API module, not every
                    # helper in the dependency closure. Attaching the same test file to helper
                    # paths creates false positives like NameError on symbols the helper does not
                    # define. Keep file-based tests on the first/public verify target only.
                    if verify_index == 0:
                        if promoted_verify_test_path is not None:
                            arguments["test_file_path"] = str(promoted_verify_test_path)
                        elif task.verify_test_path:
                            arguments["test_file_path"] = str(workspace / task.verify_test_path)
                    try:
                        tool_started = time.time()
                        response = client.call_tool("verify", arguments)
                        tool_duration_ms = int((time.time() - tool_started) * 1000)
                    except TimeoutError as exc:
                        attempt_cj_results.append(
                            {
                                "path": rel_path,
                                "tool_name": "verify",
                                "duration_ms": 120000,
                                "response": {
                                    "overall_ok": False,
                                    "stages": [
                                        {
                                            "name": "verify_tool_call",
                                            "ok": False,
                                            "error": str(exc),
                                        }
                                    ],
                                },
                            }
                        )
                        result["tool_usage"]["verify_calls"] += 1
                        attempt_cj_total_ms += 120000
                        verify_failed = True
                        break
                    parsed = response["result"].get("parsed")
                    item = {
                        "path": rel_path,
                        "tool_name": "verify",
                        "duration_ms": tool_duration_ms,
                        "response": parsed,
                    }
                    attempt_cj_results.append(item)
                    result["tool_usage"]["verify_calls"] += 1
                    attempt_cj_total_ms += tool_duration_ms
                    if isinstance(parsed, dict) and not parsed.get("overall_ok", False):
                        verify_failed = True
        attempt_record["court_jester"] = {
            "results": attempt_cj_results,
            "verify_failed": verify_failed,
            "total_ms": attempt_cj_total_ms,
        }
        result["timings"]["court_jester_total_ms"] += attempt_cj_total_ms
        court_jester_results = attempt_cj_results

        public_started = time.time()
        final_public_results = run_commands(
            task.public_check_commands,
            workspace,
            run_dir,
            f"public_attempt_{attempt}",
        )
        public_checks_ms = int((time.time() - public_started) * 1000)
        result["timings"]["public_checks_ms"] += public_checks_ms
        final_public_ok = all(item.exit_code == 0 for item in final_public_results) if final_public_results else True
        attempt_record["public_checks"] = [asdict(item) for item in final_public_results]
        attempt_record["public_failed"] = not final_public_ok
        attempt_record["public_checks_ms"] = public_checks_ms
        attempt_hidden_results: list[CommandResult] = []
        attempt_hidden_ok = True
        attempt_hidden_checks_ran = False
        attempt_hidden_checks_sampled = False
        if hidden_checks_requested and policy.max_repair_rounds > 0:
            attempt_hidden_checks_ran = final_public_ok or should_sample_hidden_on_public_failure(hidden_seed)
            attempt_hidden_checks_sampled = (not final_public_ok) and attempt_hidden_checks_ran
            attempt_hidden_results = (
                run_commands(
                    [task.hidden_check_command],
                    workspace,
                    run_dir,
                    f"hidden_attempt_{attempt}",
                    extra_env={"CJ_HIDDEN_SEED": hidden_seed},
                )
                if attempt_hidden_checks_ran
                else []
            )
            hidden_checks_ms = sum(item.duration_ms for item in attempt_hidden_results)
            result["timings"]["hidden_checks_ms"] += hidden_checks_ms
            attempt_hidden_ok = (
                all(item.exit_code == 0 for item in attempt_hidden_results)
                if attempt_hidden_results
                else True
            )
            final_hidden_results = attempt_hidden_results
            final_hidden_ok = attempt_hidden_ok
            final_hidden_checks_ran = attempt_hidden_checks_ran
            final_hidden_checks_sampled = attempt_hidden_checks_sampled
        attempt_record["hidden_checks"] = [asdict(item) for item in attempt_hidden_results]
        attempt_record["hidden_failed"] = attempt_hidden_checks_ran and not attempt_hidden_ok
        attempt_record["hidden_checks_ran"] = attempt_hidden_checks_ran
        attempt_record["hidden_checks_sampled_on_public_failure"] = attempt_hidden_checks_sampled

        repair_trigger_source = select_repair_trigger_source(
            policy=policy,
            verify_failed=verify_failed,
            public_ok=final_public_ok,
            hidden_checks_ran=attempt_hidden_checks_ran,
            hidden_ok=attempt_hidden_ok,
        )
        promoted_repros: list[str] = []
        if repair_trigger_source == "verify":
            promoted_repros = collect_promoted_verify_repros(task.language, attempt_cj_results)
            feedback = format_verify_feedback(
                attempt_cj_results,
                promoted_repros=promoted_repros,
                task=task,
                include_first_party_checklist=policy.structured_first_party_feedback,
            )
            critic_feedback = build_critic_feedback(
                policy=policy,
                workspace=workspace,
                task=task,
                feedback=feedback,
                promoted_repros=promoted_repros,
                history=attempt_history,
            )
            if critic_feedback:
                feedback = "\n\n".join([feedback, "External critic advice:", critic_feedback])
                critic_feedbacks.append(critic_feedback)
            if policy.promote_verify_repros and task.verify_test_path:
                promoted_verify_test_path = write_promoted_verify_test(
                    workspace=workspace,
                    task=task,
                    attempt=attempt,
                    promoted_repros=promoted_repros,
                )
            else:
                promoted_verify_test_path = None
        elif repair_trigger_source == "public":
            feedback = format_public_failure_feedback(final_public_results)
            promoted_verify_test_path = None
        elif repair_trigger_source == "hidden":
            feedback = format_hidden_failure_feedback(attempt_hidden_results)
            promoted_verify_test_path = None
        else:
            feedback = None
            promoted_verify_test_path = None
        attempt_record["repair_trigger_source"] = repair_trigger_source
        attempt_record["promoted_verify_test_path"] = (
            str(promoted_verify_test_path) if promoted_verify_test_path is not None else None
        )
        attempt_record["promoted_repros"] = promoted_repros
        attempt_record["critic_feedback"] = critic_feedbacks[-1] if critic_feedbacks else None

        if repair_trigger_source is None:
            break
        if policy.replay_attempt_history:
            attempt_history.append(
                {
                    "attempt": attempt,
                    "summary": extract_provider_summary(provider_result),
                    "changed_files": provider_result.changed_files,
                    "feedback": feedback,
                    "promoted_repros": promoted_repros,
                }
            )
        if attempt >= policy.max_repair_rounds or not provider.supports_repair:
            break

    assert provider_result is not None
    result["attempts"] = attempts
    result["provider_result"] = attempts[-1]["provider_result"]
    result["critic_feedbacks"] = critic_feedbacks
    prior_repair_trigger_sources = [
        attempt["repair_trigger_source"]
        for attempt in attempts
        if attempt.get("repair_trigger_source")
    ]
    if not prior_repair_trigger_sources:
        prior_repair_trigger_source = None
    elif len(set(prior_repair_trigger_sources)) == 1:
        prior_repair_trigger_source = prior_repair_trigger_sources[0]
    else:
        prior_repair_trigger_source = "multiple"
    if provider_result.unsupported:
        result["status"] = "unsupported_provider"
        result["success"] = False
        result["failure_category"] = "provider_unsupported"
        result["failure_details"] = {"provider_error_kind": "unsupported"}
        result["attempt_count"] = len(attempts)
        result["repair_attempted"] = len(attempts) > 1
        result["repair_trigger_source"] = prior_repair_trigger_source
        result["repair_trigger_sources"] = prior_repair_trigger_sources
        result["failure_provenance"] = prior_repair_trigger_sources
        finalize_result(result)
        write_json(run_dir / "result.json", result)
        return result
    if provider_result.failed:
        if use_task_gold_patches:
            result["status"] = "gold_patch_apply_error"
            result["success"] = False
            result["failure_category"] = "gold_patch_apply_error"
            result["failure_details"] = {
                "failure_reason": provider_result.failure_reason,
                "prior_repair_trigger_source": prior_repair_trigger_source,
                "prior_repair_trigger_sources": prior_repair_trigger_sources,
            }
            result["attempt_count"] = len(attempts)
            result["repair_attempted"] = False
            result["repair_trigger_source"] = prior_repair_trigger_source
            result["repair_trigger_sources"] = prior_repair_trigger_sources
            result["failure_provenance"] = prior_repair_trigger_sources + ["gold_patch"]
            finalize_result(result)
            write_json(run_dir / "result.json", result)
            return result
        provider_error_kind = classify_provider_failure(provider_result)
        result["status"] = "provider_auth_error" if provider_error_kind == "auth_required" else "provider_error"
        result["provider_error_kind"] = provider_error_kind
        result["success"] = False
        result["failure_category"] = (
            "provider_auth_error"
            if provider_error_kind == "auth_required"
            else "provider_usage_limited"
            if provider_error_kind == "usage_limited"
            else "provider_infra_busy"
            if provider_error_kind == "capacity_busy"
            else "provider_infra_error"
            if provider_error_kind in {"internal_server_error", "transport_error"}
            else "provider_error"
        )
        result["failure_details"] = {
            "provider_error_kind": provider_error_kind,
            "prior_repair_trigger_source": prior_repair_trigger_source,
            "prior_repair_trigger_sources": prior_repair_trigger_sources,
        }
        result["attempt_count"] = len(attempts)
        result["repair_attempted"] = len(attempts) > 1
        result["repair_trigger_source"] = prior_repair_trigger_source
        result["repair_trigger_sources"] = prior_repair_trigger_sources
        result["failure_provenance"] = prior_repair_trigger_sources + ["provider"]
        finalize_result(result)
        write_json(run_dir / "result.json", result)
        return result

    after = snapshot_tree(workspace)
    changed_files = sorted(compute_changed_files(before, after))
    result["changed_files"] = changed_files
    diff_path = run_dir / "patch.diff"
    diff_text = unified_diff(before, after)
    diff_path.write_text(diff_text)
    result["patch_diff_path"] = str(diff_path)

    result["court_jester"] = {
        "mode": policy.court_jester_mode,
        "results": court_jester_results,
        "verify_failed": attempts[-1].get("court_jester", {}).get("verify_failed", False),
    }

    public_results = final_public_results
    public_ok = final_public_ok
    hidden_checks_ran = final_hidden_checks_ran
    hidden_checks_sampled = final_hidden_checks_sampled
    hidden_results = final_hidden_results
    if not hidden_checks_ran:
        hidden_checks_ran = hidden_checks_requested and (
            public_ok or should_sample_hidden_on_public_failure(hidden_seed)
        )
        hidden_checks_sampled = hidden_checks_requested and (not public_ok) and hidden_checks_ran
        hidden_results = (
            run_commands(
                [task.hidden_check_command] if task.hidden_check_command else [],
                workspace,
                run_dir,
                "hidden",
                extra_env={"CJ_HIDDEN_SEED": hidden_seed},
            )
            if hidden_checks_ran
            else []
        )
        result["timings"]["hidden_checks_ms"] += sum(item.duration_ms for item in hidden_results)

    hidden_ok = all(item.exit_code == 0 for item in hidden_results) if hidden_results else True
    verify_failed = attempts[-1].get("court_jester", {}).get("verify_failed", False)
    verify_gate_ok = not (policy.block_on_failed_verify and verify_failed)
    success = public_ok and hidden_ok and verify_gate_ok
    attempt_count = len(attempts)
    verify_failed_attempts = sum(
        1
        for attempt in attempts
        if attempt.get("court_jester", {}).get("verify_failed")
    )
    public_failed_attempts = sum(1 for attempt in attempts if attempt.get("public_failed"))
    repair_attempted = attempt_count > 1
    repaired_after_verify_failure = success and verify_failed_attempts > 0
    repaired_after_public_failure = success and public_failed_attempts > 0
    repair_trigger_sources = [
        attempt["repair_trigger_source"]
        for attempt in attempts
        if attempt.get("repair_trigger_source")
    ]
    if not repair_trigger_sources:
        repair_trigger_source = None
    elif len(set(repair_trigger_sources)) == 1:
        repair_trigger_source = repair_trigger_sources[0]
    else:
        repair_trigger_source = "multiple"
    hidden_failed = hidden_checks_ran and not hidden_ok

    result["public_checks"] = [asdict(item) for item in public_results]
    result["hidden_checks"] = [asdict(item) for item in hidden_results]
    result["verify_summary"] = summarize_verify_results(court_jester_results)
    result["verify_gate_ok"] = verify_gate_ok
    result["public_checks_pass"] = public_ok
    result["hidden_checks_pass"] = hidden_ok
    result["hidden_checks_ran"] = hidden_checks_ran
    result["hidden_checks_skipped"] = hidden_checks_requested and not hidden_checks_ran
    result["hidden_checks_sampled_on_public_failure"] = hidden_checks_sampled
    result["verify_failed"] = verify_failed
    result["public_failed"] = not public_ok
    result["hidden_failed"] = hidden_failed
    result["success"] = success
    result["attempt_count"] = attempt_count
    result["repair_attempted"] = repair_attempted
    result["repair_trigger_source"] = repair_trigger_source
    result["repair_trigger_sources"] = repair_trigger_sources
    result["verify_failed_attempts"] = verify_failed_attempts
    result["public_failed_attempts"] = public_failed_attempts
    result["repaired_after_verify_failure"] = repaired_after_verify_failure
    result["repaired_after_public_failure"] = repaired_after_public_failure
    result["failure_provenance"] = [
        source
        for source, failed in (
            ("verify", verify_failed),
            ("public", not public_ok),
            ("hidden", hidden_failed),
        )
        if failed
    ]
    failure_category, failure_details = classify_outcome(
        success=success,
        public_ok=public_ok,
        hidden_ok=hidden_ok,
        verify_failed=verify_failed,
        verify_results=court_jester_results,
    )
    result["failure_category"] = failure_category
    result["failure_details"] = failure_details
    result["status"] = "completed"
    finalize_result(result)
    write_json(run_dir / "result.json", result)
    return result


def run_commands(
    commands: list[list[str]] | list[None],
    workspace: Path,
    run_dir: Path,
    label: str,
    extra_env: dict[str, str] | None = None,
) -> list[CommandResult]:
    results: list[CommandResult] = []
    for index, command in enumerate(commands):
        if not command:
            continue
        argv = [substitute_token(token, workspace) for token in command]
        start = time.time()
        completed = subprocess.run(
            argv,
            cwd=workspace,
            capture_output=True,
            text=True,
            env={**os.environ, **extra_env} if extra_env else None,
        )
        duration_ms = int((time.time() - start) * 1000)
        stdout_path = run_dir / f"{label}_{index}.stdout.txt"
        stderr_path = run_dir / f"{label}_{index}.stderr.txt"
        stdout_path.write_text(completed.stdout)
        stderr_path.write_text(completed.stderr)
        results.append(
            CommandResult(
                argv=argv,
                exit_code=completed.returncode,
                duration_ms=duration_ms,
                stdout_path=str(stdout_path),
                stderr_path=str(stderr_path),
            )
        )
    return results


def substitute_token(token: str, workspace: Path) -> str:
    return (
        token.replace("{workspace}", str(workspace.resolve()))
        .replace("{bench_root}", str(BENCH_ROOT))
        .replace("{repo_root}", str(REPO_ROOT))
    )


def snapshot_tree(root: Path) -> dict[str, str]:
    snapshot: dict[str, str] = {}
    ignored_prefixes = (".bench_", ".ruff_cache/", ".npm/", "Library/")
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(root).as_posix()
        if path.name == ".DS_Store":
            continue
        if rel.startswith(".bench_") or "/.bench_" in rel:
            continue
        if rel.startswith("__pycache__/") or "/__pycache__/" in rel:
            continue
        if rel.startswith(ignored_prefixes):
            continue
        if is_text_file(path):
            snapshot[rel] = path.read_text()
        else:
            snapshot[rel] = hashlib.sha256(path.read_bytes()).hexdigest()
    return snapshot


def snapshot_workspace_for_retry(workspace: Path, backup_dir: Path) -> None:
    if backup_dir.exists():
        shutil.rmtree(backup_dir)
    shutil.copytree(workspace, backup_dir)


def restore_workspace_from_retry_snapshot(backup_dir: Path, workspace: Path) -> None:
    if workspace.exists():
        shutil.rmtree(workspace)
    shutil.copytree(backup_dir, workspace)


def setup_cache_root() -> Path:
    return Path(os.getenv("CJ_SETUP_CACHE_ROOT", "/tmp/court-jester-setup-cache"))


def setup_cache_dir(cache_key: str) -> Path:
    slug = slugify(cache_key)[:64] or "setup-cache"
    digest = hashlib.sha256(cache_key.encode("utf-8")).hexdigest()[:12]
    return setup_cache_root() / f"{slug}-{digest}"


def prepare_workspace_for_run(task: TaskManifest, workspace: Path, run_dir: Path) -> WorkspaceSetupResult:
    start = time.time()
    commands = task.setup_commands
    cache_dir = setup_cache_dir(task.setup_cache_key) if task.setup_cache_key and commands else None
    if cache_dir is not None and cache_dir.exists():
        if workspace.exists():
            shutil.rmtree(workspace)
        shutil.copytree(cache_dir, workspace)
        return WorkspaceSetupResult(
            success=True,
            cache_hit=True,
            duration_ms=int((time.time() - start) * 1000),
            commands=[],
            cache_dir=str(cache_dir),
        )
    if not commands:
        return WorkspaceSetupResult(
            success=True,
            cache_hit=False,
            duration_ms=int((time.time() - start) * 1000),
            commands=[],
            cache_dir=str(cache_dir) if cache_dir is not None else None,
        )
    results = run_commands(commands, workspace, run_dir, "setup")
    success = all(item.exit_code == 0 for item in results)
    if success and cache_dir is not None:
        cache_dir.parent.mkdir(parents=True, exist_ok=True)
        if cache_dir.exists():
            shutil.rmtree(cache_dir)
        shutil.copytree(workspace, cache_dir)
    failure_reason = None
    if not success:
        for item in results:
            if item.exit_code == 0:
                continue
            stderr = Path(item.stderr_path).read_text() if Path(item.stderr_path).exists() else ""
            stdout = Path(item.stdout_path).read_text() if Path(item.stdout_path).exists() else ""
            failure_reason = first_nonempty_text(stderr, stdout) or f"setup command failed: {' '.join(item.argv)}"
            break
    return WorkspaceSetupResult(
        success=success,
        cache_hit=False,
        duration_ms=int((time.time() - start) * 1000),
        commands=results,
        cache_dir=str(cache_dir) if cache_dir is not None else None,
        failure_reason=failure_reason,
    )


def serialize_setup_result(result: WorkspaceSetupResult) -> dict[str, Any]:
    return {
        "success": result.success,
        "cache_hit": result.cache_hit,
        "duration_ms": result.duration_ms,
        "cache_dir": result.cache_dir,
        "failure_reason": result.failure_reason,
        "commands": [asdict(item) for item in result.commands],
    }


def apply_task_gold_patch(
    task: TaskManifest,
    workspace: Path,
    run_dir: Path,
    attempt: int,
) -> tuple[ProviderResult, CommandResult | None]:
    if not task.gold_patch_path:
        return (
            ProviderResult(
                failed=True,
                failure_reason="Task gold patch mode requested but task.gold_patch_path is not set",
            ),
            None,
        )
    patch_path = workspace / task.gold_patch_path
    if not patch_path.exists():
        return (
            ProviderResult(
                failed=True,
                failure_reason=f"Task gold patch not found: {patch_path}",
            ),
            None,
        )
    argv = ["git", "apply", "--reject", "--whitespace=nowarn", str(patch_path.resolve())]
    start = time.time()
    completed = subprocess.run(argv, cwd=workspace, capture_output=True, text=True)
    duration_ms = int((time.time() - start) * 1000)
    stdout_path = run_dir / f"gold_patch_attempt_{attempt}_0.stdout.txt"
    stderr_path = run_dir / f"gold_patch_attempt_{attempt}_0.stderr.txt"
    stdout_path.write_text(completed.stdout)
    stderr_path.write_text(completed.stderr)
    command_result = CommandResult(
        argv=argv,
        exit_code=completed.returncode,
        duration_ms=duration_ms,
        stdout_path=str(stdout_path),
        stderr_path=str(stderr_path),
    )
    if completed.returncode != 0:
        return (
            ProviderResult(
                transcript=[completed.stdout, completed.stderr],
                exit_code=completed.returncode,
                failed=True,
                failure_reason=completed.stderr.strip()
                or completed.stdout.strip()
                or "task gold patch apply failed",
            ),
            command_result,
        )
    changed_files = task.gold_changed_files or infer_changed_files_from_patch(patch_path.read_text())
    return (
        ProviderResult(
            changed_files=changed_files,
            transcript=[completed.stdout, completed.stderr],
            parsed_summary={
                "status": "completed",
                "summary": "Applied task gold patch.",
                "files_changed": changed_files,
            },
        ),
        command_result,
    )


def infer_changed_files_from_patch(patch_text: str) -> list[str]:
    changed: list[str] = []
    seen: set[str] = set()
    for line in patch_text.splitlines():
        if not line.startswith("+++ b/"):
            continue
        path = line[len("+++ b/") :].strip()
        if not path or path == "/dev/null" or path in seen:
            continue
        seen.add(path)
        changed.append(path)
    return changed


def finalize_result(result: dict[str, Any]) -> None:
    finished_at_ms = int(time.time() * 1000)
    result["timestamps"]["finished_at_epoch_ms"] = finished_at_ms
    started_at_ms = int(result["timestamps"].get("started_at_epoch_ms", finished_at_ms))
    result["timings"]["end_to_end_ms"] = finished_at_ms - started_at_ms


def stringify_output(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return str(value)


def classify_provider_failure(provider_result: ProviderResult) -> str:
    haystack = "\n".join(
        filter(
            None,
            [
                stringify_output(provider_result.failure_reason),
                *(stringify_output(item) for item in provider_result.transcript),
            ],
        )
    ).lower()
    auth_markers = ("not logged in", "please run /login", "login required", "authentication")
    if any(marker in haystack for marker in auth_markers):
        return "auth_required"
    usage_limit_markers = ("usage limit", "quota", "try again at")
    if any(marker in haystack for marker in usage_limit_markers):
        return "usage_limited"
    capacity_markers = (
        "all inference nodes that can serve this model are currently busy",
        "retry shortly",
        "http 503",
    )
    if any(marker in haystack for marker in capacity_markers):
        return "capacity_busy"
    internal_server_markers = (
        "internal server error",
        "http 500",
        "http 502",
        "http 504",
        "unexpectedcontenttype",
    )
    if any(marker in haystack for marker in internal_server_markers):
        return "internal_server_error"
    transport_markers = (
        "transport channel closed",
        "connection reset",
        "broken pipe",
        "curl: (6)",
        "could not resolve host",
        "curl: (56)",
    )
    if any(marker in haystack for marker in transport_markers):
        return "transport_error"
    return "generic"


def supports_provider_retry(model: ModelManifest) -> bool:
    return model.provider in {"codex_cli", "claude_cli", "openai_compat_chat"}


def provider_retry_limit() -> int:
    raw = os.getenv("CJ_PROVIDER_INFRA_RETRIES", "2")
    try:
        return max(0, int(raw))
    except ValueError:
        return 2


def should_retry_provider_failure(provider_error_kind: str) -> bool:
    return provider_error_kind in PROVIDER_RETRYABLE_KINDS


def provider_retry_delay_seconds(provider_error_kind: str, retry_index: int) -> float:
    if provider_error_kind == "capacity_busy":
        return [5.0, 15.0][min(retry_index, 1)]
    if provider_error_kind == "internal_server_error":
        return [2.0, 5.0][min(retry_index, 1)]
    if provider_error_kind == "transport_error":
        return [1.0, 3.0][min(retry_index, 1)]
    return 0.0


def classify_outcome(
    *,
    success: bool,
    public_ok: bool,
    hidden_ok: bool,
    verify_failed: bool,
    verify_results: list[dict[str, Any]],
) -> tuple[str, dict[str, Any]]:
    if success:
        return "success", {}

    verify_failure_kind, verify_failure_stage, verify_failure_path = classify_verify_failure(verify_results)
    details = {
        "verify_failure_kind": verify_failure_kind,
        "verify_failure_stage": verify_failure_stage,
        "verify_failure_path": verify_failure_path,
    }

    if verify_failed:
        if verify_failure_kind == "timeout":
            return "verify_infra_timeout", details
        if public_ok and hidden_ok:
            return "verify_stronger_than_eval", details
        if not hidden_ok and public_ok:
            return "verify_caught_hidden_bug", details
        if not public_ok:
            return "verify_caught_public_bug", details
        return "verify_failed", details

    if not public_ok:
        return "public_check_failure", details
    if not hidden_ok:
        return "hidden_semantic_miss", details
    return "unknown_failure", details


def classify_verify_failure(
    verify_results: list[dict[str, Any]],
) -> tuple[str | None, str | None, str | None]:
    for item in verify_results:
        parsed = item.get("response")
        if not isinstance(parsed, dict):
            continue
        if parsed.get("overall_ok", False):
            continue
        for stage in parsed.get("stages", []):
            if stage.get("ok", True):
                continue
            detail = stage.get("detail") if isinstance(stage.get("detail"), dict) else {}
            error = (stage.get("error") or "").lower()
            stdout = str(detail.get("stdout", "")).lower()
            stderr = str(detail.get("stderr", "")).lower()
            haystack = "\n".join([error, stdout, stderr])
            if "timed out" in haystack:
                return "timeout", stage.get("name"), item.get("path")
            return "stage_failure", stage.get("name"), item.get("path")
        return "overall_failure", None, item.get("path")
    return None, None, None


def summarize_verify_results(verify_results: list[dict[str, Any]]) -> dict[str, Any]:
    failed_paths: list[str] = []
    failed_stages: dict[str, int] = {}
    stage_durations_ms: dict[str, int] = {}
    fuzz_failure_count = 0

    for item in verify_results:
        parsed = item.get("response")
        path = item.get("path")
        if not isinstance(parsed, dict):
            continue
        if not parsed.get("overall_ok", False) and isinstance(path, str):
            failed_paths.append(path)
        for stage in parsed.get("stages", []):
            if not isinstance(stage, dict):
                continue
            stage_name = str(stage.get("name", "unknown"))
            try:
                stage_durations_ms[stage_name] = stage_durations_ms.get(stage_name, 0) + int(
                    stage.get("duration_ms", 0)
                )
            except (TypeError, ValueError):
                pass
            if not stage.get("ok", True):
                failed_stages[stage_name] = failed_stages.get(stage_name, 0) + 1
            detail = stage.get("detail")
            if isinstance(detail, dict):
                fuzz_failures = detail.get("fuzz_failures")
                if isinstance(fuzz_failures, list):
                    fuzz_failure_count += len(fuzz_failures)

    return {
        "failed_paths": failed_paths,
        "failed_stage_counts": failed_stages,
        "stage_durations_ms": stage_durations_ms,
        "fuzz_failure_count": fuzz_failure_count,
    }


def is_text_file(path: Path) -> bool:
    try:
        path.read_text()
        return True
    except UnicodeDecodeError:
        return False


def compute_changed_files(before: dict[str, str], after: dict[str, str]) -> set[str]:
    return {path for path in set(before) | set(after) if before.get(path) != after.get(path)}


def unified_diff(before: dict[str, str], after: dict[str, str]) -> str:
    chunks: list[str] = []
    for path in sorted(set(before) | set(after)):
        old = before.get(path, "")
        new = after.get(path, "")
        if old == new:
            continue
        old_lines = old.splitlines(keepends=True)
        new_lines = new.splitlines(keepends=True)
        diff = difflib.unified_diff(
            old_lines,
            new_lines,
            fromfile=f"a/{path}",
            tofile=f"b/{path}",
        )
        chunks.extend(diff)
    return "".join(chunks)


def normalize_for_json(value: Any) -> Any:
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    if isinstance(value, dict):
        return {str(key): normalize_for_json(item) for key, item in value.items()}
    if isinstance(value, list):
        return [normalize_for_json(item) for item in value]
    if isinstance(value, tuple):
        return [normalize_for_json(item) for item in value]
    return value


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.write_text(json.dumps(normalize_for_json(payload), indent=2, sort_keys=True) + "\n")


def extract_provider_summary(provider_result: ProviderResult) -> str:
    parsed = provider_result.parsed_summary
    if isinstance(parsed, dict):
        summary = parsed.get("summary")
        if isinstance(summary, str) and summary.strip():
            return summary.strip()
    for item in provider_result.transcript:
        if not isinstance(item, str):
            continue
        snippet = first_meaningful_line(item)
        if snippet:
            return snippet
    return ""


def build_critic_feedback(
    *,
    policy: PolicyManifest,
    workspace: Path,
    task: TaskManifest,
    feedback: str,
    promoted_repros: list[str],
    history: list[dict[str, object]],
) -> str | None:
    critic_model_id = policy.critic_model_id
    if not critic_model_id:
        return None
    critic_manifest_path = BENCH_ROOT / "models" / f"{critic_model_id}.json"
    if not critic_manifest_path.exists():
        return None
    critic_model = load_model(critic_manifest_path)
    critic_provider = provider_from_manifest(critic_model)
    try:
        return critic_provider.critique(
            workspace,
            task,
            feedback=feedback,
            promoted_repros=promoted_repros,
            history=history,
        )
    except Exception:
        return None


def format_public_failure_feedback(items: list[CommandResult]) -> str:
    lines = [
        "public checks failed. Repair the workspace using these concrete failures.",
        "Prioritize the smallest code change that makes the public checks pass.",
    ]
    for item in items:
        if item.exit_code == 0:
            continue
        command = " ".join(item.argv)
        lines.append(f"- Command: {command}")
        stderr = Path(item.stderr_path).read_text() if Path(item.stderr_path).exists() else ""
        stdout = Path(item.stdout_path).read_text() if Path(item.stdout_path).exists() else ""
        snippet = first_nonempty_text(stderr, stdout)
        if snippet:
            lines.append(f"  Evidence: {snippet}")
    return "\n".join(lines)


def format_verify_feedback(
    items: list[dict[str, Any]],
    *,
    promoted_repros: list[str] | None = None,
    task: TaskManifest | None = None,
    include_first_party_checklist: bool = False,
) -> str:
    lines = [
        "court-jester verify failed. Repair the workspace using these concrete failures.",
        "Prioritize the smallest code change that eliminates the failing repros.",
    ]
    if promoted_repros:
        lines.append("Required repros to fix on the next attempt:")
        for repro in promoted_repros:
            lines.append(f"- {repro}")
    checklist = (
        build_first_party_repair_checklist(task, items)
        if include_first_party_checklist
        else []
    )
    if checklist:
        lines.append("Court Jester repair checklist:")
        for item in checklist:
            lines.append(f"- {item}")
    for item in items:
        response = item.get("response")
        if not isinstance(response, dict) or response.get("overall_ok", False):
            continue
        path = item["path"]
        lines.append(f"- File: {path}")
        for summary_line in summarize_verify_failures(response):
            lines.append(f"  {summary_line}")
    return "\n".join(lines)


def collect_promoted_verify_repros(language: str, items: list[dict[str, Any]]) -> list[str]:
    repros: list[str] = []
    seen: set[str] = set()
    for item in items:
        response = item.get("response")
        if not isinstance(response, dict):
            continue
        for stage in response.get("stages", []):
            if stage.get("ok", True):
                continue
            detail = stage.get("detail") if isinstance(stage.get("detail"), dict) else {}
            error = str(stage.get("error") or "").strip()
            assertion_repro = extract_assertion_repro(error, detail)
            if assertion_repro and assertion_repro not in seen:
                seen.add(assertion_repro)
                repros.append(assertion_repro)
            fuzz_failures = detail.get("fuzz_failures")
            if isinstance(fuzz_failures, list):
                for failure in fuzz_failures[:3]:
                    assertion = build_fuzz_repro_assertion(language, failure)
                    if assertion and assertion not in seen:
                        seen.add(assertion)
                        repros.append(assertion)
            if len(repros) >= 3:
                return repros[:3]
    return repros[:3]


def build_first_party_repair_checklist(
    task: TaskManifest | None,
    items: list[dict[str, Any]],
) -> list[str]:
    checklist: list[str] = []
    seen: set[str] = set()

    def add(line: str) -> None:
        if line not in seen:
            seen.add(line)
            checklist.append(line)

    haystack = collect_verify_haystack(items).lower()
    if "nullish string leak" in haystack:
        add("Do not leak nullish values into output strings.")
        add("Drop dict/list/object inputs instead of converting them to strings.")
        add("Preserve the original order of any remaining valid scalar list items.")
    if "normalize" in haystack or "accent" in haystack or "non-ascii" in haystack:
        add("Normalize accepted text values before encoding them into the final output.")
    if "not defined" in haystack or "cannot find name" in haystack:
        add("Resolve the missing symbol by fixing both the definition/export and every import or call site that uses it.")
    if "referenceerror" in haystack:
        add("Do not add a new helper call unless the target symbol is also wired into the current file correctly.")
    if "assert.equal" in haystack or "assert " in haystack:
        add("Change behavior on the exact cited repro before making broader refactors.")
    if "property_violation" in haystack:
        add("Avoid cosmetic edits that leave the cited failing property unchanged.")

    return checklist[:5]


def collect_verify_haystack(items: list[dict[str, Any]]) -> str:
    chunks: list[str] = []
    for item in items:
        response = item.get("response")
        if not isinstance(response, dict):
            continue
        for stage in response.get("stages", []):
            if not isinstance(stage, dict):
                continue
            detail = stage.get("detail") if isinstance(stage.get("detail"), dict) else {}
            chunks.append(str(stage.get("error") or ""))
            chunks.append(str(detail.get("stderr") or ""))
            chunks.append(str(detail.get("stdout") or ""))
            fuzz_failures = detail.get("fuzz_failures")
            if isinstance(fuzz_failures, list):
                for failure in fuzz_failures:
                    chunks.append(str(failure))
    return "\n".join(chunks)


def build_fuzz_repro_assertion(language: str, failure: Any) -> str | None:
    if not isinstance(failure, dict):
        return None
    function = str(failure.get("function") or "").strip()
    input_value = str(failure.get("input") or "").strip()
    message = str(failure.get("message") or "").strip()
    if not function or not input_value:
        return None
    observed_output = extract_observed_output(message)
    if observed_output is None:
        return None
    if language == "python":
        return f"assert {function}(*{input_value}) != {json.dumps(observed_output)}"
    if language == "typescript":
        return f"assert.notEqual({function}(...{input_value}), {json.dumps(observed_output)});"
    return None


def extract_observed_output(message: str) -> str | None:
    match = re.search(r": '([^']*)'$", message)
    if not match:
        return None
    return match.group(1)


def write_promoted_verify_test(
    *,
    workspace: Path,
    task: TaskManifest,
    attempt: int,
    promoted_repros: list[str],
) -> Path | None:
    if not task.verify_test_path or not promoted_repros:
        return None
    original = workspace / task.verify_test_path
    if not original.exists():
        return None
    suffix = original.suffix
    generated = original.with_name(f".bench_promoted_verify_attempt_{attempt + 1}{suffix}")
    lines = [original.read_text().rstrip(), "", promoted_repro_block(task.language, promoted_repros)]
    generated.write_text("\n".join(line for line in lines if line) + "\n")
    return generated


def promoted_repro_block(language: str, promoted_repros: list[str]) -> str:
    if language == "python":
        header = [
            "# Court Jester promoted repros",
            "# These cases were harvested from the previous failed verify attempt.",
        ]
        return "\n".join(header + promoted_repros)
    if language == "typescript":
        header = [
            "// Court Jester promoted repros",
            "// These cases were harvested from the previous failed verify attempt.",
        ]
        return "\n".join(header + promoted_repros)
    return "\n".join(promoted_repros)


def format_hidden_failure_feedback(items: list[CommandResult]) -> str:
    lines = [
        "hidden evaluation failed. Repair the workspace using these concrete failures.",
        "Prioritize the smallest code change that satisfies the failing hidden cases.",
    ]
    for item in items:
        if item.exit_code == 0:
            continue
        command = " ".join(item.argv)
        lines.append(f"- Command: {command}")
        stderr = Path(item.stderr_path).read_text() if Path(item.stderr_path).exists() else ""
        stdout = Path(item.stdout_path).read_text() if Path(item.stdout_path).exists() else ""
        snippet = first_nonempty_text(stderr, stdout)
        if snippet:
            lines.append(f"  Evidence: {snippet}")
    return "\n".join(lines)


def summarize_verify_failures(response: dict[str, Any]) -> list[str]:
    lines: list[str] = []
    for stage in response.get("stages", []):
        if stage.get("ok", True):
            continue
        stage_name = stage.get("name", "unknown")
        detail = stage.get("detail") if isinstance(stage.get("detail"), dict) else {}
        error = str(stage.get("error") or "").strip()
        lines.append(f"Stage: {stage_name}")

        assertion_repro = extract_assertion_repro(error, detail)
        if assertion_repro:
            lines.append(f"Counterexample: {assertion_repro}")

        fuzz_failures = detail.get("fuzz_failures")
        if isinstance(fuzz_failures, list) and fuzz_failures:
            for failure in fuzz_failures[:3]:
                if not isinstance(failure, dict):
                    continue
                function = failure.get("function", "<unknown>")
                severity = failure.get("severity", "failure")
                input_value = failure.get("input", "<unknown>")
                message = str(failure.get("message") or "").strip()
                lines.append(
                    f"Repro: {function}{input_value} -> {severity}"
                )
                if message:
                    lines.append(f"Message: {message}")

        snippet = first_nonempty_text(
            error,
            str(detail.get("stderr") or ""),
            str(detail.get("stdout") or ""),
        )
        if snippet:
            lines.append(f"Evidence: {snippet}")
    if not lines:
        lines.append("No structured verify failure details were available.")
    return lines


def extract_assertion_repro(error: str, detail: dict[str, Any]) -> str | None:
    candidates = [
        error,
        str(detail.get("stderr") or ""),
        str(detail.get("stdout") or ""),
    ]
    for value in candidates:
        for raw_line in value.splitlines():
            line = raw_line.strip()
            if not line.startswith("assert "):
                continue
            repro = line[len("assert ") :].strip()
            if repro:
                return repro[:300]
    return None


def first_nonempty_text(*values: str) -> str | None:
    for value in values:
        snippet = first_meaningful_line(value)
        if snippet:
            return snippet
    return None


def first_meaningful_line(value: str) -> str | None:
    for raw_line in value.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith("__COURT_JESTER_FUZZ_JSON__"):
            continue
        if line.startswith("[") and line.endswith("]"):
            continue
        return line[:240]
    return None


def should_sample_hidden_on_public_failure(hidden_seed: str) -> bool:
    try:
        return int(hidden_seed[:2], 16) % 4 == 0
    except ValueError:
        return False
