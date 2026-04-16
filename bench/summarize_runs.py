from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path
from typing import Any


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Summarize benchmark runs.")
    parser.add_argument("results_dir", help="Directory containing run result.json files.")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    results_dir = Path(args.results_dir)
    rows = []
    for path in sorted(results_dir.glob("*/result.json")):
        rows.append(json.loads(path.read_text()))

    grouped: dict[tuple[str, str], list[dict[str, object]]] = defaultdict(list)
    bucket_grouped: dict[tuple[str, str, str], list[dict[str, object]]] = defaultdict(list)
    bug_class_grouped: dict[tuple[str, str, str], list[dict[str, object]]] = defaultdict(list)
    task_grouped: dict[tuple[str, str, str], list[dict[str, object]]] = defaultdict(list)
    for row in rows:
        grouped[(row["model_id"], row["policy_id"])].append(row)
        bucket_grouped[(row.get("bucket", "unknown"), row["model_id"], row["policy_id"])].append(row)
        bug_class = (
            row.get("task_metadata", {}).get("bug_class")
            if isinstance(row.get("task_metadata"), dict)
            else None
        ) or "unknown"
        bug_class_grouped[(str(bug_class), row["model_id"], row["policy_id"])].append(row)
        task_grouped[(row["task_id"], row["model_id"], row["policy_id"])].append(row)

    grouped_summaries = summarize_grouped(grouped)
    bucket_summaries = summarize_grouped(bucket_grouped)
    bug_class_summaries = summarize_grouped(bug_class_grouped)
    task_summaries = summarize_grouped(task_grouped)

    print("headline_repair_loop_success")
    print(
        "model_id,policy_id,policy_role,total,successes,success_rate,repaired_after_verify_failure,"
        "repaired_after_public_failure,verify_failed_runs,public_failed_runs,hidden_failed_runs,"
        "verify_triggered_repairs,verify_recovery_rate,successes_per_hour,minutes_per_success,"
        "product_successes_per_hour,product_minutes_per_success,"
        "repair_trigger_sources"
    )
    for (model_id, policy_id), summary in sorted(grouped_summaries.items()):
        if not policy_id.startswith("repair-loop"):
            continue
        print(
            f"{model_id},{policy_id},{policy_role(policy_id)},{summary['total']},{summary['successes']},"
            f"{summary['success_rate']:.2f},{summary['repaired_after_verify_failure']},"
            f"{summary['repaired_after_public_failure']},{summary['verify_failed_runs']},"
            f"{summary['public_failed_runs']},{summary['hidden_failed_runs']},"
            f"{summary['verify_triggered_repairs']},{format_metric(summary['verify_recovery_rate'])},"
            f"{format_metric(summary['successes_per_hour'])},{format_metric(summary['minutes_per_success'])},"
            f"{format_metric(summary['product_successes_per_hour'])},"
            f"{format_metric(summary['product_minutes_per_success'])},"
            f"{summary['repair_trigger_sources']}"
        )

    print()
    print(
        "model_id,policy_id,policy_role,total,successes,success_rate,hidden_passes,verify_failures,"
        "verify_failed_runs,public_failed_runs,hidden_failed_runs,repair_attempts,verify_triggered_repairs,"
        "repaired_after_verify_failure,repaired_after_public_failure,verify_recovery_rate,avg_attempts,"
        "avg_end_to_end_ms,total_end_to_end_hours,successes_per_hour,minutes_per_success,"
        "avg_product_loop_ms,total_product_loop_hours,product_successes_per_hour,product_minutes_per_success,"
        "avg_hidden_eval_ms,avg_setup_ms,avg_harness_overhead_ms,avg_agent_trace_setup_ms,"
        "avg_agent_trace_summary_ms,avg_agent_trace_event_count,avg_agent_trace_overhead_estimate_ms,"
        "avg_court_jester_ms,"
        "avg_verify_calls,repeats_observed,repair_trigger_sources,repair_feedback_styles,failure_categories"
    )
    for (model_id, policy_id), summary in sorted(grouped_summaries.items()):
        print(
            f"{model_id},{policy_id},{policy_role(policy_id)},{summary['total']},{summary['successes']},"
            f"{summary['success_rate']:.2f},"
            f"{summary['hidden_passes']},{summary['verify_failures']},"
            f"{summary['verify_failed_runs']},{summary['public_failed_runs']},{summary['hidden_failed_runs']},"
            f"{summary['repair_attempts']},{summary['verify_triggered_repairs']},"
            f"{summary['repaired_after_verify_failure']},{summary['repaired_after_public_failure']},"
            f"{format_metric(summary['verify_recovery_rate'])},{summary['avg_attempts']:.2f},"
            f"{summary['avg_end_to_end_ms']:.2f},{format_metric(summary['total_end_to_end_hours'])},"
            f"{format_metric(summary['successes_per_hour'])},{format_metric(summary['minutes_per_success'])},"
            f"{summary['avg_product_loop_ms']:.2f},{format_metric(summary['total_product_loop_hours'])},"
            f"{format_metric(summary['product_successes_per_hour'])},"
            f"{format_metric(summary['product_minutes_per_success'])},"
            f"{summary['avg_hidden_eval_ms']:.2f},{summary['avg_setup_ms']:.2f},"
            f"{summary['avg_harness_overhead_ms']:.2f},{summary['avg_agent_trace_setup_ms']:.2f},"
            f"{summary['avg_agent_trace_summary_ms']:.2f},{summary['avg_agent_trace_event_count']:.2f},"
            f"{summary['avg_agent_trace_overhead_estimate_ms']:.2f},"
            f"{summary['avg_court_jester_ms']:.2f},{summary['avg_verify_calls']:.2f},"
            f"{summary['repeats_observed']},{summary['repair_trigger_sources']},{summary['repair_feedback_styles']},"
            f"{summary['failure_categories']}"
        )

    print()
    print(
        "policy_lift_vs_baseline"
    )
    print(
        "model_id,policy_id,policy_role,total,successes,baseline_successes,additional_successes_vs_baseline,"
        "success_rate,baseline_success_rate,success_rate_lift,total_end_to_end_hours,baseline_end_to_end_hours,"
        "extra_end_to_end_hours_vs_baseline,successes_per_hour,baseline_successes_per_hour,"
        "successes_per_hour_lift,marginal_minutes_per_saved_task,total_product_loop_hours,"
        "baseline_product_loop_hours,extra_product_loop_hours_vs_baseline,product_successes_per_hour,"
        "baseline_product_successes_per_hour,product_successes_per_hour_lift,"
        "marginal_product_minutes_per_saved_task"
    )
    for row in iter_lift_rows(grouped_summaries):
        print(
            f"{row['label_1']},{row['policy_id']},{policy_role(row['policy_id'])},{row['total']},"
            f"{row['successes']},{row['baseline_successes']},{row['additional_successes_vs_baseline']},"
            f"{row['success_rate']:.2f},{row['baseline_success_rate']:.2f},{row['success_rate_lift']:.2f},"
            f"{format_metric(row['total_end_to_end_hours'])},{format_metric(row['baseline_end_to_end_hours'])},"
            f"{format_metric(row['extra_end_to_end_hours_vs_baseline'])},"
            f"{format_metric(row['successes_per_hour'])},{format_metric(row['baseline_successes_per_hour'])},"
            f"{format_metric(row['successes_per_hour_lift'])},{format_metric(row['marginal_minutes_per_saved_task'])},"
            f"{format_metric(row['total_product_loop_hours'])},{format_metric(row['baseline_product_loop_hours'])},"
            f"{format_metric(row['extra_product_loop_hours_vs_baseline'])},"
            f"{format_metric(row['product_successes_per_hour'])},"
            f"{format_metric(row['baseline_product_successes_per_hour'])},"
            f"{format_metric(row['product_successes_per_hour_lift'])},"
            f"{format_metric(row['marginal_product_minutes_per_saved_task'])}"
        )

    print()
    print(
        "bucket,model_id,policy_id,policy_role,total,successes,success_rate,hidden_passes,verify_failures,"
        "verify_failed_runs,public_failed_runs,hidden_failed_runs,repair_attempts,verify_triggered_repairs,"
        "repaired_after_verify_failure,repaired_after_public_failure,verify_recovery_rate,avg_attempts,"
        "avg_end_to_end_ms,total_end_to_end_hours,successes_per_hour,minutes_per_success,"
        "avg_product_loop_ms,total_product_loop_hours,product_successes_per_hour,product_minutes_per_success,"
        "avg_hidden_eval_ms,avg_setup_ms,avg_harness_overhead_ms,avg_agent_trace_setup_ms,"
        "avg_agent_trace_summary_ms,avg_agent_trace_event_count,avg_agent_trace_overhead_estimate_ms,"
        "avg_court_jester_ms,"
        "avg_verify_calls,repeats_observed,repair_trigger_sources,repair_feedback_styles,failure_categories"
    )
    for (bucket, model_id, policy_id), summary in sorted(bucket_summaries.items()):
        print(
            f"{bucket},{model_id},{policy_id},{policy_role(policy_id)},{summary['total']},{summary['successes']},"
            f"{summary['success_rate']:.2f},"
            f"{summary['hidden_passes']},{summary['verify_failures']},"
            f"{summary['verify_failed_runs']},{summary['public_failed_runs']},{summary['hidden_failed_runs']},"
            f"{summary['repair_attempts']},{summary['verify_triggered_repairs']},"
            f"{summary['repaired_after_verify_failure']},{summary['repaired_after_public_failure']},"
            f"{format_metric(summary['verify_recovery_rate'])},{summary['avg_attempts']:.2f},"
            f"{summary['avg_end_to_end_ms']:.2f},{format_metric(summary['total_end_to_end_hours'])},"
            f"{format_metric(summary['successes_per_hour'])},{format_metric(summary['minutes_per_success'])},"
            f"{summary['avg_product_loop_ms']:.2f},{format_metric(summary['total_product_loop_hours'])},"
            f"{format_metric(summary['product_successes_per_hour'])},"
            f"{format_metric(summary['product_minutes_per_success'])},"
            f"{summary['avg_hidden_eval_ms']:.2f},{summary['avg_setup_ms']:.2f},"
            f"{summary['avg_harness_overhead_ms']:.2f},{summary['avg_agent_trace_setup_ms']:.2f},"
            f"{summary['avg_agent_trace_summary_ms']:.2f},{summary['avg_agent_trace_event_count']:.2f},"
            f"{summary['avg_agent_trace_overhead_estimate_ms']:.2f},"
            f"{summary['avg_court_jester_ms']:.2f},{summary['avg_verify_calls']:.2f},"
            f"{summary['repeats_observed']},{summary['repair_trigger_sources']},{summary['repair_feedback_styles']},"
            f"{summary['failure_categories']}"
        )

    print()
    print(
        "bug_class,model_id,policy_id,policy_role,total,successes,success_rate,hidden_passes,verify_failures,"
        "verify_failed_runs,public_failed_runs,hidden_failed_runs,repair_attempts,verify_triggered_repairs,"
        "repaired_after_verify_failure,repaired_after_public_failure,verify_recovery_rate,avg_attempts,"
        "avg_end_to_end_ms,total_end_to_end_hours,successes_per_hour,minutes_per_success,"
        "avg_product_loop_ms,total_product_loop_hours,product_successes_per_hour,product_minutes_per_success,"
        "avg_hidden_eval_ms,avg_setup_ms,avg_harness_overhead_ms,avg_agent_trace_setup_ms,"
        "avg_agent_trace_summary_ms,avg_agent_trace_event_count,avg_agent_trace_overhead_estimate_ms,"
        "avg_court_jester_ms,"
        "avg_verify_calls,repeats_observed,repair_trigger_sources,repair_feedback_styles,failure_categories"
    )
    for (bug_class, model_id, policy_id), summary in sorted(bug_class_summaries.items()):
        print(
            f"{bug_class},{model_id},{policy_id},{policy_role(policy_id)},{summary['total']},{summary['successes']},"
            f"{summary['success_rate']:.2f},{summary['hidden_passes']},{summary['verify_failures']},"
            f"{summary['verify_failed_runs']},{summary['public_failed_runs']},{summary['hidden_failed_runs']},"
            f"{summary['repair_attempts']},{summary['verify_triggered_repairs']},"
            f"{summary['repaired_after_verify_failure']},{summary['repaired_after_public_failure']},"
            f"{format_metric(summary['verify_recovery_rate'])},{summary['avg_attempts']:.2f},"
            f"{summary['avg_end_to_end_ms']:.2f},{format_metric(summary['total_end_to_end_hours'])},"
            f"{format_metric(summary['successes_per_hour'])},{format_metric(summary['minutes_per_success'])},"
            f"{summary['avg_product_loop_ms']:.2f},{format_metric(summary['total_product_loop_hours'])},"
            f"{format_metric(summary['product_successes_per_hour'])},"
            f"{format_metric(summary['product_minutes_per_success'])},"
            f"{summary['avg_hidden_eval_ms']:.2f},{summary['avg_setup_ms']:.2f},"
            f"{summary['avg_harness_overhead_ms']:.2f},{summary['avg_agent_trace_setup_ms']:.2f},"
            f"{summary['avg_agent_trace_summary_ms']:.2f},{summary['avg_agent_trace_event_count']:.2f},"
            f"{summary['avg_agent_trace_overhead_estimate_ms']:.2f},"
            f"{summary['avg_court_jester_ms']:.2f},{summary['avg_verify_calls']:.2f},"
            f"{summary['repeats_observed']},{summary['repair_trigger_sources']},{summary['repair_feedback_styles']},"
            f"{summary['failure_categories']}"
        )

    print()
    print("bug_class_lift_vs_baseline")
    print(
        "bug_class,model_id,policy_id,policy_role,total,successes,baseline_successes,"
        "additional_successes_vs_baseline,success_rate,baseline_success_rate,success_rate_lift,"
        "total_end_to_end_hours,baseline_end_to_end_hours,extra_end_to_end_hours_vs_baseline,"
        "successes_per_hour,baseline_successes_per_hour,successes_per_hour_lift,"
        "marginal_minutes_per_saved_task,total_product_loop_hours,baseline_product_loop_hours,"
        "extra_product_loop_hours_vs_baseline,product_successes_per_hour,"
        "baseline_product_successes_per_hour,product_successes_per_hour_lift,"
        "marginal_product_minutes_per_saved_task"
    )
    for row in iter_lift_rows(bug_class_summaries):
        print(
            f"{row['label_1']},{row['label_2']},{row['policy_id']},{policy_role(row['policy_id'])},{row['total']},"
            f"{row['successes']},{row['baseline_successes']},{row['additional_successes_vs_baseline']},"
            f"{row['success_rate']:.2f},{row['baseline_success_rate']:.2f},{row['success_rate_lift']:.2f},"
            f"{format_metric(row['total_end_to_end_hours'])},{format_metric(row['baseline_end_to_end_hours'])},"
            f"{format_metric(row['extra_end_to_end_hours_vs_baseline'])},"
            f"{format_metric(row['successes_per_hour'])},{format_metric(row['baseline_successes_per_hour'])},"
            f"{format_metric(row['successes_per_hour_lift'])},{format_metric(row['marginal_minutes_per_saved_task'])},"
            f"{format_metric(row['total_product_loop_hours'])},{format_metric(row['baseline_product_loop_hours'])},"
            f"{format_metric(row['extra_product_loop_hours_vs_baseline'])},"
            f"{format_metric(row['product_successes_per_hour'])},"
            f"{format_metric(row['baseline_product_successes_per_hour'])},"
            f"{format_metric(row['product_successes_per_hour_lift'])},"
            f"{format_metric(row['marginal_product_minutes_per_saved_task'])}"
        )

    print()
    print(
        "task_id,model_id,policy_id,policy_role,total,successes,success_rate,hidden_passes,verify_failures,"
        "verify_failed_runs,public_failed_runs,hidden_failed_runs,repair_attempts,verify_triggered_repairs,"
        "repaired_after_verify_failure,repaired_after_public_failure,verify_recovery_rate,avg_attempts,"
        "avg_end_to_end_ms,total_end_to_end_hours,successes_per_hour,minutes_per_success,"
        "avg_product_loop_ms,total_product_loop_hours,product_successes_per_hour,product_minutes_per_success,"
        "avg_hidden_eval_ms,avg_setup_ms,avg_harness_overhead_ms,avg_agent_trace_setup_ms,"
        "avg_agent_trace_summary_ms,avg_agent_trace_event_count,avg_agent_trace_overhead_estimate_ms,"
        "avg_court_jester_ms,"
        "avg_verify_calls,repeats_observed,repair_trigger_sources,repair_feedback_styles,failure_categories"
    )
    for (task_id, model_id, policy_id), summary in sorted(task_summaries.items()):
        print(
            f"{task_id},{model_id},{policy_id},{policy_role(policy_id)},{summary['total']},{summary['successes']},"
            f"{summary['success_rate']:.2f},{summary['hidden_passes']},{summary['verify_failures']},"
            f"{summary['verify_failed_runs']},{summary['public_failed_runs']},{summary['hidden_failed_runs']},"
            f"{summary['repair_attempts']},{summary['verify_triggered_repairs']},"
            f"{summary['repaired_after_verify_failure']},{summary['repaired_after_public_failure']},"
            f"{format_metric(summary['verify_recovery_rate'])},{summary['avg_attempts']:.2f},"
            f"{summary['avg_end_to_end_ms']:.2f},{format_metric(summary['total_end_to_end_hours'])},"
            f"{format_metric(summary['successes_per_hour'])},{format_metric(summary['minutes_per_success'])},"
            f"{summary['avg_product_loop_ms']:.2f},{format_metric(summary['total_product_loop_hours'])},"
            f"{format_metric(summary['product_successes_per_hour'])},"
            f"{format_metric(summary['product_minutes_per_success'])},"
            f"{summary['avg_hidden_eval_ms']:.2f},{summary['avg_setup_ms']:.2f},"
            f"{summary['avg_harness_overhead_ms']:.2f},{summary['avg_agent_trace_setup_ms']:.2f},"
            f"{summary['avg_agent_trace_summary_ms']:.2f},{summary['avg_agent_trace_event_count']:.2f},"
            f"{summary['avg_agent_trace_overhead_estimate_ms']:.2f},"
            f"{summary['avg_court_jester_ms']:.2f},{summary['avg_verify_calls']:.2f},"
            f"{summary['repeats_observed']},{summary['repair_trigger_sources']},{summary['repair_feedback_styles']},"
            f"{summary['failure_categories']}"
        )
    return 0


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
