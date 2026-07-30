# Configuration composition property evidence — 2026-07-30

## Result

The new black-box `sorotte-client-app` integration suite passed at its default,
scheduled, and stress depths. It did not surface a product defect.

The suite is bound to branch base
`0748e4a8f07bad4ab30b26b22535ec969c3b10cf`. The test source
`crates/sorotte-client-app/tests/configuration_composition_properties.rs` was
842 lines and had SHA-256
`11C19D0D0BEC825DC8A83DD298CB982329A0A6502C04D464865C14C72496405B` at
the recorded validation point.

## Boundary and oracle

The tests use only the public app boundary:

1. Build a `StoredClientSettingsV1` from an independently generated model.
2. Upsert it into existing `sorotte.ini` text.
3. Parse the rendered INI back into the stored DTO.
4. Resolve the runtime snapshot and its validation issues.
5. Project the stored settings through
   `StoredClientSettingsEnvPresence` into a startup configuration plan.

The oracle is not a production parser, normalizer, formatter, or projection
helper. A generated model independently enumerates the expected values and
uses local enum labels solely to compare typed outcomes. The production
pipeline is therefore compared against the inputs' intended semantics rather
than against another call to itself.

Every generated model sets all 30 environment-overridable fields:

- host, port, server password, username, and room;
- autoplay, same-filename autoplay, ready-at-start, shared-playlist,
  pause-on-leave, and both playlist loop controls;
- trusted-domain policy and domain list;
- four desync switches and three desync thresholds;
- unpause action, autoplay minimum, and both privacy modes; and
- duration, same-room, warning, non-controller, and different-room OSD
  switches.

Generated values are canonical and valid by construction. Strings, ports,
booleans, lists, finite fractional thresholds, and every enum variant are in
the strategy domain. The fixed seed is
`0xC0F1_6C0A_2026_0730`; Proptest shrinking is bounded at 20,000 iterations
and failure persistence is disabled so repository state cannot influence
reproduction.

## Mechanical contracts

The integration binary contains one ordinary budget test and three generated
properties:

1. **Round-trip, projection, and idempotence.** All 30 supported fields survive
   INI render/parse exactly, the runtime snapshot has no validation issues,
   the environment-absent plan equals the independent model, and rendering
   the parsed DTO again is byte-for-byte idempotent.
2. **Forward-compatible preservation.** Each round-trip starts with a generated
   unknown section, comment, unknown key, and unknown keys in both known
   sections. All five sentinels must remain exact physical INI lines.
3. **Per-field noninterference.** Mutating one selected stored field must change
   that field in both the parsed DTO and runtime plan while leaving all other
   29 fields unchanged.
4. **Environment suppression isolation.** Marking one selected environment
   field present must remove exactly its matching stored override while
   leaving the other 29 projected overrides unchanged.

Ordinary rooms are used deliberately so room suppression has a single output;
controlled-room password derivation has separate deterministic coverage.

## Case-budget behavior

`PROPTEST_CASES` is shared with the existing scheduled test depth:

- absent: 512 cases per property;
- scheduled: 2,048 cases per property;
- maximum: 100,000 cases per property;
- zero, negative, fractional, empty, malformed, and non-Unicode values:
  fail closed before generated execution.

The ordinary budget test proves the pure parser boundaries, including that
values over 100,000 are capped. Two live entrypoint probes additionally proved
that `PROPTEST_CASES=0` and `PROPTEST_CASES=malformed` each fail the selected
property immediately with:

```text
PROPTEST_CASES must be an integer from 1 to 100000
```

## Executed proof

Default depth:

```powershell
cargo test --locked -p sorotte-client-app `
  --test configuration_composition_properties -- --nocapture
```

Result: 4/4 tests passed. The three properties executed 1,536 generated cases
in 0.19 seconds.

Scheduled depth:

```powershell
$env:PROPTEST_CASES = "2048"
cargo test --locked -p sorotte-client-app `
  --test configuration_composition_properties -- --nocapture
```

Result: 4/4 tests passed. The three properties executed 6,144 generated cases
in 0.75 seconds. The all-field round-trip alone covered 61,440 field
projections; the noninterference and suppression properties each checked all
30 result positions for every generated case.

Stress depth:

```powershell
$env:PROPTEST_CASES = "10000"
cargo test --locked -p sorotte-client-app `
  --test configuration_composition_properties -- --nocapture
```

Result: 4/4 tests passed. The three properties executed 30,000 generated cases
in 3.80 seconds.

Live fail-closed probes:

```powershell
$env:PROPTEST_CASES = "0"
cargo test --locked -p sorotte-client-app `
  --test configuration_composition_properties `
  supported_fields_roundtrip_project_and_remain_idempotent -- --exact --nocapture

$env:PROPTEST_CASES = "malformed"
cargo test --locked -p sorotte-client-app `
  --test configuration_composition_properties `
  supported_fields_roundtrip_project_and_remain_idempotent -- --exact --nocapture
```

Both commands failed as required before executing a generated case.

Full owning-crate gate:

```powershell
cargo test --locked -p sorotte-client-app --all-features -- --nocapture
```

Result: 185/185 library tests, 4/4 new integration tests, and doc tests passed.
The new integration binary again completed its default 1,536 generated cases
in 0.19 seconds.

Strict lint and formatting:

```powershell
cargo clippy --locked -p sorotte-client-app --all-targets --all-features `
  -- -D warnings
cargo fmt -p sorotte-client-app -- --check
git diff --check -- crates/sorotte-client-app/Cargo.toml Cargo.lock `
  crates/sorotte-client-app/tests/configuration_composition_properties.rs
```

All passed. Strict Clippy completed in 2.66 seconds.

## Scope limits

This slice intentionally proves composition of canonical supported values. It
does not claim exhaustive malformed-value behavior, filesystem atomicity,
controlled-room parsing, embedded-host-port fallback, or actual process
environment mutation. Those are separate boundaries; environment precedence
is exercised through the same pure presence structure used by the application
to suppress stored overrides.

The suite adds only a test dependency on the workspace's already pinned
Proptest version. No production behavior changed.
