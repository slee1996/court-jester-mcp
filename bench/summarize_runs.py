from __future__ import annotations

import argparse
import hashlib
import json
import math
import random
from collections import defaultdict
from pathlib import Path
from typing import Any, Iterable

from bench.runner import report_verdict


BENCH_ARTIFACT_SCHEMA_VERSION = 1
VERIFY_SCHEMA_VERSION_REQUIRED = 3
_OUTCOMES = {"pass", "fail", "abstain"}
_SHADOW_PRECEDENCE = {"revert": 6, "followup_fix": 5, "hidden_failure": 4, "public_failure": 3, "success": 2, "unknown": 1}

def _number(value: object) -> float | None:
    try:
        if isinstance(value, bool) or value is None:
            return None
        value = float(value)
        return value if math.isfinite(value) else None
    except (TypeError, ValueError):
        return None

def _nearest_rank(values: Iterable[float], percentile: float) -> float | None:
    ordered = sorted(float(value) for value in values)
    if not ordered:
        return None
    return ordered[max(1, math.ceil(percentile * len(ordered))) - 1]

def _load_artifacts(results_dir: Path, allow_legacy: bool = False) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    matrix: dict[str, Any] = {}
    matrix_path = results_dir / "matrix.json"
    if matrix_path.exists():
        try:
            loaded = json.loads(matrix_path.read_text())
            if isinstance(loaded, dict):
                matrix = loaded
        except (OSError, json.JSONDecodeError):
            matrix = {"validation_error": "invalid_matrix_json"}
    if matrix_path.exists() and (matrix.get("artifact_schema_version") != BENCH_ARTIFACT_SCHEMA_VERSION or matrix.get("verify_schema_version_required") != VERIFY_SCHEMA_VERSION_REQUIRED):
        matrix["_schema_invalid"] = True
    paths = sorted(results_dir.glob("*/result.json"))
    if (results_dir / "result.json").exists():
        paths = [results_dir / "result.json"] + paths
    if not matrix_path.exists():
        invalid: list[dict[str, str]] = [{"path": "matrix.json", "reason": "missing_matrix"}]
    elif matrix.get("_schema_invalid") or matrix.get("validation_error"):
        invalid = [{"path": "matrix.json", "reason": "missing_or_mismatched_schema"}]
    else:
        invalid = []
    rows: list[dict[str, Any]] = []
    for path in paths:
        try:
            value = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            invalid.append({"path": str(path.relative_to(results_dir)), "reason": "invalid_json"})
            continue
        if not isinstance(value, dict):
            invalid.append({"path": str(path.relative_to(results_dir)), "reason": "result_not_object"})
            continue
        reasons: list[str] = []
        if value.get("artifact_schema_version") != BENCH_ARTIFACT_SCHEMA_VERSION:
            reasons.append("missing_or_mismatched_artifact_schema")
        if value.get("verify_schema_version_required") != VERIFY_SCHEMA_VERSION_REQUIRED:
            reasons.append("missing_or_mismatched_verify_schema")
        if reasons and not allow_legacy:
            invalid.append({"path": str(path.relative_to(results_dir)), "reason": ";".join(reasons)})
            continue
        if reasons:
            value = dict(value)
            value["legacy_artifact"] = True
            value["legacy_reasons"] = reasons
        value.setdefault("_artifact_path", str(path.relative_to(results_dir)))
        rows.append(value)
    expected = matrix.get("expected_total", matrix.get("expected_cells"))
    expected_total = int(expected) if isinstance(expected, int) and expected >= 0 else len(paths)
    return rows, {"artifact_schema_version": BENCH_ARTIFACT_SCHEMA_VERSION, "verify_schema_version_required": VERIFY_SCHEMA_VERSION_REQUIRED, "expected_total": expected_total, "present_total": len(rows), "invalid_artifacts": invalid, "legacy_included": bool(allow_legacy and any(row.get("legacy_artifact") for row in rows)), "matrix": matrix}

def _report_observation(row: dict[str, Any]) -> str | None:
    """Return the aggregate verdict represented by the row's retained reports."""
    court_jester = row.get("court_jester")
    if isinstance(court_jester, dict) and "results" in court_jester:
        results = court_jester.get("results")
        if not isinstance(results, list) or not results:
            return None
        verdicts: list[str] = []
        for item in results:
            if not isinstance(item, dict) or item.get("tool_error") is not None:
                return None
            verdict = report_verdict(item.get("response"))
            if verdict is None:
                return None
            verdicts.append(verdict)
    else:
        verdict = report_verdict(row.get("verify_report"))
        if verdict is None:
            return None
        verdicts = [verdict]

    if "fail" in verdicts:
        return "fail"
    if "inconclusive" in verdicts:
        return "abstain"
    return "pass"


