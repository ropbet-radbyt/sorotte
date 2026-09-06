"""Export bounded, privacy-projected native diagnostics, never qualification.

Raw logs, images, configs and runner diagnostic directories stay private. The
projection retains structured causal events and explicit unavailable records;
an uploadable directory is always fresh and identifies one source/run/attempt.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
from typing import Any

KIND = "sorotte-native-diagnostic"
MAX_FILE_BYTES = 4 * 1024 * 1024
MAX_RECORDS = 256
MAX_FILES = 128
SAFE_FILES = {
    "preflight.json", "guest-preflight.json", "desktop-preflight.json",
    "source-binding.json", "lane-outcomes.json", "native-artifact-inventory.json",
    "invocation.json", "contract-summary.json", "harness-report.json",
    "native-report.json", "shared-lifecycle-summary.json", "cleanup.json",
    "completion.json", "host-run.json", "assigned-job.json", "tool-inputs.json",
    "platform-gate.json", "display-matrix.json", "manifest.json",
    "native-readiness.json", "process.json",
    "mpv-observations.jsonl", "shared-lifecycle-evidence.jsonl",
    "shared-lifecycle-merged.jsonl", "primary-lifecycle.jsonl",
    "mpv-observation.jsonl", "gui-lifecycle.jsonl", "real-mpv-state.json",
    "session-exchange.json", "menu-interactions.json", "owned-mpv-recovery.json",
    "faulting-http-recovery.json", "hard-media-failure.json", "stalled-http.json", "diagnostic.json",
}
SECRET_KEY = re.compile(r"password|token|secret|credential|authorization|cookie|private.?key|environment|config_text", re.I)
PATH_KEY = re.compile(r"(?:path|directory|root|endpoint|username|user_name|user|computer|identity|session_name|url|host)$", re.I)
WINDOWS_PATH = re.compile(r"(?:\\\\[?.]\\)?[A-Za-z]:[\\/][^\s\"'<>]*|\\\\[^\s\"'<>]+")
UNIX_PATH = re.compile(r"(?<![\w:])/(?:home|Users|tmp|var|private|mnt|opt|run)/[^\s\"'<>]*")
URL = re.compile(r"(?:https?|plex)://[^\s\"'<>]+", re.I)
TOKEN = re.compile(r"\b(?:gh[pousr]_[A-Za-z0-9_]+|github_pat_[A-Za-z0-9_]+|Bearer\s+\S+|token[=:]\S+|password[=:]\S+)", re.I)
IDENTIFIER = re.compile(r"^[A-Za-z0-9_.-]{1,100}$")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def redact_text(value: str, secrets: tuple[str, ...]) -> str:
    for secret in secrets:
        value = value.replace(secret, "<redacted>")
    # A path with spaces cannot safely be trimmed by a token regex. Withhold
    # the entire freeform field; structured event/result/PID fields survive.
    if WINDOWS_PATH.search(value) or UNIX_PATH.search(value) or URL.search(value):
        return "<private-context>"
    value = TOKEN.sub("<redacted>", value)
    return value[:2048]


def project(value: Any, secrets: tuple[str, ...], *, key: str = "", depth: int = 0,
            identities: dict[str, str] | None = None) -> Any:
    if identities is None:
        identities = {}
    if depth > 16:
        return "<depth-limit>"
    if SECRET_KEY.search(key):
        return "<redacted>"
    if isinstance(value, dict):
        # File inventories often put private paths in keys, not values.
        return {redact_text(str(k), secrets): project(v, secrets, key=str(k), depth=depth + 1, identities=identities)
                for k, v in list(value.items())[:MAX_RECORDS]}
    if isinstance(value, list):
        return [project(v, secrets, depth=depth + 1, identities=identities) for v in value[-MAX_RECORDS:]]
    if isinstance(value, str):
        if PATH_KEY.search(key):
            # Preserve equality/foreign-endpoint attribution within a bundle
            # without publishing the private spelling or a guessable hash.
            return identities.setdefault(value, f"private-identity-{len(identities) + 1:03}")
        return redact_text(value, secrets)
    if value is None or type(value) in (bool, int, float):
        return value
    return "<unsupported>"


def is_link(path: Path) -> bool:
    info = path.lstat()
    return path.is_symlink() or bool(getattr(info, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT)


def discover(root: Path) -> list[Path]:
    """Never follow a symlink/junction, including an ancestor of the input root."""
    for parent in (root, *root.parents):
        if parent.exists() and is_link(parent):
            raise ValueError("diagnostic input contains a link or reparse point")
    if not root.is_dir():
        return []
    result = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if not is_link(Path(directory) / d)
                         and d not in {".git", "_diag", "node_modules"})
        for name in sorted(files):
            candidate = Path(directory) / name
            if (name in SAFE_FILES or re.fullmatch(r"record-[0-9]{3}\.json", name)) and not is_link(candidate):
                result.append(candidate)
                if len(result) > MAX_FILES:
                    raise ValueError("diagnostic input exceeds the bounded file inventory")
    return result


def export(root: Path, output: Path, *, source_sha: str, run_id: str,
           run_attempt: int, stage: str, cleanup: str,
           mode_outcomes: dict[str, str], secrets: tuple[str, ...] = ()) -> dict:
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise ValueError("source SHA must be immutable lowercase 40-hex")
    if not IDENTIFIER.fullmatch(run_id) or not IDENTIFIER.fullmatch(stage) or type(run_attempt) is not int or run_attempt < 1:
        raise ValueError("invalid diagnostic attempt identity")
    if cleanup not in {"passed", "failed", "unavailable", "pending"}:
        raise ValueError("invalid cleanup outcome")
    for name, status in mode_outcomes.items():
        if not IDENTIFIER.fullmatch(name) or status not in {"success", "failure", "cancelled", "skipped", "unavailable"}:
            raise ValueError("invalid native mode outcome")
    output = output.absolute()
    for parent in output.parents:
        if parent.exists() and is_link(parent):
            raise ValueError("diagnostic output contains a link or reparse point")
    output.mkdir(parents=True, exist_ok=False)
    entries: list[dict] = []
    identities: dict[str, str] = {}
    try:
        paths = discover(root.absolute())
    except (OSError, ValueError):
        paths = []
        entries.append({"status": "unavailable", "reason": "input-inventory-unavailable"})
    for number, path in enumerate(paths):
        # Use ordinal output names: an artifact's relative directory may itself
        # contain a user's name, media title, or private IPC endpoint.
        entry = {"id": number, "kind": path.name, "status": "unavailable"}
        try:
            if path.stat().st_size > MAX_FILE_BYTES:
                raise ValueError("oversize")
            raw = path.read_bytes()
            if len(raw) > MAX_FILE_BYTES:
                raise ValueError("oversize")
            decoded = raw.decode("utf-8-sig")
            if path.suffix == ".jsonl":
                lines = [line for line in decoded.splitlines() if line.strip()]
                value = [json.loads(line) for line in lines]
                entry["record_count"] = len(value)
                entry["retained_records"] = min(len(value), MAX_RECORDS)
            else:
                value = json.loads(decoded)
            projected = project(value, secrets, identities=identities)
            data = (json.dumps(projected, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
            name = f"record-{number:03}.json"
            (output / name).write_bytes(data)
            entry.update(status="exported", file=name, sha256=digest(data), size_bytes=len(data))
        except (OSError, ValueError, TypeError, UnicodeError, RecursionError):
            entry["reason"] = "malformed-oversize-or-unreadable-structured-record"
        entries.append(entry)
    if not paths:
        entries.append({"status": "unavailable", "reason": "no-structured-evidence-produced"})
    result = {
        "schema_version": 1, "kind": KIND, "authoritative": False,
        "source_sha": source_sha, "run_id": run_id, "run_attempt": run_attempt,
        "stage": stage, "cleanup": cleanup, "mode_outcomes": mode_outcomes,
        "disposition": "unclassified", "records": entries,
        "withheld": ["raw logs", "screenshots", "configuration files", "runner credentials"],
    }
    (output / "diagnostic.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["export"])
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--run-attempt", type=int, required=True)
    parser.add_argument("--stage", required=True)
    parser.add_argument("--cleanup", required=True)
    parser.add_argument("--mode-outcome", action="append", default=[])
    args = parser.parse_args()
    secrets = tuple(value for name, value in os.environ.items()
                    if SECRET_KEY.search(name) and len(value) >= 4)
    modes = {}
    for item in args.mode_outcome:
        key, separator, value = item.partition("=")
        if not separator or key in modes:
            parser.error("mode-outcome must be unique name=status")
        modes[key] = value or "unavailable"
    export(args.root, args.output, source_sha=args.source_sha, run_id=args.run_id,
           run_attempt=args.run_attempt, stage=args.stage, cleanup=args.cleanup,
           mode_outcomes=modes, secrets=secrets)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
