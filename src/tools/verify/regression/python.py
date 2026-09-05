"""CLI-backed regression; absence of the original failure alone is not success."""
import json
import os
from pathlib import Path
import subprocess
import unittest


class CourtJesterRegression(unittest.TestCase):
    def test_recorded_check(self):
        bundle = Path(__file__).resolve().parent
        manifest = json.loads((bundle / "regression.json").read_text())
        self.assertEqual(manifest["artifact_schema_version"], 1)
        self.assertEqual(manifest["artifact_type"], "court_jester_regression")
        root = bundle
        for _ in range(manifest["project_levels"]):
            root = root.parent
        source = (root / manifest["source_file"]).resolve()
        self.assertTrue(source.is_file(), "current regression source is unavailable")
        source.relative_to(root)  # Fail if a source symlink escapes the checkout.
        mode = manifest.get("replay_mode", "current_source")
        self.assertIn(mode, ("current_source", "differential_live"))
        candidate_args = ["--candidate-project-dir", str(root)] if mode == "differential_live" else []
        result = subprocess.run([
            os.environ.get("COURT_JESTER_BINARY", "court-jester"), "replay",
            "--report", str(bundle / "report.json"), "--finding", manifest["finding_id"],
            "--dependency-project-dir", str(root),
            *candidate_args,
        ], cwd=root, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        replay = json.loads(result.stdout)
        self.assertEqual(replay["schema_version"], 3)
        self.assertEqual(replay["finding_id"], manifest["finding_id"])
        self.assertEqual(replay["outcome"], "not_reproduced")
        self.assertIs(replay.get("check_passed"), True, "recorded check did not pass: " + result.stdout)


if __name__ == "__main__":
    unittest.main()
