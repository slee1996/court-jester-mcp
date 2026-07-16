from __future__ import annotations

import subprocess
import sys
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
CHECKER = REPO_ROOT / "scripts" / "check_release.py"


class ReleaseContractTest(unittest.TestCase):
    def run_checker(self, tag: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(CHECKER), "--tag", tag],
            cwd=REPO_ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_current_release_metadata_is_coherent(self) -> None:
        result = self.run_checker("v0.2.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("release metadata valid for v0.2.0", result.stdout)

    def test_mismatched_tag_is_rejected(self) -> None:
        result = self.run_checker("v9.9.9")
        self.assertEqual(result.returncode, 1)
        self.assertIn("does not match Cargo package version", result.stderr)


if __name__ == "__main__":
    unittest.main()
