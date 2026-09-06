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
import urllib.request
import uuid

PROFILE = Path(__file__).resolve().parents[1] / "verification/windows-native-guest.json"


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
    output.mkdir(parents=True, exist_ok=False)
    payload = output / "tools"
    payload.mkdir()
    for name in profile["tool_directories"]:
        shutil.copytree(tools / name, payload / name)
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


def collect_installed(sources: dict, output: Path) -> Path:
    """Construct the portable layout from explicitly selected installed tools.

    Copy only compiler/runtime responsibilities, excluding Python user packages
    and credentials. Sources are local inputs to the sealed manifest, not a
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
    runtime = output / "python312"
    runtime.mkdir()
    for pattern in ("python*.exe", "python*.dll", "vcruntime*.dll", "LICENSE.txt"):
        for path in roots["python"].glob(pattern):
            shutil.copy2(path, runtime / path.name)
    shutil.copytree(roots["python"] / "DLLs", runtime / "DLLs")
    shutil.copytree(roots["python"] / "Lib", runtime / "Lib",
                    ignore=shutil.ignore_patterns("site-packages", "__pycache__", "sitecustomize.py", "usercustomize.py"))
    # pip carries its dependencies under pip/_vendor. Copy no unrelated host
    # packages, startup hooks or .pth files into the pristine guest interpreter.
    packages = runtime / "Lib/site-packages"
    packages.mkdir()
    shutil.copytree(roots["python"] / "Lib/site-packages/pip", packages / "pip",
                    ignore=shutil.ignore_patterns("__pycache__"))
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
    files = inventory(bundle / "tools")
    if value["files"] != files or set(value["profile"]["required_files"]) - files.keys():
        raise ValueError("tool input closure changed or is incomplete")
    for name, item in value["profile"]["downloads"].items():
        if files.get(name) != item["sha256"]:
            raise ValueError("download differs from reviewed guest profile")
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