def _stored_observation(row: dict[str, Any]) -> tuple[str, str] | None:
    value = row.get("verifier_observation")
    if not isinstance(value, dict):
        return None
    outcome = value.get("outcome")
    reason = value.get("reason")
    if not isinstance(outcome, str) or outcome not in _OUTCOMES:
        return None
    if not isinstance(reason, str) or not reason:
        return None
    for field in ("failure_stage", "failure_path"):
        if value.get(field) is not None and not isinstance(value.get(field), str):
            return None
    report_schema_version = value.get("report_schema_version")
    if report_schema_version is not None and (
        isinstance(report_schema_version, bool) or not isinstance(report_schema_version, int)
    ):
        return None
    if outcome == "abstain":
        return outcome, reason
    if report_schema_version != VERIFY_SCHEMA_VERSION_REQUIRED:
        return None
    if _report_observation(row) != outcome:
        return None
    return outcome, reason


def _observation(row: dict[str, Any]) -> tuple[str, str]:
    status = str(row.get("status") or "")
    if row.get("dry_run") is True or status == "dry_run":
        return "abstain", "dry_run"

    for terminal_category in (row.get("failure_category"), status):
        if not isinstance(terminal_category, str):
            continue
        if terminal_category.startswith("provider_") or terminal_category in {
            "setup_error",
            "gold_patch_apply_error",
        }:
            return "abstain", terminal_category

    stored = _stored_observation(row)
    if stored is not None:
        return stored

    verdict = report_verdict(row.get("verify_report"))
    if verdict == "inconclusive":
        return "abstain", "verify_inconclusive"
    if verdict in {"pass", "fail"}:
        return verdict, f"verify_{verdict}ed"
    if row.get("legacy_artifact") is True and row.get("verify_failed") is True:
        return "fail", "legacy_verify_failed"
    return "abstain", ("timeout" if "timeout" in status else str(row.get("failure_reason") or status or "missing_verifier_observation"))

def _expected_label(row: dict[str, Any]) -> str | None:
    metadata = row.get("task_metadata")
    candidates: list[object] = [metadata.get("expected_verify_outcome") if isinstance(metadata, dict) else None, row.get("expected_verify_outcome"), row.get("expected_outcome"), row.get("gold_outcome"), row.get("label"), metadata.get("label") if isinstance(metadata, dict) else None]
    for value in candidates:
        normalized = str(value).strip().lower() if value is not None else ""
        if normalized in {"pass", "fail"}:
            return normalized
    return None

def _confusion(rows: list[dict[str, Any]]) -> dict[str, Any]:
    tp = fn = tn = fp = abstentions = labeled = 0
    reasons: defaultdict[str, int] = defaultdict(int)
    for row in rows:
        observed, reason = _observation(row)
        expected = _expected_label(row)
        if expected not in {"pass", "fail"}:
            continue
        labeled += 1
        if observed == "abstain":
            abstentions += 1
            reasons[reason or "abstain"] += 1
        elif expected == "fail" and observed == "fail": tp += 1
        elif expected == "fail": fn += 1
        elif observed == "pass": tn += 1
        else: fp += 1
    def ratio(numerator: int, denominator: int) -> float | None:
        return optional_ratio(numerator, denominator)
    recall = ratio(tp, tp + fn)
    specificity = ratio(tn, tn + fp)
    return {"tp": tp, "fn": fn, "tn": tn, "fp": fp, "labeled": labeled, "abstentions": abstentions, "abstention_rate": ratio(abstentions, labeled), "reason_counts": dict(sorted(reasons.items())), "precision": ratio(tp, tp + fp), "recall": recall, "specificity": specificity, "npv": ratio(tn, tn + fn), "balanced_accuracy": optional_ratio((recall or 0.0) + (specificity or 0.0), 2 if recall is not None and specificity is not None else 0), "f1": ratio(2 * tp, 2 * tp + fp + fn), "fpr": ratio(fp, fp + tn), "fnr": ratio(fn, fn + tp)}

def _success_value(row: dict[str, Any]) -> bool | None:
    value = row.get("success")
    return value if isinstance(value, bool) else None

