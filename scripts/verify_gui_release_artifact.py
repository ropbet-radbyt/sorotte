#!/usr/bin/env python3
"""Verify and exercise the exact Sorotte GUI release artifact selected for upload."""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unicodedata
import zipfile
from datetime import datetime
from pathlib import Path, PurePosixPath

# The server and GUI consumers deliberately share only the adversarial archive/JSON
# primitives. Their package identities, manifests, inventories, and runtime proofs
# remain independent.
try:
    from verify_server_release_artifact import (
        SOURCE_SHA_RE,
        MAX_ARCHIVE_BYTES,
        MAX_EXPANDED_BYTES,
        MAX_FILE_BYTES,
        VerificationError,
        _copy_member,
        _load_manifest,
        _require_exact_object_keys,
        _require_regular_file,
        _validated_member_path,
        safe_extract_archive,
        sha256_file,
        verify_checksum,
        write_report,
    )
except ModuleNotFoundError:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from verify_server_release_artifact import (  # type: ignore[no-redef]
        SOURCE_SHA_RE,
        MAX_ARCHIVE_BYTES,
        MAX_EXPANDED_BYTES,
        MAX_FILE_BYTES,
        VerificationError,
        _copy_member,
        _load_manifest,
        _require_exact_object_keys,
        _require_regular_file,
        _validated_member_path,
        safe_extract_archive,
        sha256_file,
        verify_checksum,
        write_report,
    )


ARCHIVE_RE = re.compile(
    r"^sorotte-gui-(?P<version>[0-9A-Za-z][0-9A-Za-z.+-]*)-windows-x86_64\.zip$"
)
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
UTC_TIMESTAMP_RE = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
UPDATE_MANIFEST = "sorotte-update-manifest.json"
INSTALL_MANIFEST = "sorotte-install.json"
GUI_EXE = "sorotte-gui.exe"
UPDATER_EXE = "sorotte-gui-updater.exe"
BOOTSTRAP_EXE = "sorotte-gui-updater-bootstrap.exe"
JOURNAL_FILE = ".sorotte-update-journal-v1.jsonl"
PACKAGE_PAYLOADS = {
    GUI_EXE,
    UPDATER_EXE,
    "README.md",
    "LICENSE",
    "resources/sorotte_syncplayintf.lua",
}
PACKAGE_FILES = PACKAGE_PAYLOADS | {INSTALL_MANIFEST}
SYMBOL_FILES = {
    "sorotte_gui.pdb",
    "sorotte-gui.pdb",
    "sorotte_gui_updater.pdb",
    "sorotte-gui-updater.pdb",
}
RUNTIME_TIMEOUT_SECONDS = 30.0


def _path(root: Path, relative: str) -> Path:
    return root.joinpath(*PurePosixPath(relative).parts)


def _canonical_gui_member_path(raw_path: str, *, is_directory: bool) -> str:
    if not raw_path:
        raise VerificationError("archive contains an empty member path")
    if "\x00" in raw_path or any(ord(character) < 32 for character in raw_path):
        raise VerificationError(
            f"archive member contains a control character: {raw_path!r}"
        )
    if unicodedata.normalize("NFC", raw_path) != raw_path:
        raise VerificationError(
            f"archive member path is not NFC-normalized: {raw_path!r}"
        )
    if "/" in raw_path and "\\" in raw_path:
        raise VerificationError(
            f"GUI archive member mixes path separators: {raw_path!r}"
        )
    canonical = raw_path.replace("\\", "/")
    return _validated_member_path(canonical, is_directory=is_directory)


