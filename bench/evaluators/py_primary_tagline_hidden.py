from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_primary_tagline_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "tagline.py", "tagline_under_test")

    primary_tagline = module.primary_tagline
    assert primary_tagline(None) == "general"
    assert primary_tagline({"segments": []}) == "general"
    assert primary_tagline({"segments": ["   ", "Growth "]}) == "Growth"
    assert primary_tagline({"segments": ["  ", "\t"]}) == "general"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
