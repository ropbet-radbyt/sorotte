#!/usr/bin/env python3
"""Report manual/scheduled evidence freshness without inventing successful runs."""
from __future__ import annotations
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import sys

from verification_tools import ROOT, digest


def evaluate(data: dict, now: datetime) -> list[dict]:
    if data.get("schema_version") != 1: raise ValueError("unsupported assurance schema")
    seen = set()
    result = []
    if not data.get("capabilities"): raise ValueError("assurance inventory cannot be empty")
    for capability in data["capabilities"]:
        if capability["id"] in seen: raise ValueError("duplicate capability")
        seen.add(capability["id"])
        for key in ("owner", "command", "environment", "cadence", "execution"):
            if not capability.get(key): raise ValueError(f"capability requires {key}")
        if capability["execution"] not in ("scheduled", "manual", "maintenance"):
            raise ValueError("unknown capability execution class")
        if type(capability.get("max_age_days")) is not int or capability["max_age_days"] <= 0:
            raise ValueError("capability requires a positive freshness budget in days")
        evidence = capability.get("evidence")
        status = "unavailable"
        if evidence:
            if not re.fullmatch("[0-9a-f]{40}", evidence["source_sha"]): raise ValueError("evidence requires exact source")
            instant = datetime.fromisoformat(evidence["completed_at"].replace("Z", "+00:00"))
            if instant.utcoffset() is None: raise ValueError("evidence requires an explicit timezone")
            age = (now - instant).total_seconds() / 86400
            if age < 0: raise ValueError("future evidence")
            if not re.fullmatch("[0-9a-f]{64}", evidence.get("artifact_sha256", "")) or not re.fullmatch(r"https://[^\s]+", evidence.get("url", "")):
                raise ValueError("evidence requires durable artifact identity")
            status = "current" if age <= capability["max_age_days"] else "stale"
        result.append(dict(capability, freshness=status))
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=Path, default=ROOT / "coverage/assurance-capabilities.json")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        rows = evaluate(json.loads(args.registry.read_text(encoding="utf-8")), datetime.now(timezone.utc))
        result = {"schema_version": 1, "kind": "assurance-freshness", "registry_sha256": digest(args.registry), "capabilities": rows}
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        for row in rows: print(f"{row['id']}: {row['freshness']} ({row['owner']})")
        return 0
    except (KeyError, ValueError, OSError) as error:
        print(error, file=sys.stderr)
        return 1


if __name__ == "__main__": raise SystemExit(main())
