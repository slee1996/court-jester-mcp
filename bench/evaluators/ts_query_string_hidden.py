from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_query_string_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "query.ts",
        """
assert.equal(mod.canonicalQuery({ tag: ["pro", null, " beta "] }), "tag=pro&tag=beta");
assert.equal(mod.canonicalQuery({ q: "  ", page: 2 }), "page=2");
assert.equal(mod.canonicalQuery({ q: "naïve café" }), "q=naive%20cafe");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
