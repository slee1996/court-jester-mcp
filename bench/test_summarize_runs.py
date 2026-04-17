import unittest

from bench.summarize_runs import iter_lift_rows, summarize_items


class SummarizeRunsTest(unittest.TestCase):
    def test_summarize_items_reports_time_normalized_and_verify_recovery_metrics(self) -> None:
        summary = summarize_items(
            [
                {
                    "success": True,
                    "hidden_checks_pass": True,
                    "court_jester": {"verify_failed": True},
                    "verify_failed": False,
                    "public_failed": False,
                    "hidden_failed": False,
                    "repair_attempted": True,
                    "repair_trigger_source": "verify",
                    "repair_feedback_style": "detailed",
                    "attempt_count": 2,
                    "repaired_after_verify_failure": True,
                    "repaired_after_public_failure": False,
                    "repeat_ordinal": 1,
                    "timings": {
                        "end_to_end_ms": 60_000,
                        "court_jester_total_ms": 80,
                        "product_loop_ms": 40_000,
                        "benchmark_scoring_ms": 5_000,
                        "setup_ms": 10_000,
                        "harness_overhead_ms": 5_000,
                        "agent_trace_setup_ms": 8,
                        "agent_trace_summary_ms": 2,
                        "agent_trace_event_count": 10,
                        "agent_trace_overhead_estimate_ms": 210,
                    },
                    "tool_usage": {"verify_calls": 2},
                },
                {
                    "success": False,
                    "hidden_checks_pass": False,
                    "court_jester": {"verify_failed": False},
                    "verify_failed": False,
                    "public_failed": True,
                    "hidden_failed": False,
                    "repair_attempted": False,
                    "attempt_count": 1,
                    "repaired_after_verify_failure": False,
                    "repaired_after_public_failure": False,
                    "repeat_ordinal": 1,
                    "timings": {
                        "end_to_end_ms": 30_000,
                        "court_jester_total_ms": 0,
                        "product_loop_ms": 15_000,
                        "benchmark_scoring_ms": 2_000,
                        "setup_ms": 2_000,
                        "harness_overhead_ms": 3_000,
                        "agent_trace_setup_ms": 0,
                        "agent_trace_summary_ms": 0,
                        "agent_trace_event_count": 0,
                        "agent_trace_overhead_estimate_ms": 0,
                    },
                    "tool_usage": {"verify_calls": 0},
                    "failure_category": "public_failure",
                },
            ]
        )

        self.assertEqual(summary["verify_triggered_repairs"], 1)
        self.assertAlmostEqual(summary["verify_recovery_rate"], 1.0)
        self.assertAlmostEqual(summary["total_end_to_end_hours"], 0.025)
        self.assertAlmostEqual(summary["successes_per_hour"], 40.0)
        self.assertAlmostEqual(summary["minutes_per_success"], 1.5)
        self.assertAlmostEqual(summary["total_product_loop_hours"], 55_000 / 3_600_000.0)
        self.assertAlmostEqual(summary["product_successes_per_hour"], 3600.0 / 55.0)
        self.assertAlmostEqual(summary["product_minutes_per_success"], 55_000 / 60_000.0)
        self.assertAlmostEqual(summary["avg_hidden_eval_ms"], 3500.0)
        self.assertAlmostEqual(summary["avg_setup_ms"], 6000.0)
        self.assertAlmostEqual(summary["avg_harness_overhead_ms"], 4000.0)
        self.assertAlmostEqual(summary["avg_agent_trace_setup_ms"], 4.0)
        self.assertAlmostEqual(summary["avg_agent_trace_summary_ms"], 1.0)
        self.assertAlmostEqual(summary["avg_agent_trace_event_count"], 5.0)
        self.assertAlmostEqual(summary["avg_agent_trace_overhead_estimate_ms"], 105.0)
        self.assertEqual(summary["repair_feedback_styles"], '{"detailed": 1}')

    def test_summarize_items_reports_verify_expectation_classifier_metrics(self) -> None:
        summary = summarize_items(
            [
                {
                    "verify_failed": True,
                    "task_metadata": {
                        "expected_verify_outcome": "fail",
                        "expected_verify_failure_kinds": ["execute"],
                    },
                    "verify_summary": {"failed_stage_counts": {"execute": 1}},
                    "failure_details": {"verify_failure_stage": "execute"},
                },
                {
                    "verify_failed": False,
                    "task_metadata": {
                        "expected_verify_outcome": "fail",
                        "expected_verify_failure_kinds": ["execute"],
                    },
                    "verify_summary": {"failed_stage_counts": {}},
                    "failure_details": {},
                },
                {
                    "verify_failed": False,
                    "task_metadata": {
                        "expected_verify_outcome": "pass",
                        "expected_verify_failure_kinds": [],
                    },
                    "verify_summary": {"failed_stage_counts": {}},
                    "failure_details": {},
                },
                {
                    "verify_failed": True,
                    "task_metadata": {
                        "expected_verify_outcome": "pass",
                        "expected_verify_failure_kinds": [],
                    },
                    "verify_summary": {"failed_stage_counts": {"test": 1}},
                    "failure_details": {"verify_failure_stage": "test"},
                },
            ]
        )

        self.assertEqual(summary["verify_expectation_items"], 4)
        self.assertEqual(summary["expected_verify_passes"], 2)
        self.assertEqual(summary["expected_verify_fails"], 2)
        self.assertEqual(summary["verify_true_positives"], 1)
        self.assertEqual(summary["verify_false_negatives"], 1)
        self.assertEqual(summary["verify_true_negatives"], 1)
        self.assertEqual(summary["verify_false_positives"], 1)
        self.assertAlmostEqual(summary["verify_outcome_accuracy"], 0.5)
        self.assertAlmostEqual(summary["verify_recall"], 0.5)
        self.assertAlmostEqual(summary["verify_specificity"], 0.5)
        self.assertAlmostEqual(summary["verify_precision"], 0.5)
        self.assertEqual(summary["verify_failure_kind_expectations"], 2)
        self.assertEqual(summary["verify_failure_kind_hits"], 1)
        self.assertAlmostEqual(summary["verify_failure_kind_hit_rate"], 0.5)

    def test_iter_lift_rows_compares_policy_against_baseline(self) -> None:
        rows = iter_lift_rows(
            {
                ("codex-default", "baseline"): {
                    "total": 2,
                    "successes": 1,
                    "success_rate": 0.5,
                    "total_end_to_end_ms": 120_000.0,
                    "total_end_to_end_hours": 120_000.0 / 3_600_000.0,
                    "successes_per_hour": 30.0,
                    "total_product_loop_ms": 90_000.0,
                    "total_product_loop_hours": 90_000.0 / 3_600_000.0,
                    "product_successes_per_hour": 40.0,
                },
                ("codex-default", "repair-loop-verify-only"): {
                    "total": 2,
                    "successes": 2,
                    "success_rate": 1.0,
                    "total_end_to_end_ms": 180_000.0,
                    "total_end_to_end_hours": 180_000.0 / 3_600_000.0,
                    "successes_per_hour": 40.0,
                    "total_product_loop_ms": 120_000.0,
                    "total_product_loop_hours": 120_000.0 / 3_600_000.0,
                    "product_successes_per_hour": 60.0,
                },
            }
        )

        self.assertEqual(len(rows), 1)
        row = rows[0]
        self.assertEqual(row["label_1"], "codex-default")
        self.assertEqual(row["policy_id"], "repair-loop-verify-only")
        self.assertEqual(row["additional_successes_vs_baseline"], 1)
        self.assertAlmostEqual(row["success_rate_lift"], 0.5)
        self.assertAlmostEqual(row["successes_per_hour_lift"], 10.0)
        self.assertAlmostEqual(row["marginal_minutes_per_saved_task"], 1.0)
        self.assertAlmostEqual(row["product_successes_per_hour_lift"], 20.0)
        self.assertAlmostEqual(row["marginal_product_minutes_per_saved_task"], 0.5)


if __name__ == "__main__":
    unittest.main()
