from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_primary_plan_code_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "plans.py", "plans_under_test")

    primary_plan_code = module.primary_plan_code
    assert primary_plan_code(None) == "FREE"
    assert primary_plan_code({"plans": []}) == "FREE"
    assert primary_plan_code({"plans": ["   ", " team "]}) == "TEAM"
    assert primary_plan_code({"plans": [None, " pro "]}) == "PRO"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
