from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_packaging_specifier_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "specifiers.py", "specifiers_under_test")
    allows = module.allows

    assert allows("1.0a1", ">=1.0") is False
    assert allows("1.0", ">=1.0") is True
    assert allows("1.4.6", "~=1.4.5") is True
    assert allows("1.5.0", "~=1.4.5") is False
    assert allows("2.0.0", "~=1.4") is False
    assert allows("1.4.5", "~=1.4") is True
    assert allows("1.0rc1", "==1.0") is False
    assert allows("1.0a1", "==1.0a1") is True
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