def safe_extract_gui_archive(
    archive_path: Path,
    destination: Path,
) -> list[str]:
    """Extract the closed Windows GUI inventory using the updater's separator contract."""

    _require_regular_file(archive_path, "GUI release archive")
    archive_size = archive_path.stat().st_size
    if archive_size <= 0:
        raise VerificationError(f"GUI release archive is empty: {archive_path}")
    if archive_size > MAX_ARCHIVE_BYTES:
        raise VerificationError(
            f"GUI release archive exceeds the size limit: {archive_path}"
        )
    if destination.exists():
        raise VerificationError(
            f"extraction destination must not already exist: {destination}"
        )
    destination.mkdir(parents=True)
    observed: set[str] = set()
    folded: dict[str, str] = {}
    normalized_backslashes: list[str] = []
    expanded_bytes = 0
    normalized_infos: list[tuple[zipfile.ZipInfo, str]] = []
    try:
        with zipfile.ZipFile(archive_path) as archive:
            for info in archive.infolist():
                if info.flag_bits & 0x1:
                    raise VerificationError(
                        f"encrypted ZIP member is not allowed: {info.filename!r}"
                    )
                is_directory = info.is_dir()
                mode = (info.external_attr >> 16) & 0xFFFF
                file_type = stat.S_IFMT(mode)
                if stat.S_ISLNK(mode):
                    raise VerificationError(
                        f"ZIP symbolic link is not allowed: {info.filename!r}"
                    )
                if is_directory:
                    raise VerificationError(
                        f"GUI archive contains an unexpected directory entry: {info.filename!r}"
                    )
                if file_type not in (0, stat.S_IFREG):
                    raise VerificationError(
                        f"ZIP special file is not allowed: {info.filename!r}"
                    )
                relative = _canonical_gui_member_path(
                    info.filename,
                    is_directory=False,
                )
                previous = folded.get(relative.casefold())
                if relative in observed or previous is not None:
                    raise VerificationError(
                        f"GUI archive contains duplicate or case-colliding paths: "
                        f"{previous or relative!r} and {relative!r}"
                    )
                if info.file_size <= 0:
                    raise VerificationError(
                        f"archive member must not be empty: {relative}"
                    )
                if info.file_size > MAX_FILE_BYTES:
                    raise VerificationError(
                        f"archive member exceeds the size limit: {relative}"
                    )
                expanded_bytes += info.file_size
                if expanded_bytes > MAX_EXPANDED_BYTES:
                    raise VerificationError(
                        "GUI archive exceeds the expanded-size limit"
                    )
                observed.add(relative)
                folded[relative.casefold()] = relative
                if "\\" in info.filename:
                    normalized_backslashes.append(relative)
                normalized_infos.append((info, relative))
            if observed != PACKAGE_FILES:
                raise VerificationError(
                    f"archive inventory mismatch; "
                    f"missing={sorted(PACKAGE_FILES - observed) or 'none'}, "
                    f"extra={sorted(observed - PACKAGE_FILES) or 'none'}"
                )
            for info, relative in normalized_infos:
                output_path = _path(destination, relative)
                with archive.open(info, "r") as source:
                    _copy_member(source, output_path, info.file_size)
    except (OSError, zipfile.BadZipFile, NotImplementedError) as error:
        if isinstance(error, VerificationError):
            raise
        raise VerificationError(f"could not safely read GUI ZIP archive: {error}") from error
    return sorted(normalized_backslashes)


def _expect_scalar(
    value: dict[str, object],
    key: str,
    expected: object,
    description: str,
) -> None:
    actual = value[key]
    if type(actual) is not type(expected) or actual != expected:
        raise VerificationError(
            f"{description} {key} mismatch: expected {expected!r}, received {actual!r}"
        )


def _validate_utc_timestamp(value: object, description: str) -> str:
    if not isinstance(value, str) or UTC_TIMESTAMP_RE.fullmatch(value) is None:
        raise VerificationError(
            f"{description} must be a canonical UTC timestamp in YYYY-MM-DDTHH:MM:SSZ form"
        )
    try:
        datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ")
    except ValueError as error:
        raise VerificationError(f"{description} is not a valid UTC timestamp: {value}") from error
    return value


def find_release_archive(artifacts_dir: Path) -> tuple[Path, str]:
    if not artifacts_dir.is_dir():
        raise VerificationError(f"artifact directory is missing: {artifacts_dir}")
    candidates = [
        path
        for path in artifacts_dir.iterdir()
        if ARCHIVE_RE.fullmatch(path.name) is not None
    ]
    for path in candidates:
        _require_regular_file(path, "GUI release archive")
    if len(candidates) != 1:
        names = ", ".join(sorted(path.name for path in candidates)) or "none"
        raise VerificationError(
            f"expected exactly one primary GUI release archive, found {len(candidates)}: {names}"
        )
    match = ARCHIVE_RE.fullmatch(candidates[0].name)
    assert match is not None
    return candidates[0], match.group("version")


