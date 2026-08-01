# Controlled-room configuration property evidence — 2026-07-30

## Result

The new black-box `sorotte-client-app` integration suite passed at its default,
scheduled, and stress depths. Four fixed-seed properties executed 2,048,
8,192, and 40,000 cases respectively (50,240 total). No production code or
dependency changed, and the slice did not surface a product defect.

The implementation base was
`9a31b5acfe7e4e0150bdbbe3c31ed7e4155d8614`. At the recorded validation
point, the 446-line test source
`crates/sorotte-client-app/tests/controlled_room_configuration_properties.rs`
had SHA-256
`44793178EDC1B6E4ED95187A6B77214F859DFBBE9C2073F81803E00C40B6D151`.

## Public boundaries and independent oracle

The suite uses only exports from `sorotte_client_app::app_boundary`:

1. `normalize_controlled_room_input_legacy_compatible` for legacy inline
   controlled-room parsing;
2. `controlled_room_base_name_legacy_compatible` for command-facing room
   presentation;
3. `upsert_sorotte_ini_stored_client_settings_mvp` and
   `parse_sorotte_ini_stored_client_settings_mvp` for persistence;
4. `stored_client_settings_runtime_snapshot_legacy_compatible` for typed
   runtime resolution and validation; and
5. `stored_client_settings_config_plan_legacy_compatible` with
   `StoredClientSettingsEnvPresence` for startup composition.

The oracle does not invoke a production parsing or normalization helper. It
separately locates delimiters by byte index, validates a trimmed non-empty
base and an exact 12-byte ASCII-alphanumeric hash, applies the legacy plus
prefix, and filters passwords to ASCII alphanumeric characters and hyphens
before uppercasing. A separate source-selection model gives an explicit
non-blank room precedence over the first non-blank room-history entry.

Generated inputs vary:

- pre-prefixed and unprefixed room bases;
- bases containing an additional colon;
- surrounding base/hash whitespace and mixed-case hash characters;
- passwords containing allowed ASCII, discarded punctuation and whitespace,
  and non-ASCII characters;
- explicit-room, room-history fallback, blank-explicit fallback, ordinary
  room, and malformed-room composition; and
- independently selected presence bits for every environment field other than
  the room being observed.

The fixed Proptest seed is `0xC0F1_700D_2026_0730`. Shrinking is bounded at
20,000 iterations and failure persistence is disabled, so a failure is
reproducible without repository-local regression state.

## Mechanical contracts

Four generated properties cover separate concerns:

1. **Normalization, reconstruction, and presentation.** Inline legacy inputs
   equal the independent model. Canonical names are idempotent; reconstructing
   a canonical name with its normalized password preserves both values; and
   command-facing base-name extraction removes only the controlled prefix and
   hash.
2. **Malformed and passwordless fail-closed behavior.** Empty bases, short,
   long, punctuated, and non-ASCII hashes, ordinary names, all-discarded
   passwords, and hash-only legacy rooms never manufacture a password. The
   normalizer, runtime snapshot, typed runtime config, and startup plan all
   agree, and absent credentials retain `TlsPolicy::PreferTls`.
3. **Persistence, precedence, and composition.** Render/parse followed by a
   fresh canonical rewrite is byte-idempotent. Runtime meaning survives
   whitespace canonicalization. Explicit/fallback precedence equals the
   independent model. Arbitrary unrelated environment-presence bits cannot
   perturb the room outputs; setting only `room` present suppresses exactly
   the room and its derived password.
4. **Credential typing, redaction, and isolation.** Generated server and room
   canaries resolve into separate `SecretValue` fields, select
   `TlsPolicy::RequireTls`, and never appear in `Debug` output for stored
   settings, runtime snapshots, or startup plans. Server-password environment
   shadowing leaves the controlled-room credential intact, while room
   shadowing leaves the server credential intact.

## Case-budget contract

`PROPTEST_CASES` has the same bounded behavior as the adjacent configuration
property suites:

- absent: 512 cases per property;
- scheduled: 2,048 cases per property;
- stress used here: 10,000 cases per property;
- maximum: 100,000 cases per property, with larger positive values capped;
- zero, malformed, or non-Unicode values: fail closed before generated
  execution.

Live probes with `PROPTEST_CASES=0` and `PROPTEST_CASES=malformed` both exited
101 before a generated case and emitted:

```text
PROPTEST_CASES must be an integer from 1 to 100000
```

## Executed proof

Default depth:

```powershell
cargo test --locked -p sorotte-client-app `
  --test controlled_room_configuration_properties -- --nocapture
```

Result: 4/4 tests passed; 512 cases per property, 2,048 generated cases total,
in 0.09 seconds.

Scheduled depth:

```powershell
$env:PROPTEST_CASES = "2048"
cargo test --locked -p sorotte-client-app `
  --test controlled_room_configuration_properties -- --nocapture
Remove-Item Env:PROPTEST_CASES
```

Result: 4/4 tests passed; 2,048 cases per property, 8,192 generated cases
total, in 0.35 seconds.

Stress depth:

```powershell
$env:PROPTEST_CASES = "10000"
cargo test --locked -p sorotte-client-app `
  --test controlled_room_configuration_properties -- --nocapture
Remove-Item Env:PROPTEST_CASES
```

Result: 4/4 tests passed; 10,000 cases per property, 40,000 generated cases
total, in 1.65 seconds.

Focused strict lint and source checks:

```powershell
cargo clippy --locked -p sorotte-client-app `
  --test controlled_room_configuration_properties -- -D warnings
cargo fmt -p sorotte-client-app -- --check
git diff --no-index --check -- NUL `
  crates/sorotte-client-app/tests/controlled_room_configuration_properties.rs
git diff --no-index --check -- NUL `
  docs/evidence/test-coverage/controlled-room-configuration-properties-20260730.md
```

Strict Clippy and formatting passed. Each no-index check returned the expected
status 1 because its assigned path is a new file and emitted no whitespace
diagnostic. Focused strict Clippy completed in 0.65 seconds.

## Scope limits

This is a bounded generated grammar, not arbitrary-byte fuzzing. It models the
environment through the same public presence structure used by startup
composition rather than mutating the real process environment. It does not
exercise CLI argument parsing, a live network session, server authorization,
or GUI entry.

The persisted legacy format intentionally contains inline room credentials,
and the public legacy normalizer necessarily returns the extracted password as
a `String`. This slice proves typed runtime redaction and output
noninterference; it does not claim encryption at rest or that callers cannot
log a plaintext normalizer result.
