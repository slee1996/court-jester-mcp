from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path


BENCH_ROOT = Path(__file__).resolve().parents[1]


def hidden_asset_source(workspace: Path, relative_test_path: str) -> Path | None:
    fixture = os.environ.get("CJ_REPO_FIXTURE") or workspace.name
    candidate = BENCH_ROOT / "hidden_assets" / fixture / relative_test_path
    if candidate.exists():
        return candidate
    return None


def materialize_hidden_suite(source_path: Path, target_path: Path) -> list[Path]:
    target_path.parent.mkdir(parents=True, exist_ok=True)
    copied: list[Path] = []
    for sibling in source_path.parent.glob("hidden*.ts"):
        destination = target_path.parent / sibling.name
        shutil.copy2(sibling, destination)
        copied.append(destination)
    return copied


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit("usage: ts_workspace_test.py <workspace> <relative-test-path>")

    workspace = Path(sys.argv[1]).resolve()
    relative_test_path = sys.argv[2]
    test_path = workspace / relative_test_path
    copied_paths: list[Path] = []
    if not test_path.exists():
        source_path = hidden_asset_source(workspace, relative_test_path)
        if source_path is None:
            raise FileNotFoundError(f"Hidden test file not found in workspace or hidden assets: {relative_test_path}")
        copied_paths = materialize_hidden_suite(source_path, test_path)
    try:
        completed = subprocess.run(
            ["bun", str(test_path)],
            cwd=workspace,
            capture_output=True,
            text=True,
        )
    finally:
        for copied_path in copied_paths:
            copied_path.unlink(missing_ok=True)
    if completed.returncode != 0:
        raise AssertionError(
            "bun test failed\n"
            f"stdout:\n{completed.stdout}\n"
            f"stderr:\n{completed.stderr}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
