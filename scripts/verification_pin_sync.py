"""Preview, check or write tooling projections from the reviewed input manifest.

No downloads, version resolution, test execution or authority changes. Only
declared scalar locations are edited; unrelated text and historical evidence
are left byte-for-byte intact. A complete plan is validated before any write.
"""
from __future__ import annotations

import argparse
import ast
from dataclasses import dataclass
import difflib
import json
from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = "coverage/verification-tools.toml"
VERSION = r"[0-9]+\.[0-9]+\.[0-9]+"


def tooling(manifest: dict) -> dict:
    if manifest.get("schema_version") != 1:
        raise ValueError("unsupported tool manifest")
    for table in ("tools", "rust-windows", "actions"):
        if not isinstance(manifest.get(table), dict):
            raise ValueError(f"{table} must be a manifest table")
    values = manifest["tools"]
    for key in ("rust", "cargo-nextest", "cargo-llvm-cov", "syft", "cosign"):
        if not isinstance(values.get(key), str) or not re.fullmatch(VERSION, values[key]):
            raise ValueError(f"tools.{key} must be an exact release version")
    compiler = manifest["rust-windows"]
    if (not all(isinstance(compiler.get(key), str) for key in ("commit", "host", "llvm"))
            or not re.fullmatch(r"[0-9a-f]{40}", compiler["commit"])
            or compiler["host"] != "x86_64-pc-windows-msvc"
            or not re.fullmatch(VERSION, compiler["llvm"])):
        raise ValueError("rust-windows must identify the exact reviewed compiler, host and LLVM")
    for name, action in manifest["actions"].items():
        if (not re.fullmatch(r"[\w.-]+/[\w.-]+", name) or not isinstance(action, dict)
                or set(action) != {"sha", "reference"}
                or not isinstance(action["sha"], str)
                or not re.fullmatch(r"[0-9a-f]{40}", action["sha"])
                or not isinstance(action["reference"], str) or not action["reference"]
                or any(c in action["reference"] for c in "\r\n")):
            raise ValueError(f"action {name} requires a full immutable SHA and review label")
    return values


def python_projections(manifest: dict) -> dict[str, dict[str, str]]:
    tools = tooling(manifest)
    compiler = manifest["rust-windows"]
    return {
        "scripts/coverage_windows_process_lanes.py": {
            "PINNED_CARGO_LLVM_COV_VERSION": tools["cargo-llvm-cov"],
            "PINNED_RUST_RELEASE": tools["rust"],
            "PINNED_RUST_COMMIT": compiler["commit"],
            "PINNED_RUST_HOST": compiler["host"],
            "PINNED_LLVM_VERSION": compiler["llvm"],
        },
        "scripts/coverage_profile_lanes.py": {"PINNED_CARGO_LLVM_COV_VERSION": tools["cargo-llvm-cov"]},
        "scripts/diff_coverage.py": {"CARGO_LLVM_COV_VERSION": tools["cargo-llvm-cov"]},
        "scripts/llvm_cov_line_map.py": {"SUPPORTED_CARGO_LLVM_COV_VERSION": tools["cargo-llvm-cov"]},
        "scripts/nextest_ci.py": {"PINNED_NEXTEST_VERSION": tools["cargo-nextest"]},
        "scripts/verify_server_container.py": {"PINNED_SYFT_VERSION": tools["syft"]},
    }


@dataclass(frozen=True)
class Change:
    path: str
    before: bytes
    after: bytes


