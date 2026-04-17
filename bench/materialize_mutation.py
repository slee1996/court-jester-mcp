from __future__ import annotations

import shutil
import sys
from pathlib import Path


def resolve_target(workspace: Path, relative_target: str) -> Path:
    workspace = workspace.resolve()
    target = (workspace / relative_target).resolve()
    target.relative_to(workspace)
    return target


def materialize_mutation(workspace: Path, relative_target: str, source_path: Path) -> Path:
    source = source_path.expanduser().resolve()
    if not source.is_file():
        raise FileNotFoundError(f"mutation source file not found: {source}")
    target = resolve_target(workspace, relative_target)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)
    return target


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(
            "usage: python bench/materialize_mutation.py <workspace> <relative-target> <source-file>",
            file=sys.stderr,
        )
        return 2

    workspace = Path(argv[1]).expanduser().resolve()
    if not workspace.is_dir():
        print(f"workspace directory not found: {workspace}", file=sys.stderr)
        return 2

    materialize_mutation(workspace, argv[2], Path(argv[3]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
