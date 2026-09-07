"""Stage and verify a disposable Windows Sandbox native-smoke run.

The host builds; the guest executes the unchanged strict native contract.
Only the staged payload and a fresh result directory are shared with the guest.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import uuid
import xml.etree.ElementTree as ET

import gui_native_smoke_contract as contract

GUEST_INPUT = r"C:\SorotteSandboxInput"
GUEST_OUTPUT = r"C:\SorotteSandboxOutput"
GUEST_WORK = r"C:\SorotteSandboxWork"
LEGACY_SHA = "d1c5f85af377c960c5a940707c4d01bc84fd9c3f"


def digest(path: pathlib.Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def read_json(path: pathlib.Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def write_json(path: pathlib.Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def git(repo: pathlib.Path, *args: str) -> bytes:
    return subprocess.check_output(
        ["git", "-c", f"safe.directory={repo.as_posix()}", "-C", str(repo), *args],
        timeout=120,
    )


def source_state(repo: pathlib.Path) -> dict:
    paths = git(
        repo, "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--",
        "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo", "crates",
        "fixtures", "requirements", "resources", "scripts",
    ).decode().split("\0")
    files = {name: digest(repo / name) for name in sorted(set(paths)) if name}
    return {"source_sha": git(repo, "rev-parse", "HEAD").decode().strip(), "files": files}


def payload_inventory(payload: pathlib.Path) -> dict[str, str]:
    result = {}
    for path in sorted(payload.rglob("*")):
        if path.is_symlink() or getattr(path, "is_junction", lambda: False)():
            raise ValueError(f"payload contains a link: {path}")
        if path.is_file() and path != payload / "manifest.json":
            result[path.relative_to(payload).as_posix()] = digest(path)
    return result


def validate_payload(run: pathlib.Path) -> dict:
    payload = run / "payload"
    manifest = read_json(payload / "manifest.json")
    if manifest.get("schema_version") != 1 or manifest.get("kind") != "sorotte-windows-sandbox":
        raise ValueError("unsupported sandbox manifest")
    uuid.UUID(manifest["run_id"])
    if manifest["files"] != payload_inventory(payload):
        raise ValueError("sandbox payload differs from its recorded inventory")
    expected = (run / "manifest.sha256").read_text().strip()
    if digest(payload / "manifest.json") != expected:
        raise ValueError("sandbox manifest was changed after preparation")
    if manifest["scenarios"] != list(contract.DEFAULT_REQUIRED_SCENARIOS):
        raise ValueError("sandbox must run the complete strict scenario inventory")
    if manifest["input_mode"] != "strict-physical":
        raise ValueError("sandbox must use strict physical input")
    timeout = manifest["timeout_ms"]
    if type(timeout) is not int or not 1 <= timeout <= 300000:
        raise ValueError("invalid sandbox scenario timeout")
    if manifest["wall_clock_timeout_ms"] != timeout * (len(manifest["scenarios"]) + 1) + 30000:
        raise ValueError("invalid sandbox watchdog timeout")
    expected_xml = sandbox_xml(run, manifest["source_root"], expected)
    if (run / "run.wsb").read_text(encoding="utf-8") != expected_xml:
        raise ValueError("sandbox configuration differs from its isolated launch contract")
    return manifest


def sandbox_xml(run: pathlib.Path, source_root: str, manifest_sha: str) -> str:
    config = ET.Element("Configuration")
    for name, value in (
        ("vGPU", "Enable"), ("MemoryInMB", "8192"), ("Networking", "Disable"),
        ("ClipboardRedirection", "Disable"), ("AudioInput", "Disable"),
        ("VideoInput", "Disable"), ("PrinterRedirection", "Disable"),
    ):
        ET.SubElement(config, name).text = value
    folders = ET.SubElement(config, "MappedFolders")
    # Rust's compatibility probe paths contain CARGO_MANIFEST_DIR. Mirror only
    # those two staged helper files at that guest path, never the live checkout.
    for host, guest, readonly in (
        (run / "payload", GUEST_INPUT, True),
        (run / "output", GUEST_OUTPUT, False),
        (run / "payload" / "compat-probes",
         str(pathlib.PureWindowsPath(source_root) / "crates/sorotte-compat/scripts"), True),
    ):
        folder = ET.SubElement(folders, "MappedFolder")
        ET.SubElement(folder, "HostFolder").text = str(host)
        ET.SubElement(folder, "SandboxFolder").text = guest
        ET.SubElement(folder, "ReadOnly").text = str(readonly).lower()
    logon = ET.SubElement(config, "LogonCommand")
    ET.SubElement(logon, "Command").text = (
        'powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden '
        f'-File "{GUEST_INPUT}\\gui-sandbox-guest.ps1" -ManifestSha256 {manifest_sha}'
    )
    ET.indent(config)
    return ET.tostring(config, encoding="unicode") + "\n"


def stage_python_requirements(repo: pathlib.Path, payload: pathlib.Path) -> pathlib.Path:
    # Keep pip's relative constraint include next to the staged requirement file.
    # Both files participate in the source snapshot and sealed payload inventory.
    names = ("legacy-python-interop.txt", "verification-constraints.txt")
    for name in names:
        if not (repo / "requirements" / name).is_file():
            raise ValueError(f"required Sandbox Python input is missing: {name}")
    for name in names:
        shutil.copy2(repo / "requirements" / name, payload / name)
    return payload / names[0]


def prepare(repo: pathlib.Path, run: pathlib.Path, target: pathlib.Path, timeout_ms: int) -> None:
    before = read_json(run / "source-state.json")
    if before != source_state(repo):
        raise ValueError("source changed while the native binaries were being built")
    payload = run / "payload"
    payload.mkdir()  # Existing runs are never overwritten or reused.
    (run / "output").mkdir()
    binaries = payload / "bin"
    binaries.mkdir()
    for name in ("sorotte-gui.exe", "sorotte-gui-native-smoke.exe"):
        shutil.copy2(target / "debug" / name, binaries / name)
    for name in ("gui-sandbox-guest.ps1", "gui-native-smoke-process.ps1", "gui_native_smoke_contract.py"):
        shutil.copy2(repo / "scripts" / name, payload / name)
    probes = payload / "compat-probes"
    probes.mkdir()
    for name in ("python_live_peer_probe.py", "python_handshake_probe.py"):
        shutil.copy2(repo / "crates/sorotte-compat/scripts" / name, probes / name)
    legacy = repo / ".interop-cache/syncplay-legacy"
    if git(legacy, "rev-parse", "HEAD").decode().strip() != LEGACY_SHA:
        raise ValueError("legacy Syncplay checkout is not the reviewed pinned revision")
    git(legacy, "archive", "--format=zip", "--prefix=legacy/",
        f"--output={payload / 'legacy.zip'}", LEGACY_SHA)

    # Copy the interpreter and standard library, excluding unrelated installed
    # packages and user customizations. Install only the declared dependencies.
    python_root = pathlib.Path(sys.base_prefix)
    runtime = payload / "python"
    runtime.mkdir()
    for pattern in ("python*.exe", "python*.dll", "vcruntime*.dll", "LICENSE.txt"):
        for path in python_root.glob(pattern):
            shutil.copy2(path, runtime / path.name)
    shutil.copytree(python_root / "DLLs", runtime / "DLLs")
    shutil.copytree(
        python_root / "Lib", runtime / "Lib",
        ignore=shutil.ignore_patterns("site-packages", "__pycache__", "test", "tests",
                                      "sitecustomize.py", "usercustomize.py"),
    )
    requirements = stage_python_requirements(repo, payload)
    with (run / "python-install.log").open("w", encoding="utf-8") as log:
        subprocess.run(
            [sys.executable, "-m", "pip", "--isolated", "install", "--disable-pip-version-check",
             "--no-compile", "--only-binary=:all:", "--target", str(runtime / "Lib/site-packages"),
             "-r", str(requirements)],
            check=True, stdout=log, stderr=subprocess.STDOUT, timeout=300,
        )
    env = dict(os.environ, PYTHONHOME=str(runtime), PYTHONNOUSERSITE="1", PYTHONDONTWRITEBYTECODE="1")
    env.pop("PYTHONPATH", None)
    subprocess.run(
        [str(runtime / "python.exe"), "-c",
         "import twisted, OpenSSL, service_identity, ssl; print('Sandbox Python dependencies ready')"],
        env=env, check=True, timeout=30,
    )
    manifest = {
        "schema_version": 1, "kind": "sorotte-windows-sandbox",
        "run_id": str(uuid.uuid4()), "source_sha": before["source_sha"],
        "source_state_sha256": digest(run / "source-state.json"),
        "source_root": str(repo), "host_computer": os.environ.get("COMPUTERNAME", ""),
        "python_version": sys.version, "legacy_sha": LEGACY_SHA,
        "input_mode": "strict-physical", "scenarios": list(contract.DEFAULT_REQUIRED_SCENARIOS),
        "timeout_ms": timeout_ms, "wall_clock_timeout_ms": timeout_ms * (len(contract.DEFAULT_REQUIRED_SCENARIOS) + 1) + 30000,
        "files": payload_inventory(payload),
    }
    if before != source_state(repo):
        raise ValueError("source changed while staging the native test payload")
    write_json(payload / "manifest.json", manifest)
    manifest_sha = digest(payload / "manifest.json")
    (run / "manifest.sha256").write_text(manifest_sha + "\n")
    (run / "run.wsb").write_text(sandbox_xml(run, str(repo), manifest_sha), encoding="utf-8")
    validate_payload(run)


def validate_result(run: pathlib.Path) -> dict:
    manifest = validate_payload(run)
    output = run / "output"
    result = read_json(output / "completion.json")
    if result["run_id"] != manifest["run_id"] or result["manifest_sha256"] != digest(run / "payload/manifest.json"):
        raise ValueError("guest result belongs to a different sandbox run")
    if result["status"] != "passed":
        raise ValueError(f"sandbox guest failed: {result.get('error')}")
    if not result["guest_preflight_passed"] or result["validator_exit_code"] != 0:
        raise ValueError("guest did not pass its desktop and strict evidence checks")
    if result["runner"]["timed_out"] or result["runner"]["start_error"]:
        raise ValueError("native runner timed out or could not start")
    for name in ("sorotte-gui.exe", "sorotte-gui-native-smoke.exe"):
        expected = manifest["files"][f"bin/{name}"]
        if result["binary_sha256_before"][name] != expected or result["binary_sha256_after"][name] != expected:
            raise ValueError(f"guest executable changed: {name}")
    report_text = (output / "native-report.json").read_text(encoding="utf-8-sig")
    report = contract.validate_native_smoke(
        report_text, (output / "native-stderr.log").read_text(encoding="utf-8-sig"),
        manifest["scenarios"], producer_exit_code=result["runner"]["exit_code"],
    )
    # samefile() cannot compare a disposed guest's path with a host file.
    # Bind its exact guest path and the before/after hashes above instead.
    expected_path = pathlib.PureWindowsPath(GUEST_WORK) / "bin/sorotte-gui.exe"
    # Rust canonicalize() emits the Win32 extended-length prefix. It names
    # the same local executable that the guest validator checked with samefile.
    # Removing only this prefix still rejects UNC, device, and unrelated paths.
    reported_path = pathlib.PureWindowsPath(report["binary"].removeprefix("\\\\?\\"))
    if reported_path != expected_path:
        raise ValueError("native report names an unexpected guest executable")
    return {"status": "passed", "run_id": manifest["run_id"],
            "source_sha": manifest["source_sha"], "input_mode": "strict-physical",
            "required_scenarios": manifest["scenarios"], "ci_attested": False}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("source-state", "prepare", "validate-payload", "validate-result"))
    parser.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[1])
    parser.add_argument("--run-directory", type=pathlib.Path, required=True)
    parser.add_argument("--target-directory", type=pathlib.Path)
    parser.add_argument("--timeout-ms", type=int, default=80000)
    args = parser.parse_args()
    repo, run = args.repo_root.resolve(), args.run_directory.resolve()
    if args.mode == "source-state":
        write_json(run / "source-state.json", source_state(repo))
    elif args.mode == "prepare":
        if args.timeout_ms <= 0 or args.timeout_ms > 300000:
            parser.error("timeout-ms must be between 1 and 300000")
        prepare(repo, run, (args.target_directory or repo / "target").resolve(), args.timeout_ms)
    elif args.mode == "validate-payload":
        validate_payload(run)
    else:
        result = validate_result(run)
        write_json(run / "sandbox-summary.json", result)
        print(json.dumps(result))


if __name__ == "__main__":
    main()
