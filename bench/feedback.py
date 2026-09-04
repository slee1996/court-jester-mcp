"""Format repair feedback and repro assertions from recorded evidence."""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from .common import TaskManifest
from .results import CommandResult
from .reporting import (
    report_verdict,
    stage_status,
    stage_is_failed,
    stage_message,
    stage_findings,
    finding_function,
    finding_input,
    finding_message,
)


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


def normalize_feedback_path(path: str, workspace: Path | None) -> str:
    candidate = Path(path)
    if workspace is not None and candidate.is_absolute():
        try:
            return candidate.relative_to(workspace).as_posix()
        except ValueError:
            return candidate.as_posix()
    return candidate.as_posix()


def resolve_local_import_path(source_path: Path, import_path: str) -> Path | None:
    if not import_path.startswith("."):
        return None
    target = source_path.parent / import_path
    candidates: list[Path] = []
    if target.suffix:
        candidates.append(target)
    else:
        candidates.extend(
            [
                target.with_suffix(".ts"),
                target.with_suffix(".tsx"),
                target.with_suffix(".js"),
                target.with_suffix(".jsx"),
                target / "index.ts",
                target / "index.tsx",
                target / "index.js",
                target / "index.jsx",
            ]
        )
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return None


def local_import_paths(source_path: Path) -> list[str]:
    if not source_path.exists():
        return []
    text = source_path.read_text()
    imports: list[str] = []
    for match in re.finditer(
        r'^\s*(?:import|export)\s+(?:[^"\']+\s+from\s+)?["\']([^"\']+)["\']',
        text,
        re.MULTILINE,
    ):
        import_path = match.group(1)
        if import_path.startswith("."):
            imports.append(import_path)
    return imports


def infer_tests_only_owner_paths(
    *,
    workspace: Path,
    task: TaskManifest,
    failed_path: str,
) -> list[str]:
    source_path = workspace / failed_path
    verify_paths = {Path(path).as_posix() for path in task.verify_paths}
    owners: list[str] = []
    for import_path in local_import_paths(source_path):
        resolved = resolve_local_import_path(source_path, import_path)
        if resolved is None:
            continue
        relative = normalize_feedback_path(str(resolved), workspace)
        if relative != failed_path and relative in verify_paths and relative not in owners:
            owners.append(relative)
    return owners


def verify_feedback_scope_lines(
    item: dict[str, Any],
    *,
    workspace: Path | None,
    task: TaskManifest | None,
) -> list[str]:
    path = item.get("path")
    if not isinstance(path, str) or not path:
        return []
    normalized_path = normalize_feedback_path(path, workspace)
    if task is None or not task.verify_tests_only or workspace is None:
        return [f"File: {normalized_path}"]
    owner_paths = infer_tests_only_owner_paths(
        workspace=workspace,
        task=task,
        failed_path=normalized_path,
    )
    if owner_paths:
        lines = [f"Likely owner files: {', '.join(owner_paths)}"]
        if normalized_path not in owner_paths:
            lines.append(f"Related call site: {normalized_path}")
        return lines
    return [f"Related source file: {normalized_path}"]


def format_verify_feedback(
    items: list[dict[str, Any]],
    *,
    workspace: Path | None = None,
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
        if not isinstance(response, dict) or report_verdict(response) not in {"fail", "inconclusive"}:
            continue
        for scope_line in verify_feedback_scope_lines(
            item,
            workspace=workspace,
            task=task,
        ):
            lines.append(f"- {scope_line}")
        for summary_line in summarize_verify_failures(response, task=task):
            lines.append(f"  {summary_line}")
    return "\n".join(lines)


def collect_promoted_verify_repros(language: str, items: list[dict[str, Any]]) -> list[str]:
    repros: list[str] = []
    seen: set[str] = set()
    for item in items:
        response = item.get("response")
        if not isinstance(response, dict) or report_verdict(response) != "fail":
            continue
        for stage in response.get("stages", []):
            if not stage_is_failed(stage):
                continue
            detail = stage.get("detail") if isinstance(stage.get("detail"), dict) else {}
            message = stage_message(stage).strip()
            assertion_repro = extract_assertion_repro(message, detail)
            if assertion_repro and assertion_repro not in seen:
                seen.add(assertion_repro)
                repros.append(assertion_repro)
            for finding in stage_findings(stage)[:3]:
                assertion = build_fuzz_repro_assertion(language, finding)
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
            chunks.append(stage_message(stage))
            chunks.append(str(detail.get("stderr") or ""))
            chunks.append(str(detail.get("stdout") or ""))
            for finding in stage_findings(stage):
                chunks.append(str(finding))
    return "\n".join(chunk for chunk in chunks if chunk)


def build_fuzz_repro_assertion(language: str, failure: Any) -> str | None:
    if not isinstance(failure, dict):
        return None
    function = finding_function(failure).strip()
    input_value = finding_input(failure).strip()
    message = finding_message(failure).strip()
    if not function or not input_value:
        return None
    observed_output = extract_observed_output(message)
    if observed_output is None:
        return None
    if language == "python":
        return f"assert {function}(*{input_value}) != {json.dumps(observed_output)}"
    if language == "typescript":
        return f"expect({function}(...{input_value})).not.toBe({json.dumps(observed_output)});"
    return None


def extract_observed_output(message: str) -> str | None:
    match = re.search(r": '([^']*)'$", message)
    if not match:
        return None
    return match.group(1)


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


def should_suppress_verify_evidence(
    *,
    task: TaskManifest | None,
    stage_name: str,
    snippet: str,
) -> bool:
    if task is None or not task.verify_tests_only:
        return False
    return stage_name == "test" and snippet.strip().lower() == "process timed out"


def summarize_verify_failures(
    response: dict[str, Any],
    *,
    task: TaskManifest | None = None,
) -> list[str]:
    lines: list[str] = []
    for stage in response.get("stages", []):
        if stage_status(stage) not in {"failed", "inconclusive"}:
            continue
        stage_name = stage.get("name", "unknown")
        detail = stage.get("detail") if isinstance(stage.get("detail"), dict) else {}
        message = stage_message(stage).strip()
        lines.append(f"Stage: {stage_name}")

        assertion_repro = extract_assertion_repro(message, detail)
        if assertion_repro:
            lines.append(f"Counterexample: {assertion_repro}")

        findings = stage_findings(stage)
        for finding in findings[:3]:
            function = finding_function(finding) or "<unknown>"
            severity = finding.get("severity", "failure")
            input_value = finding_input(finding) or "<unknown>"
            finding_text = finding_message(finding).strip()
            lines.append(f"Repro: {function}{input_value} -> {severity}")
            if finding_text:
                lines.append(f"Message: {finding_text}")

        snippet = first_nonempty_text(
            message,
            str(detail.get("stderr") or ""),
            str(detail.get("stdout") or ""),
        )
        if snippet and not should_suppress_verify_evidence(
            task=task,
            stage_name=stage_name,
            snippet=snippet,
        ):
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
