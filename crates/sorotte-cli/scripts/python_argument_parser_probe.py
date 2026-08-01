#!/usr/bin/env python3
"""Side-effect-free probe of the pinned Syncplay ConfigurationGetter parser."""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import pathlib
import sys
from collections.abc import Mapping
from typing import Any


SCHEMA = "sorotte-pinned-configuration-getter-arguments-v1"
MAX_INPUT_BYTES = 1024 * 1024


def parse_input() -> Mapping[str, Any]:
    data = sys.stdin.buffer.read(MAX_INPUT_BYTES + 1)
    if len(data) > MAX_INPUT_BYTES:
        raise ValueError("probe input exceeds the bounded size")
    value = json.loads(data)
    if not isinstance(value, dict) or set(value) != {"schema", "cases"}:
        raise ValueError("probe input has an invalid top-level shape")
    if value["schema"] != SCHEMA or not isinstance(value["cases"], list):
        raise ValueError("probe input schema is unsupported")
    return value


def parse_case(configuration_getter: type[Any], case: Mapping[str, Any]) -> dict[str, Any]:
    if not isinstance(case, dict) or set(case) != {"id", "arguments"}:
        raise ValueError("probe case has an invalid shape")
    case_id = case["id"]
    arguments = case["arguments"]
    if not isinstance(case_id, str) or not isinstance(arguments, list):
        raise ValueError("probe case id or arguments have invalid types")
    if not all(isinstance(argument, str) for argument in arguments):
        raise ValueError("probe arguments must all be strings")

    getter = configuration_getter()
    getter._config["noGui"] = True
    getter._config["forceGuiPrompt"] = False
    getter._getConfigurationFilePath = lambda: ""
    getter._parseConfigFile = lambda *args, **kwargs: None
    getter._checkConfig = lambda *args, **kwargs: None
    getter._saveConfig = lambda *args, **kwargs: None
    getter._loadRelativeConfiguration = lambda *args, **kwargs: []

    previous_argv = sys.argv
    sys.argv = ["syncplay"] + arguments
    captured_stdout = io.StringIO()
    captured_stderr = io.StringIO()
    try:
        try:
            with contextlib.redirect_stdout(captured_stdout), contextlib.redirect_stderr(
                captured_stderr
            ):
                config = getter.getConfiguration()
        except SystemExit:
            return {"id": case_id, "accepted": False}
    finally:
        sys.argv = previous_argv

    return {
        "id": case_id,
        "accepted": True,
        "host": config["host"],
        "name": config["name"],
        "room": config["room"] or None,
        "password": "<redacted>" if config["password"] else None,
        "debug": bool(config["debug"]),
        "force_gui_prompt": bool(config["forceGuiPrompt"]),
        "file": config["file"],
        "player_args": config["playerArgs"],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--legacy-root", required=True)
    args = parser.parse_args()
    legacy_root = pathlib.Path(args.legacy_root).resolve()
    if not legacy_root.joinpath("syncplay", "ui", "ConfigurationGetter.py").is_file():
        raise ValueError("legacy root does not contain ConfigurationGetter.py")
    sys.path.insert(0, str(legacy_root))
    from syncplay.ui.ConfigurationGetter import ConfigurationGetter

    request = parse_input()
    result = {
        "schema": SCHEMA,
        "cases": [parse_case(ConfigurationGetter, case) for case in request["cases"]],
    }
    json.dump(result, sys.stdout, sort_keys=True, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
