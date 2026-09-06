"""Prepare and verify the reviewed portable Windows runner input closure.

The profile is versioned in the repo. Operator-installed MSVC/SDK and portable
tools are copied into a closed, content-hashed bundle; none of the host's home,
Git credentials, caches or live checkout is mapped into the guest.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import urllib.request
import uuid

PROFILE = Path(__file__).resolve().parents[1] / "verification/windows-native-guest.json"
PYTHON_PROBE = Path(__file__).with_name("native_python_probe.py")
PYTHON_REQUIREMENTS = Path(__file__).resolve().parents[1] / "requirements/legacy-python-interop.txt"
PYTHON_POLICY_REQUIREMENTS = PYTHON_REQUIREMENTS.with_name("ci-policy.txt")
PYTHON_CONSTRAINTS = PYTHON_REQUIREMENTS.with_name("verification-constraints.txt")
NATIVE_CANARIES = Path(__file__).resolve().parents[1] / "coverage/native-harness-canaries.json"


def python_contract(version: str) -> dict:
    def pins(path: Path, *, allow_constraint: bool = False) -> dict[str, str]:
        result = {}
        for raw in path.read_text(encoding="utf-8").splitlines():
            line = raw.split("#", 1)[0].strip()
            if not line or (allow_constraint and line == "-c verification-constraints.txt"):
                continue
            match = re.fullmatch(r"([A-Za-z0-9_.-]+)==([A-Za-z0-9+_.-]+)", line)
            if match is None:
                raise ValueError("native Python inputs require exact reviewed package pins")
            name = re.sub(r"[-_.]+", "-", match[1]).lower()
            if name in result:
                raise ValueError("native Python inputs repeat a package pin")
            result[name] = match[2]
        return result

    constraints = pins(PYTHON_CONSTRAINTS)
    requirements = pins(PYTHON_REQUIREMENTS, allow_constraint=True)
    # Native readiness also runs the reviewed Python policy canaries. Their
    # parser imports are prerequisites even though interop itself does not use
    # them; bind both existing input files rather than testing only interop.
    for name, policy_version in pins(PYTHON_POLICY_REQUIREMENTS, allow_constraint=True).items():
        if name in requirements and requirements[name] != policy_version:
            raise ValueError("native Python policy and interop pins disagree")
        requirements[name] = policy_version
    requirements["pip"] = constraints["pip"]
    return {"schema_version": 1, "kind": "sorotte-native-python-contract", "python_version": version,
            "requirements": requirements, "constraints": constraints,
            "imports": ["unittest", "yaml", "pip._internal.cli.main", "twisted.internet.reactor", "OpenSSL.SSL",
                        "cryptography.hazmat.bindings._rust", "service_identity.pyopenssl", "zope.interface", "_cffi_backend"],
            "requirements_sha256": digest(PYTHON_REQUIREMENTS), "constraints_sha256": digest(PYTHON_CONSTRAINTS),
            "policy_requirements_sha256": digest(PYTHON_POLICY_REQUIREMENTS),
            "canary_inventory_sha256": digest(NATIVE_CANARIES)}


def probe_python(runtime: Path, contract: dict, *, collect_files: bool = False) -> dict:
    arguments = [str(runtime / "python.exe"), "-I", "-B", str(PYTHON_PROBE),
                 "--contract-json", json.dumps(contract, separators=(",", ":"))]
    if collect_files:
        arguments.append("--collect-files")
    try:
        process = subprocess.run(arguments, capture_output=True, text=True, encoding="utf-8", timeout=45,
                                 creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError("selected native Python readiness could not complete") from error
    if process.returncode:
        raise ValueError("selected native Python is not ready: " + process.stderr.strip()[-2000:])
    if len(process.stdout) > 8 * 1024 * 1024:
        raise ValueError("native Python readiness output exceeded its bound")
    result = json.loads(process.stdout)
    if (result.get("kind") != "sorotte-native-python-readiness" or result.get("result") != "passed"
            or result.get("python_version") != contract["python_version"] or result.get("isolated") is not True
            or result.get("pip_command") != "passed" or result.get("imports") != contract["imports"]):
        raise ValueError("native Python readiness result is incomplete")
    return result


def read(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def digest(path: Path) -> str:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def inventory(root: Path) -> dict[str, str]:
    result = {}
    for parent in (root, *root.parents):
        if parent.exists() and (parent.is_symlink() or getattr(parent.lstat(), "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT):
            raise ValueError("bundle root contains a link or reparse point")
    for path in sorted(root.rglob("*")):
        if path.is_symlink() or getattr(path.lstat(), "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
            raise ValueError("bundle contains a link or reparse point")
        if path.is_file():
            name = path.relative_to(root).as_posix()
            if any(part.lower() in {".git", ".ssh", ".credentials", ".credentials_rsaparams", ".runner"} for part in path.parts):
                raise ValueError("bundle contains private credentials or runner registration")
            result[name] = digest(path)
    return result


def validate_profile(profile: dict) -> None:
    if profile.get("schema_version") != 1 or profile.get("kind") != "sorotte-windows-native-guest-profile":
        raise ValueError("unsupported Windows guest profile")
    if profile.get("max_jobs") != 1 or profile.get("runner_contract") != "sorotte-ephemeral-interactive-windows-v1":
        raise ValueError("only the one-job isolated interactive contract is supported")
    for name in [*profile["required_files"], *profile["tool_directories"], *profile["downloads"]]:
        path = PurePosixPath(name)
        if path.is_absolute() or ".." in path.parts or "\\" in name or ":" in name:
            raise ValueError("tool path escapes bundle")
    for value in profile["downloads"].values():
        if not re.fullmatch(r"[0-9a-f]{64}", value["sha256"]) or not value["url"].startswith("https://"):
            raise ValueError("download must have HTTPS origin and reviewed SHA-256")


def download(url: str, expected: str, destination: Path) -> None:
    temporary = destination.with_name(destination.name + ".partial-" + str(uuid.uuid4()))
    try:
        with urllib.request.urlopen(url, timeout=60) as response, temporary.open("xb") as handle:
            if not response.geturl().startswith("https://"):
                raise ValueError("download redirected outside HTTPS")
            shutil.copyfileobj(response, handle)
        if digest(temporary) != expected:
            raise ValueError("download digest differs from reviewed profile")
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)


def prepare(tools: Path, output: Path, profile_path: Path = PROFILE) -> dict:
    profile = read(profile_path)
    validate_profile(profile)
    # Inventory first: copytree must never dereference a link into host files.
    inventory(tools)
    contract = python_contract(profile["python_version"])
    probe_python(tools / "python312", contract)
    output.mkdir(parents=True, exist_ok=False)
    payload = output / "tools"
    payload.mkdir()
    for name in profile["tool_directories"]:
        shutil.copytree(tools / name, payload / name)
    shutil.copy2(PYTHON_PROBE, payload / "python-runtime-probe.py")
    (payload / "python-runtime-contract.json").write_text(json.dumps(contract, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    for name, item in profile["downloads"].items():
        source = tools / name
        if source.is_file():
            if digest(source) != item["sha256"]:
                raise ValueError(f"cached download digest differs: {name}")
            shutil.copy2(source, payload / name)
        else:
            download(item["url"], item["sha256"], payload / name)
    files = inventory(payload)
    missing = set(profile["required_files"]) - files.keys()
    if missing:
        raise ValueError("tool bundle is incomplete: " + ", ".join(sorted(missing)))
    manifest = {"schema_version": 1, "kind": "sorotte-native-runner-inputs",
                "profile_sha256": digest(profile_path), "profile": profile, "files": files}
    path = output / "tools-manifest.json"
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (output / "manifest.sha256").write_text(digest(path) + "\n", encoding="ascii")
    return validate(output)


def collect_python_runtime(source: Path, output: Path) -> Path:
    """Copy a clean full runtime and only the constrained interop dependencies."""
    inventory(source)
    contract = python_contract(read(PROFILE)["python_version"])
    readiness = probe_python(source, contract, collect_files=True)
    output.mkdir(parents=True, exist_ok=False)
    for pattern in ("python*.exe", "python*.dll", "vcruntime*.dll", "LICENSE.txt"):
        for path in source.glob(pattern):
            shutil.copy2(path, output / path.name)
    shutil.copytree(source / "DLLs", output / "DLLs")
    shutil.copytree(source / "Lib", output / "Lib",
                    ignore=shutil.ignore_patterns("site-packages", "__pycache__", "sitecustomize.py", "usercustomize.py"))
    for name in readiness["distribution_files"]:
        path = PurePosixPath(name)
        if path.is_absolute() or ".." in path.parts or path.parts[:2] != ("Lib", "site-packages"):
            raise ValueError("Python dependency file escapes the approved package directory")
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source / name, destination)
    # In particular, namespace imports must still work without copied .pth code.
    probe_python(output, contract)
    return output


def collect_installed(sources: dict, output: Path) -> Path:
    """Construct the portable layout from explicitly selected installed tools.

    Copy only compiler/runtime responsibilities and the pinned interop package
    closure, excluding unrelated user packages and credentials. Sources are
    local inputs to the sealed manifest, not a
    claim that an arbitrary installed version has been qualified already.
    """
    expected = {"msvc", "windows_sdk", "git", "powershell", "cmake", "ninja", "7zip", "python"}
    if set(sources) != expected or any(not isinstance(path, str) for path in sources.values()):
        raise ValueError("installed tool sources must name exactly the reviewed tool inventory")
    roots = {key: Path(value).absolute() for key, value in sources.items()}
    for path in roots.values():
        inventory(path if path.is_dir() else path.parent)
    profile = read(PROFILE)
    if roots["msvc"].name != profile["msvc_version"]:
        raise ValueError("installed MSVC source differs from the reviewed guest profile")
    output.mkdir(parents=True, exist_ok=False)
    collect_python_runtime(roots["python"], output / "python312")
    for name in ("git", "powershell", "cmake", "7zip"):
        shutil.copytree(roots[name], output / name)
    for source, target in (("bin/Hostx64/x64", "bin"), ("include", "include"), ("lib/x64", "lib")):
        shutil.copytree(roots["msvc"] / source, output / "msvc" / target)
    sdk = roots["windows_sdk"]
    version = profile["sdk_version"]
    shutil.copytree(sdk / "bin" / version / "x64", output / "sdk/bin")
    for name in ("ucrt", "shared", "um", "winrt"):
        shutil.copytree(sdk / "Include" / version / name, output / "sdk/include" / name)
    for name in ("ucrt", "um"):
        shutil.copytree(sdk / "Lib" / version / name / "x64", output / "sdk/lib" / name)
    (output / "ninja").mkdir()
    shutil.copy2(roots["ninja"], output / "ninja/ninja.exe")
    return output


def validate(bundle: Path) -> dict:
    path = bundle / "tools-manifest.json"
    if digest(path) != (bundle / "manifest.sha256").read_text().strip():
        raise ValueError("tool manifest changed after preparation")
    value = read(path)
    if value.get("schema_version") != 1 or value.get("kind") != "sorotte-native-runner-inputs":
        raise ValueError("unsupported tool input manifest")
    validate_profile(value["profile"])
    if value["profile_sha256"] != digest(PROFILE) or value["profile"] != read(PROFILE):
        raise ValueError("tool inputs use a different reviewed guest profile")
    contract = python_contract(value["profile"]["python_version"])
    probe_path = bundle / "tools/python-runtime-probe.py"
    contract_path = bundle / "tools/python-runtime-contract.json"
    if (not probe_path.is_file() or probe_path.read_bytes() != PYTHON_PROBE.read_bytes()
            or not contract_path.is_file() or read(contract_path) != contract):
        raise ValueError("tool inputs lack the current reviewed native Python readiness contract")
    files = inventory(bundle / "tools")
    if value["files"] != files or set(value["profile"]["required_files"]) - files.keys():
        raise ValueError("tool input closure changed or is incomplete")
    for name, item in value["profile"]["downloads"].items():
        if files.get(name) != item["sha256"]:
            raise ValueError("download differs from reviewed guest profile")
    probe_python(bundle / "tools/python312", contract)
    return value


def validate_assignment(value: dict, run: dict, job: dict, profile: dict) -> None:
    """Validate real GitHub API observations; no newest-green/source-only reuse."""
    if not re.fullmatch(r"[0-9a-f]{40}", value.get("source_sha", "")):
        raise ValueError("invalid trusted source SHA")
    if any(type(value.get(key)) is not int or value[key] <= 0 for key in ("run_id", "run_attempt", "job_id")):
        raise ValueError("invalid exact run/attempt/job assignment")
    if (run.get("id") != value["run_id"] or run.get("run_attempt") != value["run_attempt"]
            or run.get("head_sha") != value["source_sha"]
            or run.get("path") not in profile["allowed_workflows"]
            or run.get("event") not in {"workflow_dispatch", "push", "schedule"}
            or run.get("head_repository", {}).get("full_name") != profile["repository"]):
        raise ValueError("workflow source, provenance, or attempt differs from authorization")
    if (job.get("id") != value["job_id"] or job.get("run_id") != value["run_id"]
            or job.get("head_sha") != value["source_sha"] or job.get("status") != "queued"
            or not {"self-hosted", "Windows", "X64", "sorotte-native-interactive", "sorotte-ephemeral"}.issubset(job.get("labels", []))):
        raise ValueError("authorized job is not the exact queued isolated Windows job")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["prepare", "collect-installed", "validate", "validate-assignment"])
    parser.add_argument("--tools-root", type=Path)
    parser.add_argument("--sources", type=Path)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--assignment", type=Path)
    parser.add_argument("--run", type=Path)
    parser.add_argument("--job", type=Path)
    args = parser.parse_args()
    if args.mode == "prepare":
        if args.tools_root is None:
            parser.error("prepare requires --tools-root")
        prepare(args.tools_root.resolve(), args.bundle.absolute())
    elif args.mode == "collect-installed":
        if args.sources is None:
            parser.error("collect-installed requires --sources")
        staging = args.bundle.with_name(args.bundle.name + "-installed-inputs")
        collect_installed(read(args.sources), staging)
        prepare(staging, args.bundle.absolute())
    elif args.mode == "validate":
        validate(args.bundle)
    else:
        value = validate(args.bundle)
        validate_assignment(read(args.assignment), read(args.run), read(args.job), value["profile"])


if __name__ == "__main__":
    main()
