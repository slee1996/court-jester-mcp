from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_support_email_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "support.py", "support_under_test")

    support_email_domain = module.support_email_domain
    assert support_email_domain(None) == "unknown"
    assert support_email_domain({"contacts": None}) == "unknown"
    assert support_email_domain({"contacts": {"support_email": "ops"}}) == "unknown"
    assert support_email_domain({"contacts": {"support_email": "   "}}) == "unknown"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
