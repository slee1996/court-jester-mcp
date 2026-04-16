from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_qs_parse_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    body = "\n".join(
        [
            'assert.deepEqual(mod.parseQuery("filter[city]=Paris&filter[zip]=75001"), { filter: { city: "Paris", zip: "75001" } });',
            'assert.deepEqual(mod.parseQuery("filter[nested][city]=Denver"), { filter: { nested: { city: "Denver" } } });',
            'assert.deepEqual(mod.parseQuery("a[]=b&a[]=c"), { a: ["b", "c"] });',
            'assert.deepEqual(mod.parseQuery("tag=pro&tag=beta&tag=prod"), { tag: ["pro", "beta", "prod"] });',
        ]
    )
    run_bun_assertions(workspace / "parse.ts", body)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
