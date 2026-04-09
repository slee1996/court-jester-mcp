from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: average_order_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "metrics.py", "metrics_under_test")
    average = module.average_order_value
    assert average(0, 0) == 0.0
    assert average(450, 0) == 0.0
    assert average(0, 5) == 0.0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