class Projection:
    def __init__(self, root: Path, relative: str):
        self.relative = relative
        self.path = root / relative
        if (not self.path.is_file() or self.path.is_symlink()
                or not self.path.resolve().is_relative_to(root.resolve())):
            raise ValueError(f"missing or indirect projection: {relative}")
        self.before = self.path.read_bytes()
        self.text = self.before.decode("utf-8")

    def substitute(self, pattern: str, replacement, *, count: int | None = None):
        text, found = re.subn(pattern, replacement, self.text, flags=re.MULTILINE)
        if count is not None and found != count:
            raise ValueError(f"{self.relative}: expected {count} pin locations, found {found}: {pattern}")
        self.text = text

    def assignments(self, values: dict[str, str]):
        tree = ast.parse(self.text)
        replacements = []
        lines = self.text.splitlines(keepends=True)
        offsets = [0]
        for line in lines:
            offsets.append(offsets[-1] + len(line.encode("utf-8")))
        for name, value in values.items():
            nodes = [node.value for node in tree.body if isinstance(node, (ast.Assign, ast.AnnAssign))
                     and any(isinstance(target, ast.Name) and target.id == name for target in
                             (node.targets if isinstance(node, ast.Assign) else [node.target]))]
            if len(nodes) != 1 or not isinstance(nodes[0], ast.Constant) or not isinstance(nodes[0].value, str):
                raise ValueError(f"{self.relative}:{name} requires one literal string assignment")
            node = nodes[0]
            if node.value != value:
                replacements.append((offsets[node.lineno - 1] + node.col_offset,
                                     offsets[node.end_lineno - 1] + node.end_col_offset,
                                     json.dumps(value).encode("utf-8")))
        raw = self.text.encode("utf-8")
        for start, end, replacement in sorted(replacements, reverse=True):
            raw = raw[:start] + replacement + raw[end:]
        self.text = raw.decode("utf-8")

    def change(self) -> Change | None:
        after = self.text.encode("utf-8")
        return Change(self.relative, self.before, after) if after != self.before else None

    def workflow(self, manifest: dict):
        # Parse positions instead of dumping YAML: quotes, line endings, shell
        # blocks and unrelated workflow settings keep their original bytes.
        try:
            import yaml
        except ImportError as error:
            raise ValueError("install requirements/ci-policy.txt before updating workflow pins") from error

        def mapping(node):
            if not isinstance(node, yaml.MappingNode):
                raise ValueError(f"{self.relative}: expected a workflow mapping")
            result = {}
            for key, value in node.value:
                if not isinstance(key, yaml.ScalarNode) or key.value in result:
                    raise ValueError(f"{self.relative}: ambiguous workflow mapping")
                result[key.value] = value
            return result

        replacements = {}

        def scalar(node, value):
            if not isinstance(node, yaml.ScalarNode) or node.style not in (None, "", "'", '"'):
                raise ValueError(f"{self.relative}: pin must be a single scalar")
            if node.value != value:
                replacement = json.dumps(value) if node.style == '"' else (
                    "'" + value.replace("'", "''") + "'" if node.style == "'" else value)
                span = (node.start_mark.index, node.end_mark.index)
                if span in replacements and replacements[span] != replacement:
                    raise ValueError(f"{self.relative}: shared YAML pin has conflicting uses")
                replacements[span] = replacement

        def action(settings):
            if "uses" not in settings:
                return
            node = settings["uses"]
            if not isinstance(node, yaml.ScalarNode):
                raise ValueError(f"{self.relative}: action must be an explicit reference")
            if node.value.startswith("./"):
                return
            name, separator, _ = node.value.rpartition("@")
            if not separator or name not in manifest["actions"]:
                raise ValueError(f"{self.relative}: undeclared action {node.value}")
            approved = manifest["actions"][name]
            scalar(node, name + "@" + approved["sha"])
            # Existing upstream version annotations are projections too. Keep
            # other comments (and absent comments) intact.
            end = node.end_mark.index
            annotation = re.match(r"([ \t]*# )(v[^\s]+|stable resolved \d{4}-\d{2}-\d{2})(?=[ \t]*\r?$)",
                                  self.text[end:], flags=re.MULTILINE)
            if annotation and annotation[2] != approved["reference"]:
                replacements[(end + annotation.start(2), end + annotation.end(2))] = approved["reference"]
            inputs = mapping(settings["with"]) if "with" in settings else {}
            tools = manifest["tools"]
            if name == "dtolnay/rust-toolchain":
                toolchain = inputs["toolchain"]
                if re.fullmatch(VERSION, toolchain.value) or toolchain.value == "stable":
                    scalar(toolchain, tools["rust"])
                elif not re.fullmatch(r"nightly-\d{4}-\d{2}-\d{2}", toolchain.value):
                    raise ValueError(f"{self.relative}: unsupported Rust toolchain expression {toolchain.value!r}")
            elif name == "taiki-e/install-action":
                tool = inputs["tool"]
                tool_name = tool.value.partition("@")[0]
                if tool_name in ("cargo-nextest", "cargo-llvm-cov"):
                    scalar(tool, tool_name + "@" + tools[tool_name])
            elif name == "anchore/sbom-action":
                scalar(inputs["syft-version"], "v" + tools["syft"])
            elif name == "sigstore/cosign-installer":
                scalar(inputs["cosign-release"], "v" + tools["cosign"])
            elif name == "actions/cache" and "key" in inputs:
                scalar(inputs["key"], re.sub(
                    rf"(cargo-downloads-\$\{{\{{ runner.os \}}\}}-){VERSION}(-)",
                    lambda m: m[1] + tools["rust"] + m[2], inputs["key"].value))

        try:
            loader = getattr(yaml, "CBaseLoader", yaml.BaseLoader)
            if any(isinstance(token, (yaml.AnchorToken, yaml.AliasToken)) for token in yaml.scan(self.text, Loader=loader)):
                raise ValueError(f"{self.relative}: YAML aliases need explicit pin locations")
            workflow = mapping(yaml.compose(self.text, Loader=loader))
            for job in mapping(workflow["jobs"]).values():
                settings = mapping(job)
                action(settings)
                if "steps" in settings:
                    if not isinstance(settings["steps"], yaml.SequenceNode):
                        raise ValueError("steps must be a sequence")
                    for step in settings["steps"].value:
                        action(mapping(step))
        except (yaml.YAMLError, KeyError, AttributeError, TypeError) as error:
            raise ValueError(f"{self.relative}: cannot locate workflow pins: {error}") from error
        for (start, end), replacement in sorted(replacements.items(), reverse=True):
            self.text = self.text[:start] + replacement + self.text[end:]


