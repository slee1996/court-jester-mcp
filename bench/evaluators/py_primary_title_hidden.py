from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_primary_title_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "titles.py", "titles_under_test")

    primary_title = module.primary_title
    assert primary_title(None) == "Untitled"
    assert primary_title([]) == "Untitled"
    assert primary_title(["   "]) == "Untitled"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
