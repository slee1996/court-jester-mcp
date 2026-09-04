import json
import tempfile
import unittest
from pathlib import Path

from bench.fuzz_effectiveness import DEFAULT_MANIFEST, evaluate_case, load_manifest, summarize


class FuzzEffectivenessTest(unittest.TestCase):
    def test_observation_requires_unknown_input_evidence_and_consistent_exit(self) -> None:
        case = {
            "id": "uncertain", "kind": "observation",
            "expected_verdict": "inconclusive",
            "expected_finding": {"function": "read", "category": "exception", "input_classification": "unknown"},
        }
        finding = {"location": {"function": "read"}, "category": "exception", "input_classification": "valid"}
        report = {"verdict": "inconclusive", "_benchmark_exit_code": 3,
                  "stages": [{"detail": {"findings": [finding]}}]}
        self.assertFalse(evaluate_case(case, report)["matched"])
        finding["input_classification"] = "unknown"
        self.assertTrue(evaluate_case(case, report)["matched"])
        for exit_code in (0, 1, 2, -9):
            report["_benchmark_exit_code"] = exit_code
            self.assertFalse(evaluate_case(case, report)["matched"], exit_code)

    def test_manifest_covers_recall_and_specificity(self) -> None:
        manifest = load_manifest(DEFAULT_MANIFEST)
        kinds = {case["kind"] for case in manifest["cases"]}
        techniques = {case["technique"] for case in manifest["cases"]}

        self.assertEqual(kinds, {"mutation", "control", "observation"})
        self.assertGreaterEqual(len(techniques), 6)

    def test_mutation_requires_the_expected_finding(self) -> None:
        case = {
            "id": "predicate",
            "kind": "mutation",
            "technique": "predicate_aware_seed",
            "expected_verdict": "fail",
            "expected_stage": {"name": "execute", "status": "failed"},
            "expected_finding": {"function": "decode_mode", "category": "exception"},
        }
        wrong_report = {
            "verdict": "fail",
            "stages": [
                {
                    "name": "execute",
                    "status": "failed",
                    "detail": {
                        "findings": [
                            {
                                "category": "exception",
                                "location": {"function": "another_function"},
                            }
                        ]
                    },
                }
            ],
        }
        matched_report = {
            **wrong_report,
            "stages": [
                {
                    "name": "execute",
                    "status": "failed",
                    "detail": {
                        "findings": [
                            {
                                "category": "exception",
                                "location": {"function": "decode_mode"},
                            }
                        ]
                    },
                }
            ],
        }

        self.assertFalse(evaluate_case(case, wrong_report)["matched"])
        self.assertTrue(evaluate_case(case, matched_report)["matched"])

    def test_control_fails_specificity_when_any_finding_is_emitted(self) -> None:
        case = {
            "id": "clean-control",
            "kind": "control",
            "technique": "specificity_control",
            "expected_verdict": "pass",
            "expected_stage": {"name": "execute", "status": "passed"},
        }
        report = {
            "verdict": "pass",
            "stages": [
                {
                    "name": "execute",
                    "status": "passed",
                    "detail": {
                        "findings": [
                            {
                                "category": "property",
                                "location": {"function": "clean"},
                            }
                        ]
                    },
                }
            ],
        }

        result = evaluate_case(case, report)

        self.assertFalse(result["matched"])
        self.assertIn("control emitted 1 finding(s)", result["mismatches"])

    def test_manifest_rejects_case_ids_that_escape_artifact_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest_path = Path(temporary) / "cases.json"
            manifest_path.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "cases": [{"id": "../../outside"}],
                    }
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "safe artifact slug"):
                load_manifest(manifest_path)

    def test_summary_reports_mutation_recall_and_specificity(self) -> None:
        results = [
            {"kind": "mutation", "matched": True},
            {"kind": "mutation", "matched": False},
            {"kind": "control", "matched": True},
            {"kind": "observation", "matched": True},
        ]

        summary = summarize(results)

        self.assertEqual(summary["mutation_recall"], 0.5)
        self.assertEqual(summary["specificity"], 1.0)
        self.assertEqual(summary["matched"], 3)
        self.assertEqual(summary["observations"], 1)
        self.assertEqual(summary["observations_matched"], 1)


if __name__ == "__main__":
    unittest.main()
