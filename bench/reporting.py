"""Interpret verifier reports without running agents or mutating workspaces."""

from __future__ import annotations

import json
from typing import Any
from .common import VERIFY_SCHEMA_VERSION_REQUIRED


def report_schema_version(report: Any) -> int | None:
    if not isinstance(report, dict):
        return None
    version = report.get("schema_version")
    return version if isinstance(version, int) else None


def report_verdict(report: Any) -> str | None:
    """Return a verdict only from a structurally valid schema-v3 report."""
    if report_schema_version(report) != VERIFY_SCHEMA_VERSION_REQUIRED:
        return None

    verdict = report.get("verdict")
    strength = report.get("strength")
    stages = report.get("stages")
    summary = report.get("summary")
    if not isinstance(verdict, str) or verdict not in {"pass", "fail", "inconclusive"}:
        return None
    if not isinstance(strength, str) or strength not in {
        "none",
        "parse_only",
        "static_checked",
        "runtime_smoke",
        "property_checked",
        "authoritative_tests",
    }:
        return None
    if not isinstance(summary, dict):
        return None
    if not isinstance(stages, list) or not stages:
        return None
    for stage in stages:
        if not isinstance(stage, dict):
            return None
        name = stage.get("name")
        status = stage.get("status")
        duration_ms = stage.get("duration_ms")
        if not isinstance(name, str) or not name.strip():
            return None
        if not isinstance(status, str) or status not in {
            "passed",
            "failed",
            "inconclusive",
            "advisory",
            "skipped",
        }:
            return None
        if isinstance(duration_ms, bool) or not isinstance(duration_ms, int) or duration_ms < 0:
            return None
    return verdict


def report_is_failed(report: Any) -> bool:
    return report_verdict(report) == "fail"


def report_is_inconclusive(report: Any) -> bool:
    return report_verdict(report) == "inconclusive"


def stage_status(stage: Any) -> str | None:
    if not isinstance(stage, dict):
        return None
    status = stage.get("status")
    if not isinstance(status, str):
        return None
    return status if status in {"passed", "failed", "inconclusive", "advisory", "skipped"} else None


def stage_is_failed(stage: Any) -> bool:
    return stage_status(stage) == "failed"


def stage_message(stage: Any) -> str:
    if not isinstance(stage, dict):
        return ""
    return str(stage.get("message") or "")


def stage_findings(stage: Any) -> list[dict[str, Any]]:
    if not isinstance(stage, dict):
        return []
    detail = stage.get("detail")
    findings = detail.get("findings") if isinstance(detail, dict) else None
    return [finding for finding in findings if isinstance(finding, dict)] if isinstance(findings, list) else []
def _report_diagnostics(report: Any) -> list[dict[str, Any]]:
    """Collect typed diagnostics without interpreting human-readable output.

    Diagnostics were added additively to schema v3.  A few stage producers put
    them in ``detail`` while the final report puts them at the top level, so
    consume both locations and de-duplicate by their serialized content.
    """
    if not isinstance(report, dict):
        return []
    values: list[dict[str, Any]] = []
    candidates: list[Any] = [report.get("diagnostics")]
    for stage in report.get("stages", []):
        if not isinstance(stage, dict):
            continue
        detail = stage.get("detail")
        if isinstance(detail, dict):
            candidates.append(detail.get("diagnostics"))
            execution = detail.get("execution")
            if isinstance(execution, dict):
                candidates.append(execution.get("diagnostics"))
    seen: set[str] = set()
    for candidate in candidates:
        if not isinstance(candidate, list):
            continue
        for diagnostic in candidate:
            if not isinstance(diagnostic, dict):
                continue
            key = json.dumps(diagnostic, sort_keys=True, separators=(",", ":"))
            if key not in seen:
                seen.add(key)
                values.append(diagnostic)
    return values


def _report_findings(report: Any) -> list[dict[str, Any]]:
    findings: list[dict[str, Any]] = []
    if not isinstance(report, dict):
        return findings
    for stage in report.get("stages", []):
        findings.extend(stage_findings(stage))
    return findings


