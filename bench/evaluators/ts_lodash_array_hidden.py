from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_lodash_array_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    body = "\n".join(
        [
            "const values = [0, 1, 2, 3, 4, 5];",
            "assert.deepEqual(mod.chunk([], 3), []);",
            "assert.deepEqual(mod.chunk(values, -1), []);",
            "assert.deepEqual(mod.chunk(values, Number.NEGATIVE_INFINITY), []);",
            "assert.deepEqual(mod.chunk(values, values.length / 4), [[0], [1], [2], [3], [4], [5]]);",
            "assert.deepEqual(mod.flatten([1, [2, [3, [4]], 5]]), [1, 2, [3, [4]], 5]);",
            "assert.deepEqual(mod.flatten({0: 'a'}), []);",
            "assert.deepEqual(mod.uniq([2, 1, 2]), [2, 1]);",
            "assert.deepEqual(mod.uniq([NaN, NaN, 1]), [NaN, 1]);",
            "const first = { a: 1 };",
            "const second = { a: 1 };",
            "assert.deepEqual(mod.uniq([first, second, first]), [first, second]);",
        ]
    )
    run_bun_assertions(workspace / "array.ts", body)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
