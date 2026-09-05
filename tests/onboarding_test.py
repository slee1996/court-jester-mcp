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
            marker = root / "entrypoint-ran"
            status = 0
            if "--show-config" in argv:
                value = {"execution_started": False, "limits": {"memory_mb": 256}}
            elif "--probe-entrypoint" not in argv:
                value = {"verdict": "pass", "checks": []}
                if fault == "default_execution":
                    marker.touch()
            else:
                dependency = root / "dependency.py"
                ready = dependency.exists() and "return value" in dependency.read_text()
                if ready:
                    marker.touch()
                if fault == "missing_dependency_passes" and not dependency.exists():
                    ready = True
                status = 0 if ready else 1
                value = {"verdict": "pass" if ready else "fail", "checks": [{"name": "entrypoint_probe", "status": "passed" if ready else "failed"}]}
                value["checks"][0]["detail"] = {"test_stage": {"detail": {"target_module_loaded": ready and fault != "missing_load_evidence"}}}
                if fault == "wrong_exit" and ready:
                    status = 1
            return subprocess.CompletedProcess(argv, status, json.dumps(value), "")
        return command

    def evaluate(self, fault=None):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"fixture binary")
            with patch("scripts.check_onboarding.subprocess.run", side_effect=self.transcript(fault)):
                return run(binary)

    def test_complete_transcript_is_accepted_and_bound_to_binary(self):
        result = self.evaluate()
        self.assertEqual(result["status"], "passed")
        self.assertEqual(len(result["phases"]), 5)
        self.assertTrue(result["binary_unchanged"])
        self.assertEqual(len(result["binary_sha256"]), 64)

    def test_false_readiness_and_unrequested_execution_are_rejected(self):
        for fault in ["default_execution", "missing_dependency_passes", "wrong_exit", "missing_load_evidence"]:
            with self.subTest(fault=fault):
                self.assertEqual(self.evaluate(fault)["status"], "failed")

    def test_timeout_is_failed_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"fixture binary")
            with patch("scripts.check_onboarding.subprocess.run", side_effect=subprocess.TimeoutExpired("doctor", 30)):
                self.assertEqual(run(binary)["status"], "failed")


if __name__ == "__main__":
    unittest.main()
