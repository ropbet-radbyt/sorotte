"""Offline readiness probe for the exact Python interpreter selected by Actions.

Run with -I -B. It installs nothing and cannot borrow user-site/PYTHONPATH
packages. Collection can request the installed, constrained dependency files.
"""
from __future__ import annotations

import argparse
import importlib
import importlib.metadata
import json
from pathlib import Path
import re
import ssl
import subprocess
import sys


def canonical(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


def probe(contract: dict, *, collect_files: bool = False) -> dict:
    if contract.get("schema_version") != 1 or contract.get("kind") != "sorotte-native-python-contract":
        raise ValueError("unsupported native Python contract")
    actual = ".".join(map(str, sys.version_info[:3]))
    if actual != contract["python_version"]:
        raise ValueError(f"Python version {actual} differs from required {contract['python_version']}")
    if not sys.flags.isolated or not sys.dont_write_bytecode:
        raise ValueError("native Python probe requires isolated -I -B execution")
    # Exercise the command used by the workflow, rather than merely finding a
    # directory called pip. This child uses the same exact isolated interpreter.
    pip = subprocess.run([sys.executable, "-I", "-B", "-m", "pip", "--version"],
                         capture_output=True, text=True, encoding="utf-8", timeout=15)
    if pip.returncode:
        raise ValueError("selected Python cannot execute pip: " + pip.stderr.strip()[-1000:])
    from pip._vendor.packaging.markers import default_environment
    from pip._vendor.packaging.requirements import Requirement

    constraints = contract["constraints"]
    pending = list(contract["requirements"])
    versions = {}
    files: set[str] = set()
    root = Path(sys.prefix).resolve()
    package_root = root / "Lib/site-packages" if sys.platform == "win32" else root / f"lib/python{sys.version_info.major}.{sys.version_info.minor}/site-packages"
    environment = {**default_environment(), "extra": ""}
    while pending:
        name = canonical(pending.pop())
        if name in versions:
            continue
        expected = constraints.get(name)
        if expected is None:
            raise ValueError(f"required dependency lacks a reviewed constraint: {name}")
        distribution = importlib.metadata.distribution(name)
        if not Path(distribution.locate_file("")).resolve().is_relative_to(package_root):
            raise ValueError(f"required dependency is outside the selected Python runtime: {name}")
        if distribution.version != expected:
            raise ValueError(f"required dependency {name} is {distribution.version}, expected {expected}")
        if name in contract["requirements"] and contract["requirements"][name] != expected:
            raise ValueError(f"requirement and constraint disagree: {name}")
        versions[name] = distribution.version
        for dependency in distribution.requires or []:
            requirement = Requirement(dependency)
            if requirement.marker and not requirement.marker.evaluate(environment):
                continue
            dependency_name = canonical(requirement.name)
            pinned = constraints.get(dependency_name)
            if pinned is None or not requirement.specifier.contains(pinned):
                raise ValueError(f"dependency constraint does not satisfy {name}: {dependency_name}")
            pending.append(dependency_name)
        if collect_files:
            if distribution.files is None:
                raise ValueError(f"installed dependency has no file inventory: {name}")
            for relative in distribution.files:
                source = Path(distribution.locate_file(relative)).resolve()
                # Entry-point launchers outside site-packages are unnecessary:
                # workflow invocations always select python -m explicitly.
                if not source.is_relative_to(package_root):
                    continue
                if source.suffix == ".pyc" or "__pycache__" in source.parts:
                    continue
                # Legacy namespace wheels can ship .pth startup code. Copy none
                # of it; the clean destination is probed again, including the
                # namespace import, before it can become an Actions tool cache.
                if source.suffix == ".pth":
                    continue
                if source.name in {"sitecustomize.py", "usercustomize.py"}:
                    raise ValueError(f"required dependency includes a Python startup hook: {name}")
                if not source.is_file():
                    raise ValueError(f"installed dependency file is missing: {name}")
                files.add(source.relative_to(root).as_posix())
    imported = []
    for name in contract["imports"]:
        importlib.import_module(name)
        imported.append(name)
    result = {"schema_version": 1, "kind": "sorotte-native-python-readiness", "result": "passed",
              "python_version": actual, "openssl_version": ssl.OPENSSL_VERSION,
              "pip_command": "passed", "distributions": dict(sorted(versions.items())),
              "imports": imported, "isolated": True}
    if collect_files:
        if not files or len(files) > 20000:
            raise ValueError("required Python dependency file inventory is empty or unbounded")
        result["distribution_files"] = sorted(files)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--contract", type=Path)
    source.add_argument("--contract-json")
    parser.add_argument("--collect-files", action="store_true")
    args = parser.parse_args()
    try:
        contract = json.loads(args.contract.read_text(encoding="utf-8") if args.contract else args.contract_json)
        result = probe(contract, collect_files=args.collect_files)
    except Exception as error:
        print(json.dumps({"schema_version": 1, "kind": "sorotte-native-python-readiness",
                          "result": "failed", "error": str(error)}), file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
