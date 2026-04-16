from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_qs_stringify_deep_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    body = "\n".join(
        [
            'assert.equal(mod.stringifyQuery({ filter: { nested: { cities: ["Denver", "Boulder"] } } }), "filter%5Bnested%5D%5Bcities%5D%5B%5D=Denver&filter%5Bnested%5D%5Bcities%5D%5B%5D=Boulder");',
            'assert.equal(mod.stringifyQuery({ filter: { city: "New York", zip: null }, page: 2 }), "filter%5Bcity%5D=New%20York&page=2");',
            'assert.equal(mod.stringifyQuery({ filter: {}, q: "alpha" }), "q=alpha");',
        ]
    )
    run_bun_assertions(workspace / "stringify.ts", body)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
