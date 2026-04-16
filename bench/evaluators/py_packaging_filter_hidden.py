from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_packaging_filter_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "filtering.py", "filtering_under_test")
    filter_versions = module.filter_versions

    assert filter_versions(["1.0a1"], "") == ["1.0a1"]
    assert filter_versions(["1.0a1", "1.0"], "") == ["1.0"]
    assert filter_versions(["1.0a1", "1.0b1"], "") == ["1.0a1", "1.0b1"]
    assert filter_versions(["1.2", "1.5a1"], ">=1.5") == ["1.5a1"]
    assert filter_versions(["1.0", "1.0.post1"], "==1.0") == ["1.0"]
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
