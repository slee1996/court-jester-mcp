import copy
import json
import unittest
from unittest.mock import patch
from pathlib import Path
import tempfile
from types import SimpleNamespace

from bench.test_quality_corpus import MANIFEST, OUTCOMES, VALIDATION_FAULTS, check_case, check_validation, run


def report_for(case):
    expected = case["expected"]
    detail = {"mode": "advisory", "baseline_eligible": True,
              "counts": {"planned": 1, **{outcome: int(outcome == expected["outcome"]) for outcome in OUTCOMES}},
              "mutants": [{"outcome": expected["outcome"], "entered_mutated_surface": expected["entered_mutated_surface"]}],
              "coupling_findings": [{"kind": kind} for kind in expected["coupling"]]}
    status = "passed" if expected["outcome"] == "killed" and not expected["coupling"] else "advisory"
    return {"schema_version": 3, "verdict": "pass", "stages": [{"name": "test_quality", "status": status, "detail": detail}]}


class TestQualityCorpusTests(unittest.TestCase):
    def test_runtime_only_is_incomplete_and_failed_validation_fails_combined_gate(self):
        case = json.loads(MANIFEST.read_text())["cases"][0]
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "verifier"
            binary.write_bytes(b"test binary")
            manifest = Path(directory) / "manifest.json"
            manifest.write_text(json.dumps({"schema_version": 1, "suite": "test", "scope": "test", "cases": [case]}))
            with patch("bench.test_quality_corpus.subprocess.run", return_value=SimpleNamespace(
                    returncode=0, stdout=json.dumps(report_for(case)))):
                result = run(binary, manifest)
                self.assertEqual(result["status"], "passed")
                self.assertEqual(result["validation"]["status"], "not_run")
                self.assertFalse(result["classification_evidence_complete"])
                for status in ("passed", "failed"):
                    with patch("bench.test_quality_corpus.run_validation", return_value={"status": status}):
                        result = run(binary, manifest, binary)
                    self.assertEqual(result["status"], status)
                    self.assertEqual(result["classification_evidence_complete"], status == "passed")
                    self.assertEqual(result["summary"], {"cases": 1, "matched": 1})

    def test_validation_matrix_and_identity_contract(self):
        report = {"artifact_schema_version": 1, "suite": "test-quality-validation-v1",
                  "evidence_kind": "fault_injected_validation_boundary_not_generated_runtime_mutants",
                  "status": "passed", "verifier_binary_sha256": "a" * 64, "validator_binary_sha256": "b" * 64,
                  "validation_source_sha256": "c" * 64, "fixture_source_sha256": "d" * 64,
                  "cases": [{"id": f"{language}-{fault}", "language": language, "fault": fault,
                             "expected": expected, "observed": expected, "matched": True,
                             "classification": "valid" if expected == "valid" else "invalid",
                             "mutant_execution_started": False}
                            for language in ("python", "typescript") for fault, expected in VALIDATION_FAULTS.items()]}
        self.assertEqual(check_validation(report, "a" * 64, "b" * 64), [])
        for mutate in [
            lambda r: r.pop("verifier_binary_sha256"),
            lambda r: r.update(validator_binary_sha256="e" * 64),
            lambda r: r.update(validation_source_sha256="wrong"),
            lambda r: r["cases"].pop(),
            lambda r: r["cases"].append(r["cases"][0]),
            lambda r: r["cases"][1].update(observed="valid"),
            lambda r: r["cases"][1].update(classification="valid"),
            lambda r: r["cases"][0].update(mutant_execution_started=True),
            lambda r: r["cases"][0].update(matched=1),
            lambda r: r.update(cases=[None]),
            lambda r: r.update(artifact_schema_version=True),
        ]:
            bad = copy.deepcopy(report)
            mutate(bad)
            self.assertTrue(check_validation(bad, "a" * 64, "b" * 64))

    def setUp(self):
        self.cases = json.loads(MANIFEST.read_text())["cases"]

    def test_runtime_matrix_covers_both_languages_without_claiming_invalid_campaigns(self):
        expected = {(language, scenario) for language in ("python", "typescript") for scenario in ("killed", "survived", "blocked", "no_coverage", "coupling")}
        actual = {(case["language"], case["id"].split("-", 1)[1]) for case in self.cases}
        self.assertEqual(actual, expected)
        for case in self.cases:
            self.assertEqual(check_case(case, report_for(case), 0), [])

    def test_wrong_counts_unreached_survivors_and_scores_are_rejected(self):
        case = next(case for case in self.cases if case["id"] == "python-survived")
        for mutate in [
            lambda detail: detail["counts"].update(survived=0),
            lambda detail: detail["counts"].update(survived=True),
            lambda detail: detail["mutants"][0].update(entered_mutated_surface=False),
            lambda detail: detail["mutants"][0].update(outcome="killed"),
            lambda detail: detail.update(score=100),
            lambda detail: detail.update(planning_error="unavailable"),
        ]:
            report = report_for(case)
            mutate(report["stages"][0]["detail"])
            self.assertTrue(check_case(case, report, 0))

    def test_coupling_and_clean_baseline_are_required_independently(self):
        case = next(case for case in self.cases if case["id"] == "typescript-coupling")
        good = report_for(case)
        self.assertTrue(check_case(case, good, 1))
        for key, value in [("coupling_findings", []), ("baseline_eligible", False), ("mode", "gating")]:
            report = copy.deepcopy(good)
            report["stages"][0]["detail"][key] = value
            self.assertTrue(check_case(case, report, 0))
        self.assertTrue(check_case(case, {}, 0))
        self.assertTrue(check_case(case, [], 0))


if __name__ == "__main__":
    unittest.main()
