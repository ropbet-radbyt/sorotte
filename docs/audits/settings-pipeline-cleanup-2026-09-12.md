# Settings pipeline cleanup

Scoped from main `3f1fd37b86aa80a0025fda57dd94ed9c7044fb83` after the compatibility,
player observation and GUI runtime cleanup passes.

## Implemented scope

- Declare all 107 Sorotte INI field bindings once, including typed parsing and
  formatting. The reader, writer and three-way merge use the same bindings.
  Merging no longer renders and reparses a whole INI document to compare fields.
- Remove `StoredClientSettingsEnvPresence`, `StoredClientSettingsConfigPlan` and
  their projection/copying chain. Apply saved settings directly to the CLI from
  the validated runtime snapshot, with an explicit environment reader.
- Derive labels when constructing all 94 GUI settings controls, centralize
  checkbox rendering and consolidate the 29 checkbox assignments.
- Move the independent generated composition and controlled-room test models to
  the CLI application boundary. Retain runtime normalization tests in client-app
  and add a CLI mutation shard with the same zero-survivor/timeout policy.

## Preserved contracts

Syncplay interoperability and settings import remain supported. INI key spelling,
insertion order, duplicate handling, escaped percent/control characters, BOMs,
unknown content, atomic replacement and secret clearing retain their behavior.
An unsupported nonempty language clears an earlier value; other invalid inputs
retain the earlier valid value. Unbracketed string lists remain supported.

Stored values, validated runtime values and unsaved GUI drafts remain distinct.
Invalid or unfinished GUI text is not overwritten by effective defaults. CLI
environment precedence retains the invalid-port exception, both username names,
public-server and room-history fallbacks, and explicit controller credentials.

## Validation boundaries

A temporary comparison against the unchanged main INI reader/writer checked
16,692 inputs and 100,152 writer outputs across all fields, mixed-case duplicate
keys, malformed values, escaping, BOMs and Plex identity clearing. Both sides
produced identical results. The old implementations were removed after that
comparison; existing persistence tests and focused regressions remain.

PR qualification includes workspace behavior and lint, GUI configuration and
semantic coverage, hosted coverage/mutation/fuzz/dependency/package gates, and
the exact-source Windows native workflows. Their final results are recorded in
the PR. This pass does not publish a release.