def _pair_rows(rows: list[dict[str, Any]], baseline_policy: str, candidate_policy: str, bootstrap_samples: int) -> dict[str, Any]:
    def key(row: dict[str, Any]) -> tuple[str, str, int]:
        try: repeat = int(row.get("repeat_index", row.get("repeat_ordinal", 0)))
        except (TypeError, ValueError): repeat = 0
        return str(row.get("task_id", "")), str(row.get("model_id", "")), repeat
    baseline: dict[tuple[str, str, int], dict[str, Any]] = {}
    candidate: dict[tuple[str, str, int], dict[str, Any]] = {}
    for row in rows:
        if row.get("policy_id") == baseline_policy: baseline[key(row)] = row
        elif row.get("policy_id") == candidate_policy: candidate[key(row)] = row
    both_success = candidate_only = baseline_only = both_fail = eligible = unmatched = ineligible = 0
    differences: list[float] = []
    for pair_key in sorted(set(baseline) | set(candidate)):
        left, right = baseline.get(pair_key), candidate.get(pair_key)
        if left is None or right is None:
            unmatched += 1
            continue
        if not isinstance(left.get("hidden_seed_sha256"), str) or left.get("hidden_seed_sha256") != right.get("hidden_seed_sha256"):
            ineligible += 1
            continue
        if _observation(left)[0] == "abstain" or _observation(right)[0] == "abstain":
            ineligible += 1
            continue
        base_success, cand_success = _success_value(left), _success_value(right)
        if base_success is None or cand_success is None:
            ineligible += 1
            continue
        eligible += 1
        differences.append(float(cand_success) - float(base_success))
        if base_success and cand_success: both_success += 1
        elif cand_success: candidate_only += 1
        elif base_success: baseline_only += 1
        else: both_fail += 1
    discordant_n = candidate_only + baseline_only
    if discordant_n:
        probs = [math.comb(discordant_n, i) / (2 ** discordant_n) for i in range(discordant_n + 1)]
        p_value = min(1.0, 2 * min(sum(probs[:min(candidate_only, baseline_only) + 1]), sum(probs[max(candidate_only, baseline_only):])))
    else:
        p_value = None
    point = optional_ratio(candidate_only - baseline_only, eligible)
    lower = upper = None
    samples = max(0, int(bootstrap_samples))
    if differences and samples:
        rng = random.Random(0)
        estimates = [sum(differences[rng.randrange(len(differences))] for _ in differences) / len(differences) for _ in range(samples)]
        lower, upper = _nearest_rank(estimates, 0.025), _nearest_rank(estimates, 0.975)
    return {"baseline_policy": baseline_policy, "candidate_policy": candidate_policy, "both_success": both_success, "candidate_only": candidate_only, "baseline_only": baseline_only, "both_fail": both_fail, "eligible": eligible, "unmatched": unmatched, "ineligible": ineligible, "discordant": discordant_n, "paired_lift": point, "mcnemar_p_value": p_value, "mcnemar_exact_p_value": p_value, "bootstrap_samples": samples, "bootstrap_lower": lower, "bootstrap_upper": upper, "bootstrap_lower_bound": lower, "bootstrap_upper_bound": upper, "bootstrap_ci": {"lower": lower, "upper": upper}}

def _duration_values(rows: list[dict[str, Any]], names: tuple[str, ...]) -> list[float]:
    values: list[float] = []
    for row in rows:
        timings = row.get("timings") if isinstance(row.get("timings"), dict) else {}
        value = next((row.get(name, timings.get(name)) for name in names if row.get(name, timings.get(name)) is not None), None)
        number = _number(value)
        if number is not None: values.append(number)
    return values

def _slo(rows: list[dict[str, Any]], expected_total: int) -> dict[str, Any]:
    statuses = [str(row.get("status", "")).lower() for row in rows]
    reasons = [_observation(row)[1].lower() for row in rows]
    provider_errors = sum(1 for status, reason in zip(statuses, reasons) if ("provider" in status and ("error" in status or "fail" in status)) or "provider_error" in reason)
    timeouts = sum(1 for status, reason in zip(statuses, reasons) if "timeout" in status or "timeout" in reason)
    setup = sum(1 for status, reason in zip(statuses, reasons) if (any(token in status for token in ("setup", "gold_patch", "workspace")) and "fail" in status) or "setup" in reason or "gold_patch" in reason)
    abstentions = sum(1 for row in rows if _observation(row)[0] == "abstain")
    schema = sum(1 for row, reason in zip(rows, reasons) if row.get("legacy_artifact") or row.get("verify_schema_mismatch") or "schema" in reason)
    denominator = expected_total if expected_total > 0 else 0
    end_to_end = _duration_values(rows, ("end_to_end_ms", "duration_ms"))
    verify = _duration_values(rows, ("verify_duration_ms", "court_jester_total_ms", "verify_ms"))
    return {"expected_total": expected_total, "present_total": len(rows), "completion_rate": optional_ratio(len(rows), expected_total), "provider_error_rate": optional_ratio(provider_errors, denominator), "timeout_rate": optional_ratio(timeouts, denominator), "setup_gold_patch_rate": optional_ratio(setup, denominator), "abstention_rate": optional_ratio(abstentions, denominator), "schema_mismatch_rate": optional_ratio(schema, denominator), "provider_errors": provider_errors, "timeouts": timeouts, "setup_gold_patch_failures": setup, "abstentions": abstentions, "schema_mismatches": schema, "verify_duration_ms": {"p50": _nearest_rank(verify, .50), "p95": _nearest_rank(verify, .95)}, "end_to_end_duration_ms": {"p50": _nearest_rank(end_to_end, .50), "p95": _nearest_rank(end_to_end, .95)}}

def _read_jsonl(path: str | Path | None) -> list[dict[str, Any]]:
    if not path or not Path(path).exists(): return []
    rows: list[dict[str, Any]] = []
    for line in Path(path).read_text().splitlines():
        try: value = json.loads(line)
        except json.JSONDecodeError: continue
        if isinstance(value, dict): rows.append(value)
    return rows

