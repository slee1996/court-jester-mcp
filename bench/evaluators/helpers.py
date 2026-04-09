from __future__ import annotations

import importlib.util
import json
import os
import random
import subprocess
import sys
from pathlib import Path


def load_python_module(module_path: Path, module_name: str) -> object:
    parent = str(module_path.parent)
    remove_after = False
    if parent not in sys.path:
        sys.path.insert(0, parent)
        remove_after = True
    spec = importlib.util.spec_from_file_location(module_name, module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Could not import {module_path}")
    module = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(module)
    finally:
        if remove_after:
            try:
                sys.path.remove(parent)
            except ValueError:
                pass
    return module


def run_bun_assertions(module_path: Path, body: str) -> None:
    script = "\n".join(
        [
            'import assert from "node:assert/strict";',
            f"const mod = await import({json.dumps(module_path.resolve().as_uri())});",
            body,
        ]
    )
    completed = subprocess.run(
        ["bun", "-e", script],
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise AssertionError(
            "bun assertions failed\n"
            f"stdout:\n{completed.stdout}\n"
            f"stderr:\n{completed.stderr}"
        )


def hidden_rng() -> random.Random:
    seed = os.environ.get("CJ_HIDDEN_SEED", "court-jester-hidden-seed")
    return random.Random(seed)
