"""Shared immutable tool/reference inputs and reproducible environment identities."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "coverage/verification-tools.toml"


def pins() -> dict:
    with MANIFEST.open("rb") as stream:
        result = tomllib.load(stream)
    if result.get("schema_version") != 1:
        raise ValueError("unsupported tool manifest")
    return result


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git(*args: str, root: Path = ROOT) -> str:
    return subprocess.check_output(
        ["git", "-c", f"safe.directory={root.as_posix()}", *args], cwd=root,
        text=True, encoding="utf-8", stderr=subprocess.PIPE,
    ).strip()


def identity(root: Path = ROOT) -> dict:
    # The repository's ignore policy excludes build/campaign scratch. Retain a
    # broad closure until a narrower dependency graph has independent evidence.
    source_paths = ["."]
    diff = git("diff", "--binary", "HEAD", "--", *source_paths, root=root)
    working = hashlib.sha256()
    tracked = git("ls-files", "-z", "--", *source_paths, root=root)
    untracked = git("ls-files", "--others", "--exclude-standard", "-z", "--", *source_paths, root=root)
    # Hash actual bytes as consumed by tools, including line endings Git's text
    # filters can normalize away. A source SHA alone does not bind a dirty run.
    for name in sorted(set(filter(None, (tracked + "\0" + untracked).split("\0")))):
        path = root / name
        if path.is_symlink():
            raise ValueError("verification source contains an indirect input")
        working.update(name.encode("utf-8") + b"\0")
        if not path.exists():
            working.update(b"deleted\0")
        elif not path.is_file():
            raise ValueError("verification source contains a non-file input")
        else:
            working.update(b"file\0" + bytes.fromhex(digest(path)))
    return {
        "source_sha": git("rev-parse", "HEAD", root=root),
        "tool_manifest_sha256": digest(MANIFEST),
        "working_source_dirty": bool(diff or untracked),
        "working_source_sha256": working.hexdigest(),
        "cargo_lock_sha256": digest(root / "Cargo.lock"),
        "platform": platform.system(), "machine": platform.machine(),
        "python": platform.python_version(),
        "runner_image": os.environ.get("ImageOS"),
        "runner_image_version": os.environ.get("ImageVersion"),
        "run_id": os.environ.get("GITHUB_RUN_ID"),
        "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
    }


def validate_pin_projections(root: Path = ROOT) -> dict:
    """Read duplicated pin literals without importing or executing their wrappers."""
    import ast
    import re

    root = root.resolve()
    checked = set()

    def read(relative: str) -> str:
        checked.add(relative)
        path = root / relative
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"pin projection input is missing or indirect: {relative}")
        return path.read_text(encoding="utf-8-sig")

    manifest = tomllib.loads(read("coverage/verification-tools.toml"))
    if manifest.get("schema_version") != 1:
        raise ValueError("unsupported tool manifest")
    tools, references = manifest["tools"], manifest["references"]
    trees = {}

    def tree(relative: str):
        if relative not in trees:
            trees[relative] = ast.parse(read(relative), filename=relative)
        return trees[relative]

    def literal(relative: str, symbol: str):
        assignments = []
        for node in tree(relative).body:
            targets = node.targets if isinstance(node, ast.Assign) else [node.target] if isinstance(node, ast.AnnAssign) else []
            if any(isinstance(target, ast.Name) and target.id == symbol for target in targets):
                assignments.append(node.value)
        if len(assignments) != 1:
            raise ValueError(f"pin projection {relative}:{symbol} must have one literal assignment")
        try:
            return ast.literal_eval(assignments[0])
        except (ValueError, TypeError) as error:
            raise ValueError(f"pin projection {relative}:{symbol} must remain a static literal") from error

    def equal(observed, expected, label: str):
        if observed != expected:
            raise ValueError(f"pin projection differs from central manifest: {label}: {observed!r} != {expected!r}")

    equal(tomllib.loads(read("rust-toolchain.toml"))["toolchain"]["channel"], tools["rust"], "Rust toolchain")
    equal(tomllib.loads(read("Cargo.toml"))["workspace"]["package"]["rust-version"], tools["rust"], "workspace Rust version")
    equal(tomllib.loads(read("coverage/mutation-policy.toml"))["cargo_mutants_version"], tools["cargo-mutants"], "mutation policy")
    projections = {
        "scripts/compat_live_interop.py": {
            "PINNED_LEGACY_SYNCPLAY_SHA": references["legacy-sha"],
            "PINNED_LEGACY_SYNCPLAY_REPOSITORY": references["legacy-url"].removeprefix("https://github.com/"),
            "SUPPORTED_PYTHON_MINIMUM": tuple(map(int, tools["python-min"].split("."))),
            "SUPPORTED_PYTHON_MAXIMUM_EXCLUSIVE": tuple(map(int, tools["python-max-exclusive"].split("."))),
        },
        "scripts/coverage_profile_lanes.py": {
            "PINNED_CARGO_LLVM_COV_VERSION": tools["cargo-llvm-cov"],
            "PINNED_LEGACY_SYNCPLAY_SHA": references["legacy-sha"],
        },
        "scripts/coverage_windows_process_lanes.py": {
            "PINNED_CARGO_LLVM_COV_VERSION": tools["cargo-llvm-cov"],
            "PINNED_RUST_RELEASE": tools["rust"],
        },
        "scripts/diff_coverage.py": {"CARGO_LLVM_COV_VERSION": tools["cargo-llvm-cov"]},
        "scripts/llvm_cov_line_map.py": {"SUPPORTED_CARGO_LLVM_COV_VERSION": tools["cargo-llvm-cov"]},
        "scripts/nextest_ci.py": {"PINNED_NEXTEST_VERSION": tools["cargo-nextest"]},
        "scripts/gui_sandbox_bundle.py": {"LEGACY_SHA": references["legacy-sha"]},
        "fuzz/run_protocol_fuzz.py": {"EXPECTED_CARGO_FUZZ_VERSION": "cargo-fuzz " + tools["cargo-fuzz"]},
    }
    for relative, symbols in projections.items():
        for symbol, expected in symbols.items():
            equal(literal(relative, symbol), expected, f"{relative}:{symbol}")
    equal(literal("scripts/coverage_ci_guard.py", "PINNED_LINE_MAP_PRODUCER")["cargo_llvm_cov_version"],
          tools["cargo-llvm-cov"], "coverage CI line-map producer")
    # These pins are call arguments/receipt fields rather than module constants.
    # Inspect the AST; importing the canary would load the mutation machinery.
    canary = "scripts/mutation_tool_canary.py"
    tool_calls, evaluation_calls, receipts = [], [], []
    for node in ast.walk(tree(canary)):
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            if node.func.attr == "verify_tool":
                if len(node.args) != 2:
                    raise ValueError("mutation tool canary pin must remain explicit")
                tool_calls.append(ast.literal_eval(node.args[1]))
            if node.func.attr == "evaluate_results":
                evaluation_calls.extend(ast.literal_eval(item.value) for item in node.keywords if item.arg == "expected_version")
        if isinstance(node, ast.Dict):
            receipts.extend(ast.literal_eval(value) for key, value in zip(node.keys, node.values)
                            if isinstance(key, ast.Constant) and key.value == "cargo_mutants_version")
    equal(tool_calls, [tools["cargo-mutants"]], "mutation canary tool check")
    equal(evaluation_calls, [tools["cargo-mutants"]] * 2, "mutation canary result readers")
    equal(receipts, [tools["cargo-mutants"]], "mutation canary receipt")

    normalize = lambda name: re.sub(r"[-_.]+", "-", name).lower()
    central_python = {normalize(name): version for name, version in manifest["python"].items()}
    interop = literal("scripts/compat_live_interop.py", "PINNED_PACKAGES")
    equal({name: value[1] for name, value in interop.items()},
          {name: central_python[name] for name in ("cryptography", "pyopenssl", "service-identity", "twisted")},
          "interop Python package versions")
    requirement_line = re.compile(r"([A-Za-z0-9][A-Za-z0-9_.-]*)==([A-Za-z0-9][A-Za-z0-9_.+!-]*)")

    def package_pins(text: str, label: str, *, directive: bool) -> dict:
        values, directives = {}, 0
        for raw in text.splitlines():
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            if directive and line == "-c verification-constraints.txt":
                directives += 1
                continue
            match = requirement_line.fullmatch(line)
            if match is None or normalize(match.group(1)) in values:
                raise ValueError(f"{label} contains an unsupported or duplicate Python input")
            values[normalize(match.group(1))] = match.group(2)
        if directives != int(directive):
            raise ValueError(f"{label} must include the reviewed constraints exactly once")
        return values

    resolution = manifest["python-resolution"]
    equal(resolution["constraints"], "requirements/verification-constraints.txt", "constraints path")
    constraints_text = read(resolution["constraints"])
    constraints = package_pins(constraints_text, "verification constraints", directive=False)
    equal(hashlib.sha256(constraints_text.encode("utf-8")).hexdigest(), resolution["constraints-lf-sha256"], "reviewed constraints digest")
    for name, version in central_python.items():
        equal(constraints.get(name), version, f"constraints:{name}")
    environments = {"ci-policy": {"pyyaml", "cryptography"}, "dependency-audit": {"pip-audit"}, "legacy-python-interop": set(interop)}
    for environment, selected in environments.items():
        path = f"requirements/{environment}.txt"
        equal(package_pins(read(path), path, directive=True), {name: central_python[name] for name in selected}, path)
    return {"status": "passed", "manifest_sha256": digest(root / "coverage/verification-tools.toml"),
            "checked_files": sorted(checked), "constraints_packages": len(constraints)}


def verify_legacy(root: Path) -> str:
    actual = git("rev-parse", "HEAD", root=root)
    if actual != pins()["references"]["legacy-sha"]:
        raise ValueError(f"legacy source mismatch: {actual}")
    if git("status", "--porcelain", "--untracked-files=all", root=root):
        raise ValueError("legacy checkout is not clean")
    return actual


def build_key(*, features: list[str], profile: str, instrumentation: str, target: str) -> str:
    """A namespace, never proof: callers must still validate restored inputs/bytes."""
    value = identity()
    value.update(features=sorted(features), profile=profile,
                 instrumentation=instrumentation, target=target)
    for volatile in ("run_id", "run_attempt"):
        value.pop(volatile, None)
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] != "verify-legacy":
        raise SystemExit("usage: verification_tools.py verify-legacy CHECKOUT")
    print(verify_legacy(Path(sys.argv[2]).resolve()))