def _symbols_archive_name(archive_path: Path) -> str:
    return f"{archive_path.stem}-symbols.zip"


def _symbols_declared(artifacts_dir: Path, archive_path: Path) -> bool:
    symbols_name = _symbols_archive_name(archive_path)
    return (artifacts_dir / symbols_name).exists() or (
        artifacts_dir / f"{symbols_name}.sha256"
    ).exists()


def _validate_artifact_directory_entries(
    artifacts_dir: Path,
    archive_path: Path,
    *,
    symbols_present: bool,
) -> None:
    allowed = {
        archive_path.name,
        f"{archive_path.name}.sha256",
        UPDATE_MANIFEST,
    }
    if symbols_present:
        symbols_name = _symbols_archive_name(archive_path)
        allowed.update({symbols_name, f"{symbols_name}.sha256"})
    actual: set[str] = set()
    for path in artifacts_dir.iterdir():
        if path.is_dir():
            raise VerificationError(
                f"artifact directory contains an unexpected directory: {path.name}"
            )
        _require_regular_file(path, "artifact directory entry")
        actual.add(path.name)
    if actual != allowed:
        raise VerificationError(
            f"artifact directory inventory mismatch; "
            f"missing={sorted(allowed - actual) or 'none'}, "
            f"extra={sorted(actual - allowed) or 'none'}"
        )


def verify_update_manifest(
    path: Path,
    *,
    archive_path: Path,
    archive_digest: str,
    version: str,
    expected_source_sha: str,
    expected_channel: str,
) -> dict[str, object]:
    manifest = _load_manifest(path)
    _require_exact_object_keys(
        manifest,
        {
            "schema",
            "app",
            "channel",
            "version",
            "git_sha",
            "created_at_utc",
            "target",
            "package",
            "sha256",
        },
        "update manifest",
    )
    for key, expected in {
        "schema": "sorotte-gui-update-manifest-v1",
        "app": "sorotte-gui",
        "channel": expected_channel,
        "version": version,
        "git_sha": expected_source_sha,
        "target": "windows-x86_64",
        "package": archive_path.name,
        "sha256": archive_digest,
    }.items():
        _expect_scalar(manifest, key, expected, "update manifest")
    _validate_utc_timestamp(manifest["created_at_utc"], "update manifest created_at_utc")
    return manifest


