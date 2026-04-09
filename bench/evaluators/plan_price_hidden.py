from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: plan_price_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "billing.py", "billing_under_test")
    plan_price = module.plan_price
    assert plan_price(None) == 0
    assert plan_price("") == 0
    assert plan_price("enterprise") == 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
