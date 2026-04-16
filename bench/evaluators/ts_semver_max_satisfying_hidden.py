from __future__ import annotations

import json
import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_semver_max_satisfying_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    body = "\n".join(
        [
            'assert.equal(mod.maxSatisfying(["1.3.0-beta.1", "1.2.9"], "^1.2.3"), "1.2.9");',
            'assert.equal(mod.maxSatisfying(["1.2.3-alpha.1", "1.2.3-alpha.2"], "1.2.3-alpha.2"), "1.2.3-alpha.2");',
            'assert.equal(mod.maxSatisfying(["0.0.3", "0.0.4", "0.0.5"], "^0.0.3"), "0.0.3");',
            'assert.equal(mod.maxSatisfying(["1.2.3+build.1", "1.2.3+build.9"], "1.2.3"), "1.2.3");',
            'assert.equal(mod.maxSatisfying(["1.2.3-beta.1", "1.2.3"], "^1.2.3"), "1.2.3");',
        ]
    )
    run_bun_assertions(workspace / "versions.ts", body)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