def verify_install_manifest(
    package_root: Path,
    *,
    update_manifest: dict[str, object],
) -> tuple[dict[str, object], list[dict[str, object]], list[str]]:
    manifest = _load_manifest(package_root / INSTALL_MANIFEST)
    _require_exact_object_keys(
        manifest,
        {
            "schema",
            "app",
            "channel",
            "version",
            "git_sha",
            "created_at_utc",
            "target",
            "files",
        },
        "install manifest",
    )
    for key, expected in {
        "schema": "sorotte-gui-install-manifest-v2",
        "app": "sorotte-gui",
        "channel": update_manifest["channel"],
        "version": update_manifest["version"],
        "git_sha": update_manifest["git_sha"],
        "created_at_utc": update_manifest["created_at_utc"],
        "target": "windows-x86_64",
    }.items():
        _expect_scalar(manifest, key, expected, "install manifest")
    _validate_utc_timestamp(manifest["created_at_utc"], "install manifest created_at_utc")

    files = manifest["files"]
    if not isinstance(files, list):
        raise VerificationError("install manifest files must be an array")
    observed: set[str] = set()
    observed_casefolded: dict[str, str] = {}
    verified: list[dict[str, object]] = []
    windows_separator_entries: list[str] = []
    for index, entry in enumerate(files):
        if not isinstance(entry, dict):
            raise VerificationError(f"install manifest files[{index}] must be an object")
        _require_exact_object_keys(
            entry,
            {"path", "sha256"},
            f"install manifest files[{index}]",
        )
        relative = entry["path"]
        digest = entry["sha256"]
        if not isinstance(relative, str):
            raise VerificationError(
                f"install manifest files[{index}].path must be a string"
            )
        normalized = _canonical_gui_member_path(relative, is_directory=False)
        previous = observed_casefolded.get(normalized.casefold())
        if previous is not None:
            raise VerificationError(
                f"install manifest contains duplicate or case-colliding paths: "
                f"{previous!r} and {relative!r}"
            )
        if normalized not in PACKAGE_PAYLOADS:
            raise VerificationError(
                f"install manifest contains unexpected file entry: {relative}"
            )
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise VerificationError(
                f"install manifest contains invalid SHA-256 for {relative}"
            )
        payload = _path(package_root, normalized)
        _require_regular_file(payload, f"install payload {normalized}")
        actual_digest = sha256_file(payload)
        if actual_digest != digest:
            raise VerificationError(
                f"install manifest digest mismatch for {normalized}: "
                f"expected {digest}, received {actual_digest}"
            )
        observed.add(normalized)
        observed_casefolded[normalized.casefold()] = relative
        if "\\" in relative:
            windows_separator_entries.append(normalized)
        verified.append(
            {
                "path": normalized,
                "manifestPath": relative,
                "size": payload.stat().st_size,
                "sha256": actual_digest,
            }
        )
    if observed != PACKAGE_PAYLOADS:
        raise VerificationError(
            f"install manifest file inventory mismatch; "
            f"missing={sorted(PACKAGE_PAYLOADS - observed) or 'none'}, "
            f"extra={sorted(observed - PACKAGE_PAYLOADS) or 'none'}"
        )
    return (
        manifest,
        sorted(verified, key=lambda entry: str(entry["path"])),
        sorted(windows_separator_entries),
    )


def _read_symbols_inventory(symbols_path: Path) -> set[str]:
    _require_regular_file(symbols_path, "GUI symbols archive")
    observed: set[str] = set()
    folded: dict[str, str] = {}
    try:
        with zipfile.ZipFile(symbols_path) as archive:
            for info in archive.infolist():
                if info.is_dir():
                    raise VerificationError(
                        f"symbols archive contains an unexpected directory: {info.filename}"
                    )
                relative = _validated_member_path(info.filename, is_directory=False)
                previous = folded.get(relative.casefold())
                if relative in observed or previous is not None:
                    raise VerificationError(
                        f"symbols archive contains duplicate or case-colliding entries: "
                        f"{previous or relative!r} and {relative!r}"
                    )
                if relative not in SYMBOL_FILES:
                    raise VerificationError(
                        f"symbols archive contains unexpected file: {relative}"
                    )
                observed.add(relative)
                folded[relative.casefold()] = relative
    except (OSError, zipfile.BadZipFile) as error:
        if isinstance(error, VerificationError):
            raise
        raise VerificationError(f"could not inspect GUI symbols archive: {error}") from error
    if not observed:
        raise VerificationError("symbols archive must contain at least one known PDB")
    return observed


def _verify_optional_symbols(
    artifacts_dir: Path,
    archive_path: Path,
    extraction_parent: Path,
) -> dict[str, object] | None:
    symbols_name = _symbols_archive_name(archive_path)
    symbols_path = artifacts_dir / symbols_name
    checksum_path = artifacts_dir / f"{symbols_name}.sha256"
    if not symbols_path.exists() and not checksum_path.exists():
        return None
    _, digest = verify_checksum(symbols_path)
    inventory = _read_symbols_inventory(symbols_path)
    safe_extract_archive(
        symbols_path,
        extraction_parent / "symbols",
        root_name=None,
        expected_relative_files=inventory,
    )
    return {
        "archive": symbols_name,
        "checksum": checksum_path.name,
        "sha256": digest,
        "files": sorted(inventory),
    }