def resolve_shadow_outcomes(records: list[dict[str, Any]], outcomes: list[dict[str, Any]]) -> dict[str, Any]:
    resolved: dict[str, dict[str, Any]] = {}
    for outcome in outcomes:
        key, value = str(outcome.get("key", "")), str(outcome.get("outcome", "unknown"))
        if not key or value not in _SHADOW_PRECEDENCE: continue
        previous = resolved.get(key)
        rank = (_SHADOW_PRECEDENCE[value], str(outcome.get("timestamp", "")))
        old_rank = (_SHADOW_PRECEDENCE.get(str(previous.get("outcome")), 0), str(previous.get("timestamp", ""))) if previous else (-1, "")
        if rank > old_rank: resolved[key] = outcome
    counts: defaultdict[str, int] = defaultdict(int)
    for record in records: counts[str(resolved.get(str(record.get("key", "")), {}).get("outcome", "unresolved"))] += 1
    return {"records": len(records), "resolved": len(records) - counts.get("unresolved", 0), "unresolved": counts.get("unresolved", 0), "outcome_counts": dict(sorted(counts.items())), "unresolved_rate": optional_ratio(counts.get("unresolved", 0), len(records))}

def evaluate_gate(summary: dict[str, Any], policy: str = "none", known_good_summaries: list[dict[str, Any]] | None = None) -> dict[str, Any]:
    if summary.get("dry_run"):
        return {
            "policy": policy,
            "eligible": False,
            "passed": False,
            "failures": ["dry_run_input"],
            "metrics": summary.get("slo", {}),
        }
    if policy == "none":
        return {"policy": policy, "eligible": True, "passed": True, "failures": [], "metrics": summary.get("slo", {})}
    failures: list[str] = []
    slo = summary.get("slo", {}) if isinstance(summary.get("slo"), dict) else {}
    paired = summary.get("paired", {}) if isinstance(summary.get("paired"), dict) else {}
    validation = summary.get("validation", {}) if isinstance(summary.get("validation"), dict) else {}
    if summary.get("dry_run"): failures.append("dry_run_input")
    if validation.get("legacy_included"): failures.append("legacy_artifacts")
    if validation.get("invalid_artifacts"): failures.append("invalid_artifacts")
    if paired.get("eligible", 0) <= 0: failures.append("missing_pair_cells")
    if paired.get("unmatched", 0) or paired.get("ineligible", 0): failures.append("incomplete_pair_cells")
    requirements = {"completion_rate": ("min", .98), "provider_error_rate": ("max", .03), "timeout_rate": ("max", .02), "setup_gold_patch_rate": ("eq", 0.0), "abstention_rate": ("max", .02), "schema_mismatch_rate": ("eq", 0.0)}
    for name, (operator, threshold) in requirements.items():
        value = slo.get(name)
        if value is None:
            failures.append(f"null_{name}")
        elif (operator == "min" and value < threshold) or (operator == "max" and value > threshold) or (operator == "eq" and value != threshold):
            failures.append(name)
    if not known_good_summaries:
        failures.append("missing_known_good_summary")
    for known_good in known_good_summaries or []:
        confusion = known_good.get("confusion", {}) if isinstance(known_good, dict) else {}
        metrics = known_good.get("metrics", {}) if isinstance(known_good, dict) else {}
        if not confusion and isinstance(metrics, dict):
            confusion = metrics.get("confusion", {})
        if confusion.get("fp", known_good.get("false_positives") if isinstance(known_good, dict) else None) != 0:
            failures.append("known_good_false_positives")
    if paired.get("paired_lift") is None or paired.get("paired_lift") <= 0: failures.append("paired_lift")
    if policy == "strict-heldout" and (paired.get("bootstrap_lower") is None or paired.get("bootstrap_lower") <= 0): failures.append("bootstrap_lower_bound")
    failures = sorted(set(failures))
    return {"policy": policy, "eligible": not failures, "passed": not failures, "failures": failures, "metrics": {**slo, "paired_lift": paired.get("paired_lift"), "bootstrap_lower": paired.get("bootstrap_lower")}}

def _build_summary(results_dir: str | Path, baseline_policy: str, candidate_policy: str, bootstrap_samples: int, *, allow_legacy: bool = False, shadow_records: str | Path | None = None, shadow_outcomes: str | Path | None = None) -> dict[str, Any]:
    rows, metadata = _load_artifacts(Path(results_dir), allow_legacy)
    if metadata["invalid_artifacts"] and not allow_legacy:
        reasons = ", ".join(f"{item['path']}: {item['reason']}" for item in metadata["invalid_artifacts"])
        raise ValueError(f"artifact validation failed: {reasons}")
    gate_rows = [row for row in rows if not row.get("legacy_artifact")]
    grouped_rows: defaultdict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in rows: grouped_rows[(str(row.get("model_id", "unknown")), str(row.get("policy_id", "unknown")))].append(row)
    confusion = _confusion(gate_rows)
    slo = _slo(gate_rows, int(metadata["expected_total"]))
    summary: dict[str, Any] = {"artifact_schema_version": BENCH_ARTIFACT_SCHEMA_VERSION, "verify_schema_version_required": VERIFY_SCHEMA_VERSION_REQUIRED, "rows": len(rows), "validation": metadata, "confusion": confusion, "slo": slo, "paired": _pair_rows(gate_rows, baseline_policy, candidate_policy, bootstrap_samples), "groups": {f"{model}/{policy}": summarize_items(values) for (model, policy), values in sorted(grouped_rows.items())}, "dry_run": bool(rows) and all(bool(row.get("dry_run")) for row in rows)}
    summary["shadow"] = resolve_shadow_outcomes(_read_jsonl(shadow_records), _read_jsonl(shadow_outcomes)) if shadow_records or shadow_outcomes else {"records": 0, "resolved": 0, "unresolved": 0, "outcome_counts": {}, "unresolved_rate": None}
    summary["gate"] = evaluate_gate(summary, "none")
    summary["metrics"] = {"confusion": confusion, "slo": slo, "paired": summary["paired"], **{name: value for name, value in slo.items() if name.endswith("rate")}}
    return summary
