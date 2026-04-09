from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_secondary_support_email_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "contact.py", "contact_under_test")

    secondary_support_email = module.secondary_support_email
    assert secondary_support_email(None) == "help@example.com"
    assert secondary_support_email({"contacts": None}) == "help@example.com"
    assert secondary_support_email({"contacts": {"emails": ["owner@example.com"]}}) == "help@example.com"
    assert (
        secondary_support_email(
            {"contacts": {"emails": ["owner@example.com", "   "]}}
        )
        == "help@example.com"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
