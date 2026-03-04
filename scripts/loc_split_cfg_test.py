import argparse
import bisect
import json
import re
from collections import defaultdict
from pathlib import Path


ATTR_RE = re.compile(r"^\s*#\[cfg\(test\)\]\s*$")


def compute_line_starts(text: str) -> list[int]:
    starts = [0]
    for i, ch in enumerate(text):
        if ch == "\n":
            starts.append(i + 1)
    return starts


def line_for_index(starts: list[int], idx: int) -> int:
    return bisect.bisect_right(starts, idx)


def parse_raw_string_start(text: str, i: int):
    n = len(text)
    j = i
    if text.startswith("br", i) or text.startswith("rb", i):
        j += 2
    elif text.startswith("r", i):
        j += 1
    else:
        return None
    hashes = 0
    while j < n and text[j] == "#":
        hashes += 1
        j += 1
    if j < n and text[j] == '"':
        return (j + 1, hashes)
    return None


def skip_line_comment(text: str, i: int) -> int:
    end = text.find("\n", i)
    return len(text) if end == -1 else end + 1


def skip_block_comment(text: str, i: int) -> int:
    n = len(text)
    depth = 1
    i += 2
    while i < n and depth > 0:
        if text.startswith("/*", i):
            depth += 1
            i += 2
        elif text.startswith("*/", i):
            depth -= 1
            i += 2
        else:
            i += 1
    return i


def skip_string(text: str, i: int) -> int:
    n = len(text)
    i += 1
    while i < n:
        if text[i] == "\\":
            i += 2
            continue
        if text[i] == '"':
            return i + 1
        i += 1
    return n


def skip_raw_string(text: str, body_start: int, hashes: int) -> int:
    terminator = '"' + ("#" * hashes)
    n = len(text)
    i = body_start
    while i < n:
        quote = text.find('"', i)
        if quote == -1:
            return n
        if text.startswith(terminator, quote):
            return quote + len(terminator)
        i = quote + 1
    return n


def find_matching_brace(text: str, open_idx: int) -> int:
    n = len(text)
    depth = 1
    i = open_idx + 1
    while i < n:
        if text.startswith("//", i):
            i = skip_line_comment(text, i)
            continue
        if text.startswith("/*", i):
            i = skip_block_comment(text, i)
            continue
        raw = parse_raw_string_start(text, i)
        if raw is not None:
            body_start, hashes = raw
            i = skip_raw_string(text, body_start, hashes)
            continue
        ch = text[i]
        if ch == '"':
            i = skip_string(text, i)
            continue
        if ch == "{":
            depth += 1
            i += 1
            continue
        if ch == "}":
            depth -= 1
            i += 1
            if depth == 0:
                return i - 1
            continue
        i += 1
    raise RuntimeError(f"Unmatched brace after byte offset {open_idx}")


def find_cfg_test_spans(text: str) -> list[tuple[int, int]]:
    spans: list[tuple[int, int]] = []
    starts = compute_line_starts(text)
    lines = text.splitlines(keepends=True)
    cursor = 0

    for line_no, line in enumerate(lines, start=1):
        line_start = cursor
        cursor += len(line)
        if not ATTR_RE.match(line.rstrip("\r\n")):
            continue

        j = cursor
        n = len(text)
        while j < n:
            if text.startswith("//", j):
                j = skip_line_comment(text, j)
                continue
            if text.startswith("/*", j):
                j = skip_block_comment(text, j)
                continue
            raw = parse_raw_string_start(text, j)
            if raw is not None:
                body_start, hashes = raw
                j = skip_raw_string(text, body_start, hashes)
                continue
            ch = text[j]
            if ch == '"':
                j = skip_string(text, j)
                continue
            if ch == "{":
                close_idx = find_matching_brace(text, j)
                spans.append((line_no, line_for_index(starts, close_idx)))
                break
            if ch == ";":
                spans.append((line_no, line_for_index(starts, j)))
                break
            j += 1
        else:
            raise RuntimeError(f"Could not find body for #[cfg(test)] at line {line_no}")

    spans.sort()
    merged: list[list[int]] = []
    for start, end in spans:
        if merged and start <= merged[-1][1] + 1:
            merged[-1][1] = max(merged[-1][1], end)
        else:
            merged.append([start, end])
    return [tuple(span) for span in merged]