def build_summary(results_dir: str | Path, baseline_policy: str, candidate_policy: str, bootstrap_samples: int) -> dict[str, Any]:
    return _build_summary(results_dir, baseline_policy, candidate_policy, bootstrap_samples)

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Summarize benchmark runs.")
    parser.add_argument("results_dir")
    parser.add_argument("--summary-json")
    parser.add_argument("--baseline-policy", default="baseline")
    parser.add_argument("--candidate-policy", default="repair-loop-verify-only")
    parser.add_argument("--bootstrap-samples", type=int, default=10000)
    parser.add_argument("--allow-legacy-artifacts", action="store_true")
    parser.add_argument("--known-good-summary", action="append", default=[])
    parser.add_argument("--gate-policy", choices=["none", "private-beta-default", "strict-heldout"], default="none")
    parser.add_argument("--fail-on-gate", action="store_true")
    parser.add_argument("--shadow-records")
    parser.add_argument("--shadow-outcomes")
    return parser.parse_args()

def main() -> int:
    args = parse_args()
    for configured in (args.shadow_records, args.shadow_outcomes):
        if configured and not Path(configured).parent.exists(): raise SystemExit(f"configured shadow path parent does not exist: {Path(configured).parent}")
    summary = _build_summary(args.results_dir, args.baseline_policy, args.candidate_policy, args.bootstrap_samples, allow_legacy=args.allow_legacy_artifacts, shadow_records=args.shadow_records, shadow_outcomes=args.shadow_outcomes)
    known_good: list[dict[str, Any]] = []
    for path in args.known_good_summary:
        try:
            loaded = json.loads(Path(path).read_text())
            if isinstance(loaded, dict): known_good.append(loaded)
        except (OSError, json.JSONDecodeError): known_good.append({"invalid": path})
    summary["gate"] = evaluate_gate(summary, args.gate_policy, known_good)
    rendered = json.dumps(summary, sort_keys=True, indent=2)
    if args.summary_json: Path(args.summary_json).write_text(rendered + "\n")
    print(rendered)
    return 1 if args.fail_on_gate and (not summary["gate"]["eligible"] or not summary["gate"]["passed"]) else 0


def summarize_grouped(
    grouped: dict[tuple[Any, ...], list[dict[str, object]]]
) -> dict[tuple[Any, ...], dict[str, object]]:
    return {key: summarize_items(items) for key, items in grouped.items()}


