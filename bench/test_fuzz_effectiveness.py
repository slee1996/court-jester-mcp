import json
import tempfile
import unittest
from pathlib import Path

from bench.fuzz_effectiveness import DEFAULT_MANIFEST, evaluate_case, load_manifest, summarize


class FuzzEffectivenessTest(unittest.TestCase):
    def test_v3_separates_admission_from_observation_without_rewriting_v2(self) -> None:
        legacy = load_manifest(DEFAULT_MANIFEST.with_name("fuzz_effectiveness_cases_v2.json"))
        current = load_manifest(DEFAULT_MANIFEST.with_name("fuzz_effectiveness_cases_v3.json"))
        self.assertEqual(legacy["suite"], "fuzz-effectiveness-v2")
        self.assertEqual(current["suite"], "fuzz-effectiveness-v3")
        self.assertEqual(len(legacy["cases"]), 9)
        self.assertEqual(len(current["cases"]), 13)
        self.assertEqual(sum(case["kind"] == "mutation" for case in legacy["cases"]), 6)
        self.assertEqual(sum(case["kind"] == "observation" for case in current["cases"]), 5)
        for case in current["cases"]:
            if case["kind"] == "observation":
                self.assertEqual(case["expected_verdict"], "inconclusive")
                self.assertEqual(case["expected_finding"]["input_classification"], "unknown")
        for language in ("python", "typescript"):
            pair = [case for case in current["cases"]
                    if case["language"] == language and case["technique"] == "closed_domain_runtime_contract"]
            self.assertEqual({case["kind"] for case in pair}, {"mutation", "control"})

    def test_v4_keeps_differential_fixture_and_preserves_historical_denominator(self) -> None:
        legacy = load_manifest(DEFAULT_MANIFEST.with_name("fuzz_effectiveness_cases_v3.json"))
        current = load_manifest(DEFAULT_MANIFEST)
        self.assertEqual(current["suite"], "fuzz-effectiveness-v4")
        self.assertEqual(len(current["cases"]), 13)
        self.assertEqual(sum(case["kind"] == "mutation" for case in legacy["cases"]), 4)
        self.assertEqual(sum(case["kind"] == "mutation" for case in current["cases"]), 3)
        for before, after in zip(legacy["cases"], current["cases"]):
            if before["id"] != "python-differential-regression":
                self.assertEqual(before, after)
                continue
            self.assertEqual(before["source"], after["source"])
            self.assertEqual(before["base_source"], after["base_source"])
            self.assertEqual(before["kind"], "mutation")
            self.assertEqual(after["kind"], "observation")
            self.assertEqual(after["expected_verdict"], "inconclusive")
            self.assertEqual(after["expected_finding"]["input_classification"], "unknown")

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
