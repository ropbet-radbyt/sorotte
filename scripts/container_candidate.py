"""Seal and restore an already-tested Docker image without rebuilding or running it."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import sys

import artifact_input
import candidate_authority
import verify_server_container as container


def evidence(directory: Path, source: str, repository: str) -> tuple[dict, dict]:
    candidate_authority.sha(source, "container candidate")
    runtime_path = directory / "runtime/runtime-report.json"
    sbom_path = directory / "sbom.spdx.json"
    report_path = directory / "sbom-report.json"
    runtime = container.parse_runtime_report(runtime_path)
    sbom = container.parse_sbom_report(report_path)
    local = runtime["localImage"]
    if (local["sourceSha"] != source or local["source"] != f"https://github.com/{repository}"
            or sbom["sourceSha"] != source or sbom["source"] != local["source"]
            or sbom["localImageId"] != local["id"] or sbom["rootfsDiffIds"] != local["rootfsDiffIds"]
            or sbom["sbomSha256"] != "sha256:" + artifact_input.sha256_file(sbom_path)):
        raise ValueError("container runtime and SBOM do not identify the same tested source and image")
    hashes = {path.relative_to(directory).as_posix(): artifact_input.sha256_file(path)
              for path in (runtime_path, sbom_path, report_path)}
    return local, hashes


def seal(source: str, image: str, directory: Path, output: Path, repository: str) -> dict:
    local, hashes = evidence(directory, source, repository)
    observed = container.inspect_local_image(image, expected_source_sha=source, expected_source_url=local["source"])
    if observed != local:
        raise ValueError("container changed after runtime qualification")
    output.mkdir(parents=True, exist_ok=False)
    archive = output / "candidate-image.tar"
    # Save the immutable config ID, not a mutable local tag.
    subprocess.run(["docker", "image", "save", "--output", str(archive), local["id"]], check=True, timeout=300)
    value = {"schema_version": 1, "kind": "sorotte-tested-container-export", "status": "passed",
             "source_sha": source, "repository": repository, "local_image": local, "evidence_sha256": hashes,
             "archive": {"name": archive.name, "sha256": artifact_input.sha256_file(archive), "size": archive.stat().st_size}}
    candidate_authority.write(output / "container.json", value)
    return value


def load(source: str, image: str, directory: Path, bundle: Path, repository: str) -> dict:
    local, hashes = evidence(directory, source, repository)
    files = [path for path in bundle.iterdir() if path.name != ".authority"]
    if ({path.name for path in files} != {"container.json", "candidate-image.tar"}
            or any(path.is_symlink() or not path.is_file() for path in files)):
        raise ValueError("container export must contain exactly its regular manifest and image archive")
    value = artifact_input.strict_json_load(bundle / "container.json", expected_type=dict)
    archive = bundle / "candidate-image.tar"
    expected = {"schema_version": 1, "kind": "sorotte-tested-container-export", "status": "passed",
                "source_sha": source, "repository": repository, "local_image": local, "evidence_sha256": hashes,
                "archive": {"name": archive.name, "sha256": artifact_input.sha256_file(archive), "size": archive.stat().st_size}}
    if value != expected or archive.stat().st_size == 0:
        raise ValueError("container export or its original qualification evidence changed")
    subprocess.run(["docker", "image", "load", "--input", str(archive)], check=True, timeout=300)
    observed = container.inspect_local_image(local["id"], expected_source_sha=source, expected_source_url=local["source"])
    if observed != local:
        raise ValueError("restored image differs from the qualified image identity")
    subprocess.run(["docker", "image", "tag", local["id"], image], check=True, timeout=30)
    return {"schema_version": 1, "kind": "sorotte-tested-container-restored", "status": "passed",
            "source_sha": source, "local_image": observed, "rebuilt": False, "application_tests_executed": False}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("seal", "load", "roundtrip"))
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--image", required=True)
    parser.add_argument("--evidence-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--bundle-dir", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY", ""))
    args = parser.parse_args(argv)
    try:
        if args.command == "seal":
            if args.output_dir is None:
                raise ValueError("sealing requires a fresh output directory")
            seal(args.source_sha, args.image, args.evidence_dir, args.output_dir, args.repository)
        else:
            if args.bundle_dir is None or args.report is None:
                raise ValueError("restoration requires the exact candidate bundle and a fresh report")
            if args.command == "roundtrip":
                # Used only by the nonpublishing candidate job on its owned,
                # disposable daemon. Removing by the verified ID prevents a
                # mutable tag from selecting an unrelated image.
                local, _ = evidence(args.evidence_dir, args.source_sha, args.repository)
                subprocess.run(["docker", "image", "rm", local["id"]], check=True, timeout=60)
                present = subprocess.run(["docker", "image", "inspect", local["id"]],
                                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
                if present.returncode == 0:
                    raise ValueError("cold restore qualification requires the original image to be absent")
            value = load(args.source_sha, args.image, args.evidence_dir, args.bundle_dir, args.repository)
            value["cold_restore"] = args.command == "roundtrip"
            candidate_authority.write(args.report, value)
        return 0
    except (ValueError, OSError, KeyError, container.VerificationError, subprocess.SubprocessError) as error:
        print(f"container candidate {args.command} failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
