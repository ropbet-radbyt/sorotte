#!/usr/bin/env python3
"""Probe final endpoint validation in the pinned Syncplay ConfigurationGetter."""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import pathlib
import sys
from collections.abc import Mapping
from typing import Any


SCHEMA = "sorotte-pinned-configuration-getter-endpoint-v1"
MAX_INPUT_BYTES = 1024 * 1024


def read_request() -> Mapping[str, Any]:
    data = sys.stdin.buffer.read(MAX_INPUT_BYTES + 1)
    if len(data) > MAX_INPUT_BYTES:
        raise ValueError("probe input exceeds the bounded size")
    request = json.loads(data)
    if not isinstance(request, dict) or set(request) != {"schema", "cases"}:
        raise ValueError("probe input has an invalid top-level shape")
    if request["schema"] != SCHEMA or not isinstance(request["cases"], list):
        raise ValueError("probe input schema is unsupported")
    return request


def validate_case(
    configuration_getter: type[Any], invalid_config_value: type[Exception], case: Mapping[str, Any]
) -> dict[str, Any]:
    if not isinstance(case, dict) or set(case) != {"id", "arguments"}:
        raise ValueError("endpoint case has an invalid shape")
    case_id = case["id"]
    arguments = case["arguments"]
    if not isinstance(case_id, str) or not isinstance(arguments, list):
        raise ValueError("endpoint case id or arguments have invalid types")
    if not all(isinstance(argument, str) for argument in arguments):
        raise ValueError("endpoint arguments must all be strings")

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
    try:
        try:
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
                io.StringIO()
            ):
                getter.getConfiguration()
        except SystemExit:
            return {"id": case_id, "accepted": False}
    finally:
        sys.argv = previous_argv

    getter._required = ["host"]
    getter._boolean = []
    getter._serialised = []
    getter._tristate = []
    getter._numeric = []
    getter._hexadecimal = []
    try:
        getter._validateArguments()
    except invalid_config_value:
        return {"id": case_id, "accepted": False}
    return {"id": case_id, "accepted": True}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--legacy-root", required=True)
    args = parser.parse_args()
    legacy_root = pathlib.Path(args.legacy_root).resolve()
    if not legacy_root.joinpath("syncplay", "ui", "ConfigurationGetter.py").is_file():
        raise ValueError("legacy root does not contain ConfigurationGetter.py")
    sys.path.insert(0, str(legacy_root))
    from syncplay.ui.ConfigurationGetter import ConfigurationGetter, InvalidConfigValue

    request = read_request()
    result = {
        "schema": SCHEMA,
        "cases": [
            validate_case(ConfigurationGetter, InvalidConfigValue, case)
            for case in request["cases"]
        ],
    }
    json.dump(result, sys.stdout, sort_keys=True, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