def _find_visible_window(pid: int) -> str | None:
    if os.name != "nt":
        return None
    from ctypes import wintypes

    titles: list[str] = []
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def visit(window: int, _parameter: int) -> bool:
        process_id = wintypes.DWORD()
        user32.GetWindowThreadProcessId(window, ctypes.byref(process_id))
        if process_id.value != pid or not user32.IsWindowVisible(window):
            return True
        length = user32.GetWindowTextLengthW(window)
        if length <= 0:
            return True
        buffer = ctypes.create_unicode_buffer(length + 1)
        user32.GetWindowTextW(window, buffer, len(buffer))
        title = buffer.value.strip()
        if title:
            titles.append(title)
        return True

    if not user32.EnumWindows(callback_type(visit), 0):
        error = ctypes.get_last_error()
        if error:
            raise VerificationError(f"could not enumerate GUI windows (Win32 error {error})")
    return titles[0] if titles else None


def smoke_test_gui(gui_path: Path, runtime_root: Path) -> dict[str, object]:
    if os.name != "nt":
        raise VerificationError("GUI runtime smoke is supported only on Windows")
    _require_regular_file(gui_path, "packaged GUI executable")
    profile_root = runtime_root / "gui-profile"
    profile_root.mkdir(parents=True)
    config_path = profile_root / "sorotte.ini"
    environment = dict(os.environ)
    for name in list(environment):
        if name.upper().startswith(("SOROTTE_", "SYNCPLAY_")):
            environment.pop(name, None)
    environment.update(
        {
            "APPDATA": str(profile_root / "appdata"),
            "LOCALAPPDATA": str(profile_root / "localappdata"),
            "SOROTTE_CLIENT_CONFIG_PATH": str(config_path),
            "SOROTTE_CLIENT_INSTALL_ROOT": str(profile_root / "install-root"),
            "SOROTTE_GUI_TEST_DISABLE_STARTUP_SAVED_CONNECT": "true",
            "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS": "[]",
            "SOROTTE_GUI_UPDATE_CHECK_RESPONSE": (
                '{"version-status":"uptodate","version-message":'
                '"artifact verification fixture"}'
            ),
        }
    )
    started = time.monotonic()
    process: subprocess.Popen[bytes] | None = None
    title: str | None = None
    try:
        process = subprocess.Popen(
            [str(gui_path)],
            cwd=gui_path.parent,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        deadline = time.monotonic() + 15.0
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise VerificationError(
                    f"packaged GUI exited before exposing a visible main window "
                    f"(exit {process.returncode})"
                )
            title = _find_visible_window(process.pid)
            if title is not None:
                break
            time.sleep(0.05)
        if title is None:
            raise VerificationError(
                "packaged GUI did not expose a visible main window before the deadline"
            )
    except OSError as error:
        raise VerificationError(f"packaged GUI launch failed: {error}") from error
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
    return {
        "performed": True,
        "visibleMainWindow": True,
        "windowTitle": title,
        "networkIsolation": (
            "fresh config; saved connect disabled; public-server and update sources injected"
        ),
        "elapsedMilliseconds": round((time.monotonic() - started) * 1000),
    }


def _manifest_bytes(version: str, payloads: dict[str, bytes]) -> bytes:
    manifest = {
        "schema": "sorotte-gui-install-manifest-v2",
        "app": "sorotte-gui",
        "version": version,
        "target": "windows-x86_64",
        "files": [
            {
                "path": relative,
                "sha256": hashlib.sha256(payloads[relative]).hexdigest(),
            }
            for relative in sorted(payloads)
        ],
    }
    return (json.dumps(manifest, separators=(",", ":"), sort_keys=True) + "\n").encode()


def _seed_old_install(target: Path, packaged_updater: Path) -> dict[str, dict[str, object]]:
    target.mkdir(parents=True)
    payloads = {
        relative: (
            packaged_updater.read_bytes()
            if relative == UPDATER_EXE
            else f"old artifact-consumer fixture for {relative}\n".encode()
        )
        for relative in PACKAGE_PAYLOADS
    }
    for relative, body in payloads.items():
        destination = _path(target, relative)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(body)
    manifest = _manifest_bytes("0.0.0-artifact-verifier", payloads)
    (target / INSTALL_MANIFEST).write_bytes(manifest)
    snapshot = dict(payloads)
    snapshot[INSTALL_MANIFEST] = manifest
    return {
        relative: {"size": len(body), "sha256": hashlib.sha256(body).hexdigest()}
        for relative, body in snapshot.items()
    }


def _snapshot(root: Path, relative_files: set[str]) -> dict[str, dict[str, object]]:
    result: dict[str, dict[str, object]] = {}
    for relative in relative_files:
        path = _path(root, relative)
        _require_regular_file(path, f"runtime target {relative}")
        result[relative] = {"size": path.stat().st_size, "sha256": sha256_file(path)}
    return result


def _assert_snapshot(
    root: Path,
    expected: dict[str, dict[str, object]],
    description: str,
) -> None:
    actual = _snapshot(root, set(expected))
    if actual != expected:
        changed = sorted(
            relative
            for relative in set(actual) | set(expected)
            if actual.get(relative) != expected.get(relative)
        )
        raise VerificationError(f"{description} snapshot mismatch: {changed}")


def _transaction_leftovers(target: Path) -> list[str]:
    leftovers: list[str] = []
    for path in target.rglob("*"):
        relative = path.relative_to(target).as_posix()
        name = path.name
        if (
            name == JOURNAL_FILE
            or name.startswith(".sorotte-update-stage-")
            or ".sorotte-new-" in name
            or ".sorotte-old-" in name
        ):
            leftovers.append(relative)
    return sorted(leftovers)


def _updater_arguments(
    *,
    package_path: Path,
    package_digest: str,
    target: Path,
    log_path: Path,
) -> list[str]:
    return [
        "--pid",
        str(2**32 - 1),
        "--package",
        str(package_path),
        "--package-sha256",
        package_digest,
        "--target-dir",
        str(target),
        "--target-exe",
        GUI_EXE,
        "--log",
        str(log_path),
    ]


def _wait_for_successful_update(
    target: Path,
    expected: dict[str, dict[str, object]],
    log_path: Path,
) -> str:
    deadline = time.monotonic() + RUNTIME_TIMEOUT_SECONDS
    last_log = ""
    while time.monotonic() < deadline:
        last_log = (
            log_path.read_text(encoding="utf-8", errors="replace")
            if log_path.exists()
            else ""
        )
        if "update completed" in last_log and not (target / JOURNAL_FILE).exists():
            _assert_snapshot(target, expected, "completed update")
            return last_log
        time.sleep(0.025)
    raise VerificationError(
        "detached packaged updater did not complete before the deadline; "
        f"log={last_log[-1000:]!r}"
    )


def smoke_test_updater_success(
    package_root: Path,
    package_path: Path,
    package_digest: str,
    runtime_root: Path,
) -> dict[str, object]:
    target = runtime_root / "update-success"
    _seed_old_install(target, package_root / UPDATER_EXE)
    expected = _snapshot(package_root, PACKAGE_FILES)
    log_path = runtime_root / "update-success.log"
    command = [
        str(target / UPDATER_EXE),
        *_updater_arguments(
            package_path=package_path,
            package_digest=package_digest,
            target=target,
            log_path=log_path,
        ),
    ]
    started = time.monotonic()
    try:
        completed = subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=20,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise VerificationError(f"installed packaged updater bootstrap failed: {error}") from error
    if completed.returncode != 0:
        log = log_path.read_text(encoding="utf-8", errors="replace") if log_path.exists() else ""
        raise VerificationError(
            f"installed packaged updater bootstrap exited {completed.returncode}: "
            f"{log or 'no updater log was written'}"
        )
    log = _wait_for_successful_update(target, expected, log_path)
    leftovers = _transaction_leftovers(target)
    if leftovers:
        raise VerificationError(f"successful update left transaction artifacts: {leftovers}")
    return {
        "performed": True,
        "installedBootstrap": True,
        "selfReplacement": True,
        "exactPackageInstalled": True,
        "transactionArtifactsRemoved": True,
        "logSha256": hashlib.sha256(log.encode()).hexdigest(),
        "elapsedMilliseconds": round((time.monotonic() - started) * 1000),
    }


def smoke_test_updater_rollback(
    package_root: Path,
    package_path: Path,
    package_digest: str,
    runtime_root: Path,
) -> dict[str, object]:
    target = runtime_root / "update-rollback"
    original = _seed_old_install(target, package_root / UPDATER_EXE)
    helper_dir = target / f".sorotte-update-bootstrap-{os.getpid()}-{time.time_ns()}"
    helper_dir.mkdir()
    helper = helper_dir / BOOTSTRAP_EXE
    shutil.copy2(package_root / UPDATER_EXE, helper)
    helper_digest = sha256_file(helper)
    log_path = runtime_root / "update-rollback.log"
    readme_path = target / "README.md"
    readme_path.chmod(stat.S_IREAD)
    command = [
        str(helper),
        *_updater_arguments(
            package_path=package_path,
            package_digest=package_digest,
            target=target,
            log_path=log_path,
        ),
        "--detached-helper-sha256",
        helper_digest,
    ]
    started = time.monotonic()
    try:
        completed = subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=RUNTIME_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise VerificationError(f"packaged updater rollback experiment failed: {error}") from error
    finally:
        readme_path.chmod(stat.S_IREAD | stat.S_IWRITE)
    output = (completed.stdout + completed.stderr).decode(
        "utf-8",
        errors="replace",
    ).strip()
    if completed.returncode == 0:
        raise VerificationError("fault-injected packaged updater unexpectedly succeeded")
    for oracle in (
        "failed atomically replacing",
        "all changed files were rolled back",
    ):
        if oracle not in output:
            raise VerificationError(
                f"fault-injected updater did not report {oracle!r}: {output[-1000:]}"
            )
    _assert_snapshot(target, original, "rolled-back update")
    leftovers = _transaction_leftovers(target)
    if leftovers:
        raise VerificationError(f"rolled-back update left transaction artifacts: {leftovers}")
    shutil.rmtree(helper_dir)
    return {
        "performed": True,
        "fault": "read-only README replacement target",
        "nonzeroExit": True,
        "rollbackOracle": "all changed files were rolled back",
        "originalInstallRestored": True,
        "transactionArtifactsRemoved": True,
        "errorSha256": hashlib.sha256(output.encode()).hexdigest(),
        "elapsedMilliseconds": round((time.monotonic() - started) * 1000),
    }


def run_runtime_experiments(
    package_root: Path,
    package_path: Path,
    package_digest: str,
    runtime_root: Path,
) -> dict[str, object]:
    if os.name != "nt":
        raise VerificationError("GUI release runtime experiments require Windows")
    runtime_root.mkdir()
    started = time.monotonic()
    gui = smoke_test_gui(package_root / GUI_EXE, runtime_root)
    update = smoke_test_updater_success(
        package_root,
        package_path,
        package_digest,
        runtime_root,
    )
    rollback = smoke_test_updater_rollback(
        package_root,
        package_path,
        package_digest,
        runtime_root,
    )
    return {
        "performed": True,
        "guiLaunch": gui,
        "updaterSuccess": update,
        "updaterRollback": rollback,
        "elapsedMilliseconds": round((time.monotonic() - started) * 1000),
    }


def verify_release(
    artifacts_dir: Path,
    expected_source_sha: str,
    expected_channel: str,
    *,
    runtime_smoke: bool = True,
    work_dir: Path | None = None,
) -> dict[str, object]:
    expected_source_sha = expected_source_sha.lower()
    if SOURCE_SHA_RE.fullmatch(expected_source_sha) is None:
        raise VerificationError("expected source SHA must be exactly 40 hexadecimal characters")
    if expected_channel not in {"stable", "dev"}:
        raise VerificationError("expected channel must be 'stable' or 'dev'")
    artifacts_dir = artifacts_dir.resolve()
    archive_path, version = find_release_archive(artifacts_dir)
    symbols_declared = _symbols_declared(artifacts_dir, archive_path)
    _validate_artifact_directory_entries(
        artifacts_dir,
        archive_path,
        symbols_present=symbols_declared,
    )
    checksum_path, archive_digest = verify_checksum(archive_path)
    update_manifest_path = artifacts_dir / UPDATE_MANIFEST
    update_manifest = verify_update_manifest(
        update_manifest_path,
        archive_path=archive_path,
        archive_digest=archive_digest,
        version=version,
        expected_source_sha=expected_source_sha,
        expected_channel=expected_channel,
    )
    update_manifest_digest = sha256_file(update_manifest_path)
    if work_dir is None:
        work_dir = artifacts_dir.parent / "verification-work"
    work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="gui-artifact-", dir=work_dir) as temporary:
        extraction_parent = Path(temporary)
        package_root = extraction_parent / "package"
        windows_separator_entries = safe_extract_gui_archive(
            archive_path,
            package_root,
        )
        _, verified_files, manifest_windows_separator_entries = verify_install_manifest(
            package_root,
            update_manifest=update_manifest,
        )
        install_manifest_path = package_root / INSTALL_MANIFEST
        symbols = _verify_optional_symbols(
            artifacts_dir,
            archive_path,
            extraction_parent,
        )
        runtime = (
            run_runtime_experiments(
                package_root,
                archive_path,
                archive_digest,
                extraction_parent / "runtime",
            )
            if runtime_smoke
            else {"performed": False}
        )
        if sha256_file(archive_path) != archive_digest:
            raise VerificationError("GUI release archive changed while it was being verified")
        if sha256_file(update_manifest_path) != update_manifest_digest:
            raise VerificationError("GUI update manifest changed while it was being verified")
        _validate_artifact_directory_entries(
            artifacts_dir,
            archive_path,
            symbols_present=symbols_declared,
        )
        return {
            "schemaVersion": 1,
            "status": "verified",
            "archive": {
                "name": archive_path.name,
                "size": archive_path.stat().st_size,
                "sha256": archive_digest,
                "checksum": checksum_path.name,
                "checksumSha256": sha256_file(checksum_path),
                "windowsSeparatorEntries": windows_separator_entries,
            },
            "package": {
                "name": "sorotte-gui",
                "version": version,
                "platform": "windows",
                "architecture": "x86_64",
                "channel": expected_channel,
                "sourceSha": expected_source_sha,
                "updateManifest": UPDATE_MANIFEST,
                "updateManifestSha256": update_manifest_digest,
                "installManifestSha256": sha256_file(install_manifest_path),
                "manifestWindowsSeparatorEntries": manifest_windows_separator_entries,
                "files": verified_files,
            },
            "runtimeProof": runtime,
            "symbols": symbols,
        }


