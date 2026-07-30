#!/usr/bin/env python3
"""Opt-in disposable block replay harness for Sorotte SQLite durability.

The privileged mode constructs every block device from newly created sparse
regular files. It accepts no existing image, mount, mapper, loop, or device
path. The default plan and preflight modes are read-only and nonprivileged.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import hashlib
import json
import os
import pathlib
import platform
import re
import secrets
import shutil
import stat
import subprocess
import sys
import tempfile
from typing import Any, Final, Sequence

try:
    import pwd
except ImportError:  # pragma: no cover - Windows policy tests never enter privileged mode
    pwd = None  # type: ignore[assignment]


SCHEMA: Final = "sorotte-disposable-powerloss-v1"
ENABLE_TOKEN: Final = "owned-disposable-images-only-v1"
CONFIRMATION_TOKEN: Final = "sorotte-owned-images-only"
ROOT_PARENT_TEXT: Final = "/var/tmp"
ROOT_PARENT: Final = pathlib.Path(ROOT_PARENT_TEXT)
ROOT_PREFIX: Final = "sorotte-powerloss-"
MARKER_NAME: Final = ".sorotte-powerloss-owned-v1"
DATA_IMAGE_BYTES: Final = 256 * 1024 * 1024
LOG_IMAGE_BYTES: Final = 512 * 1024 * 1024
IMAGE_SPECS: Final = {
    "live-data.img": DATA_IMAGE_BYTES,
    "write-log.img": LOG_IMAGE_BYTES,
    "replay-baseline.img": DATA_IMAGE_BYTES,
    "replay-app-ack.img": DATA_IMAGE_BYTES,
    "replay-syncfs.img": DATA_IMAGE_BYTES,
}
REPLAY_MARKS: Final = (
    ("baseline-flushed", "verify-baseline", "baseline"),
    ("replacement-app-ack", "verify-old-or-new", "baseline-or-replacement"),
    ("replacement-syncfs", "verify-replacement", "replacement"),
)
REQUIRED_TOOLS: Final = (
    "blockdev",
    "cargo",
    "dmsetup",
    "e2fsck",
    "findmnt",
    "losetup",
    "mkfs.ext4",
    "mount",
    "replay-log",
    "runuser",
    "sync",
    "umount",
)
LOOP_PATTERN: Final = re.compile(r"^/dev/loop[0-9]+$")
DM_NAME_PATTERN: Final = re.compile(r"^sorotte-pl-[0-9]+-[0-9a-f]{12}$")
NONCE_PATTERN: Final = re.compile(r"^[0-9a-f]{32}$")
MAJOR_MINOR_PATTERN: Final = re.compile(r"^([0-9]+):([0-9]+)$")


class SafetyError(RuntimeError):
    """Raised before an ownership or target invariant can be violated."""


@dataclasses.dataclass(frozen=True)
class OwnedWorkspace:
    parent: pathlib.Path
    root: pathlib.Path
    nonce: str

    @property
    def marker(self) -> pathlib.Path:
        return self.root / MARKER_NAME

    def image(self, name: str) -> pathlib.Path:
        if name not in IMAGE_SPECS:
            raise SafetyError(f"unrecognized owned image name {name!r}")
        return self.root / name


@dataclasses.dataclass(frozen=True)
class LoopBinding:
    device: pathlib.Path
    image: pathlib.Path
    expected_bytes: int
    nonce: str


@dataclasses.dataclass(frozen=True)
class DmBinding:
    name: str
    device: pathlib.Path
    data_loop: LoopBinding
    log_loop: LoopBinding
    nonce: str


@dataclasses.dataclass(frozen=True)
class MountBinding:
    target: pathlib.Path
    source: pathlib.Path
    source_rdev: int
    nonce: str


def _euid() -> int:
    getter = getattr(os, "geteuid", None)
    return int(getter()) if getter is not None else -1


def _marker_contents(nonce: str) -> str:
    return f"sorotte-powerloss-owned-v1\nnonce={nonce}\n"


def _lstat_real_directory(path: pathlib.Path, label: str) -> os.stat_result:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise SafetyError(f"{label} metadata failed: {error}") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise SafetyError(f"{label} must be a real directory, not a symlink")
    return metadata


def _lstat_regular_file(path: pathlib.Path, label: str) -> os.stat_result:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise SafetyError(f"{label} metadata failed: {error}") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise SafetyError(f"{label} must be a real regular file, not a symlink")
    return metadata


def _resolve_existing(path: pathlib.Path, label: str) -> pathlib.Path:
    try:
        return path.resolve(strict=True)
    except OSError as error:
        raise SafetyError(f"{label} canonicalization failed: {error}") from error


def _validate_nonce(nonce: str) -> None:
    if NONCE_PATTERN.fullmatch(nonce) is None:
        raise SafetyError("ownership nonce must contain exactly 32 lowercase hex digits")


def assert_owned_root(workspace: OwnedWorkspace) -> pathlib.Path:
    _validate_nonce(workspace.nonce)
    _lstat_real_directory(workspace.parent, "workspace parent")
    canonical_parent = _resolve_existing(workspace.parent, "workspace parent")
    _lstat_real_directory(workspace.root, "workspace root")
    canonical_root = _resolve_existing(workspace.root, "workspace root")
    if canonical_root.parent != canonical_parent:
        raise SafetyError("workspace root must be a direct child of its fixed parent")
    if not canonical_root.name.startswith(ROOT_PREFIX):
        raise SafetyError(f"workspace root must start with {ROOT_PREFIX!r}")
    root_metadata = _lstat_real_directory(canonical_root, "canonical workspace root")
    if _euid() >= 0 and root_metadata.st_uid != _euid():
        raise SafetyError("workspace root must be owned by the current effective user")

    marker_metadata = _lstat_regular_file(workspace.marker, "ownership marker")
    if _euid() >= 0 and marker_metadata.st_uid != _euid():
        raise SafetyError("ownership marker must be owned by the current effective user")
    try:
        marker = workspace.marker.read_text(encoding="utf-8")
    except OSError as error:
        raise SafetyError(f"ownership marker read failed: {error}") from error
    if marker != _marker_contents(workspace.nonce):
        raise SafetyError("ownership marker does not match the recorded nonce")
    return canonical_root


def create_owned_workspace(parent: pathlib.Path = ROOT_PARENT) -> OwnedWorkspace:
    _lstat_real_directory(parent, "workspace parent")
    canonical_parent = _resolve_existing(parent, "workspace parent")
    root = pathlib.Path(tempfile.mkdtemp(prefix=ROOT_PREFIX, dir=canonical_parent))
    # The unprivileged originating user must traverse to the mounted database.
    # Images and reports remain root-only regular files within this non-listable root.
    os.chmod(root, 0o711)
    nonce = secrets.token_hex(16)
    workspace = OwnedWorkspace(parent=canonical_parent, root=root, nonce=nonce)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(workspace.marker, flags, 0o600)
    try:
        os.write(descriptor, _marker_contents(nonce).encode("ascii"))
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    os.chmod(workspace.marker, 0o444)
    assert_owned_root(workspace)
    return workspace


def create_sparse_image(
    workspace: OwnedWorkspace,
    name: str,
    expected_bytes: int,
) -> pathlib.Path:
    assert_owned_root(workspace)
    if IMAGE_SPECS.get(name) != expected_bytes:
        raise SafetyError("image name and exact size are not in the fixed image specification")
    image = workspace.image(name)
    if image.parent != workspace.root:
        raise SafetyError("owned image must be an immediate child of the workspace root")
    flags = os.O_RDWR | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(image, flags, 0o600)
    try:
        os.ftruncate(descriptor, expected_bytes)
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    assert_owned_image(workspace, image, expected_bytes)
    return image


def assert_owned_image(
    workspace: OwnedWorkspace,
    image: pathlib.Path,
    expected_bytes: int,
) -> pathlib.Path:
    canonical_root = assert_owned_root(workspace)
    if image.parent != canonical_root or IMAGE_SPECS.get(image.name) != expected_bytes:
        raise SafetyError("image path and size must match one fixed owned image specification")
    metadata = _lstat_regular_file(image, f"owned image {image.name}")
    if _euid() >= 0 and metadata.st_uid != _euid():
        raise SafetyError("owned image must be owned by the current effective user")
    if metadata.st_size != expected_bytes:
        raise SafetyError(
            f"owned image {image.name!r} must be exactly {expected_bytes} bytes"
        )
    canonical_image = _resolve_existing(image, f"owned image {image.name}")
    if canonical_image.parent != canonical_root:
        raise SafetyError("owned image canonical path escaped the workspace root")
    return canonical_image


def _block_device_identity(path: pathlib.Path, label: str) -> tuple[int, int]:
    try:
        metadata = path.stat()
    except OSError as error:
        raise SafetyError(f"{label} metadata failed: {error}") from error
    if not stat.S_ISBLK(metadata.st_mode):
        raise SafetyError(f"{label} must be a block device")
    if not hasattr(os, "major") or not hasattr(os, "minor"):
        raise SafetyError("block-device identity requires Unix major/minor support")
    return os.major(metadata.st_rdev), os.minor(metadata.st_rdev)


def _dm_table_operand_identity(operand: str, label: str) -> tuple[int, int]:
    rendered = MAJOR_MINOR_PATTERN.fullmatch(operand)
    if rendered is not None:
        return int(rendered.group(1)), int(rendered.group(2))
    path = pathlib.Path(operand)
    if not path.is_absolute():
        raise SafetyError(
            f"{label} must render as major:minor or an absolute block-device path"
        )
    return _block_device_identity(path, label)


def validate_log_writes_table(
    table: str,
    *,
    expected_sectors: int,
    data_identity: tuple[int, int],
    log_identity: tuple[int, int],
) -> None:
    fields = table.split()
    if len(fields) != 5:
        raise SafetyError("mapper table must contain exactly five log-writes fields")
    if (
        fields[0] != "0"
        or fields[1] != str(expected_sectors)
        or fields[2] != "log-writes"
    ):
        raise SafetyError("mapper table is not the fixed single log-writes target")
    actual_data_identity = _dm_table_operand_identity(
        fields[3], "log-writes data operand"
    )
    actual_log_identity = _dm_table_operand_identity(
        fields[4], "log-writes log operand"
    )
    if actual_data_identity != data_identity:
        raise SafetyError(
            "log-writes data operand no longer identifies the recorded data loop"
        )
    if actual_log_identity != log_identity:
        raise SafetyError(
            "log-writes log operand no longer identifies the recorded log loop"
        )


def plan_document(repo_root: pathlib.Path) -> dict[str, Any]:
    return {
        "schema": SCHEMA,
        "mode": "read-only-plan",
        "repo_root": str(repo_root),
        "network_activity": False,
        "accepted_existing_device_or_mount_arguments": [],
        "workspace_parent": ROOT_PARENT_TEXT,
        "workspace_construction": (
            "mkdtemp direct child; nonce-bound marker; O_EXCL regular sparse files"
        ),
        "image_sizes_bytes": IMAGE_SPECS,
        "block_stack": (
            "owned sparse file -> verified loop -> nonce-named dm-log-writes mapper"
        ),
        "revalidation": (
            "marker, canonical path, file type, owner, exact size, loop backing file, "
            "mapper dependencies, and mount source are checked immediately before actions"
        ),
        "phases": [
            "mkfs through dm-log-writes",
            "production worker baseline write and acknowledgement",
            "syncfs and baseline-flushed mark",
            "production worker replacement write and acknowledgement",
            "replacement-app-ack mark",
            "syncfs and replacement-syncfs mark",
            "replay each mark to a fresh owned image",
            "mount/restart and verify SQLite integrity plus complete old/new row",
        ],
        "replay_expectations": {
            "baseline-flushed": "baseline",
            "replacement-app-ack": "baseline-or-replacement",
            "replacement-syncfs": "replacement",
        },
        "required_tools": list(REQUIRED_TOOLS),
        "privileged_run_requires": {
            "platform": "Linux or WSL2 Linux",
            "effective_uid": 0,
            "sudo_origin": "non-root SUDO_UID and SUDO_GID",
            "confirmation": CONFIRMATION_TOKEN,
            "device_mapper_target": "log-writes",
        },
        "automatic_cleanup": False,
        "claim_limit": (
            "Capability alone is not durability evidence; a completed report proves only "
            "the observed disposable dm-log-writes replay marks, not physical media."
        ),
    }


def collect_preflight(repo_root: pathlib.Path) -> dict[str, Any]:
    tools = {tool: shutil.which(tool) for tool in REQUIRED_TOOLS}
    version_text = ""
    if pathlib.Path("/proc/version").is_file():
        try:
            version_text = pathlib.Path("/proc/version").read_text(
                encoding="utf-8", errors="replace"
            )
        except OSError:
            version_text = ""
    dm_targets: list[str] = []
    dm_error: str | None = None
    if tools["dmsetup"] is not None:
        completed = subprocess.run(
            [tools["dmsetup"], "targets"],
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode == 0:
            dm_targets = sorted(
                line.split()[0] for line in completed.stdout.splitlines() if line.strip()
            )
        else:
            dm_error = completed.stderr.strip() or f"exit {completed.returncode}"

    required_repo_paths = (
        repo_root / "Cargo.toml",
        repo_root / "crates" / "sorotte-server" / "Cargo.toml",
        repo_root
        / "crates"
        / "sorotte-server"
        / "src"
        / "tests"
        / "persistence_power_loss_harness_tests.rs",
    )
    return {
        "schema": SCHEMA,
        "mode": "read-only-preflight",
        "platform": platform.system(),
        "platform_release": platform.release(),
        "wsl": "microsoft" in version_text.lower(),
        "effective_uid": _euid(),
        "running_as_root": _euid() == 0,
        "sudo_uid_present": bool(os.environ.get("SUDO_UID")),
        "tools": tools,
        "missing_tools": sorted(tool for tool, found in tools.items() if found is None),
        "dm_targets": dm_targets,
        "dm_targets_error": dm_error,
        "log_writes_target_present": "log-writes" in dm_targets,
        "repo_paths_present": {
            str(path.relative_to(repo_root)): path.is_file()
            for path in required_repo_paths
        },
        "destructive_actions_attempted": False,
        "capability_prerequisites_present": (
            platform.system() == "Linux"
            and all(found is not None for found in tools.values())
            and "log-writes" in dm_targets
            and all(path.is_file() for path in required_repo_paths)
        ),
        "ready_for_privileged_run": (
            platform.system() == "Linux"
            and _euid() == 0
            and bool(os.environ.get("SUDO_UID"))
            and all(found is not None for found in tools.values())
            and "log-writes" in dm_targets
            and all(path.is_file() for path in required_repo_paths)
        ),
    }


class Recorder:
    def __init__(self) -> None:
        self.commands: list[dict[str, Any]] = []

    def run(
        self,
        command: Sequence[str],
        *,
        cwd: pathlib.Path | None = None,
        stdin: str | None = None,
        check: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        if not command or any(not isinstance(argument, str) for argument in command):
            raise SafetyError("commands must be nonempty string argument arrays")
        started = dt.datetime.now(dt.timezone.utc)
        completed = subprocess.run(
            list(command),
            cwd=cwd,
            input=stdin,
            check=False,
            capture_output=True,
            text=True,
        )
        self.commands.append(
            {
                "argv": list(command),
                "cwd": str(cwd) if cwd is not None else None,
                "started_utc": started.isoformat(),
                "exit_code": completed.returncode,
                "stdout": completed.stdout,
                "stderr": completed.stderr,
            }
        )
        if check and completed.returncode != 0:
            raise RuntimeError(
                f"command failed ({completed.returncode}): {command!r}\n"
                f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
            )
        return completed


class DisposableReplayHarness:
    def __init__(
        self,
        workspace: OwnedWorkspace,
        repo_root: pathlib.Path,
        recorder: Recorder,
        original_uid: int,
        original_gid: int,
        original_user: str,
        original_home: pathlib.Path,
    ) -> None:
        self.workspace = workspace
        self.repo_root = repo_root
        self.recorder = recorder
        self.original_uid = original_uid
        self.original_gid = original_gid
        self.original_user = original_user
        self.original_home = original_home
        self.loops: list[LoopBinding] = []
        self.mappers: list[DmBinding] = []
        self.mounts: list[MountBinding] = []
        self.observations: dict[str, str] = {}

    def _loop_backing_file(self, device: pathlib.Path) -> pathlib.Path:
        result = self.recorder.run(
            ["losetup", "--noheadings", "--output", "BACK-FILE", str(device)]
        )
        backing = result.stdout.strip()
        if not backing or "\n" in backing:
            raise SafetyError(f"loop {device} did not report exactly one backing file")
        return _resolve_existing(pathlib.Path(backing), f"loop {device} backing file")

    def assert_owned_loop(self, binding: LoopBinding) -> None:
        if binding.nonce != self.workspace.nonce:
            raise SafetyError("loop binding nonce does not match the workspace")
        if LOOP_PATTERN.fullmatch(str(binding.device)) is None:
            raise SafetyError("loop binding is not an exact /dev/loopN path")
        metadata = binding.device.stat()
        if not stat.S_ISBLK(metadata.st_mode):
            raise SafetyError("recorded loop path is no longer a block device")
        image = assert_owned_image(
            self.workspace, binding.image, binding.expected_bytes
        )
        if self._loop_backing_file(binding.device) != image:
            raise SafetyError("loop backing file no longer matches the recorded owned image")
        size_text = self.recorder.run(
            ["blockdev", "--getsize64", str(binding.device)]
        ).stdout.strip()
        if size_text != str(binding.expected_bytes):
            raise SafetyError(
                f"loop size changed: expected {binding.expected_bytes}, found {size_text!r}"
            )

    def attach_loop(self, image_name: str) -> LoopBinding:
        expected_bytes = IMAGE_SPECS[image_name]
        image = assert_owned_image(
            self.workspace, self.workspace.image(image_name), expected_bytes
        )
        result = self.recorder.run(
            [
                "losetup",
                "--find",
                "--show",
                "--nooverlap",
                "--direct-io=on",
                str(image),
            ]
        )
        device = pathlib.Path(result.stdout.strip())
        binding = LoopBinding(device, image, expected_bytes, self.workspace.nonce)
        self.loops.append(binding)
        self.assert_owned_loop(binding)
        return binding

    def detach_loop(self, binding: LoopBinding) -> None:
        self.assert_owned_loop(binding)
        self.recorder.run(["losetup", "--detach", str(binding.device)])
        self.loops.remove(binding)

    def assert_owned_mapper(self, binding: DmBinding) -> None:
        if binding.nonce != self.workspace.nonce:
            raise SafetyError("mapper binding nonce does not match the workspace")
        if DM_NAME_PATTERN.fullmatch(binding.name) is None:
            raise SafetyError("mapper name is outside the fixed nonce-named pattern")
        expected_name = (
            f"sorotte-pl-{os.getpid()}-{self.workspace.nonce[:12]}"
        )
        if binding.name != expected_name:
            raise SafetyError("mapper name does not match this process and workspace nonce")
        self.assert_owned_loop(binding.data_loop)
        self.assert_owned_loop(binding.log_loop)
        metadata = binding.device.stat()
        if not stat.S_ISBLK(metadata.st_mode):
            raise SafetyError("recorded mapper path is no longer a block device")
        size_text = self.recorder.run(
            ["blockdev", "--getsize64", str(binding.device)]
        ).stdout.strip()
        if size_text != str(DATA_IMAGE_BYTES):
            raise SafetyError(
                f"mapper size changed: expected {DATA_IMAGE_BYTES}, found {size_text!r}"
            )
        table = self.recorder.run(
            ["dmsetup", "table", "--showkeys", binding.name]
        ).stdout.strip()
        validate_log_writes_table(
            table,
            expected_sectors=DATA_IMAGE_BYTES // 512,
            data_identity=_block_device_identity(
                binding.data_loop.device, "recorded data loop"
            ),
            log_identity=_block_device_identity(
                binding.log_loop.device, "recorded log loop"
            ),
        )
        dependencies = self.recorder.run(
            ["dmsetup", "deps", "--noheadings", "-o", "devname", binding.name]
        ).stdout
        actual = set(re.findall(r"\(([^)]+)\)", dependencies))
        expected = {binding.data_loop.device.name, binding.log_loop.device.name}
        if actual != expected:
            raise SafetyError(
                f"mapper dependencies changed: expected {sorted(expected)}, "
                f"found {sorted(actual)}"
            )

    def create_mapper(
        self, data_loop: LoopBinding, log_loop: LoopBinding
    ) -> DmBinding:
        self.assert_owned_loop(data_loop)
        self.assert_owned_loop(log_loop)
        name = f"sorotte-pl-{os.getpid()}-{self.workspace.nonce[:12]}"
        if DM_NAME_PATTERN.fullmatch(name) is None:
            raise SafetyError("constructed mapper name failed its fixed pattern")
        existing = self.recorder.run(
            ["dmsetup", "info", name], check=False
        )
        if existing.returncode == 0:
            raise SafetyError("refusing to replace an existing mapper")
        sectors = DATA_IMAGE_BYTES // 512
        table = (
            f"0 {sectors} log-writes {data_loop.device} {log_loop.device}"
        )
        self.assert_owned_loop(data_loop)
        self.assert_owned_loop(log_loop)
        self.recorder.run(["dmsetup", "create", name, "--table", table])
        binding = DmBinding(
            name=name,
            device=pathlib.Path("/dev/mapper") / name,
            data_loop=data_loop,
            log_loop=log_loop,
            nonce=self.workspace.nonce,
        )
        self.mappers.append(binding)
        self.assert_owned_mapper(binding)
        return binding

    def mark(self, binding: DmBinding, mark: str) -> None:
        if mark not in {item[0] for item in REPLAY_MARKS} | {"filesystem-created"}:
            raise SafetyError("unrecognized fixed replay mark")
        self.assert_owned_mapper(binding)
        self.recorder.run(
            ["dmsetup", "message", binding.name, "0", "mark", mark]
        )

    def remove_mapper(self, binding: DmBinding) -> None:
        self.assert_owned_mapper(binding)
        self.recorder.run(["dmsetup", "remove", binding.name])
        self.mappers.remove(binding)

    def _find_mount_source(self, target: pathlib.Path) -> pathlib.Path | None:
        result = self.recorder.run(
            [
                "findmnt",
                "--noheadings",
                "--output",
                "SOURCE",
                "--mountpoint",
                str(target),
            ],
            check=False,
        )
        if result.returncode != 0:
            return None
        source = result.stdout.strip()
        if not source or "\n" in source:
            raise SafetyError("mount target did not report exactly one source")
        return pathlib.Path(source)

    def assert_owned_mount(self, binding: MountBinding) -> None:
        if binding.nonce != self.workspace.nonce:
            raise SafetyError("mount binding nonce does not match the workspace")
        root = assert_owned_root(self.workspace)
        _lstat_real_directory(binding.target, "recorded mount target")
        if binding.target.parent != root:
            raise SafetyError("recorded mount target escaped the workspace root")
        actual_source = self._find_mount_source(binding.target)
        if actual_source is None:
            raise SafetyError("recorded mount target is no longer mounted")
        metadata = actual_source.stat()
        if not stat.S_ISBLK(metadata.st_mode) or metadata.st_rdev != binding.source_rdev:
            raise SafetyError("mount source no longer matches the recorded block device")

    def mount_device(
        self,
        source: pathlib.Path,
        target_name: str,
        *,
        loop: LoopBinding | None = None,
        mapper: DmBinding | None = None,
    ) -> MountBinding:
        root = assert_owned_root(self.workspace)
        if target_name != "mount":
            raise SafetyError("mount target is outside the fixed target inventory")
        if (loop is None) == (mapper is None):
            raise SafetyError("mount requires exactly one recorded loop or mapper binding")
        if loop is not None:
            self.assert_owned_loop(loop)
            if source != loop.device:
                raise SafetyError("mount source differs from its recorded loop binding")
        if mapper is not None:
            self.assert_owned_mapper(mapper)
            if source != mapper.device:
                raise SafetyError("mount source differs from its recorded mapper binding")
        target = root / target_name
        if target.exists():
            _lstat_real_directory(target, "reused mount target")
            if any(target.iterdir()):
                raise SafetyError("unmounted target directory must be empty before reuse")
        else:
            target.mkdir(mode=0o711)
            os.chmod(target, 0o711)
        _lstat_real_directory(target, "new mount target")
        if self._find_mount_source(target) is not None:
            raise SafetyError("refusing to cover an existing mount")
        source_metadata = source.stat()
        if not stat.S_ISBLK(source_metadata.st_mode):
            raise SafetyError("mount source must be a recorded block device")
        self.recorder.run(
            [
                "mount",
                "-t",
                "ext4",
                "-o",
                "nosuid,nodev,noexec,noatime,nodiratime",
                str(source),
                str(target),
            ]
        )
        binding = MountBinding(
            target=target,
            source=source,
            source_rdev=source_metadata.st_rdev,
            nonce=self.workspace.nonce,
        )
        self.mounts.append(binding)
        self.assert_owned_mount(binding)
        return binding

    def sync_mount(self, binding: MountBinding) -> None:
        self.assert_owned_mount(binding)
        self.recorder.run(["sync", "-f", str(binding.target)])

    def unmount(self, binding: MountBinding) -> None:
        self.assert_owned_mount(binding)
        self.recorder.run(["umount", str(binding.target)])
        if self._find_mount_source(binding.target) is not None:
            raise SafetyError("mount remained present after exact-target unmount")
        self.mounts.remove(binding)

    def run_driver(self, mount: MountBinding, phase: str) -> str:
        self.assert_owned_mount(mount)
        if phase not in {
            "seed-baseline",
            "write-replacement",
            "verify-baseline",
            "verify-old-or-new",
            "verify-replacement",
        }:
            raise SafetyError("unrecognized fixed Rust driver phase")
        data_directory = mount.target / "sorotte"
        if not data_directory.exists():
            data_directory.mkdir(mode=0o700)
            os.chown(data_directory, self.original_uid, self.original_gid)
        _lstat_real_directory(data_directory, "mounted database directory")
        db_path = data_directory / "rooms.sqlite3"
        expected_db_path = (
            assert_owned_root(self.workspace)
            / mount.target.name
            / "sorotte"
            / "rooms.sqlite3"
        )
        if db_path != expected_db_path:
            raise SafetyError("database path escaped its fixed owned mount location")
        environment = [
            f"HOME={self.original_home}",
            f"PATH={os.environ.get('PATH', '')}",
            f"SOROTTE_PERSISTENCE_POWERLOSS_ENABLE={ENABLE_TOKEN}",
            f"SOROTTE_PERSISTENCE_POWERLOSS_ROOT={self.workspace.root}",
            f"SOROTTE_PERSISTENCE_POWERLOSS_NONCE={self.workspace.nonce}",
            f"SOROTTE_PERSISTENCE_POWERLOSS_DB_PATH={db_path}",
            f"SOROTTE_PERSISTENCE_POWERLOSS_PHASE={phase}",
        ]
        for optional in ("CARGO_HOME", "RUSTUP_HOME"):
            if os.environ.get(optional):
                environment.append(f"{optional}={os.environ[optional]}")
        result = self.recorder.run(
            [
                "runuser",
                "--user",
                self.original_user,
                "--",
                "env",
                *environment,
                "cargo",
                "test",
                "--locked",
                "-p",
                "sorotte-server",
                "--lib",
                "room_persistence_disposable_block_driver",
                "--",
                "--nocapture",
                "--test-threads=1",
            ],
            cwd=self.repo_root,
        )
        matches = re.findall(
            r"^SOROTTE_POWERLOSS_RESULT=(baseline|replacement)$",
            result.stdout,
            re.MULTILINE,
        )
        if len(matches) != 1:
            raise RuntimeError("Rust driver did not emit exactly one bounded state result")
        return matches[0]

    def format_live_mapper(self, mapper: DmBinding) -> None:
        self.assert_owned_mapper(mapper)
        self.recorder.run(
            [
                "mkfs.ext4",
                "-F",
                "-q",
                "-E",
                "lazy_itable_init=0,lazy_journal_init=0",
                str(mapper.device),
            ]
        )
        self.assert_owned_mapper(mapper)

    def replay_mark(
        self,
        log_loop: LoopBinding,
        image_name: str,
        mark: str,
        verify_phase: str,
        expected: str,
    ) -> None:
        self.assert_owned_loop(log_loop)
        replay_loop = self.attach_loop(image_name)
        self.assert_owned_loop(log_loop)
        self.assert_owned_loop(replay_loop)
        self.recorder.run(
            [
                "replay-log",
                "--log",
                str(log_loop.device),
                "--replay",
                str(replay_loop.device),
                "--end-mark",
                mark,
                "--no-discard",
            ]
        )
        replay_mount = self.mount_device(
            replay_loop.device, "mount", loop=replay_loop
        )
        observed = self.run_driver(replay_mount, verify_phase)
        if expected == "baseline-or-replacement":
            if observed not in {"baseline", "replacement"}:
                raise RuntimeError("app-ack replay was not one complete generation")
        elif observed != expected:
            raise RuntimeError(
                f"replay {mark!r} recovered {observed!r}, expected {expected!r}"
            )
        self.observations[mark] = observed
        self.sync_mount(replay_mount)
        self.unmount(replay_mount)
        self.assert_owned_loop(replay_loop)
        check = self.recorder.run(
            ["e2fsck", "-f", "-n", str(replay_loop.device)], check=False
        )
        if check.returncode != 0:
            raise RuntimeError(
                f"read-only e2fsck rejected replay {mark!r}: {check.returncode}"
            )
        self.detach_loop(replay_loop)

    def execute(self) -> None:
        for name, expected_bytes in IMAGE_SPECS.items():
            create_sparse_image(self.workspace, name, expected_bytes)
        data_loop = self.attach_loop("live-data.img")
        log_loop = self.attach_loop("write-log.img")
        mapper = self.create_mapper(data_loop, log_loop)
        self.format_live_mapper(mapper)
        live_mount = self.mount_device(mapper.device, "mount", mapper=mapper)
        self.sync_mount(live_mount)
        self.mark(mapper, "filesystem-created")

        if self.run_driver(live_mount, "seed-baseline") != "baseline":
            raise RuntimeError("baseline phase did not observe its complete state")
        self.sync_mount(live_mount)
        self.mark(mapper, "baseline-flushed")

        if self.run_driver(live_mount, "write-replacement") != "replacement":
            raise RuntimeError("replacement phase did not observe its complete state")
        self.mark(mapper, "replacement-app-ack")
        self.sync_mount(live_mount)
        self.mark(mapper, "replacement-syncfs")

        self.unmount(live_mount)
        self.remove_mapper(mapper)
        self.detach_loop(data_loop)
        for mark, verify_phase, expected in REPLAY_MARKS:
            image_name = {
                "baseline-flushed": "replay-baseline.img",
                "replacement-app-ack": "replay-app-ack.img",
                "replacement-syncfs": "replay-syncfs.img",
            }[mark]
            self.replay_mark(log_loop, image_name, mark, verify_phase, expected)
        self.detach_loop(log_loop)

    def safe_teardown(self) -> list[str]:
        errors: list[str] = []
        for binding in list(reversed(self.mounts)):
            try:
                self.unmount(binding)
            except Exception as error:  # noqa: BLE001 - cleanup records and preserves
                errors.append(f"mount {binding.target}: {error}")
        for binding in list(reversed(self.mappers)):
            try:
                self.remove_mapper(binding)
            except Exception as error:  # noqa: BLE001 - cleanup records and preserves
                errors.append(f"mapper {binding.name}: {error}")
        for binding in list(reversed(self.loops)):
            try:
                self.detach_loop(binding)
            except Exception as error:  # noqa: BLE001 - cleanup records and preserves
                errors.append(f"loop {binding.device}: {error}")
        return errors


def _original_user() -> tuple[int, int, str, pathlib.Path]:
    if pwd is None:
        raise SafetyError("privileged mode requires the Unix pwd module")
    try:
        uid = int(os.environ["SUDO_UID"])
        gid = int(os.environ["SUDO_GID"])
    except (KeyError, ValueError) as error:
        raise SafetyError(
            "privileged mode must originate from sudo with numeric SUDO_UID/SUDO_GID"
        ) from error
    if uid == 0:
        raise SafetyError("privileged mode must originate from a non-root user")
    try:
        record = pwd.getpwuid(uid)
    except KeyError as error:
        raise SafetyError("SUDO_UID does not identify a local user") from error
    if record.pw_gid != gid:
        raise SafetyError("SUDO_GID does not match the originating user")
    return uid, gid, record.pw_name, pathlib.Path(record.pw_dir)


def _source_identity(repo_root: pathlib.Path, recorder: Recorder) -> dict[str, str]:
    head = recorder.run(
        ["git", "-c", f"safe.directory={repo_root}", "rev-parse", "HEAD"],
        cwd=repo_root,
    ).stdout.strip()
    driver = (
        repo_root
        / "crates"
        / "sorotte-server"
        / "src"
        / "tests"
        / "persistence_power_loss_harness_tests.rs"
    )
    harness = repo_root / "scripts" / "persistence_power_loss_harness.py"
    return {
        "git_head": head,
        "rust_driver_sha256": hashlib.sha256(driver.read_bytes()).hexdigest(),
        "harness_sha256": hashlib.sha256(harness.read_bytes()).hexdigest(),
    }


def _write_report(
    workspace: OwnedWorkspace,
    report: dict[str, Any],
) -> pathlib.Path:
    assert_owned_root(workspace)
    report_path = workspace.root / "run-report.json"
    if report_path.exists():
        raise SafetyError("refusing to overwrite an existing run report")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(report_path, flags, 0o600)
    try:
        payload = (json.dumps(report, indent=2, sort_keys=True) + "\n").encode("utf-8")
        os.write(descriptor, payload)
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    return report_path


def run_privileged(repo_root: pathlib.Path, confirmation: str | None) -> int:
    if confirmation != CONFIRMATION_TOKEN:
        raise SafetyError(
            f"privileged mode requires --confirm {CONFIRMATION_TOKEN}"
        )
    if platform.system() != "Linux":
        raise SafetyError("privileged mode is Linux-only")
    if _euid() != 0:
        raise SafetyError("privileged mode requires effective uid 0")
    uid, gid, username, home = _original_user()
    preflight = collect_preflight(repo_root)
    if preflight["missing_tools"]:
        raise SafetyError(
            f"missing required tools: {', '.join(preflight['missing_tools'])}"
        )
    if not preflight["log_writes_target_present"]:
        raise SafetyError("the running kernel does not expose dm-log-writes")

    fixed_parent = _resolve_existing(ROOT_PARENT, "fixed workspace parent")
    if fixed_parent != ROOT_PARENT:
        raise SafetyError("/var/tmp must already be its canonical path")
    workspace = create_owned_workspace(ROOT_PARENT)
    recorder = Recorder()
    harness = DisposableReplayHarness(
        workspace,
        repo_root,
        recorder,
        uid,
        gid,
        username,
        home,
    )
    started = dt.datetime.now(dt.timezone.utc)
    status = "failed"
    failure: str | None = None
    try:
        source = _source_identity(repo_root, recorder)
        harness.execute()
        status = "passed"
    except Exception as error:  # noqa: BLE001 - report exact failed capability run
        source = {}
        failure = f"{type(error).__name__}: {error}"
    teardown_errors = harness.safe_teardown()
    if teardown_errors:
        status = "failed"
    report = {
        "schema": SCHEMA,
        "status": status,
        "started_utc": started.isoformat(),
        "finished_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "workspace": str(workspace.root),
        "nonce": workspace.nonce,
        "source": source,
        "preflight": preflight,
        "observations": harness.observations,
        "failure": failure,
        "teardown_errors": teardown_errors,
        "commands": recorder.commands,
        "claim_limit": (
            "This report covers only its disposable dm-log-writes images and marks. "
            "It does not prove a physical disk, controller cache, host power cut, or "
            "another filesystem."
        ),
    }
    report_path = _write_report(workspace, report)
    print(json.dumps({"status": status, "report": str(report_path)}, sort_keys=True))
    return 0 if status == "passed" else 1


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument(
        "--plan-json",
        action="store_true",
        help="print the fixed nonprivileged plan without changing state",
    )
    modes.add_argument(
        "--preflight",
        action="store_true",
        help="inspect prerequisites without creating files or devices",
    )
    modes.add_argument(
        "--run",
        action="store_true",
        help="run the explicit privileged disposable-image capability",
    )
    parser.add_argument(
        "--confirm",
        help=f"privileged mode requires the exact token {CONFIRMATION_TOKEN!r}",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    repo_root = pathlib.Path(__file__).resolve().parents[1]
    if args.plan_json:
        print(json.dumps(plan_document(repo_root), indent=2, sort_keys=True))
        return 0
    if args.preflight:
        print(json.dumps(collect_preflight(repo_root), indent=2, sort_keys=True))
        return 0
    return run_privileged(repo_root, args.confirm)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except SafetyError as error:
        print(f"safety refusal: {error}", file=sys.stderr)
        raise SystemExit(2) from error
