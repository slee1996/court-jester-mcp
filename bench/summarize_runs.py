from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path


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

    print("headline_repair_loop_success")
    print(
        "model_id,policy_id,policy_role,total,successes,success_rate,repaired_after_verify_failure,"
        "repaired_after_public_failure,verify_failed_runs,public_failed_runs,hidden_failed_runs,"
        "repair_trigger_sources"
    )
    for (model_id, policy_id), items in sorted(grouped.items()):
        if not policy_id.startswith("repair-loop"):
            continue
        summary = summarize_items(items)
        print(
            f"{model_id},{policy_id},{policy_role(policy_id)},{summary['total']},{summary['successes']},"
            f"{summary['success_rate']:.2f},{summary['repaired_after_verify_failure']},"
            f"{summary['repaired_after_public_failure']},{summary['verify_failed_runs']},"
            f"{summary['public_failed_runs']},{summary['hidden_failed_runs']},"
            f"{summary['repair_trigger_sources']}"
        )

    print()
    print(
        "model_id,policy_id,policy_role,total,successes,success_rate,hidden_passes,verify_failures,"
        "verify_failed_runs,public_failed_runs,hidden_failed_runs,repair_attempts,"
        "repaired_after_verify_failure,repaired_after_public_failure,avg_attempts,avg_end_to_end_ms,"
        "avg_court_jester_ms,avg_verify_calls,repeats_observed,repair_trigger_sources,failure_categories"
    )
    for (model_id, policy_id), items in sorted(grouped.items()):
        summary = summarize_items(items)
        print(
            f"{model_id},{policy_id},{policy_role(policy_id)},{summary['total']},{summary['successes']},"
            f"{summary['success_rate']:.2f},"
            f"{summary['hidden_passes']},{summary['verify_failures']},"
            f"{summary['verify_failed_runs']},{summary['public_failed_runs']},{summary['hidden_failed_runs']},"
            f"{summary['repair_attempts']},{summary['repaired_after_verify_failure']},"
            f"{summary['repaired_after_public_failure']},{summary['avg_attempts']:.2f},"
            f"{summary['avg_end_to_end_ms']:.2f},{summary['avg_court_jester_ms']:.2f},{summary['avg_verify_calls']:.2f},"
            f"{summary['repeats_observed']},{summary['repair_trigger_sources']},{summary['failure_categories']}"
        )

    print()
    print(
        "bucket,model_id,policy_id,policy_role,total,successes,success_rate,hidden_passes,verify_failures,"
        "verify_failed_runs,public_failed_runs,hidden_failed_runs,repair_attempts,"
        "repaired_after_verify_failure,repaired_after_public_failure,avg_attempts,avg_end_to_end_ms,"
        "avg_court_jester_ms,avg_verify_calls,repeats_observed,repair_trigger_sources,failure_categories"
    )
    for (bucket, model_id, policy_id), items in sorted(bucket_grouped.items()):
        summary = summarize_items(items)
        print(
            f"{bucket},{model_id},{policy_id},{policy_role(policy_id)},{summary['total']},{summary['successes']},"
            f"{summary['success_rate']:.2f},"
            f"{summary['hidden_passes']},{summary['verify_failures']},"
            f"{summary['verify_failed_runs']},{summary['public_failed_runs']},{summary['hidden_failed_runs']},"
            f"{summary['repair_attempts']},{summary['repaired_after_verify_failure']},"
            f"{summary['repaired_after_public_failure']},{summary['avg_attempts']:.2f},"
            f"{summary['avg_end_to_end_ms']:.2f},{summary['avg_court_jester_ms']:.2f},{summary['avg_verify_calls']:.2f},"
            f"{summary['repeats_observed']},{summary['repair_trigger_sources']},{summary['failure_categories']}"
        )

    print()
    print(
        "bug_class,model_id,policy_id,policy_role,total,successes,success_rate,hidden_passes,verify_failures,"
        "verify_failed_runs,public_failed_runs,hidden_failed_runs,repair_attempts,"
        "repaired_after_verify_failure,repaired_after_public_failure,avg_attempts,avg_end_to_end_ms,"
        "avg_court_jester_ms,avg_verify_calls,repeats_observed,repair_trigger_sources,failure_categories"
    )
    for (bug_class, model_id, policy_id), items in sorted(bug_class_grouped.items()):
        summary = summarize_items(items)
        print(
            f"{bug_class},{model_id},{policy_id},{policy_role(policy_id)},{summary['total']},{summary['successes']},"
            f"{summary['success_rate']:.2f},{summary['hidden_passes']},{summary['verify_failures']},"
            f"{summary['verify_failed_runs']},{summary['public_failed_runs']},{summary['hidden_failed_runs']},"
            f"{summary['repair_attempts']},{summary['repaired_after_verify_failure']},"
            f"{summary['repaired_after_public_failure']},{summary['avg_attempts']:.2f},"
            f"{summary['avg_end_to_end_ms']:.2f},{summary['avg_court_jester_ms']:.2f},{summary['avg_verify_calls']:.2f},"
            f"{summary['repeats_observed']},{summary['repair_trigger_sources']},{summary['failure_categories']}"
        )

    print()
    print(
        "task_id,model_id,policy_id,policy_role,total,successes,success_rate,hidden_passes,verify_failures,"
        "verify_failed_runs,public_failed_runs,hidden_failed_runs,repair_attempts,"
        "repaired_after_verify_failure,repaired_after_public_failure,avg_attempts,avg_end_to_end_ms,"
        "avg_court_jester_ms,avg_verify_calls,repeats_observed,repair_trigger_sources,failure_categories"
    )
    for (task_id, model_id, policy_id), items in sorted(task_grouped.items()):
        summary = summarize_items(items)
        print(
            f"{task_id},{model_id},{policy_id},{policy_role(policy_id)},{summary['total']},{summary['successes']},"
            f"{summary['success_rate']:.2f},{summary['hidden_passes']},{summary['verify_failures']},"
            f"{summary['verify_failed_runs']},{summary['public_failed_runs']},{summary['hidden_failed_runs']},"
            f"{summary['repair_attempts']},{summary['repaired_after_verify_failure']},"
            f"{summary['repaired_after_public_failure']},{summary['avg_attempts']:.2f},"
            f"{summary['avg_end_to_end_ms']:.2f},{summary['avg_court_jester_ms']:.2f},{summary['avg_verify_calls']:.2f},"
            f"{summary['repeats_observed']},{summary['repair_trigger_sources']},{summary['failure_categories']}"
        )
    return 0


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
    avg_end_to_end_ms = (
        sum(float(item.get("timings", {}).get("end_to_end_ms", 0)) for item in items) / total
        if total
        else 0.0
    )
    avg_court_jester_ms = (
        sum(float(item.get("timings", {}).get("court_jester_total_ms", 0)) for item in items) / total
        if total
        else 0.0
    )
    avg_verify_calls = (
        sum(float(item.get("tool_usage", {}).get("verify_calls", 0)) for item in items) / total
        if total
        else 0.0
    )
    success_rate = (successes / total) if total else 0.0
    repeats_observed = max((int(item.get("repeat_ordinal", 1)) for item in items), default=0)
    failure_counts = defaultdict(int)
    repair_trigger_counts = defaultdict(int)
    for item in items:
        failure_counts[item.get("failure_category", "unknown")] += 1
        repair_source = item.get("repair_trigger_source")
        if repair_source:
            repair_trigger_counts[str(repair_source)] += 1
    serialized_counts = json.dumps(dict(sorted(failure_counts.items())), sort_keys=True)
    serialized_repair_triggers = json.dumps(dict(sorted(repair_trigger_counts.items())), sort_keys=True)
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
        "repaired_after_verify_failure": repaired_after_verify_failure,
        "repaired_after_public_failure": repaired_after_public_failure,
        "avg_attempts": avg_attempts,
        "avg_end_to_end_ms": avg_end_to_end_ms,
        "avg_court_jester_ms": avg_court_jester_ms,
        "avg_verify_calls": avg_verify_calls,
        "repeats_observed": repeats_observed,
        "repair_trigger_sources": serialized_repair_triggers,
        "failure_categories": serialized_counts,
    }


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
