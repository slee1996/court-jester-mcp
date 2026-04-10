#!/usr/bin/env python3

from __future__ import annotations

import argparse
import shutil
import stat
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent


def resolve_binary(args: argparse.Namespace) -> Path:
    if args.binary:
        binary = Path(args.binary).expanduser().resolve()
    else:
        profile = "debug" if args.debug else "release"
        binary = REPO_ROOT / "target" / profile / "court-jester-mcp"
    if not binary.exists():
        raise FileNotFoundError(
            f"Could not find {binary}. Build it first with `cargo build --release`."
        )
    return binary


def resolve_bundle_dir(args: argparse.Namespace) -> Path:
    if args.bundle_dir:
        return Path(args.bundle_dir).expanduser().resolve()
    profile = "debug" if args.debug else "release"
    return (REPO_ROOT / "dist" / f"court-jester-{profile}").resolve()


def resolve_tool(
    tool_name: str, explicit_path: str | None, require_tool: bool
) -> Path | None:
    if explicit_path:
        tool = Path(explicit_path).expanduser().resolve()
        if not tool.exists():
            raise FileNotFoundError(f"Could not find {tool_name} binary at {tool}")
        return tool

    found = shutil.which(tool_name)
    if found:
        return Path(found).resolve()

    if require_tool:
        raise FileNotFoundError(
            f"Could not find `{tool_name}` on PATH. Install it or pass --{tool_name} /absolute/path/to/{tool_name}."
        )

    return None


def copy_executable(src: Path, dst: Path) -> None:
    shutil.copy2(src, dst)
    mode = dst.stat().st_mode
    dst.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Stage a Court Jester release directory. By default this includes only court-jester-mcp."
    )
    profile = parser.add_mutually_exclusive_group()
    profile.add_argument(
        "--release",
        action="store_true",
        help="Use target/release/court-jester-mcp (default).",
    )
    profile.add_argument(
        "--debug",
        action="store_true",
        help="Use target/debug/court-jester-mcp.",
    )
    parser.add_argument(
        "--binary",
        help="Use an explicit court-jester-mcp binary instead of target/{release,debug}/court-jester-mcp.",
    )
    parser.add_argument(
        "--biome",
        help="Explicit biome binary path to include when bundling Biome.",
    )
    parser.add_argument(
        "--ruff",
        help="Explicit ruff binary path to include when bundling Ruff.",
    )
    parser.add_argument(
        "--include-biome",
        action="store_true",
        help="Include a sibling biome binary. Not recommended for public macOS releases.",
    )
    parser.add_argument(
        "--include-ruff",
        action="store_true",
        help="Include a sibling ruff binary. Not recommended for public macOS releases.",
    )
    parser.add_argument(
        "--require-biome",
        action="store_true",
        help="When bundling Biome, fail if a biome binary cannot be found.",
    )
    parser.add_argument(
        "--require-ruff",
        action="store_true",
        help="When bundling Ruff, fail if a ruff binary cannot be found.",
    )
    parser.add_argument(
        "--bundle-dir",
        help="Output directory for the staged bundle. Defaults to dist/court-jester-{profile}.",
    )
    args = parser.parse_args()

    try:
        binary = resolve_binary(args)
        bundle_dir = resolve_bundle_dir(args)
        include_biome = args.include_biome or args.biome is not None or args.require_biome
        include_ruff = args.include_ruff or args.ruff is not None or args.require_ruff
        biome = (
            resolve_tool("biome", args.biome, args.require_biome)
            if include_biome
            else None
        )
        ruff = (
            resolve_tool("ruff", args.ruff, args.require_ruff)
            if include_ruff
            else None
        )

        bundle_dir.mkdir(parents=True, exist_ok=True)

        bundled_binary = bundle_dir / "court-jester-mcp"
        copy_executable(binary, bundled_binary)

        print(f"Bundled binary: {bundled_binary}")

        if include_ruff and ruff is not None:
            bundled_ruff = bundle_dir / "ruff"
            copy_executable(ruff, bundled_ruff)
            print(f"Bundled ruff:   {bundled_ruff}")
        elif include_ruff:
            print("Bundled ruff:   requested but unavailable")
        else:
            print("Bundled ruff:   skipped by default (install Ruff separately)")

        if include_biome and biome is not None:
            bundled_biome = bundle_dir / "biome"
            copy_executable(biome, bundled_biome)
            print(f"Bundled biome:  {bundled_biome}")
        elif include_biome:
            print("Bundled biome:  requested but unavailable")
        else:
            print("Bundled biome:  skipped by default (install Biome separately)")

        print()
        print("Runtime behavior:")
        print("1. court-jester-mcp looks for ./ruff and ./biome next to itself first")
        print("2. if a sibling linter is missing, it falls back to PATH for that tool")
        print("3. public release assets should normally ship only court-jester-mcp")
        if sys.platform == "darwin" and (include_ruff or include_biome):
            print()
            print("macOS note:")
            print("Bundling third-party linters is best for controlled/local use.")
            print("For public releases, prefer shipping court-jester-mcp alone and installing Ruff/Biome separately.")
        return 0
    except Exception as exc:
        print(f"Release bundling failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
