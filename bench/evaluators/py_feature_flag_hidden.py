from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_feature_flag_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "flags.py", "flags_under_test")
    beta_checkout_enabled = module.beta_checkout_enabled
    assert beta_checkout_enabled({"flags": {"beta_checkout": False}}) is False
    assert beta_checkout_enabled({"flags": None}) is True
    assert beta_checkout_enabled({"flags": {"beta_checkout": None}}) is True
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