def count_lines(text: str, test_spans: list[tuple[int, int]]) -> dict[str, int]:
    lines = text.splitlines()
    total = len(lines)
    nonblank = [bool(line.strip()) for line in lines]
    test_mask = [False] * total

    for start, end in test_spans:
        for line_no in range(max(1, start), min(total, end) + 1):
            test_mask[line_no - 1] = True

    physical_test = sum(test_mask)
    nonblank_test = sum(1 for i, is_test in enumerate(test_mask) if is_test and nonblank[i])
    physical_total = total
    nonblank_total = sum(nonblank)
    return {
        "physical_total": physical_total,
        "physical_functional": physical_total - physical_test,
        "physical_test": physical_test,
        "nonblank_total": nonblank_total,
        "nonblank_functional": nonblank_total - nonblank_test,
        "nonblank_test": nonblank_test,
    }


def iter_rust_files(root: Path, include_all_rs: bool):
    pattern = "crates/**/*.rs" if include_all_rs else "crates/**/src/*.rs"
    return sorted(root.glob(pattern))


def build_report(root: Path, include_all_rs: bool) -> dict:
    files = []
    crate_totals: dict[str, dict[str, int]] = defaultdict(
        lambda: {
            "physical_total": 0,
            "physical_functional": 0,
            "physical_test": 0,
            "nonblank_total": 0,
            "nonblank_functional": 0,
            "nonblank_test": 0,
        }
    )
    totals = {
        "physical_total": 0,
        "physical_functional": 0,
        "physical_test": 0,
        "nonblank_total": 0,
        "nonblank_functional": 0,
        "nonblank_test": 0,
    }

    for path in iter_rust_files(root, include_all_rs):
        text = path.read_text(encoding="utf-8")
        rel = path.relative_to(root).as_posix()

        if "/tests/" in f"/{rel}/":
            # Future-proofing for integration-test files.
            total_lines = len(text.splitlines())
            nonblank_total = sum(1 for line in text.splitlines() if line.strip())
            counts = {
                "physical_total": total_lines,
                "physical_functional": 0,
                "physical_test": total_lines,
                "nonblank_total": nonblank_total,
                "nonblank_functional": 0,
                "nonblank_test": nonblank_total,
            }
            spans: list[tuple[int, int]] = [(1, total_lines)] if total_lines else []
        else:
            spans = find_cfg_test_spans(text)
            counts = count_lines(text, spans)

        file_entry = {"file": rel, "test_spans": spans, **counts}
        files.append(file_entry)

        crate = Path(rel).parts[1] if rel.startswith("crates/") else "<other>"
        for key, value in counts.items():
            totals[key] += value
            crate_totals[crate][key] += value

    return {
        "scope": "crates/**/*.rs" if include_all_rs else "crates/**/src/*.rs",
        "metric_notes": [
            "physical_* = raw source lines",
            "nonblank_* = non-empty source lines",
            "test_* = lines inside items annotated with #[cfg(test)]",
            "files under crates/**/tests/**/*.rs are counted as test lines if --all-rs is used",
        ],
        "totals": totals,
        "crate_totals": dict(sorted(crate_totals.items())),
        "files": files,
    }


def print_text_report(report: dict) -> None:
    totals = report["totals"]
    p_func = (totals["physical_functional"] / totals["physical_total"] * 100) if totals["physical_total"] else 0.0
    p_test = (totals["physical_test"] / totals["physical_total"] * 100) if totals["physical_total"] else 0.0
    nb_func = (totals["nonblank_functional"] / totals["nonblank_total"] * 100) if totals["nonblank_total"] else 0.0
    nb_test = (totals["nonblank_test"] / totals["nonblank_total"] * 100) if totals["nonblank_total"] else 0.0

    print(f"SCOPE: {report['scope']}")
    print("TOTALS")
    print(f"  physical_total:       {totals['physical_total']}")
    print(f"  physical_functional:  {totals['physical_functional']} ({p_func:.2f}%)")
    print(f"  physical_test:        {totals['physical_test']} ({p_test:.2f}%)")
    print(f"  nonblank_total:       {totals['nonblank_total']}")
    print(f"  nonblank_functional:  {totals['nonblank_functional']} ({nb_func:.2f}%)")
    print(f"  nonblank_test:        {totals['nonblank_test']} ({nb_test:.2f}%)")
    print()
    print("BY CRATE (nonblank)")
    for crate, counts in report["crate_totals"].items():
        print(
            f"  {crate:22} total={counts['nonblank_total']:6} "
            f"functional={counts['nonblank_functional']:6} test={counts['nonblank_test']:6}"
        )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Count Rust LOC and split test vs functional lines by #[cfg(test)] items."
    )
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--all-rs", action="store_true", help="Count crates/**/*.rs instead of crates/**/src/*.rs")
    parser.add_argument("--json", action="store_true", help="Emit JSON report")
    args = parser.parse_args()

    report = build_report(args.root.resolve(), args.all_rs)
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print_text_report(report)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
