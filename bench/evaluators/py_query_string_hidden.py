from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: py_query_string_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "query.py", "query_under_test")
    canonical_query = module.canonical_query
    assert canonical_query({"tag": ["pro", None, " beta "]}) == "tag=pro&tag=beta"
    assert canonical_query({"q": "  ", "page": 2}) == "page=2"
    assert canonical_query({"q": "naïve café"}) == "q=naive+cafe"
    assert canonical_query({"flags": {"beta_checkout": None}}) == ""
    assert canonical_query({"filters": [{"label": "pro"}, None, " beta "]}) == "filters=beta"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
