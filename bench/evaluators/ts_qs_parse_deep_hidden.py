from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_qs_parse_deep_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    body = "\n".join(
        [
            'assert.deepEqual(mod.parseQuery("filter[nested][tags][]=pro&filter[nested][tags][]=beta"), { filter: { nested: { tags: ["pro", "beta"] } } });',
            'assert.deepEqual(mod.parseQuery("page=2&filter[city]=Paris"), { page: "2", filter: { city: "Paris" } });',
            'assert.deepEqual(mod.parseQuery("filter[city]=New%20York&filter[tag]=alpha&filter[tag]=beta"), { filter: { city: "New York", tag: ["alpha", "beta"] } });',
        ]
    )
    run_bun_assertions(workspace / "parse.ts", body)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
