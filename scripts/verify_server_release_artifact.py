#!/usr/bin/env python3
"""Verify and smoke-test the exact sorotte-server archive selected for upload."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import socket
import stat
import subprocess
import tarfile
import tempfile
import time
import unicodedata
import zipfile
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import BinaryIO, Callable, Iterable


MAX_ARCHIVE_BYTES = 512 * 1024 * 1024
MAX_FILE_BYTES = 256 * 1024 * 1024
MAX_EXPANDED_BYTES = 512 * 1024 * 1024
COPY_CHUNK_BYTES = 1024 * 1024
SOURCE_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
ARCHIVE_RE = re.compile(
    r"^sorotte-server-(?P<version>[0-9A-Za-z][0-9A-Za-z.+-]*)-"
    r"(?P<platform>windows|linux)-x86_64"
    r"(?P<suffix>\.zip|\.tar\.gz)$"
)
CHECKSUM_RE = re.compile(r"^(?P<digest>[0-9a-f]{64})  (?P<filename>[^\r\n]+)\r?\n?$")


class VerificationError(RuntimeError):
    """Raised when a release artifact violates the consumer contract."""


@dataclass(frozen=True)
class ArchiveIdentity:
    archive_name: str
    root_name: str
    version: str
    platform: str
    architecture: str
    suffix: str
    binary_name: str


@dataclass(frozen=True)
class ArchiveMember:
    path: str
    is_directory: bool
    size: int


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(COPY_CHUNK_BYTES):
            digest.update(chunk)
    return digest.hexdigest()


def _require_regular_file(path: Path, description: str) -> None:
    try:
        mode = path.lstat().st_mode
    except FileNotFoundError as error:
        raise VerificationError(f"{description} is missing: {path}") from error
    if path.is_symlink() or not stat.S_ISREG(mode):
        raise VerificationError(f"{description} must be a regular file: {path}")


def parse_archive_identity(archive_path: Path) -> ArchiveIdentity:
    match = ARCHIVE_RE.fullmatch(archive_path.name)
    if match is None:
        raise VerificationError(f"invalid server release archive name: {archive_path.name}")
    platform = match.group("platform")
    suffix = match.group("suffix")
    expected_suffix = ".zip" if platform == "windows" else ".tar.gz"
    if suffix != expected_suffix:
        raise VerificationError(
            f"{platform} server release must use {expected_suffix}, received {suffix}"
        )
    root_name = archive_path.name[: -len(suffix)]
    return ArchiveIdentity(
        archive_name=archive_path.name,
        root_name=root_name,
        version=match.group("version"),
        platform=platform,
        architecture="x86_64",
        suffix=suffix,
        binary_name="sorotte-server.exe" if platform == "windows" else "sorotte-server",
    )


def find_release_archive(artifacts_dir: Path) -> tuple[Path, ArchiveIdentity]:
    if not artifacts_dir.is_dir():
        raise VerificationError(f"artifact directory is missing: {artifacts_dir}")
    candidates: list[tuple[Path, ArchiveIdentity]] = []
    for path in artifacts_dir.iterdir():
        if ARCHIVE_RE.fullmatch(path.name):
            _require_regular_file(path, "release archive")
            candidates.append((path, parse_archive_identity(path)))
    if len(candidates) != 1:
        names = ", ".join(sorted(path.name for path, _ in candidates)) or "none"
        raise VerificationError(
            f"expected exactly one primary server release archive, found {len(candidates)}: {names}"
        )
    return candidates[0]


def verify_checksum(archive_path: Path) -> tuple[Path, str]:
    checksum_path = archive_path.with_name(f"{archive_path.name}.sha256")
    _require_regular_file(checksum_path, "archive checksum")
    if checksum_path.stat().st_size > 256:
        raise VerificationError(f"checksum file is unexpectedly large: {checksum_path}")
    try:
        checksum_text = checksum_path.read_text(encoding="ascii")
    except UnicodeDecodeError as error:
        raise VerificationError("checksum file must contain ASCII text") from error
    match = CHECKSUM_RE.fullmatch(checksum_text)
    if match is None:
        raise VerificationError(
            "checksum must be one lowercase SHA-256 line in '<digest>  <filename>' format"
        )
    if match.group("filename") != archive_path.name:
        raise VerificationError(
            f"checksum names {match.group('filename')!r}, expected {archive_path.name!r}"
        )
    expected_digest = match.group("digest")
    actual_digest = sha256_file(archive_path)
    if actual_digest != expected_digest:
        raise VerificationError(
            f"archive checksum mismatch: expected {expected_digest}, received {actual_digest}"
        )
    return checksum_path, actual_digest


def _validated_member_path(raw_path: str, *, is_directory: bool) -> str:
    if not raw_path:
        raise VerificationError("archive contains an empty member path")
    if "\x00" in raw_path or any(ord(character) < 32 for character in raw_path):
        raise VerificationError(f"archive member contains a control character: {raw_path!r}")
    if "\\" in raw_path:
        raise VerificationError(f"archive member uses a backslash path: {raw_path!r}")
    if unicodedata.normalize("NFC", raw_path) != raw_path:
        raise VerificationError(f"archive member path is not NFC-normalized: {raw_path!r}")
    path_text = raw_path[:-1] if is_directory and raw_path.endswith("/") else raw_path
    if not path_text or path_text.startswith("/") or re.match(r"^[A-Za-z]:", path_text):
        raise VerificationError(f"archive member path is absolute: {raw_path!r}")
    path = PurePosixPath(path_text)
    if any(part in ("", ".", "..") for part in path.parts):
        raise VerificationError(f"archive member path is not normalized: {raw_path!r}")
    normalized = path.as_posix()
    if normalized != path_text:
        raise VerificationError(f"archive member path is not normalized: {raw_path!r}")
    return normalized


def _validate_inventory(
    members: Iterable[ArchiveMember],
    *,
    root_name: str | None,
    expected_files: set[str],
) -> None:
    observed_files: set[str] = set()
    observed_paths: set[str] = set()
    observed_casefolded: dict[str, str] = {}
    expanded_bytes = 0
    for member in members:
        path = member.path
        if path in observed_paths:
            raise VerificationError(f"archive contains duplicate member path: {path}")
        folded = path.casefold()
        previous = observed_casefolded.get(folded)
        if previous is not None:
            raise VerificationError(
                f"archive contains case-colliding member paths: {previous!r} and {path!r}"
            )
        observed_paths.add(path)
        observed_casefolded[folded] = path
        if member.is_directory:
            if root_name is None or path != root_name:
                raise VerificationError(f"archive contains unexpected directory: {path}")
            continue
        if member.size <= 0:
            raise VerificationError(f"archive member must not be empty: {path}")
        if member.size > MAX_FILE_BYTES:
            raise VerificationError(f"archive member exceeds the size limit: {path}")
        expanded_bytes += member.size
        if expanded_bytes > MAX_EXPANDED_BYTES:
            raise VerificationError("archive exceeds the expanded-size limit")
        observed_files.add(path)
    if observed_files != expected_files:
        missing = sorted(expected_files - observed_files)
        extra = sorted(observed_files - expected_files)
        raise VerificationError(
            f"archive inventory mismatch; missing={missing or 'none'}, extra={extra or 'none'}"
        )


def _copy_member(source: BinaryIO, destination: Path, expected_size: int) -> None:
    copied = 0
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("xb") as output:
        while chunk := source.read(COPY_CHUNK_BYTES):
            copied += len(chunk)
            if copied > expected_size or copied > MAX_FILE_BYTES:
                raise VerificationError(f"archive member expanded beyond its declared size: {destination}")
            output.write(chunk)
    if copied != expected_size:
        raise VerificationError(
            f"archive member size mismatch for {destination}: expected {expected_size}, read {copied}"
        )


def _extract_zip(
    archive_path: Path,
    destination: Path,
    *,
    root_name: str | None,
    expected_files: set[str],
) -> None:
    try:
        with zipfile.ZipFile(archive_path) as archive:
            infos = archive.infolist()
            members: list[ArchiveMember] = []
            normalized_infos: list[tuple[zipfile.ZipInfo, str, bool]] = []
            for info in infos:
                if info.flag_bits & 0x1:
                    raise VerificationError(f"encrypted ZIP member is not allowed: {info.filename!r}")
                is_directory = info.is_dir()
                mode = (info.external_attr >> 16) & 0xFFFF
                file_type = stat.S_IFMT(mode)
                if stat.S_ISLNK(mode):
                    raise VerificationError(f"ZIP symbolic link is not allowed: {info.filename!r}")
                if is_directory:
                    if file_type not in (0, stat.S_IFDIR):
                        raise VerificationError(
                            f"ZIP directory has an invalid file type: {info.filename!r}"
                        )
                elif file_type not in (0, stat.S_IFREG):
                    raise VerificationError(
                        f"ZIP special file is not allowed: {info.filename!r}"
                    )
                path = _validated_member_path(info.filename, is_directory=is_directory)
                members.append(ArchiveMember(path, is_directory, info.file_size))
                normalized_infos.append((info, path, is_directory))
            _validate_inventory(members, root_name=root_name, expected_files=expected_files)
            for info, path, is_directory in normalized_infos:
                output_path = destination.joinpath(*PurePosixPath(path).parts)
                if is_directory:
                    output_path.mkdir(parents=True, exist_ok=True)
                    continue
                with archive.open(info, "r") as source:
                    _copy_member(source, output_path, info.file_size)
    except (OSError, zipfile.BadZipFile, NotImplementedError) as error:
        if isinstance(error, VerificationError):
            raise
        raise VerificationError(f"could not safely read ZIP archive: {error}") from error


def _extract_tar(
    archive_path: Path,
    destination: Path,
    *,
    root_name: str | None,
    expected_files: set[str],
) -> None:
    try:
        with tarfile.open(archive_path, mode="r:gz") as archive:
            tar_members = archive.getmembers()
            members: list[ArchiveMember] = []
            normalized_members: list[tuple[tarfile.TarInfo, str, bool]] = []
            for member in tar_members:
                if member.isdir():
                    is_directory = True
                elif member.isfile():
                    is_directory = False
                    if member.issparse():
                        raise VerificationError(
                            f"sparse TAR member is not allowed: {member.name!r}"
                        )
                else:
                    raise VerificationError(
                        f"TAR links and special files are not allowed: {member.name!r}"
                    )
                path = _validated_member_path(member.name, is_directory=is_directory)
                members.append(ArchiveMember(path, is_directory, member.size))
                normalized_members.append((member, path, is_directory))
            _validate_inventory(members, root_name=root_name, expected_files=expected_files)
            for member, path, is_directory in normalized_members:
                output_path = destination.joinpath(*PurePosixPath(path).parts)
                if is_directory:
                    output_path.mkdir(parents=True, exist_ok=True)
                    continue
                source = archive.extractfile(member)
                if source is None:
                    raise VerificationError(f"could not read TAR member: {member.name!r}")
                with source:
                    _copy_member(source, output_path, member.size)
                os.chmod(output_path, member.mode & 0o777)
    except (OSError, tarfile.TarError) as error:
        if isinstance(error, VerificationError):
            raise
        raise VerificationError(f"could not safely read TAR archive: {error}") from error


def safe_extract_archive(
    archive_path: Path,
    destination: Path,
    *,
    root_name: str | None,
    expected_relative_files: set[str],
) -> None:
    _require_regular_file(archive_path, "release archive")
    if archive_path.stat().st_size <= 0:
        raise VerificationError(f"release archive is empty: {archive_path}")
    if archive_path.stat().st_size > MAX_ARCHIVE_BYTES:
        raise VerificationError(f"release archive exceeds the size limit: {archive_path}")
    if destination.exists():
        raise VerificationError(f"extraction destination must not already exist: {destination}")
    destination.mkdir(parents=True)
    expected_files = (
        {f"{root_name}/{name}" for name in expected_relative_files}
        if root_name is not None
        else set(expected_relative_files)
    )
    if archive_path.name.endswith(".zip"):
        _extract_zip(
            archive_path,
            destination,
            root_name=root_name,
            expected_files=expected_files,
        )
    elif archive_path.name.endswith(".tar.gz"):
        _extract_tar(
            archive_path,
            destination,
            root_name=root_name,
            expected_files=expected_files,
        )
    else:
        raise VerificationError(f"unsupported release archive type: {archive_path.name}")


def _reject_duplicate_json_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise VerificationError(f"manifest contains duplicate JSON key: {key}")
        result[key] = value
    return result


def _load_manifest(path: Path) -> dict[str, object]:
    _require_regular_file(path, "package manifest")
    if path.stat().st_size > 1024 * 1024:
        raise VerificationError("package manifest exceeds the size limit")
    try:
        value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=_reject_duplicate_json_keys)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VerificationError(f"package manifest is not valid UTF-8 JSON: {error}") from error
    if not isinstance(value, dict):
        raise VerificationError("package manifest must be a JSON object")
    return value


def _require_exact_object_keys(
    value: dict[str, object], expected: set[str], description: str
) -> None:
    actual = set(value)
    if actual != expected:
        raise VerificationError(
            f"{description} keys mismatch; missing={sorted(expected - actual) or 'none'}, "
            f"extra={sorted(actual - expected) or 'none'}"
        )


def verify_manifest(
    package_root: Path,
    identity: ArchiveIdentity,
    expected_source_sha: str,
) -> tuple[dict[str, object], list[dict[str, object]]]:
    manifest = _load_manifest(package_root / "manifest.json")
    _require_exact_object_keys(
        manifest,
        {
            "schemaVersion",
            "package",
            "version",
            "platform",
            "architecture",
            "sourceSha",
            "files",
        },
        "package manifest",
    )
    expected_scalars: dict[str, object] = {
        "schemaVersion": 1,
        "package": "sorotte-server",
        "version": identity.version,
        "platform": identity.platform,
        "architecture": identity.architecture,
        "sourceSha": expected_source_sha,
    }
    for key, expected in expected_scalars.items():
        if type(manifest[key]) is not type(expected) or manifest[key] != expected:
            raise VerificationError(
                f"manifest {key} mismatch: expected {expected!r}, received {manifest[key]!r}"
            )
    files = manifest["files"]
    if not isinstance(files, list):
        raise VerificationError("manifest files must be an array")
    expected_names = {
        identity.binary_name,
        "README.md",
        "SERVER_RELEASE.md",
        "LICENSE",
    }
    observed_names: set[str] = set()
    verified_files: list[dict[str, object]] = []
    for index, entry in enumerate(files):
        if not isinstance(entry, dict):
            raise VerificationError(f"manifest files[{index}] must be an object")
        _require_exact_object_keys(entry, {"path", "size", "sha256"}, f"manifest files[{index}]")
        relative_path = entry["path"]
        size = entry["size"]
        digest = entry["sha256"]
        if not isinstance(relative_path, str):
            raise VerificationError(f"manifest files[{index}].path must be a string")
        if relative_path in observed_names:
            raise VerificationError(f"manifest contains duplicate file entry: {relative_path}")
        if relative_path not in expected_names:
            raise VerificationError(f"manifest contains unexpected file entry: {relative_path}")
        if type(size) is not int or size <= 0 or size > MAX_FILE_BYTES:
            raise VerificationError(f"manifest contains invalid size for {relative_path}: {size!r}")
        if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
            raise VerificationError(f"manifest contains invalid SHA-256 for {relative_path}")
        payload_path = package_root / relative_path
        _require_regular_file(payload_path, f"manifest payload {relative_path}")
        actual_size = payload_path.stat().st_size
        actual_digest = sha256_file(payload_path)
        if actual_size != size:
            raise VerificationError(
                f"manifest size mismatch for {relative_path}: expected {size}, received {actual_size}"
            )
        if actual_digest != digest:
            raise VerificationError(
                f"manifest digest mismatch for {relative_path}: expected {digest}, received {actual_digest}"
            )
        observed_names.add(relative_path)
        verified_files.append(
            {"path": relative_path, "size": actual_size, "sha256": actual_digest}
        )
    if observed_names != expected_names:
        raise VerificationError(
            f"manifest file inventory mismatch; missing={sorted(expected_names - observed_names)}"
        )
    return manifest, sorted(verified_files, key=lambda item: str(item["path"]))


def _reserve_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def _wait_for_server(process: subprocess.Popen[bytes], port: int, deadline: float) -> socket.socket:
    last_error: OSError | None = None
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise VerificationError(
                f"packaged server exited before accepting a connection (exit {process.returncode})"
            )
        connection = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        connection.settimeout(min(0.25, max(0.01, deadline - time.monotonic())))
        try:
            connection.connect(("127.0.0.1", port))
            connection.settimeout(1.0)
            return connection
        except OSError as error:
            last_error = error
            connection.close()
            time.sleep(0.025)
    raise VerificationError(f"packaged server did not listen before the deadline: {last_error}")


def smoke_test_binary(binary_path: Path, version: str) -> dict[str, object]:
    _require_regular_file(binary_path, "packaged server binary")
    try:
        version_result = subprocess.run(
            [str(binary_path), "--version"],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise VerificationError(f"packaged server --version failed: {error}") from error
    version_output = version_result.stdout.decode("utf-8", errors="replace").strip()
    if version_result.returncode != 0:
        raise VerificationError(
            f"packaged server --version exited {version_result.returncode}: {version_output}"
        )
    if re.search(rf"(?<![0-9A-Za-z.]){re.escape(version)}(?![0-9A-Za-z.])", version_output) is None:
        raise VerificationError(
            f"packaged server version output does not identify {version}: {version_output!r}"
        )

    port = _reserve_loopback_port()
    command = [
        str(binary_path),
        "--port",
        str(port),
        "--ipv4-only",
        "--interface-ipv4",
        "127.0.0.1",
    ]
    process: subprocess.Popen[bytes] | None = None
    hello_received = False
    started = time.monotonic()
    with tempfile.TemporaryFile() as process_log:
        try:
            process = subprocess.Popen(command, stdout=process_log, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 10
            with _wait_for_server(process, port, deadline) as connection:
                hello = {
                    "Hello": {
                        "username": "artifact-verifier",
                        "room": {"name": "artifact-verification"},
                        "version": "1.7.5",
                    }
                }
                connection.sendall(
                    json.dumps(hello, separators=(",", ":")).encode("utf-8") + b"\r\n"
                )
                buffered = b""
                while time.monotonic() < deadline and len(buffered) <= 1024 * 1024:
                    try:
                        chunk = connection.recv(65536)
                    except socket.timeout:
                        continue
                    if not chunk:
                        break
                    buffered += chunk
                    while b"\n" in buffered:
                        raw_line, buffered = buffered.split(b"\n", 1)
                        try:
                            message = json.loads(raw_line.rstrip(b"\r").decode("utf-8"))
                        except (UnicodeDecodeError, json.JSONDecodeError):
                            continue
                        if isinstance(message, dict) and "Hello" in message:
                            hello_received = True
                            break
                    if hello_received:
                        break
                if not hello_received:
                    raise VerificationError("packaged server did not return a protocol Hello response")
        except OSError as error:
            raise VerificationError(f"packaged server smoke test failed: {error}") from error
        finally:
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            if process is not None and process.returncode is None:
                process.kill()
                process.wait(timeout=5)
    return {
        "performed": True,
        "versionOutput": version_output,
        "protocolHello": hello_received,
        "loopbackOnly": True,
        "elapsedMilliseconds": round((time.monotonic() - started) * 1000),
    }


def _verify_optional_symbols(
    artifacts_dir: Path,
    identity: ArchiveIdentity,
    extraction_parent: Path,
) -> dict[str, object] | None:
    symbols_name = f"{identity.root_name}-symbols.zip"
    symbols_path = artifacts_dir / symbols_name
    checksum_path = symbols_path.with_name(f"{symbols_path.name}.sha256")
    symbols_exists = symbols_path.exists() or checksum_path.exists()
    if not symbols_exists:
        return None
    if identity.platform != "windows":
        raise VerificationError("Linux server releases must not contain a Windows symbols archive")
    _require_regular_file(symbols_path, "symbols archive")
    _, archive_digest = verify_checksum(symbols_path)
    safe_extract_archive(
        symbols_path,
        extraction_parent / "symbols",
        root_name=None,
        expected_relative_files={"sorotte_server.pdb"},
    )
    return {
        "archive": symbols_path.name,
        "sha256": archive_digest,
        "checksum": checksum_path.name,
    }


def _validate_artifact_directory_entries(
    artifacts_dir: Path,
    identity: ArchiveIdentity,
    *,
    symbols_present: bool,
) -> None:
    allowed = {
        identity.archive_name,
        f"{identity.archive_name}.sha256",
    }
    if symbols_present:
        symbols_name = f"{identity.root_name}-symbols.zip"
        allowed.update({symbols_name, f"{symbols_name}.sha256"})
    actual: set[str] = set()
    for path in artifacts_dir.iterdir():
        if path.is_dir():
            raise VerificationError(f"artifact directory contains an unexpected directory: {path.name}")
        _require_regular_file(path, "artifact directory entry")
        actual.add(path.name)
    if actual != allowed:
        raise VerificationError(
            f"artifact directory inventory mismatch; missing={sorted(allowed - actual) or 'none'}, "
            f"extra={sorted(actual - allowed) or 'none'}"
        )


def verify_release(
    artifacts_dir: Path,
    expected_source_sha: str,
    *,
    runtime_smoke: bool = True,
    work_dir: Path | None = None,
) -> dict[str, object]:
    expected_source_sha = expected_source_sha.lower()
    if SOURCE_SHA_RE.fullmatch(expected_source_sha) is None:
        raise VerificationError("expected source SHA must be exactly 40 hexadecimal characters")
    artifacts_dir = artifacts_dir.resolve()
    archive_path, identity = find_release_archive(artifacts_dir)
    symbols_name = f"{identity.root_name}-symbols.zip"
    symbols_declared = (
        (artifacts_dir / symbols_name).exists()
        or (artifacts_dir / f"{symbols_name}.sha256").exists()
    )
    _validate_artifact_directory_entries(
        artifacts_dir, identity, symbols_present=symbols_declared
    )
    checksum_path, archive_digest = verify_checksum(archive_path)
    if work_dir is None:
        work_dir = artifacts_dir.parent / "verification-work"
    work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="server-artifact-", dir=work_dir) as temporary:
        extraction_parent = Path(temporary)
        expected_relative_files = {
            identity.binary_name,
            "README.md",
            "SERVER_RELEASE.md",
            "LICENSE",
            "manifest.json",
        }
        safe_extract_archive(
            archive_path,
            extraction_parent / "package",
            root_name=identity.root_name,
            expected_relative_files=expected_relative_files,
        )
        package_root = extraction_parent / "package" / identity.root_name
        _, verified_files = verify_manifest(package_root, identity, expected_source_sha)
        manifest_path = package_root / "manifest.json"
        runtime = (
            smoke_test_binary(package_root / identity.binary_name, identity.version)
            if runtime_smoke
            else {"performed": False}
        )
        symbols = _verify_optional_symbols(artifacts_dir, identity, extraction_parent)
        if (symbols is not None) != symbols_declared:
            raise VerificationError("symbols archive inventory changed while it was being verified")
        if sha256_file(archive_path) != archive_digest:
            raise VerificationError("release archive changed while it was being verified")
        report: dict[str, object] = {
            "schemaVersion": 1,
            "status": "verified",
            "archive": {
                "name": archive_path.name,
                "size": archive_path.stat().st_size,
                "sha256": archive_digest,
                "checksum": checksum_path.name,
                "checksumSha256": sha256_file(checksum_path),
            },
            "package": {
                "name": "sorotte-server",
                "version": identity.version,
                "platform": identity.platform,
                "architecture": identity.architecture,
                "sourceSha": expected_source_sha,
                "root": identity.root_name,
                "manifestSha256": sha256_file(manifest_path),
                "files": verified_files,
            },
            "runtimeSmoke": runtime,
            "symbols": symbols,
        }
    return report


def write_report(report: dict[str, object], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    serialized = json.dumps(report, indent=2, sort_keys=True) + "\n"
    temporary_path = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        with temporary_path.open("x", encoding="utf-8", newline="\n") as output:
            output.write(serialized)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary_path, path)
    finally:
        temporary_path.unlink(missing_ok=True)


def failure_report(expected_source_sha: str, error: Exception) -> dict[str, object]:
    return {
        "schemaVersion": 1,
        "status": "failed",
        "expectedSourceSha": expected_source_sha.lower(),
        "error": str(error),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts-dir", type=Path, required=True)
    parser.add_argument("--expected-source-sha", required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument(
        "--skip-runtime-smoke",
        action="store_true",
        help="validate archive structure and provenance without executing the packaged binary",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        report = verify_release(
            args.artifacts_dir,
            args.expected_source_sha,
            runtime_smoke=not args.skip_runtime_smoke,
        )
        write_report(report, args.report)
    except VerificationError as error:
        try:
            write_report(failure_report(args.expected_source_sha, error), args.report)
        except OSError as report_error:
            print(
                f"could not write server release failure report: {report_error}",
                file=os.sys.stderr,
            )
        print(f"server release artifact verification failed: {error}", file=os.sys.stderr)
        return 1
    print(
        f"verified {report['archive']['name']} from {report['package']['sourceSha']} "
        f"and wrote {args.report}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
