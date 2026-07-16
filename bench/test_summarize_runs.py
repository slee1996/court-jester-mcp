import json
import tempfile
import unittest
from pathlib import Path

from bench.summarize_runs import build_summary, evaluate_gate, iter_lift_rows, resolve_shadow_outcomes, summarize_items


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


    def test_build_summary_excludes_operational_abstentions_from_seeded_pairs(self) -> None:
        def artifact(**values: object) -> dict[str, object]:
            return {
                "artifact_schema_version": 1,
                "verify_schema_version_required": 3,
                **values,
            }

        def report(verdict: str, status: str) -> dict[str, object]:
            return {
                "schema_version": 3,
                "verdict": verdict,
                "strength": "property_checked",
                "summary": {},
                "stages": [{"name": "execute", "status": status, "duration_ms": 1}],
            }

        rows = [
            artifact(
                task_id="semantic",
                model_id="model",
                policy_id="baseline",
                repeat_index=0,
                hidden_seed_sha256="semantic-seed",
                success=False,
                verify_report=report("fail", "failed"),
                task_metadata={"expected_verify_outcome": "fail"},
            ),
            artifact(
                task_id="semantic",
                model_id="model",
                policy_id="candidate",
                repeat_index=0,
                hidden_seed_sha256="semantic-seed",
                success=True,
                verify_report=report("pass", "passed"),
                task_metadata={"expected_verify_outcome": "fail"},
            ),
        ]
        operational_cases = [
            ("provider", {"failure_category": "provider_timeout"}),
            ("setup", {"failure_category": "setup_error"}),
            ("gold", {"failure_category": "gold_patch_apply_error"}),
            (
                "verifier-timeout",
                {
                    "verifier_observation": {
                        "outcome": "abstain",
                        "reason": "verify_tool_timeout",
                        "failure_stage": None,
                        "failure_path": "app.py",
                        "report_schema_version": None,
                    }
                },
            ),
        ]
        for task_id, terminal_fields in operational_cases:
            rows.extend(
                [
                    artifact(
                        task_id=task_id,
                        model_id="model",
                        policy_id="baseline",
                        repeat_index=0,
                        hidden_seed_sha256=f"{task_id}-seed",
                        success=False,
                        task_metadata={"expected_verify_outcome": "pass"},
                        **terminal_fields,
                    ),
                    artifact(
                        task_id=task_id,
                        model_id="model",
                        policy_id="candidate",
                        repeat_index=0,
                        hidden_seed_sha256=f"{task_id}-seed",
                        success=True,
                        verify_report=report("pass", "passed"),
                        task_metadata={"expected_verify_outcome": "pass"},
                    ),
                ]
            )
        rows.extend(
            [
                artifact(
                    task_id="seed-mismatch",
                    model_id="model",
                    policy_id="baseline",
                    repeat_index=0,
                    hidden_seed_sha256="base-seed",
                    success=False,
                    verify_report=report("fail", "failed"),
                    task_metadata={"expected_verify_outcome": "fail"},
                ),
                artifact(
                    task_id="seed-mismatch",
                    model_id="model",
                    policy_id="candidate",
                    repeat_index=0,
                    hidden_seed_sha256="candidate-seed",
                    success=True,
                    verify_report=report("pass", "passed"),
                    task_metadata={"expected_verify_outcome": "fail"},
                ),
            ]
        )

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "matrix.json").write_text(
                json.dumps(artifact(expected_total=len(rows))), encoding="utf-8"
            )
            for index, row in enumerate(rows):
                cell = root / f"cell-{index}"
                cell.mkdir()
                (cell / "result.json").write_text(json.dumps(row), encoding="utf-8")
            summary = build_summary(root, "baseline", "candidate", bootstrap_samples=100)

        self.assertEqual(summary["artifact_schema_version"], 1)
        self.assertEqual(summary["verify_schema_version_required"], 3)
        self.assertEqual(summary["confusion"]["abstentions"], 4)
        self.assertEqual(
            summary["confusion"]["reason_counts"],
            {
                "gold_patch_apply_error": 1,
                "provider_timeout": 1,
                "setup_error": 1,
                "verify_tool_timeout": 1,
            },
        )
        self.assertEqual(summary["paired"]["candidate_only"], 1)
        self.assertEqual(summary["paired"]["both_success"], 0)
        self.assertEqual(summary["paired"]["ineligible"], 5)
        self.assertEqual(summary["paired"]["eligible"], 1)
        self.assertEqual(summary["paired"]["paired_lift"], 1.0)

    def test_current_artifact_rows_without_valid_reports_abstain(self) -> None:
        def artifact(**values: object) -> dict[str, object]:
            return {
                "artifact_schema_version": 1,
                "verify_schema_version_required": 3,
                "task_metadata": {"expected_verify_outcome": "pass"},
                **values,
            }

        malformed_report = {
            "schema_version": 3,
            "verdict": "pass",
            "strength": "property_checked",
            "summary": {},
            "stages": [{}],
        }
        rows = [
            artifact(task_id="missing-report"),
            artifact(task_id="malformed-report", verify_report=malformed_report),
            artifact(task_id="bare-verdict", verify_verdict="pass"),
            artifact(task_id="bare-failed-flag", verify_failed=False),
        ]

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "matrix.json").write_text(
                json.dumps(artifact(expected_total=len(rows))), encoding="utf-8"
            )
            for index, row in enumerate(rows):
                cell = root / f"cell-{index}"
                cell.mkdir()
                (cell / "result.json").write_text(json.dumps(row), encoding="utf-8")
            summary = build_summary(root, "baseline", "candidate", bootstrap_samples=0)

        self.assertEqual(summary["confusion"]["labeled"], 4)
        self.assertEqual(summary["confusion"]["abstentions"], 4)
        self.assertEqual(summary["confusion"]["tn"], 0)
        self.assertEqual(summary["confusion"]["fp"], 0)
        self.assertEqual(
            summary["confusion"]["reason_counts"],
            {"missing_verifier_observation": 4},
        )

    def test_build_summary_fail_dominates_inconclusive_reports(self) -> None:
        def report(verdict: str, status: str) -> dict[str, object]:
            return {
                "schema_version": 3,
                "verdict": verdict,
                "strength": "property_checked",
                "summary": {},
                "stages": [{"name": "execute", "status": status, "duration_ms": 1}],
            }

        artifact = {
            "artifact_schema_version": 1,
            "verify_schema_version_required": 3,
            "task_id": "mixed-verdicts",
            "model_id": "model",
            "policy_id": "baseline",
            "task_metadata": {"expected_verify_outcome": "fail"},
            "court_jester": {
                "results": [
                    {
                        "path": "uncertain.py",
                        "response": report("inconclusive", "inconclusive"),
                    },
                    {"path": "broken.py", "response": report("fail", "failed")},
                ]
            },
            "verifier_observation": {
                "outcome": "fail",
                "reason": "stage_failure",
                "failure_stage": "execute",
                "failure_path": "broken.py",
                "report_schema_version": 3,
            },
        }

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "matrix.json").write_text(
                json.dumps(
                    {
                        "artifact_schema_version": 1,
                        "verify_schema_version_required": 3,
                        "expected_total": 1,
                    }
                ),
                encoding="utf-8",
            )
            cell = root / "cell"
            cell.mkdir()
            (cell / "result.json").write_text(json.dumps(artifact), encoding="utf-8")
            summary = build_summary(root, "baseline", "candidate", bootstrap_samples=0)

        self.assertEqual(summary["confusion"]["tp"], 1)
        self.assertEqual(summary["confusion"]["abstentions"], 0)

    def test_build_summary_requires_matrix_unless_legacy_escape_is_explicit(self) -> None:
        from bench.summarize_runs import _build_summary

        artifact = {
            "artifact_schema_version": 1,
            "verify_schema_version_required": 3,
            "task_id": "task",
            "model_id": "model",
            "policy_id": "baseline",
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cell = root / "cell"
            cell.mkdir()
            (cell / "result.json").write_text(json.dumps(artifact), encoding="utf-8")

            with self.assertRaisesRegex(
                ValueError, r"artifact validation failed: matrix\.json: missing_matrix"
            ):
                build_summary(root, "baseline", "candidate", bootstrap_samples=0)

            legacy_summary = _build_summary(
                root,
                "baseline",
                "candidate",
                bootstrap_samples=0,
                allow_legacy=True,
            )

        self.assertEqual(legacy_summary["rows"], 1)
        self.assertEqual(
            legacy_summary["validation"]["invalid_artifacts"],
            [{"path": "matrix.json", "reason": "missing_matrix"}],
        )

    def test_shadow_outcomes_use_precedence_then_newest_timestamp(self) -> None:
        records = [{"key": "k1"}, {"key": "k2"}, {"key": "k3"}]
        outcomes = [
            {"key": "k1", "outcome": "success", "timestamp": "2026-01-01T00:00:00Z"},
            {"key": "k1", "outcome": "public_failure", "timestamp": "2026-01-01T00:01:00Z"},
            {"key": "k1", "outcome": "revert", "timestamp": "2026-01-01T00:02:00Z"},
            {"key": "k2", "outcome": "success", "timestamp": "2026-01-02T00:00:00Z"},
            {"key": "k2", "outcome": "success", "timestamp": "2026-01-03T00:00:00Z"},
        ]
        resolved = resolve_shadow_outcomes(records, outcomes)
        self.assertEqual(resolved["resolved"], 2)
        self.assertEqual(resolved["unresolved"], 1)
        self.assertEqual(resolved["outcome_counts"], {"revert": 1, "success": 1, "unresolved": 1})

    def test_gate_rejects_dry_run_and_requires_known_good_false_positive_free_summary(self) -> None:
        dry = evaluate_gate({"dry_run": True, "slo": {}}, "private-beta-default", [])
        self.assertFalse(dry["eligible"])
        self.assertIn("dry_run_input", dry["failures"])
        summary = {
            "slo": {
                "completion_rate": 1.0,
                "provider_error_rate": 0.0,
                "timeout_rate": 0.0,
                "setup_gold_patch_rate": 0.0,
                "abstention_rate": 0.0,
                "schema_mismatch_rate": 0.0,
            },
            "paired": {"eligible": 1, "unmatched": 0, "ineligible": 0, "paired_lift": 1.0, "bootstrap_lower": 0.1},
            "validation": {},
        }
        blocked = evaluate_gate(summary, "private-beta-default", [{"confusion": {"fp": 1}}])
        self.assertFalse(blocked["passed"])
        self.assertIn("known_good_false_positives", blocked["failures"])

if __name__ == "__main__":
    unittest.main()