def summarize_items(items: list[dict[str, object]]) -> dict[str, object]:
    total = len(items)
    successes = sum(1 for item in items if item.get("success"))
    hidden_passes = sum(1 for item in items if item.get("hidden_checks_pass"))
    verify_failures = sum(
        1
        for item in items
        if item.get("court_jester", {}).get("verify_failed")
    )
    verify_failed_runs = sum(1 for item in items if item.get("verify_failed"))
    public_failed_runs = sum(1 for item in items if item.get("public_failed"))
    hidden_failed_runs = sum(1 for item in items if item.get("hidden_failed"))
    repair_attempts = sum(1 for item in items if item.get("repair_attempted"))
    verify_triggered_repairs = sum(1 for item in items if "verify" in repair_sources_for_item(item))
    repaired_after_verify_failure = sum(
        1 for item in items if item.get("repaired_after_verify_failure")
    )
    repaired_after_public_failure = sum(
        1 for item in items if item.get("repaired_after_public_failure")
    )
    avg_attempts = (
        sum(int(item.get("attempt_count", 1)) for item in items) / total
        if total
        else 0.0
    )
    total_setup_ms = sum(timing_ms(item, "setup_ms") for item in items)
    total_end_to_end_ms = sum(float(item.get("timings", {}).get("end_to_end_ms", 0)) for item in items)
    avg_end_to_end_ms = (total_end_to_end_ms / total) if total else 0.0
    total_product_loop_ms = sum(product_loop_ms_for_item(item) for item in items)
    avg_product_loop_ms = (total_product_loop_ms / total) if total else 0.0
    total_hidden_eval_ms = sum(benchmark_scoring_ms_for_item(item) for item in items)
    avg_hidden_eval_ms = (total_hidden_eval_ms / total) if total else 0.0
    avg_setup_ms = (total_setup_ms / total) if total else 0.0
    total_harness_overhead_ms = sum(harness_overhead_ms_for_item(item) for item in items)
    avg_harness_overhead_ms = (total_harness_overhead_ms / total) if total else 0.0
    total_agent_trace_setup_ms = sum(timing_ms(item, "agent_trace_setup_ms") for item in items)
    avg_agent_trace_setup_ms = (total_agent_trace_setup_ms / total) if total else 0.0
    total_agent_trace_summary_ms = sum(timing_ms(item, "agent_trace_summary_ms") for item in items)
    avg_agent_trace_summary_ms = (total_agent_trace_summary_ms / total) if total else 0.0
    total_agent_trace_event_count = sum(timing_ms(item, "agent_trace_event_count") for item in items)
    avg_agent_trace_event_count = (total_agent_trace_event_count / total) if total else 0.0
    total_agent_trace_overhead_estimate_ms = sum(
        timing_ms(item, "agent_trace_overhead_estimate_ms") for item in items
    )
    avg_agent_trace_overhead_estimate_ms = (
        total_agent_trace_overhead_estimate_ms / total if total else 0.0
    )
    total_court_jester_ms = sum(
        float(item.get("timings", {}).get("court_jester_total_ms", 0)) for item in items
    )
    avg_court_jester_ms = (total_court_jester_ms / total) if total else 0.0
    avg_verify_calls = (
        sum(float(item.get("tool_usage", {}).get("verify_calls", 0)) for item in items) / total
        if total
        else 0.0
    )
    success_rate = (successes / total) if total else 0.0
    repeats_observed = max((int(item.get("repeat_ordinal", 1)) for item in items), default=0)
    total_end_to_end_hours = optional_ratio(total_end_to_end_ms, 3_600_000.0)
    successes_per_hour = optional_ratio(successes, total_end_to_end_hours)
    minutes_per_success = optional_ratio(total_end_to_end_ms / 60_000.0, successes)
    total_product_loop_hours = optional_ratio(total_product_loop_ms, 3_600_000.0)
    product_successes_per_hour = optional_ratio(successes, total_product_loop_hours)
    product_minutes_per_success = optional_ratio(total_product_loop_ms / 60_000.0, successes)
    verify_recovery_rate = optional_ratio(repaired_after_verify_failure, verify_triggered_repairs)
    (
        verify_expectation_items,
        expected_verify_passes,
        expected_verify_fails,
        verify_true_positives,
        verify_false_negatives,
        verify_true_negatives,
        verify_false_positives,
        verify_failure_kind_expectations,
        verify_failure_kind_hits,
    ) = summarize_verify_expectations(items)
    verify_outcome_accuracy = optional_ratio(
        verify_true_positives + verify_true_negatives,
        verify_expectation_items,
    )
    verify_recall = optional_ratio(verify_true_positives, expected_verify_fails)
    verify_specificity = optional_ratio(verify_true_negatives, expected_verify_passes)
    verify_precision = optional_ratio(
        verify_true_positives,
        verify_true_positives + verify_false_positives,
    )
    verify_failure_kind_hit_rate = optional_ratio(
        verify_failure_kind_hits,
        verify_failure_kind_expectations,
    )
    failure_counts = defaultdict(int)
    repair_trigger_counts = defaultdict(int)
    repair_feedback_style_counts = defaultdict(int)
    for item in items:
        failure_counts[item.get("failure_category", "unknown")] += 1
        repair_source = item.get("repair_trigger_source")
        if repair_source:
            repair_trigger_counts[str(repair_source)] += 1
        repair_feedback_style = item.get("repair_feedback_style")
        if repair_feedback_style:
            repair_feedback_style_counts[str(repair_feedback_style)] += 1
    serialized_counts = json.dumps(dict(sorted(failure_counts.items())), sort_keys=True)
    serialized_repair_triggers = json.dumps(dict(sorted(repair_trigger_counts.items())), sort_keys=True)
    serialized_feedback_styles = json.dumps(dict(sorted(repair_feedback_style_counts.items())), sort_keys=True)
    return {
        "total": total,
        "successes": successes,
        "success_rate": success_rate,
        "hidden_passes": hidden_passes,
        "verify_failures": verify_failures,
        "verify_failed_runs": verify_failed_runs,
        "public_failed_runs": public_failed_runs,
        "hidden_failed_runs": hidden_failed_runs,
        "repair_attempts": repair_attempts,
        "verify_triggered_repairs": verify_triggered_repairs,
        "repaired_after_verify_failure": repaired_after_verify_failure,
        "repaired_after_public_failure": repaired_after_public_failure,
        "verify_recovery_rate": verify_recovery_rate,
        "verify_expectation_items": verify_expectation_items,
        "expected_verify_passes": expected_verify_passes,
        "expected_verify_fails": expected_verify_fails,
        "verify_true_positives": verify_true_positives,
        "verify_false_negatives": verify_false_negatives,
        "verify_true_negatives": verify_true_negatives,
        "verify_false_positives": verify_false_positives,
        "verify_outcome_accuracy": verify_outcome_accuracy,
        "verify_recall": verify_recall,
        "verify_specificity": verify_specificity,
        "verify_precision": verify_precision,
        "verify_failure_kind_expectations": verify_failure_kind_expectations,
        "verify_failure_kind_hits": verify_failure_kind_hits,
        "verify_failure_kind_hit_rate": verify_failure_kind_hit_rate,
        "avg_attempts": avg_attempts,
        "total_end_to_end_ms": total_end_to_end_ms,
        "avg_end_to_end_ms": avg_end_to_end_ms,
        "total_end_to_end_hours": total_end_to_end_hours,
        "successes_per_hour": successes_per_hour,
        "minutes_per_success": minutes_per_success,
        "total_product_loop_ms": total_product_loop_ms,
        "avg_product_loop_ms": avg_product_loop_ms,
        "total_product_loop_hours": total_product_loop_hours,
        "product_successes_per_hour": product_successes_per_hour,
        "product_minutes_per_success": product_minutes_per_success,
        "total_hidden_eval_ms": total_hidden_eval_ms,
        "avg_hidden_eval_ms": avg_hidden_eval_ms,
        "total_setup_ms": total_setup_ms,
        "avg_setup_ms": avg_setup_ms,
        "total_harness_overhead_ms": total_harness_overhead_ms,
        "avg_harness_overhead_ms": avg_harness_overhead_ms,
        "total_agent_trace_setup_ms": total_agent_trace_setup_ms,
        "avg_agent_trace_setup_ms": avg_agent_trace_setup_ms,
        "total_agent_trace_summary_ms": total_agent_trace_summary_ms,
        "avg_agent_trace_summary_ms": avg_agent_trace_summary_ms,
        "total_agent_trace_event_count": total_agent_trace_event_count,
        "avg_agent_trace_event_count": avg_agent_trace_event_count,
        "total_agent_trace_overhead_estimate_ms": total_agent_trace_overhead_estimate_ms,
        "avg_agent_trace_overhead_estimate_ms": avg_agent_trace_overhead_estimate_ms,
        "avg_court_jester_ms": avg_court_jester_ms,
        "avg_verify_calls": avg_verify_calls,
        "repeats_observed": repeats_observed,
        "repair_trigger_sources": serialized_repair_triggers,
        "repair_feedback_styles": serialized_feedback_styles,
        "failure_categories": serialized_counts,
    }