def _is_target_finding(finding: dict[str, Any]) -> bool:
    """Return true only for an unsuppressed, non-infrastructure finding."""
    if finding.get("suppressed") is True:
        return False
    severity = str(finding.get("severity") or "").lower()
    category = str(finding.get("category") or "").lower()
    return severity in {"crash", "property_violation", "behavioral_regression"} and category != "infrastructure"


def report_terminal_cause(report: Any) -> dict[str, Any] | None:
    """Resolve a report's terminal cause using typed diagnostic precedence.

    ``target`` is intentionally narrower than a failed stage: verifier,
    environment, and resource diagnostics are benchmark abstentions.  Reports
    without typed diagnostics retain the old stage/verdict interpretation.
    """
    if not isinstance(report, dict):
        return None
    diagnostics = _report_diagnostics(report)
    typed = bool(diagnostics) or "diagnostics" in report or "diagnostics_summary" in report
    target = [
        diagnostic
        for diagnostic in diagnostics
        if str(diagnostic.get("domain") or "").lower() == "target_code"
        and str(diagnostic.get("impact") or "").lower() == "gating"
    ]
    if target:
        cause = dict(target[0])
        cause["classification"] = "target"
        return cause
    findings = [finding for finding in _report_findings(report) if _is_target_finding(finding)]
    if findings:
        return {"classification": "target", "finding": findings[0]}
    blocking = [
        diagnostic
        for diagnostic in diagnostics
        if str(diagnostic.get("impact") or "").lower() == "blocking"
    ]
    if blocking:
        cause = dict(blocking[0])
        cause["classification"] = "inconclusive"
        return cause
    if typed and report.get("verdict") == "fail":
        # A typed report with no target evidence cannot be scored as a target
        # defect, even if an older producer emitted a failed verdict.
        return {"classification": "inconclusive", "kind": "harness_protocol"}
    if report.get("verdict") == "inconclusive":
        return {"classification": "inconclusive"}
    if report.get("verdict") == "fail":
        return {"classification": "legacy"}
    return None


def report_has_target_failure(report: Any) -> bool:
    cause = report_terminal_cause(report)
    return bool(cause and cause.get("classification") in {"target", "legacy"})


def report_metadata(report: Any) -> dict[str, Any]:
    """Extract machine-readable execution context for benchmark artifacts."""
    metadata: dict[str, set[str]] = {
        "source_modes": set(),
        "network_policies": set(),
        "runtimes": set(),
        "input_origins": set(),
        "provenance": set(),
        "termination_kinds": set(),
        "failure_domains": set(),
        "failure_kinds": set(),
        "diagnostic_components": set(),
    }

    def walk(value: Any, key: str = "") -> None:
        if isinstance(value, dict):
            for name, child in value.items():
                normalized = str(name).lower()
                if normalized in {"source_mode", "network_policy", "network", "runtime", "input_origin", "provenance", "termination_kind", "failure_domain", "failure_kind", "component"}:
                    if isinstance(child, str):
                        field = {
                            "source_mode": "source_modes",
                            "network_policy": "network_policies",
                            "network": "network_policies",
                            "runtime": "runtimes",
                            "input_origin": "input_origins",
                            "provenance": "provenance",
                            "termination_kind": "termination_kinds",
                            "failure_domain": "failure_domains",
                            "failure_kind": "failure_kinds",
                            "component": "diagnostic_components",
                        }[normalized]
                        metadata[field].add(child)
                if normalized == "kind" and key in {"termination", "process"} and isinstance(child, str):
                    metadata["termination_kinds"].add(child)
                walk(child, normalized)
        elif isinstance(value, list):
            for child in value:
                walk(child, key)

    walk(report)
    return {key: sorted(values) for key, values in metadata.items() if values}


def finding_function(finding: dict[str, Any]) -> str:
    location = finding.get("location")
    if isinstance(location, dict) and location.get("function"):
        return str(location["function"])
    return str(finding.get("function") or "")


def finding_input(finding: dict[str, Any]) -> str:
    repro = finding.get("repro")
    if isinstance(repro, dict):
        if repro.get("snippet"):
            return str(repro["snippet"])
        arguments = repro.get("arguments")
        if arguments:
            return str(arguments)
    return str(finding.get("input") or "")


def finding_message(finding: dict[str, Any]) -> str:
    return str(finding.get("message") or finding.get("error_type") or "")
