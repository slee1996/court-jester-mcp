import copy
import json
import unittest

from bench.test_quality_corpus import MANIFEST, OUTCOMES, check_case


def report_for(case):
    expected = case["expected"]
    detail = {"mode": "advisory", "baseline_eligible": True,
              "counts": {"planned": 1, **{outcome: int(outcome == expected["outcome"]) for outcome in OUTCOMES}},
              "mutants": [{"outcome": expected["outcome"], "entered_mutated_surface": expected["entered_mutated_surface"]}],
              "coupling_findings": [{"kind": kind} for kind in expected["coupling"]]}
    status = "passed" if expected["outcome"] == "killed" and not expected["coupling"] else "advisory"
    return {"schema_version": 3, "verdict": "pass", "stages": [{"name": "test_quality", "status": status, "detail": detail}]}


class TestQualityCorpusTests(unittest.TestCase):
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