def iter_lift_rows(
    summaries: dict[tuple[Any, ...], dict[str, object]]
) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for key, summary in sorted(summaries.items()):
        if not key:
            continue
        policy_id = str(key[-1])
        if policy_id == "baseline":
            continue
        baseline_key = (*key[:-1], "baseline")
        baseline = summaries.get(baseline_key)
        if baseline is None:
            continue
        labels = [str(part) for part in key[:-1]]
        rows.append(
            {
                "label_1": labels[0] if labels else "",
                "label_2": labels[1] if len(labels) > 1 else "",
                "policy_id": policy_id,
                "total": summary["total"],
                "successes": summary["successes"],
                "baseline_successes": baseline["successes"],
                "additional_successes_vs_baseline": int(summary["successes"]) - int(baseline["successes"]),
                "success_rate": summary["success_rate"],
                "baseline_success_rate": baseline["success_rate"],
                "success_rate_lift": float(summary["success_rate"]) - float(baseline["success_rate"]),
                "total_end_to_end_hours": summary["total_end_to_end_hours"],
                "baseline_end_to_end_hours": baseline["total_end_to_end_hours"],
                "extra_end_to_end_hours_vs_baseline": optional_difference(
                    summary["total_end_to_end_hours"],
                    baseline["total_end_to_end_hours"],
                ),
                "successes_per_hour": summary["successes_per_hour"],
                "baseline_successes_per_hour": baseline["successes_per_hour"],
                "successes_per_hour_lift": optional_difference(
                    summary["successes_per_hour"],
                    baseline["successes_per_hour"],
                ),
                "marginal_minutes_per_saved_task": optional_ratio(
                    (
                        float(summary["total_end_to_end_ms"])
                        - float(baseline["total_end_to_end_ms"])
                    )
                    / 60_000.0,
                    int(summary["successes"]) - int(baseline["successes"]),
                ),
                "total_product_loop_hours": summary["total_product_loop_hours"],
                "baseline_product_loop_hours": baseline["total_product_loop_hours"],
                "extra_product_loop_hours_vs_baseline": optional_difference(
                    summary["total_product_loop_hours"],
                    baseline["total_product_loop_hours"],
                ),
                "product_successes_per_hour": summary["product_successes_per_hour"],
                "baseline_product_successes_per_hour": baseline["product_successes_per_hour"],
                "product_successes_per_hour_lift": optional_difference(
                    summary["product_successes_per_hour"],
                    baseline["product_successes_per_hour"],
                ),
                "marginal_product_minutes_per_saved_task": optional_ratio(
                    (
                        float(summary["total_product_loop_ms"])
                        - float(baseline["total_product_loop_ms"])
                    )
                    / 60_000.0,
                    int(summary["successes"]) - int(baseline["successes"]),
                ),
            }
        )
    return rows


def repair_sources_for_item(item: dict[str, object]) -> list[str]:
    sources = item.get("repair_trigger_sources")
    if isinstance(sources, list):
        return [str(source) for source in sources if source]
    source = item.get("repair_trigger_source")
    if source:
        return [str(source)]
    return []


