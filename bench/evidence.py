"""Portable, redaction-aware benchmark evidence bundles.

This module deliberately has no dependency on the benchmark runner.  A completed
matrix directory is treated as an immutable source tree and copied into a new
bundle without ever traversing the destination.  The public
:func:`build_evidence_bundle` function is the integration point used by the
matrix runner; the command line interface is useful for producing a bundle from
an already completed run.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sys
from pathlib import Path
from typing import Any, Iterable, Sequence

ARTIFACT_SCHEMA_VERSION = 1
VERIFY_SCHEMA_VERSION_REQUIRED = 3
REDACTION_MODES = ("none", "transcripts", "all-text")

# These names are intentionally narrow.  A benchmark result may contain other
# hashes (patches, prompts, etc.) which are not hashes of matrix.json.
_MATRIX_DIGEST_KEYS = {
    "matrix_sha256",
    "matrix_digest",
    "source_matrix_sha256",
    "source_matrix_digest",
}
_ALL_TEXT_FIELDS = {
    "prompt",
    "response",
    "stdout",
    "stderr",
    "error",
    "message",
    "diff",
    "patch",
    "trace",
    "repro",
    "content",
}
_TRANSCRIPT_FIELDS = {
    "prompt",
    "response",
    "transcript",
    "stdout",
    "stderr",
    "agent_trace",
    "trace",
}
_SENSITIVE_FILE_PARTS = {
    "prompt",
    "prompts",
    "response",
    "responses",
    "transcript",
    "transcripts",
    "stdout",
    "stderr",
}

_ABSOLUTE_PATH_PATTERN = re.compile(
    r"(?<![A-Za-z0-9:/.])/(?:[^/\s\"'<>]+/)*[^/\s\"'<>]+"
)


class EvidenceBuildError(RuntimeError):
    """Raised when strict evidence construction cannot produce a valid bundle."""

    def __init__(self, message: str, *, errors: Sequence[str] | None = None) -> None:
        self.errors = list(errors or [message])
        super().__init__(message)
EvidenceError = EvidenceBuildError


def _canonical_json(value: Any) -> str:
    """Return the canonical JSON representation used for manifest hashing."""

    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def _sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _relative_is_under(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
    except ValueError:
        return False
    return True


def _remove_fields(value: Any, fields: set[str]) -> Any:
    """Recursively remove sensitive object fields without altering list order."""

    if isinstance(value, dict):
        return {
            key: _remove_fields(item, fields)
            for key, item in value.items()
            if str(key).lower() not in fields
        }
    if isinstance(value, list):
        return [_remove_fields(item, fields) for item in value]
    return value

def _is_hidden_or_workspace(relative: Path) -> bool:
    return any(part.startswith(".") or part.lower() == "workspace" for part in relative.parts)


def _is_diff(path: Path) -> bool:
    return path.suffix.lower() in {".diff", ".patch"} or path.name.endswith(".diff")


def _is_trace_path(relative: Path) -> bool:
    return any(part.lower() in {"agent_trace", "agent-trace", "traces", "trace"} for part in relative.parts)


def _is_sensitive_name(relative: Path) -> bool:
    for part in relative.parts:
        stem = Path(part).stem.lower().replace("-", "_")
        if stem in _SENSITIVE_FILE_PARTS:
            return True
        if any(stem.startswith(f"{token}_") or stem.endswith(f"_{token}") for token in _SENSITIVE_FILE_PARTS):
            return True
    return False


def _is_runner_output_file(relative: Path) -> bool:
    """Identify command output artifacts emitted by the benchmark runner."""

    normalized = relative.name.lower().replace("-", ".").replace("_", ".")
    parts = set(normalized.split("."))
    return relative.suffix.lower() != ".json" and bool(
        parts & {"stdout", "stderr", "output", "out", "err", "log", "transcript", "response", "prompt"}
    )


def _source_kind(relative: Path) -> str:
    name = relative.name.lower()
    if name == "matrix.json":
        return "matrix"
    if name == "summary.json" and len(relative.parts) == 1:
        return "summary"
    if name == "run.json":
        return "run"
    if name == "result.json":
        return "result"
    if _is_diff(relative):
        return "diff"
    if _is_trace_path(relative) and name == "summary.json":
        return "trace_summary"
    if "evaluator" in {part.lower() for part in relative.parts} or "metadata" in name:
        return "evaluator_metadata"
    if relative.suffix.lower() == ".json":
        return "structured_json"
    return "text"


def _json_artifact_version_errors(value: Any, relative: Path) -> list[str]:
    if not isinstance(value, dict):
        return [f"{relative.as_posix()}: JSON artifact is not an object"]
    errors: list[str] = []
    artifact_version = value.get("artifact_schema_version")
    if artifact_version != ARTIFACT_SCHEMA_VERSION:
        errors.append(
            f"{relative.as_posix()}: artifact_schema_version must be {ARTIFACT_SCHEMA_VERSION}"
        )
    verify_version = value.get("verify_schema_version_required")
    if verify_version != VERIFY_SCHEMA_VERSION_REQUIRED:
        errors.append(
            f"{relative.as_posix()}: verify_schema_version_required must be {VERIFY_SCHEMA_VERSION_REQUIRED}"
        )
    return errors


def _find_digest_values(value: Any) -> Iterable[tuple[str, str]]:
    if isinstance(value, dict):
        for key, child in value.items():
            key_text = str(key).lower()
            if key_text in _MATRIX_DIGEST_KEYS and isinstance(child, str):
                yield key_text, child
            yield from _find_digest_values(child)
    elif isinstance(value, list):
        for child in value:
            yield from _find_digest_values(child)


def _verification_metadata(value: Any) -> dict[str, list[str]]:
    """Collect typed verification context without parsing human-readable text."""
    buckets: dict[str, set[str]] = {
        "source_modes": set(),
        "network_policies": set(),
        "termination_kinds": set(),
        "provenance": set(),
        "failure_domains": set(),
        "failure_kinds": set(),
    }

    def walk(node: Any, parent: str = "") -> None:
        if isinstance(node, dict):
            for key, child in node.items():
                normalized = str(key).lower()
                if isinstance(child, str):
                    target = {
                        "source_mode": "source_modes",
                        "network_policy": "network_policies",
                        "network": "network_policies",
                        "kind": "failure_kinds" if parent in {"diagnostic", "diagnostics"} else None,
                        "domain": "failure_domains",
                        "failure_domain": "failure_domains",
                        "kind": "failure_kinds" if parent in {"diagnostic", "diagnostics", "termination", "process"} else None,
                        "failure_kind": "failure_kinds",
                    }.get(normalized)
                    if target:
                        buckets[target].add(child)
                if normalized in {"termination", "process"} and isinstance(child, dict):
                    kind = child.get("kind")
                    if isinstance(kind, str):
                        buckets["termination_kinds"].add(kind)
                walk(child, normalized)
        elif isinstance(node, list):
            for child in node:
                walk(child, parent)

    walk(value)
    return {key: sorted(values) for key, values in buckets.items() if values}


def _redact_json(value: Any, mode: str) -> Any:
    if mode == "none":
        return value
    if mode == "transcripts":
        return _remove_fields(value, _TRANSCRIPT_FIELDS)
    return _remove_fields(value, _ALL_TEXT_FIELDS)


def _portable_absolute_path(path_text: str, source_root: Path) -> str:
    path = Path(path_text)
    try:
        return path.resolve(strict=False).relative_to(source_root).as_posix()
    except (OSError, ValueError):
        name = path.name
        return f"<absolute-path>/{name}" if name else "<absolute-path>"


def _sanitize_json_paths(value: Any, source_root: Path) -> Any:
    """Remove host-absolute paths while retaining source-relative references."""

    if isinstance(value, dict):
        return {key: _sanitize_json_paths(item, source_root) for key, item in value.items()}
    if isinstance(value, list):
        return [_sanitize_json_paths(item, source_root) for item in value]
    if not isinstance(value, str):
        return value
    if Path(value).is_absolute():
        return _portable_absolute_path(value, source_root)
    source_prefix = str(source_root)
    text = value.replace(source_prefix, ".")

    def replace_path(match: re.Match[str]) -> str:
        raw = match.group(0)
        path_text = raw.rstrip(",.;:)]}")
        suffix = raw[len(path_text) :]
        return _portable_absolute_path(path_text, source_root) + suffix

    return _ABSOLUTE_PATH_PATTERN.sub(replace_path, text)


def _redaction_for_file(relative: Path, mode: str) -> tuple[bool, str | None]:
    """Return (include, warning) for a source file under the selected mode."""

    if mode == "all-text":
        # JSON is the structured carrier.  Plain text (including transcripts)
        # is intentionally not portable in all-text mode.
        if relative.suffix.lower() != ".json" or _is_diff(relative) or _is_trace_path(relative):
            return False, None
    if mode in {"transcripts", "all-text"}:
        if _is_trace_path(relative) or _is_sensitive_name(relative) or _is_runner_output_file(relative):
            return False, None
    return True, None
def _display_artifact_name(value: object) -> str:
    text = str(value)
    path = Path(text)
    return path.name if path.is_absolute() else text



def _parse_optional_patterns(optional_artifacts: Sequence[str] | None) -> list[str]:
    return [str(pattern) for pattern in (optional_artifacts or ()) if str(pattern)]


def _matches_pattern(relative: Path, pattern: str) -> bool:
    # Path.match supports both a filename pattern and a relative glob.  Also
    # accept exact paths, which is useful to callers passing required artifacts.
    return relative.as_posix() == pattern or relative.match(pattern) or relative.name == pattern


def _iter_source_files(source_root: Path, destination: Path) -> list[tuple[Path, Path]]:
    """Collect files before writing, preventing destination recursion."""

    files: list[tuple[Path, Path]] = []
    for current, dirs, names in os.walk(source_root, topdown=True, followlinks=False):
        current_path = Path(current)
        # Prune destination before it can be traversed when it lives below the
        # source root.  Hidden/workspace directories are likewise never walked.
        kept_dirs: list[str] = []
        for name in dirs:
            child = current_path / name
            rel = child.relative_to(source_root)
            if _is_hidden_or_workspace(rel):
                continue
            if _relative_is_under(child, destination):
                continue
            if child.is_symlink():
                continue
            kept_dirs.append(name)
        dirs[:] = kept_dirs
        for name in names:
            path = current_path / name
            rel = path.relative_to(source_root)
            if _is_hidden_or_workspace(rel) or _relative_is_under(path, destination):
                continue
            if path.is_symlink() or not path.is_file():
                continue
            files.append((path, rel))
    return sorted(files, key=lambda pair: pair[1].as_posix())


def build_evidence_bundle(
    source_root: str | os.PathLike[str],
    output_dir: str | os.PathLike[str],
    *,
    redaction: str = "transcripts",
    strict: bool = False,
    required_artifacts: Sequence[str] | None = None,
    optional_artifacts: Sequence[str] | None = None,
    # These aliases keep the API friendly to argparse.Namespace callers while
    # retaining one canonical implementation.
    redaction_mode: str | None = None,
    strict_evidence: bool | None = None,
) -> dict[str, Any]:
    """Build and return a portable evidence manifest.

    ``source_root`` is never modified.  ``output_dir`` is the bundle directory
    itself (the caller may choose ``<run-output>/evidence``).  In strict mode,
    root ``matrix.json`` and ``summary.json`` plus at least one ``result.json``
    are required and all required JSON artifacts must carry artifact schema 1,
    verify schema 3, and agree with the source matrix digest.
    """

    if redaction_mode is not None:
        redaction = redaction_mode
    if strict_evidence is not None:
        strict = bool(strict_evidence)
    if redaction not in REDACTION_MODES:
        raise ValueError(f"redaction must be one of {', '.join(REDACTION_MODES)}")

    source = Path(source_root).expanduser().resolve()
    destination = Path(output_dir).expanduser().resolve()
    if not source.exists() or not source.is_dir():
        raise EvidenceBuildError("source root does not exist or is not a directory")
    if source == destination:
        raise EvidenceBuildError("evidence destination must differ from source root")

    source_files = _iter_source_files(source, destination)
    by_relative = {relative.as_posix(): path for path, relative in source_files}
    errors: list[str] = []
    verification_metadata: dict[str, set[str]] = {}
    warnings: list[str] = []

    matrix_rel = "matrix.json"
    summary_rel = "summary.json"
    required = list(required_artifacts or (matrix_rel, summary_rel))
    if not any(relative.name == "result.json" for _, relative in source_files):
        required.append("result.json")

    for requirement in required:
        requirement_text = _display_artifact_name(requirement)
        if not any(_matches_pattern(Path(relative), str(requirement)) for relative in by_relative):
            errors.append(f"missing required artifact: {requirement_text}")

    matrix_path = by_relative.get(matrix_rel)
    matrix_digest: str | None = None
    if matrix_path is not None:
        try:
            matrix_raw = matrix_path.read_bytes()
            matrix_digest = _sha256_bytes(matrix_raw)
            json.loads(matrix_raw.decode("utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
            errors.append(f"matrix.json: unable to read structured artifact ({type(exc).__name__})")

    # Validate required structured artifacts and any digest claims before copy.
    structured_required = {
        relative.as_posix()
        for _, relative in source_files
        if relative.name in {"matrix.json", "summary.json", "result.json", "run.json"}
    }
    for path, relative in source_files:
        if relative.suffix.lower() != ".json":
            continue
        relative_text = relative.as_posix()
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
            if relative_text in structured_required:
                errors.append(f"{relative_text}: invalid JSON ({type(exc).__name__})")
            continue
        for key, values in _verification_metadata(value).items():
            verification_metadata.setdefault(key, set()).update(values)
        if relative_text in structured_required:
            errors.extend(_json_artifact_version_errors(value, relative))
        if matrix_digest is not None and relative_text != matrix_rel:
            for key, claimed in _find_digest_values(value):
                if claimed != matrix_digest:
                    errors.append(
                        f"{relative_text}: {key} does not match source matrix digest"
                    )

    # Explicit optional artifacts generate warnings when absent.  With no
    # explicit list, report the conventional optional categories so a bundle's
    # omissions remain visible without making them gate failures.
    optional_patterns = _parse_optional_patterns(optional_artifacts)
    if optional_patterns:
        for pattern in optional_patterns:
            if not any(_matches_pattern(relative, pattern) for _, relative in source_files):
                warnings.append(f"missing optional artifact: {_display_artifact_name(pattern)}")
    else:
        if not any(_is_diff(relative) for _, relative in source_files):
            warnings.append("missing optional artifact category: diffs")
        if not any(_source_kind(relative) == "evaluator_metadata" for _, relative in source_files):
            warnings.append("missing optional artifact category: evaluator metadata")
        if not any(_source_kind(relative) == "trace_summary" for _, relative in source_files):
            warnings.append("missing optional artifact category: trace summaries")

    if strict and errors:
        raise EvidenceBuildError("strict evidence validation failed", errors=errors)
    if errors:
        # Non-strict bundles remain usable for exploratory/legacy data, but do
        # not silently turn validation failures into a wall of path-specific
        # manifest warnings.
        warnings.append(f"{len(errors)} artifact validation issue(s)")
        warnings.extend(errors[:20])
        if len(errors) > 20:
            warnings.append(f"{len(errors) - 20} additional artifact validation issue(s)")

    destination.mkdir(parents=True, exist_ok=True)
    entries: list[dict[str, Any]] = []
    for source_path, relative in source_files:
        include, _ = _redaction_for_file(relative, redaction)
        if not include:
            continue
        relative_text = relative.as_posix()
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        source_kind = _source_kind(relative)
        entry_warnings: list[str] = []
        try:
            raw = source_path.read_bytes()
            output_bytes = raw
            if relative.suffix.lower() == ".json":
                try:
                    value = json.loads(raw.decode("utf-8"))
                    value = _redact_json(value, redaction)
                    value = _sanitize_json_paths(value, source)
                    output_bytes = (_canonical_json(value) + "\n").encode("utf-8")
                except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                    if redaction != "none":
                        warnings.append(f"{relative_text}: omitted unparsed JSON during redaction ({type(exc).__name__})")
                        continue
                    entry_warnings.append(f"unparsed JSON copied as bytes ({type(exc).__name__})")
            target.write_bytes(output_bytes)
        except OSError as exc:
            entry_warnings.append(f"copy failed ({type(exc).__name__})")
            warnings.append(f"{relative_text}: copy failed")
            if strict and source_kind in {"matrix", "summary", "result", "run"}:
                raise EvidenceBuildError("strict evidence copy failed", errors=[relative_text]) from exc
            continue

        entries.append(
            {
                "path": relative_text,
                "bytes": len(output_bytes),
                "sha256": _sha256_bytes(output_bytes),
                "source_kind": source_kind,
                "warnings": entry_warnings,
                "redaction_mode": redaction,
                "source_matrix_digest": matrix_digest,
            }
        )

    manifest: dict[str, Any] = {
        "artifact_schema_version": ARTIFACT_SCHEMA_VERSION,
        "verify_schema_version_required": VERIFY_SCHEMA_VERSION_REQUIRED,
        "redaction": redaction,
        "source_matrix_digest": matrix_digest,
        "source_matrix_sha256": matrix_digest,
        "entries": entries,
        "verification_metadata": {
            key: sorted(values)
            for key, values in sorted(verification_metadata.items())
            if values
        },
        "warnings": sorted(set(warnings)),
    }
    manifest_hash = _sha256_bytes(_canonical_json(manifest).encode("utf-8"))
    manifest["manifest_hash"] = manifest_hash
    (destination / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
    return manifest


# Short aliases are useful to callers that use the terminology "bundle".
build_bundle = build_evidence_bundle
create_evidence_bundle = build_evidence_bundle


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build a portable benchmark evidence bundle.")
    parser.add_argument("source_root", help="Completed matrix output directory.")
    parser.add_argument(
        "--output-dir",
        default=None,
        help="Bundle destination (default: SOURCE_ROOT/evidence).",
    )
    parser.add_argument("--redaction", "--evidence-redaction", dest="redaction", choices=REDACTION_MODES, default="transcripts")
    parser.add_argument("--strict-evidence", "--strict", dest="strict_evidence", action="store_true")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    source = Path(args.source_root)
    destination = Path(args.output_dir) if args.output_dir else source / "evidence"
    try:
        manifest = build_evidence_bundle(
            source,
            destination,
            redaction=args.redaction,
            strict=args.strict_evidence,
        )
    except (EvidenceBuildError, OSError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        return 1
    print(json.dumps(manifest, ensure_ascii=False, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
