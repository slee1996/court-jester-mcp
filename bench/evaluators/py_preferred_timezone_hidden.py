from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_preferred_timezone_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "settings.py", "settings_under_test")

    preferred_timezone = module.preferred_timezone
    assert preferred_timezone(None) == "UTC"
    assert preferred_timezone({"preferences": None}) == "UTC"
    assert preferred_timezone({"preferences": {"timezone": "   "}}) == "UTC"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
