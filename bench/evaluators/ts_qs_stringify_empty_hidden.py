from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_qs_stringify_empty_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    body = "\n".join(
        [
            'assert.equal(mod.stringifyQuery({ a: undefined, b: null, c: "" }), "c=");',
            'assert.equal(mod.stringifyQuery({ filter: { city: null, zip: 75001 } }), "filter%5Bzip%5D=75001");',
            'assert.equal(mod.stringifyQuery({ filters: {}, q: "alpha" }), "q=alpha");',
            'assert.equal(mod.stringifyQuery({ items: [], q: "alpha" }), "q=alpha");',
        ]
    )
    run_bun_assertions(workspace / "stringify.ts", body)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
