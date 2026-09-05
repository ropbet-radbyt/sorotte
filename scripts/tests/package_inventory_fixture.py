"""Synthetic release payload inventory; independent of the production writer."""

import hashlib
import json


def inventory_bytes(payloads, *, package, target, source_sha):
    return json.dumps({
        "schema": "sorotte-dependency-inventory-v1",
        "package": package,
        "target": target,
        "features": "default",
        "dependency_kinds": ["normal", "build"],
        "source_sha": source_sha,
        "inputs": [{"path": path, "sha256": "0" * 64} for path in (
            "Cargo.toml", "Cargo.lock", f"crates/{package}/Cargo.toml", "coverage/dependency-policy.toml", "coverage/native-components.toml",
        )],
        "payload": [{"path": path, "sha256": hashlib.sha256(body).hexdigest()} for path, body in payloads.items() if path not in {"DEPENDENCIES.json", "THIRD-PARTY-NOTICES.txt"}],
        "resolution_command": ["cargo", "tree", "--locked", "-p", package, "--target", target, "--edges", "normal,build", "--prefix", "none", "--format", "{p}"],
        "resolution_sha256": "0" * 64,
        "packages": [{"name": package, "version": "0.2.9", "source": None, "license": "Apache-2.0", "repository": None, "notice_files": []}],
        "native_components": {"schema_version": 1, "component": []},
    }).encode()
