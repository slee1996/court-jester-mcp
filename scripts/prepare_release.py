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


def resolve_biome(args: argparse.Namespace) -> Path | None:
    if args.biome:
        biome = Path(args.biome).expanduser().resolve()
        if not biome.exists():
            raise FileNotFoundError(f"Could not find biome binary at {biome}")
        return biome

    found = shutil.which("biome")
    if found:
        return Path(found).resolve()

    if args.require_biome:
        raise FileNotFoundError(
            "Could not find `biome` on PATH. Install it or pass --biome /absolute/path/to/biome."
        )

    return None


def copy_executable(src: Path, dst: Path) -> None:
    shutil.copy2(src, dst)
    mode = dst.stat().st_mode
    dst.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Stage a Court Jester release bundle with an optional sibling biome binary."
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
        help="Use an explicit biome binary path instead of resolving biome from PATH.",
    )
    parser.add_argument(
        "--require-biome",
        action="store_true",
        help="Fail if a biome binary cannot be bundled.",
    )
    parser.add_argument(
        "--bundle-dir",
        help="Output directory for the staged bundle. Defaults to dist/court-jester-{profile}.",
    )
    args = parser.parse_args()

    try:
        binary = resolve_binary(args)
        bundle_dir = resolve_bundle_dir(args)
        biome = resolve_biome(args)

        bundle_dir.mkdir(parents=True, exist_ok=True)

        bundled_binary = bundle_dir / "court-jester-mcp"
        copy_executable(binary, bundled_binary)

        print(f"Bundled binary: {bundled_binary}")

        if biome is not None:
            bundled_biome = bundle_dir / "biome"
            copy_executable(biome, bundled_biome)
            print(f"Bundled biome:  {bundled_biome}")
        else:
            print("Bundled biome:  skipped (TypeScript lint will require biome on PATH)")

        print()
        print("Runtime behavior:")
        print("1. court-jester-mcp looks for ./biome next to itself first")
        print("2. if no sibling biome exists, it falls back to biome on PATH")
        return 0
    except Exception as exc:
        print(f"Release bundling failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
