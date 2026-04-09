from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_billing_country_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "billing.py", "billing_under_test")

    billing_country = module.billing_country
    assert billing_country(None) == "US"
    assert billing_country({"billing": None}) == "US"
    assert billing_country({"billing": {"country": "   "}}) == "US"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
