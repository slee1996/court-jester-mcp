from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_packaging_version_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "versioning.py", "versioning_under_test")
    compare_versions = module.compare_versions

    assert compare_versions("1.0.dev1", "1.0a1") < 0
    assert compare_versions("1.0a1", "1.0b1") < 0
    assert compare_versions("1.0b1", "1.0rc1") < 0
    assert compare_versions("1.0rc1", "1.0") < 0
    assert compare_versions("1.0", "1.0.post1") < 0
    assert compare_versions("1.0", "1.0.0") == 0
    assert compare_versions("2.0", "1.9.9") > 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