def plan(root: Path = ROOT, *, workflows: bool = True) -> tuple[list[Change], list[str]]:
    root = root.resolve()
    authority = Projection(root, MANIFEST)
    manifest = tomllib.loads(authority.text)
    tools = tooling(manifest)
    projections = {}

    def file(relative: str) -> Projection:
        if relative not in projections:
            projections[relative] = Projection(root, relative)
        return projections[relative]

    for relative, values in python_projections(manifest).items():
        file(relative).assignments(values)
    file("scripts/coverage_ci_guard.py").substitute(
        r'("cargo_llvm_cov_version": ")[^"]+("[,]?)',
        lambda m: m[1] + tools["cargo-llvm-cov"] + m[2], count=1)
    for relative, key in (("rust-toolchain.toml", "channel"), ("Cargo.toml", "rust-version")):
        file(relative).substitute(rf'^(\s*{key} = ")[^"]+("[^\r\n]*)',
                                 lambda m: m[1] + tools["rust"] + m[2], count=1)
    file("verification/windows-native-guest.json").substitute(
        r'("rust_toolchain": ")[^"]+(")', lambda m: m[1] + tools["rust"] + m[2], count=1)
    file("Dockerfile.server").substitute(
        rf"^(RUN rustup toolchain install ){VERSION}( --profile minimal && rustup default ){VERSION}(\r?)$",
        lambda m: m[1] + tools["rust"] + m[2] + tools["rust"] + m[3], count=1)
    for relative in ("AGENTS.md", "docs/DEVELOPMENT.md"):
        file(relative).substitute(rf"(Use Rust `){VERSION}(`)",
                                 lambda m: m[1] + tools["rust"] + m[2], count=1)
    for relative in ("docs/DEVELOPMENT.md", "coverage/README.md"):
        for tool in ("cargo-nextest", "cargo-llvm-cov"):
            file(relative).substitute(rf"({tool}(?: --version)? ){VERSION}(\b)",
                                     lambda m, tool=tool: m[1] + tools[tool] + m[2])
    if workflows:
        paths = sorted(path for path in (root / ".github/workflows").iterdir() if path.suffix in (".yml", ".yaml"))
        if not paths:
            raise ValueError("workflow projections are missing")
        for path in paths:
            file(path.relative_to(root).as_posix()).workflow(manifest)
    return ([change for projection in projections.values() if (change := projection.change())],
            sorted([MANIFEST, *projections]))


def write_changes(root: Path, changes: list[Change]):
    # Detect edits since planning before changing any file. Never resolve new
    # versions or certify a tool run merely because projections now agree.
    for change in changes:
        if Projection(root, change.path).before != change.before:
            raise ValueError(f"projection changed during planning: {change.path}")
    for change in changes:
        (root / change.path).write_bytes(change.after)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="report drift without editing")
    mode.add_argument("--write", action="store_true", help="write reviewed pin projections")
    parser.add_argument("--repo-root", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    try:
        changes, _ = plan(args.repo_root)
        for change in changes:
            if args.check:
                print(f"Pin update required: {change.path}")
            else:
                print("".join(difflib.unified_diff(change.before.decode().splitlines(keepends=True),
                                                  change.after.decode().splitlines(keepends=True),
                                                  fromfile=change.path, tofile=change.path)), end="")
        if args.write:
            write_changes(args.repo_root, changes)
        print(f"{'Updated' if args.write else 'Changed'} pin projections: {len(changes)} files")
        if changes and args.check:
            print("Review: python scripts/verify.py pins; apply: add --write", file=sys.stderr)
        return int(bool(changes) and not args.write)
    except (ValueError, KeyError, TypeError, OSError, SyntaxError) as error:
        print(f"Pin update failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