def failure_report(
    expected_source_sha: str,
    expected_channel: str,
    error: Exception,
) -> dict[str, object]:
    return {
        "schemaVersion": 1,
        "status": "failed",
        "expectedSourceSha": expected_source_sha.lower(),
        "expectedChannel": expected_channel,
        "error": str(error),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts-dir", type=Path, required=True)
    parser.add_argument("--expected-source-sha", required=True)
    parser.add_argument("--expected-channel", choices=("stable", "dev"), required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument(
        "--skip-runtime-smoke",
        action="store_true",
        help=(
            "validate archive structure, both manifests, payload bytes, and provenance "
            "without executing the packaged GUI or updater"
        ),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        report = verify_release(
            args.artifacts_dir,
            args.expected_source_sha,
            args.expected_channel,
            runtime_smoke=not args.skip_runtime_smoke,
        )
        write_report(report, args.report)
    except VerificationError as error:
        try:
            write_report(
                failure_report(
                    args.expected_source_sha,
                    args.expected_channel,
                    error,
                ),
                args.report,
            )
        except OSError as report_error:
            print(
                f"could not write GUI release failure report: {report_error}",
                file=sys.stderr,
            )
        print(f"GUI release artifact verification failed: {error}", file=sys.stderr)
        return 1
    print(
        f"verified {report['archive']['name']} from "
        f"{report['package']['sourceSha']} ({report['package']['channel']}) "
        f"and wrote {args.report}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
