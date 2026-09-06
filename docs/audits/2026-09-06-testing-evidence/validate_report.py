"""Validate audit-document references and evidence accounting on its baseline."""
import hashlib
import json
import pathlib
import re
import subprocess
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = pathlib.Path(__file__).resolve().parent
REPORT = OUT.parent / "testing-apparatus-audit-2026-09-06.md"
SHA = "4000eca69b52003b66e81b6998d15c555e7eb6d1"


def main():
    report = REPORT.read_text(encoding="utf-8")
    errors = []
    tasks = re.findall(r"^### (T\d\d) —", report, re.M)
    if tasks != [f"T{i:02}" for i in range(1, 19)]:
        errors.append("Task inventory is not exactly T01-T18")
    cited_tasks = set(re.findall(r"\bT\d\d\b", report))
    if not cited_tasks <= set(tasks):
        errors.append("Unknown task references")
    anchors = []
    prefix = "https://github.com/ropbet-radbyt/sorotte/blob/"
    links = re.findall(r"\]\(([^)]+)\)", report)
    for link in links:
        if link.startswith(prefix):
            suffix = link[len(prefix):]
            revision, rest = suffix.split("/", 1)
            path, line_string = rest.rsplit("#L", 1)
            path = urllib.parse.unquote(path)
            if revision != SHA:
                errors.append("Source link uses another revision: " + link)
            file = ROOT / path
            if not file.is_file():
                errors.append("Missing source: " + path)
                continue
            lines = file.read_text(encoding="utf-8-sig").splitlines()
            line = int(line_string)
            if not 1 <= line <= len(lines) or not lines[line - 1].strip():
                errors.append("Invalid or blank source anchor: " + link)
                continue
            anchors.append({"path": path, "line": line, "text": lines[line - 1],
                            "file_sha256": hashlib.sha256(file.read_bytes()).hexdigest()})
        elif not link.startswith(("https://", "http://", "#")):
            path = urllib.parse.unquote(link.split("#", 1)[0])
            if not (REPORT.parent / path).is_file():
                errors.append("Broken local link: " + link)
    git = ["git", "-c", "safe.directory=" + ROOT.as_posix()]
    head = subprocess.run([*git, "rev-parse", "HEAD"], cwd=ROOT, capture_output=True,
                          text=True, check=True).stdout.strip()
    diff = subprocess.run([*git, "diff", "--exit-code", SHA, "--", "crates", "scripts",
                           ".github", ".config", ".cargo", "coverage", "fuzz", "fixtures",
                           "resources", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"],
                          cwd=ROOT, capture_output=True, text=True)
    if head != SHA or diff.returncode != 0:
        errors.append("Audited source baseline changed")
    summary = json.loads((OUT / "hosted-summary.json").read_text())
    if summary["runs"] != sum(summary["run_conclusions"].values()):
        errors.append("Run totals disagree")
    if summary["merge_lifecycle_producers"] != 10:
        errors.append("Lifecycle producer total changed")
    if len(summary["merge_lifecycle_jobs"]) != len({(j["run_id"], j["name"]) for j in summary["merge_lifecycle_jobs"]}):
        errors.append("Duplicate release lifecycle execution")
    (OUT / "source-anchors.json").write_text(json.dumps(anchors, indent=2) + "\n", encoding="utf-8")
    value = {"status": "passed" if not errors else "failed", "baseline_sha": head,
             "tasks": len(tasks), "source_anchors": len(anchors), "links": len(links),
             "production_and_harness_source_unchanged": diff.returncode == 0,
             "report_sha256": hashlib.sha256(REPORT.read_bytes()).hexdigest(), "errors": errors}
    (OUT / "document-validation.json").write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(value, indent=2))
    raise SystemExit(bool(errors))


if __name__ == "__main__":
    main()
