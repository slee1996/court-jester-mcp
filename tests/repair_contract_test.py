from __future__ import annotations

import importlib.util
import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("repair_contract", ROOT / "scripts/check_repair_loop.py")
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class RepairContractTest(unittest.TestCase):
    def test_fixture_coverage_is_explicit(self):
        self.assertEqual(len(CHECK.CASES), 4)
        self.assertEqual(len({case["id"] for case in CHECK.CASES}), 4)
        self.assertEqual({(case["language"], case["oracle"]) for case in CHECK.CASES}, {
            (language, oracle) for language in ("python", "typescript")
            for oracle in ("runtime_contract", "declared_property")
        })

    def test_wrong_or_missing_positive_check_never_passes(self):
        for positive in (None, False, 1, "true"):
            payload = {"schema_version": 3, "finding_id": "finding", "outcome": "not_reproduced", "check_passed": positive}
            result = subprocess.CompletedProcess([], 1, json.dumps(payload), "")
            with patch.object(CHECK, "command", return_value=result), self.assertRaises(CHECK.ContractFailure):
                CHECK.replay(Path("binary"), ROOT, Path("report"), "finding", "fixed", "not_reproduced", True, [{}])

    def test_unavailable_and_empty_evidence_fail_with_separate_causes(self):
        for payload, kind in [
            ({"schema_version": 3, "verdict": "inconclusive"}, "inconclusive"),
            ({"schema_version": 3, "verdict": "fail", "findings": []}, "contract_mismatch"),
            ({}, "protocol"),
        ]:
            def fake_command(*args, **kwargs):
                args[3].append({"phase": args[2]})
                return subprocess.CompletedProcess([], 1, json.dumps(payload), "")
            with patch.object(CHECK, "command", side_effect=fake_command):
                result = CHECK.check_case(Path("binary"), CHECK.CASES[0])
            self.assertEqual(result["status"], "failed")
            self.assertEqual(result["failure"]["kind"], kind)

    def test_changed_binary_and_empty_suite_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"before")
            version = subprocess.CompletedProcess([], 0, "court-jester test\n", "")
            def changed(*args):
                binary.write_bytes(b"after")
                return {"status": "passed"}
            with patch.object(CHECK.subprocess, "run", return_value=version), patch.object(CHECK, "check_case", side_effect=changed):
                report = CHECK.check_binary(binary)
            self.assertEqual(report["status"], "failed")
            self.assertNotEqual(report["binary"]["sha256"], report["binary"]["sha256_after"])
            with patch.object(CHECK.subprocess, "run", return_value=version), patch.object(CHECK, "CASES", ()):
                self.assertEqual(CHECK.check_binary(binary)["status"], "failed")

    def test_existing_evidence_is_not_overwritten_or_executed(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory) / "evidence.json"
            evidence.write_text("original")
            with patch.object(CHECK, "check_binary") as check, contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                CHECK.main(["--binary", "missing", "--output", str(evidence)])
            check.assert_not_called()
            self.assertEqual(evidence.read_text(), "original")

    def test_quality_wires_current_binary_evidence_and_main_pushes(self):
        quality = (ROOT / ".github/workflows/quality.yml").read_text()
        self.assertIn("  push:\n    branches: [main]", quality)
        self.assertIn('node-version: "24"', quality)
        command = "python3 scripts/check_repair_loop.py --binary target/release/court-jester --output target/repair-contract.json"
        self.assertIn(command, quality)
        self.assertLess(quality.index("cargo build --locked --release --bin court-jester"), quality.index(command))
        self.assertIn("path: target/repair-contract.json", quality)
        self.assertIn("--verify-sample", quality)


if __name__ == "__main__":
    unittest.main()
