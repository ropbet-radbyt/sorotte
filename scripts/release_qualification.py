#!/usr/bin/env python3
"""Source/input-bound release reuse. Local JSON is evidence, never authorization.

Coordinated consumers obtain artifacts from their own trusted Actions run. A retry
may name an explicit successful stable-release run; newest-green discovery is not
supported. Publication authorization is independently rechecked by merge_gate.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

import artifact_input
import verification_tools

SHA = re.compile(r"^[0-9a-f]{40}$")
DIGEST = re.compile(r"^[0-9a-f]{64}$")
PLATFORMS = {"linux-x86_64": "x86_64-unknown-linux-gnu", "windows-x86_64": "x86_64-pc-windows-msvc"}
LEGACY_SHA = verification_tools.pins()["references"]["legacy-sha"]


class QualificationError(ValueError):
    pass


def run(command: list[str], root: Path | None = None) -> str:
    if command[0] == "git" and root is not None:
        command = ["git", "-c", f"safe.directory={root.resolve().as_posix()}", *command[1:]]
    return subprocess.run(command, cwd=root, check=True, text=True, capture_output=True).stdout.strip()


def digest(value: object) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def read(path: Path) -> dict:
    return artifact_input.strict_json_load(path, expected_type=dict, max_bytes=16 * 1024 * 1024, label="release receipt")


def write(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(value, output, indent=2, sort_keys=True)
        output.write("\n")


def clean_source(root: Path, sha: str) -> dict:
    if not SHA.fullmatch(sha) or run(["git", "rev-parse", "HEAD"], root) != sha:
        raise QualificationError("release source differs from exact candidate SHA")
    if run(["git", "status", "--porcelain", "--untracked-files=all"], root):
        raise QualificationError("release source must be clean")
    files = run(["git", "ls-files", "-z"], root).split("\0")
    identities = {}
    for name in sorted(filter(None, files)):
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise QualificationError(f"non-regular tracked release input: {name}")
        identities[name] = artifact_input.sha256_file(path)
    return identities


def producer() -> dict:
    return {key: os.environ.get(env, "local") for key, env in (
        ("repository", "GITHUB_REPOSITORY"), ("run_id", "GITHUB_RUN_ID"),
        ("run_attempt", "GITHUB_RUN_ATTEMPT"), ("workflow_ref", "GITHUB_WORKFLOW_REF"),
        ("workflow_sha", "GITHUB_WORKFLOW_SHA"))}


def build_inputs(root: Path, sha: str, target_platform: str, channel: str, tools: dict[str, Path]) -> dict:
    if target_platform not in PLATFORMS or channel not in {"stable", "dev"}:
        raise QualificationError("unsupported release platform or channel")
    identities = {}
    for name in ("rustc", "cargo"):
        executable = Path(run(["rustup", "which", name]))
        identities[name] = {"version": run([str(executable), "-Vv"]), "sha256": artifact_input.sha256_file(executable)}
    if not identities["rustc"]["version"].startswith(f"rustc {verification_tools.pins()['tools']['rust']} "):
        raise QualificationError("release compiler differs from the reviewed tool pin")
    identities["python"] = {"version": sys.version, "sha256": artifact_input.sha256_file(Path(sys.executable))}
    for role, path in tools.items():
        if role not in {"mpv", "ffmpeg", "native-harness"} or not path.is_file():
            raise QualificationError("unknown or missing qualification tool")
        identities[role] = {"sha256": artifact_input.sha256_file(path)}
    os_packages = run(["dpkg-query", "-W", "-f=${Package}=${Version}\n"]) if sys.platform == "linux" else "not-applicable"
    value = {
        "schema_version": 1, "kind": "sorotte-release-build-inputs", "candidate_sha": sha,
        "source_files": clean_source(root, sha), "platform": target_platform,
        "target": PLATFORMS[target_platform], "profile": "release", "features": "default",
        "instrumentation": "none", "channel": channel,
        "source_ref": os.environ.get("GITHUB_REF", "local"), "tools": identities,
        "python_packages": {dist.metadata["Name"]: dist.version for dist in importlib.metadata.distributions() if dist.metadata.get("Name")},
        "environment": {"os": platform.platform(), "runner_image": os.environ.get("ImageOS", platform.system()),
            "runner_image_version": os.environ.get("ImageVersion", os.environ.get("SOROTTE_NATIVE_IMAGE_SHA256", "local")),
            "os_packages_sha256": hashlib.sha256(os_packages.encode()).hexdigest(),
            "os_packages": os_packages,
            "rustflags": os.environ.get("RUSTFLAGS", ""), "encoded_rustflags": os.environ.get("CARGO_ENCODED_RUSTFLAGS", ""),
            "rustdocflags": os.environ.get("RUSTDOCFLAGS", "")},
        "producer": producer(),
    }
    if any(value["environment"][key] for key in ("rustflags", "encoded_rustflags", "rustdocflags")):
        raise QualificationError("release qualification forbids instrumented or custom Rust flags")
    return value


def validate_inputs(value: dict, *, sha: str, target_platform: str, channel: str) -> None:
    keys = {"schema_version", "kind", "candidate_sha", "source_files", "platform", "target", "profile", "features", "instrumentation", "channel", "source_ref", "tools", "python_packages", "environment", "producer"}
    if set(value) != keys or type(value.get("schema_version")) is not int or value["schema_version"] != 1 or value.get("kind") != "sorotte-release-build-inputs":
        raise QualificationError("build inputs have unsupported closed schema")
    expected = {"candidate_sha": sha, "platform": target_platform, "target": PLATFORMS[target_platform], "profile": "release", "features": "default", "instrumentation": "none", "channel": channel}
    if any(value.get(k) != v for k, v in expected.items()):
        raise QualificationError("build input source/platform/profile/features/channel mismatch")
    if not isinstance(value["source_files"], dict) or not {"Cargo.lock", "Cargo.toml"} <= value["source_files"].keys():
        raise QualificationError("build input source inventory is incomplete")
    if any(not isinstance(v, str) or DIGEST.fullmatch(v) is None for v in value["source_files"].values()):
        raise QualificationError("build input source digest is invalid")
    if not isinstance(value["tools"], dict) or not {"cargo", "rustc", "python", "mpv", "ffmpeg"} <= value["tools"].keys():
        raise QualificationError("build input tool inventory is incomplete")
    if target_platform == "windows-x86_64" and "native-harness" not in value["tools"]:
        raise QualificationError("Windows build input is missing the actual native driver identity")
    for tool in value["tools"].values():
        if not isinstance(tool, dict) or DIGEST.fullmatch(str(tool.get("sha256", ""))) is None:
            raise QualificationError("build input tool digest is invalid")
    if not isinstance(value["producer"], dict) or set(value["producer"]) != {"repository", "run_id", "run_attempt", "workflow_ref", "workflow_sha"}:
        raise QualificationError("build producer identity is incomplete")
    if not isinstance(value["python_packages"], dict) or any(not isinstance(name, str) or not isinstance(version, str) for name, version in value["python_packages"].items()):
        raise QualificationError("build inputs omit resolved Python distribution identity")
    environment_keys = {"os", "runner_image", "runner_image_version", "os_packages_sha256", "os_packages", "rustflags", "encoded_rustflags", "rustdocflags"}
    if (not isinstance(value["environment"], dict) or set(value["environment"]) != environment_keys
        or any(value["environment"].get(k) != "" for k in ("rustflags", "encoded_rustflags", "rustdocflags"))
        or any(not isinstance(v, str) for v in value["environment"].values())):
        raise QualificationError("build inputs contain unsupported instrumentation")
    if DIGEST.fullmatch(value["environment"]["os_packages_sha256"]) is None:
        raise QualificationError("build inputs omit operating-system package identity")
    if hashlib.sha256(value["environment"]["os_packages"].encode()).hexdigest() != value["environment"]["os_packages_sha256"]:
        raise QualificationError("build input operating-system package inventory changed")


def consume(bundle_dir: Path, complete_path: Path, root: Path, sha: str, target_platform: str, channel: str, expected_run_id: str | None) -> dict:
    import playback_release_gate as gate

    manifest = gate.read_bundle(bundle_dir, sha, target_platform)
    inputs = manifest.get("build_inputs")
    if not isinstance(inputs, dict):
        raise QualificationError("legacy bundle cannot authorize release reuse")
    validate_inputs(inputs, sha=sha, target_platform=target_platform, channel=channel)
    if clean_source(root, sha) != inputs["source_files"]:
        raise QualificationError("qualified build source inputs differ from consumer checkout")
    if expected_run_id:
        if (inputs["producer"]["run_id"] != expected_run_id
            or inputs["producer"]["repository"] != os.environ.get("GITHUB_REPOSITORY")
            or inputs["producer"]["workflow_sha"] != sha
            or inputs["source_ref"] != os.environ.get("GITHUB_REF")):
            raise QualificationError("bundle producer is not the authorized qualification run")
    report = read(complete_path)
    if report.get("kind") != gate.COMPLETE_KIND or report.get("result") != "passed" or report.get("candidate_sha") != sha:
        raise QualificationError("complete qualification is not an exact-source pass")
    if set(report.get("candidate_manifest_sha256", {})) != set(PLATFORMS):
        raise QualificationError("complete qualification is missing a platform")
    if report["candidate_manifest_sha256"][target_platform] != artifact_input.sha256_file(bundle_dir / "candidate-manifest.json"):
        raise QualificationError("complete qualification does not attest downloaded bundle bytes")
    if report.get("model_sha256") != artifact_input.sha256_file(root / "coverage/playback-lifecycle.toml"):
        raise QualificationError("complete qualification model is stale")
    if report.get("required_system_transitions") != report.get("system_transition_coverage") or not report.get("required_system_transitions"):
        raise QualificationError("complete qualification omitted required lifecycle transitions")
    return manifest


def workspace_receipt(root: Path, sha: str, target_platform: str, features: str) -> dict:
    # The writer executes the obligation; callers cannot stamp an arbitrary pass.
    if any(os.environ.get(k) for k in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTDOCFLAGS")):
        raise QualificationError("workspace receipt cannot cover instrumented execution")
    source = clean_source(root, sha)
    command = ["cargo", "test", "--locked", "--workspace"]
    if features == "all":
        command += ["--all-features"]
    subprocess.run(command, cwd=root, check=True)
    if source != clean_source(root, sha):
        raise QualificationError("source changed during workspace execution")
    return {"schema_version": 1, "kind": "sorotte-release-workspace-receipt", "result": "passed", "candidate_sha": sha,
        "platform": target_platform, "features": features, "profile": "test", "instrumentation": "none",
        "command": command, "source_files": source, "rustc": run(["rustc", "-Vv"]), "producer": producer()}


def validate_workspace(value: dict, root: Path, sha: str, target_platform: str, expected_run_id: str) -> None:
    if any(os.environ.get(k) for k in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTDOCFLAGS")):
        raise QualificationError("workspace receipt cannot be reused in an instrumented environment")
    expected = {"schema_version": 1, "kind": "sorotte-release-workspace-receipt", "result": "passed", "candidate_sha": sha,
        "platform": target_platform, "features": "default", "profile": "test", "instrumentation": "none",
        "command": ["cargo", "test", "--locked", "--workspace"], "source_files": clean_source(root, sha), "rustc": run(["rustc", "-Vv"])}
    if set(value) != set(expected) | {"producer"} or any(value.get(k) != v for k, v in expected.items()):
        raise QualificationError("workspace receipt does not prove identical default-feature execution")
    if not expected_run_id or value["producer"].get("run_id") != expected_run_id or value["producer"].get("repository") != os.environ.get("GITHUB_REPOSITORY"):
        raise QualificationError("workspace receipt lacks trusted producer provenance")


def verify_legacy(root: Path) -> None:
    if not (root / "syncplayServer.py").is_file() or run(["git", "rev-parse", "HEAD"], root) != LEGACY_SHA:
        raise QualificationError(f"legacy oracle must be pinned to {LEGACY_SHA}")
    if run(["git", "status", "--porcelain", "--untracked-files=all"], root):
        raise QualificationError("legacy oracle checkout must be clean")


def validate_server_behavior(value: dict, sha: str) -> None:
    if value.get("status") != "PASS" or value.get("sourceSha") != sha or value.get("stage") != "Behavior":
        raise QualificationError("server behavior receipt is not an exact-source behavior pass")
    if value.get("legacyOracle", {}).get("sha") != LEGACY_SHA:
        raise QualificationError("server behavior did not consume the pinned legacy oracle")
    names = [step.get("Step") for step in value.get("steps", [])]
    required = {"legacy oracle", "python prerequisites", "fmt", "sorotte-server tests", "sorotte-compat tests", "verify prior default workspace receipt", "strict live legacy compatibility", "clippy", "strict server release matrix", "verify final legacy reference", "verify final immutable input closure"}
    if len(names) != len(set(names)) or set(names) != required or any(step.get("Status") != "PASS" for step in value["steps"]):
        raise QualificationError("server behavior receipt omitted a required execution")


def validate_package(report: dict, manifest: dict) -> None:
    if report.get("status") != "verified" or report.get("package", {}).get("sourceSha") != manifest["candidate_sha"]:
        raise QualificationError("archive consumption did not pass for the qualified source")
    package = report["package"]
    if package.get("name") not in {"sorotte-server", "sorotte-gui"}:
        raise QualificationError("archive receipt has an unknown package identity")
    roles = {"server"} if package.get("name") == "sorotte-server" else {"gui", "gui-updater"}
    runtime_key = "runtimeSmoke" if roles == {"server"} else "runtimeProof"
    if report.get(runtime_key, {}).get("performed") is not True:
        raise QualificationError("archive receipt did not execute package runtime boundaries")
    files = package.get("files")
    if not isinstance(files, list):
        raise QualificationError("archive receipt has no tested file inventory")
    by_path = {item["path"]: item for item in files}
    if len(by_path) != len(files):
        raise QualificationError("archive receipt contains duplicate files")
    for role in roles:
        identity = manifest["files"].get(role)
        if identity is None or by_path.get(identity["file_name"], {}).get("sha256") != identity["sha256"]:
            raise QualificationError("archive consumed different binary bytes from the lifecycle qualification")


def validate_producer_run(value: dict, sha: str, repository: str, run_id: str, version_tag: str) -> int:
    if not re.fullmatch(r"(?:server-)?v[0-9][A-Za-z0-9._-]*", version_tag):
        raise QualificationError("promotion requires an exact stable version tag")
    if (str(value.get("id")) != run_id or value.get("head_sha") != sha
        or value.get("repository", {}).get("full_name") != repository
        or value.get("head_repository", {}).get("full_name") != repository
        or value.get("event") != "push" or value.get("head_branch") != version_tag
        or value.get("path") != ".github/workflows/stable-release.yml"
        or value.get("status") != "completed" or value.get("conclusion") != "success"
        or type(value.get("run_attempt")) is not int or value["run_attempt"] < 1):
        raise QualificationError("publication evidence producer is not the explicit successful trusted tag run")
    return value["run_attempt"]


def archive_evidence(root: Path, output: Path, sha: str) -> None:
    if not SHA.fullmatch(sha):
        raise QualificationError("evidence archive requires an exact source SHA")
    members = {}
    records = []
    kinds = {"release-authorization", "sorotte-playback-release-candidate-bundle", "sorotte-playback-release-platform-gate", "sorotte-playback-release-complete-gate", "sorotte-release-workspace-receipt"}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise QualificationError("evidence archive cannot follow symlinks")
        if path.is_dir():
            continue
        if path.suffix != ".json" or not path.is_file():
            raise QualificationError("durable qualification inventory accepts only structured JSON receipts")
        value = read(path)
        if value.get("kind") not in kinds or value.get("candidate_sha") != sha:
            raise QualificationError("durable qualification receipt has unknown authority or stale source")
        if value.get("result", value.get("status")) != "passed":
            raise QualificationError("durable qualification receipt is not passed")
        members[path.relative_to(root).as_posix()] = {"sha256": artifact_input.sha256_file(path), "size": path.stat().st_size}
        records.append(value)
    if not members:
        raise QualificationError("durable qualification evidence inventory is empty")
    observed = {read(root / name)["kind"] for name in members}
    if observed != kinds:
        raise QualificationError("durable qualification archive is missing an authority")
    for kind in kinds:
        selected = [record for record in records if record["kind"] == kind]
        if kind in {"release-authorization", "sorotte-playback-release-complete-gate"}:
            if len(selected) != 1:
                raise QualificationError("durable qualification archive duplicated a final authority")
        elif len(selected) != 2 or {record.get("platform") for record in selected} != set(PLATFORMS):
            raise QualificationError("durable qualification archive is missing or duplicating a platform")
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"sorotte-qualification-{sha}.zip"
    with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED) as package:
        for name in members:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            package.writestr(info, (root / name).read_bytes())
        info = zipfile.ZipInfo("receipt-index.json", date_time=(1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = 0o100644 << 16
        package.writestr(info, json.dumps({"schema_version": 1, "candidate_sha": sha, "files": members}, sort_keys=True, indent=2))
    (output / f"{archive.name}.sha256").write_text(f"{artifact_input.sha256_file(archive)}  {archive.name}\n", encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    legacy = sub.add_parser("verify-legacy")
    legacy.add_argument("--legacy-root", required=True, type=Path)
    behavior = sub.add_parser("verify-server-behavior")
    behavior.add_argument("--candidate-sha", required=True)
    behavior.add_argument("--receipt", required=True, type=Path)
    provenance = sub.add_parser("verify-producer-run")
    provenance.add_argument("--candidate-sha", required=True)
    provenance.add_argument("--run-id", required=True)
    provenance.add_argument("--version-tag", required=True)
    provenance.add_argument("--repository", required=True)
    archive = sub.add_parser("archive-evidence")
    archive.add_argument("--candidate-sha", required=True)
    archive.add_argument("--evidence-dir", required=True, type=Path)
    archive.add_argument("--output-dir", required=True, type=Path)
    for name in ("inputs", "consume", "verify-package", "workspace", "verify-workspace"):
        p = sub.add_parser(name)
        p.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
        p.add_argument("--candidate-sha", required=True)
        p.add_argument("--platform", choices=PLATFORMS, required=True)
        if name in {"inputs", "consume", "verify-package"}:
            p.add_argument("--channel", choices=("stable", "dev"), default="stable")
        if name in {"inputs", "workspace"}:
            p.add_argument("--output", type=Path, required=True)
        if name == "inputs":
            p.add_argument("--tool", action="append", default=[])
        if name in {"consume", "verify-package"}:
            p.add_argument("--bundle-dir", type=Path, required=True)
            p.add_argument("--complete-receipt", type=Path, required=True)
            p.add_argument("--expected-run-id", default=os.environ.get("GITHUB_RUN_ID"))
        if name == "verify-package":
            p.add_argument("--artifact-report", type=Path, required=True)
        if name == "workspace":
            p.add_argument("--features", choices=("default", "all"), default="default")
        if name == "verify-workspace":
            p.add_argument("--receipt", type=Path, required=True)
            p.add_argument("--expected-run-id", required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "verify-legacy":
            verify_legacy(args.legacy_root)
        elif args.command == "verify-server-behavior":
            validate_server_behavior(read(args.receipt), args.candidate_sha)
        elif args.command == "verify-producer-run":
            if not re.fullmatch(r"[0-9]+", args.run_id) or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repository):
                raise QualificationError("invalid explicit producer identity")
            value = json.loads(run(["gh", "api", f"repos/{args.repository}/actions/runs/{args.run_id}"]))
            attempt = validate_producer_run(value, args.candidate_sha, args.repository, args.run_id, args.version_tag)
            if os.environ.get("GITHUB_OUTPUT"):
                with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
                    output.write(f"attempt={attempt}\n")
            print(f"authorized producer {args.run_id} attempt {attempt}")
        elif args.command == "archive-evidence":
            archive_evidence(args.evidence_dir, args.output_dir, args.candidate_sha)
        elif args.command == "inputs":
            tools = {}
            for item in args.tool:
                role, separator, path = item.partition("=")
                if not separator or role in tools:
                    raise QualificationError("tool requires unique role=path")
                tools[role] = Path(shutil.which(path) or path)
            value = build_inputs(args.repo_root, args.candidate_sha, args.platform, args.channel, tools)
            validate_inputs(value, sha=args.candidate_sha, target_platform=args.platform, channel=args.channel)
            write(args.output, value)
        elif args.command in {"consume", "verify-package"}:
            manifest = consume(args.bundle_dir, args.complete_receipt, args.repo_root, args.candidate_sha, args.platform, args.channel, args.expected_run_id)
            if args.command == "verify-package":
                validate_package(read(args.artifact_report), manifest)
        elif args.command == "workspace":
            write(args.output, workspace_receipt(args.repo_root, args.candidate_sha, args.platform, args.features))
        else:
            validate_workspace(read(args.receipt), args.repo_root, args.candidate_sha, args.platform, args.expected_run_id)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"release qualification failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
