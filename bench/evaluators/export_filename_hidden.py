from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: export_filename_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "export_name.py", "export_name_under_test")
    export_filename = module.export_filename
    assert export_filename("Café Sales") == "cafe-sales.csv"
    assert export_filename("  résumé  ") == "resume.csv"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
