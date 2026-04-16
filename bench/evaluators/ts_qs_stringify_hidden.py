from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_qs_stringify_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    body = "\n".join(
        [
            'assert.equal(mod.stringifyQuery({ extra: null, q: "alpha" }), "q=alpha");',
            'assert.equal(mod.stringifyQuery({ filter: { city: "Paris", zip: 75001 } }), "filter%5Bcity%5D=Paris&filter%5Bzip%5D=75001");',
            'assert.equal(mod.stringifyQuery({ tag: ["pro", "beta"], page: 2 }), "page=2&tag=pro&tag=beta");',
            'assert.equal(mod.stringifyQuery({ filter: { nested: { city: "Denver" } } }), "filter%5Bnested%5D%5Bcity%5D=Denver");',
        ]
    )
    run_bun_assertions(workspace / "stringify.ts", body)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
