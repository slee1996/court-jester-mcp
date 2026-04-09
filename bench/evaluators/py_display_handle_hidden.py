from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_display_handle_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "handle.py", "handle_under_test")

    display_handle = module.display_handle
    assert display_handle(None) == "guest"
    assert display_handle({"profile": None, "username": "Spencer"}) == "spencer"
    assert display_handle({"profile": {"handle": "   "}, "username": " Admin "}) == "admin"
    assert display_handle({"profile": {"handle": "   "}, "username": "   "}) == "guest"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
