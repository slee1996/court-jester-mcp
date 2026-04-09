from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_lodash_object_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    body = "\n".join(
        [
            "const sourceProto = { inherited: 2 };",
            "const source = Object.create(sourceProto);",
            "source.own = 1;",
            "assert.deepEqual(mod.defaults({}, source), { own: 1, inherited: 2 });",
            "assert.deepEqual(mod.defaults({ a: undefined }, { a: 1 }), { a: 1 });",
            "assert.deepEqual(mod.pick({ '0': 'a', '1': 'b' }, 0), { '0': 'a' });",
            "assert.deepEqual(mod.pick({ 'a.b': 1, a: { b: 2 } }, [['a.b']]), { 'a.b': 1 });",
            "assert.deepEqual(mod.pick(null, 'valueOf'), {});",
            "assert.deepEqual(mod.omit({ '0': 'a' }, 0), {});",
            "assert.deepEqual(mod.omit({ 'a.b': 1, a: { b: 2 } }, [['a.b']]), { a: { b: 2 } });",
            "assert.deepEqual(mod.omit(undefined, 'valueOf'), {});",
        ]
    )
    run_bun_assertions(workspace / "object.ts", body)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
