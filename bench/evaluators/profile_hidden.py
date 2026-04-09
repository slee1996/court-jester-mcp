from __future__ import annotations

import importlib.util
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: profile_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module_path = workspace / "profile.py"
    spec = importlib.util.spec_from_file_location("profile_under_test", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Could not import {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    normalize = module.normalize_display_name
    assert normalize("") == "Anonymous"
    assert normalize("   ") == "Anonymous"
    assert normalize("a") == "A"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