def summarize_verify_expectations(
    items: list[dict[str, object]],
) -> tuple[int, int, int, int, int, int, int, int, int]:
    verify_expectation_items = 0
    expected_verify_passes = 0
    expected_verify_fails = 0
    verify_true_positives = 0
    verify_false_negatives = 0
    verify_true_negatives = 0
    verify_false_positives = 0
    verify_failure_kind_expectations = 0
    verify_failure_kind_hits = 0

    for item in items:
        expected = expected_verify_outcome_for_item(item)
        if expected is None:
            continue
        verify_expectation_items += 1
        actual_failed = bool(item.get("verify_failed"))
        if expected == "fail":
            expected_verify_fails += 1
            if actual_failed:
                verify_true_positives += 1
            else:
                verify_false_negatives += 1
        else:
            expected_verify_passes += 1
            if actual_failed:
                verify_false_positives += 1
            else:
                verify_true_negatives += 1

        expected_failure_kinds = expected_verify_failure_kinds_for_item(item)
        if expected == "fail" and expected_failure_kinds:
            verify_failure_kind_expectations += 1
            if actual_failed and verify_failure_kind_matched(item, expected_failure_kinds):
                verify_failure_kind_hits += 1

    return (
        verify_expectation_items,
        expected_verify_passes,
        expected_verify_fails,
        verify_true_positives,
        verify_false_negatives,
        verify_true_negatives,
        verify_false_positives,
        verify_failure_kind_expectations,
        verify_failure_kind_hits,
    )


def expected_verify_outcome_for_item(item: dict[str, object]) -> str | None:
    metadata = item.get("task_metadata")
    if not isinstance(metadata, dict):
        return None
    value = metadata.get("expected_verify_outcome")
    if not isinstance(value, str):
        return None
    normalized = value.strip().lower()
    if normalized in {"pass", "fail"}:
        return normalized
    return None


def expected_verify_failure_kinds_for_item(item: dict[str, object]) -> list[str]:
    metadata = item.get("task_metadata")
    if not isinstance(metadata, dict):
        return []
    value = metadata.get("expected_verify_failure_kinds")
    if not isinstance(value, list):
        return []
    return [str(kind) for kind in value if str(kind).strip()]


def verify_failure_kind_matched(item: dict[str, object], expected_failure_kinds: list[str]) -> bool:
    failure_stage = None
    failure_details = item.get("failure_details")
    if isinstance(failure_details, dict):
        raw_stage = failure_details.get("verify_failure_stage")
        if isinstance(raw_stage, str):
            failure_stage = raw_stage

    failed_stage_counts: dict[str, object] = {}
    verify_summary = item.get("verify_summary")
    if isinstance(verify_summary, dict):
        raw_counts = verify_summary.get("failed_stage_counts")
        if isinstance(raw_counts, dict):
            failed_stage_counts = raw_counts

    observed = {str(name) for name in failed_stage_counts.keys()}
    if failure_stage:
        observed.add(failure_stage)
    return any(kind in observed for kind in expected_failure_kinds)


def optional_ratio(numerator: float, denominator: float) -> float | None:
    if not denominator:
        return None
    return numerator / denominator


def optional_difference(value: float | None, baseline: float | None) -> float | None:
    if value is None or baseline is None:
        return None
    return value - baseline


def timing_ms(item: dict[str, object], key: str) -> float:
    timings = item.get("timings")
    if not isinstance(timings, dict):
        return 0.0
    value = timings.get(key, 0)
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


def product_loop_ms_for_item(item: dict[str, object]) -> float:
    timings = item.get("timings")
    if isinstance(timings, dict) and "product_loop_ms" in timings:
        return timing_ms(item, "product_loop_ms")
    return (
        timing_ms(item, "provider_apply_ms")
        + timing_ms(item, "court_jester_total_ms")
        + timing_ms(item, "public_checks_ms")
    )


def benchmark_scoring_ms_for_item(item: dict[str, object]) -> float:
    timings = item.get("timings")
    if isinstance(timings, dict) and "benchmark_scoring_ms" in timings:
        return timing_ms(item, "benchmark_scoring_ms")
    return timing_ms(item, "hidden_checks_ms")


def harness_overhead_ms_for_item(item: dict[str, object]) -> float:
    timings = item.get("timings")
    if isinstance(timings, dict) and "harness_overhead_ms" in timings:
        return timing_ms(item, "harness_overhead_ms")
    end_to_end_ms = timing_ms(item, "end_to_end_ms")
    captured_ms = (
        timing_ms(item, "setup_ms")
        + timing_ms(item, "provider_apply_ms")
        + timing_ms(item, "provider_retry_backoff_ms")
        + timing_ms(item, "court_jester_total_ms")
        + timing_ms(item, "public_checks_ms")
        + timing_ms(item, "hidden_checks_ms")
    )
    return max(0.0, end_to_end_ms - captured_ms)


def format_metric(value: float | None) -> str:
    if value is None:
        return "NA"
    return f"{value:.2f}"


def policy_role(policy_id: str) -> str:
    if policy_id == "required-final":
        return "control"
    if policy_id.startswith("repair-loop"):
        return "primary"
    if policy_id == "baseline":
        return "baseline"
    return "comparison"


if __name__ == "__main__":
    raise SystemExit(main())
