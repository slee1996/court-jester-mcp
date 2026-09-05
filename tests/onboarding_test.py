import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts.check_onboarding import run


class OnboardingContractTests(unittest.TestCase):
    def transcript(self, fault=None):
        def command(argv, **kwargs):
            root = Path(kwargs["cwd"])
            language = argv[argv.index("--language") + 1]
            source = "target.py" if language == "python" else "target.ts"
            tests = "checks.py" if language == "python" else "checks.test.ts"
            self.assertEqual(argv[argv.index("--file") + 1], source)
            self.assertNotIn("--test-file", argv)
            self.assertEqual(json.loads((root / ".court-jester.json").read_text())["targets"],
                             [{"source": source, "test_files": [tests]}])
            self.assertTrue((root / tests).is_file())
            if language == "typescript":
                self.assertEqual(json.loads((root / "package.json").read_text())["type"], "module")
            marker = root / "entrypoint-ran"
            status = 0
            if "--show-config" in argv:
                value = {"execution_started": False, "limits": {"memory_mb": 256}}
            elif "--probe-entrypoint" not in argv:
                value = {"verdict": "pass", "checks": []}
                if fault == "default_execution":
                    marker.touch()
            else:
                dependency = root / ("dependency.py" if language == "python" else "dependency.ts")
                ready = dependency.exists() and "return value" in dependency.read_text()
                loaded = dependency.exists()
                if loaded and not (fault == "failing_test_not_executed" and not ready):
                    marker.touch()
                if fault == "missing_dependency_passes" and not dependency.exists():
                    ready = True
                status = 0 if ready else 1
                value = {"verdict": "pass" if ready else "fail", "checks": [{"name": "entrypoint_probe", "status": "passed" if ready else "failed"}]}
                value["checks"][0]["detail"] = {"test_stage": {"detail": {"target_module_loaded": loaded and fault != "missing_load_evidence"}}}
                if fault == "wrong_exit" and ready:
                    status = 1
            return subprocess.CompletedProcess(argv, status, json.dumps(value), "")
        return command

    def test_failing_phase_requires_a_fresh_execution_marker(self):
        for language in ("python", "typescript"):
            with self.subTest(language=language):
                result = self.evaluate("failing_test_not_executed", language)
                self.assertEqual(len(result["phases"]), 5)
                self.assertEqual(result["status"], "failed")
                self.assertIn("did not load and execute", result["error"])

    def evaluate(self, fault=None, language="python"):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"fixture binary")
            with patch("scripts.check_onboarding.subprocess.run", side_effect=self.transcript(fault)):
                return run(binary, language)

    def test_complete_transcript_is_accepted_and_bound_to_binary(self):
        for language in ("python", "typescript"):
            with self.subTest(language=language):
                result = self.evaluate(language=language)
                self.assertEqual(result["status"], "passed")
                self.assertEqual(result["suite"], f"{language}-onboarding-v1")
                self.assertEqual(len(result["phases"]), 5)
                self.assertTrue(result["binary_unchanged"])
                self.assertEqual(len(result["binary_sha256"]), 64)

    def test_false_readiness_and_unrequested_execution_are_rejected(self):
        for language in ("python", "typescript"):
            for fault in ["default_execution", "missing_dependency_passes", "wrong_exit", "missing_load_evidence", "failing_test_not_executed"]:
                with self.subTest(fault=fault, language=language):
                    self.assertEqual(self.evaluate(fault, language)["status"], "failed")

    def test_timeout_is_failed_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"fixture binary")
            with patch("scripts.check_onboarding.subprocess.run", side_effect=subprocess.TimeoutExpired("doctor", 30)):
                self.assertEqual(run(binary)["status"], "failed")


if __name__ == "__main__":
    unittest.main()
