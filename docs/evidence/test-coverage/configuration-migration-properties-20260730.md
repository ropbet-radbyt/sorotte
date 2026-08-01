# Configuration migration property evidence — 2026-07-30

## Result

The new black-box `sorotte-client-app` migration suite passed at its default
and scheduled depths. It executed 1,536 and 6,144 generated cases
respectively, did not change production code, and did not surface a product
defect.

The recorded implementation base was
`8fc81f652d0ca0978150919b91ff6c07d8cb4174`. The 537-line test source
`crates/sorotte-client-app/tests/configuration_migration_properties.rs` had
SHA-256
`8E5C4E91075AE5F1FC17B67AF095847024322017D21297A01807D82F57D599CD`
at the recorded validation point.

## Boundary and oracle

The integration test starts from legacy INI text rather than from canonical
stored DTOs. It crosses the public application boundary through:

1. `parse_sorotte_ini_stored_client_settings_mvp`;
2. an in-place `upsert_sorotte_ini_stored_client_settings_mvp`;
3. a fresh canonical rewrite and reparse; and
4. `stored_client_settings_runtime_snapshot_legacy_compatible`.

An independently constructed `StoredClientSettingsV1` is the expected model.
The properties require the legacy parse, in-place rewrite, fresh rewrite, and
runtime snapshot to preserve that model. A second fresh rewrite must be
byte-for-byte idempotent.

The fixed seed is `0xC0F1_6D1A_2026_0730`. Failure persistence is disabled,
shrinking is bounded at 20,000 iterations, the default is 512 cases per
property, and `PROPTEST_CASES` is capped at 100,000. A missing variable uses
the default; zero, malformed, and non-Unicode case budgets fail closed.

## Mechanical contracts

Three generated properties cover distinct migration domains:

1. **Legacy scalar spellings.** Mixed section/key case, whitespace, BOM,
   CRLF/LF, boolean aliases, language aliases, enum aliases, finite numeric
   fields, and an absent post-legacy start policy retain exact DTO and runtime
   meaning. The absent policy resolves to the legacy-compatible immediate
   behavior.
2. **Legacy collections.** Semicolon, comma, bracketed, single-quoted, and
   double-quoted lists; Python-like player argument maps; and tuple/list public
   server forms converge on one idempotent stored representation.
3. **Malformed typed values.** Invalid booleans, ports, floats, unsigned and
   signed integers, enums, maps, and public-server lists cannot manufacture a
   setting. A valid username sentinel in the same file must survive, proving
   that rejecting one value does not discard unrelated valid state.

## Executed proof

Default depth:

```powershell
cargo test --locked -p sorotte-client-app `
  --test configuration_migration_properties -- --nocapture
```

Result: 3/3 tests passed; the properties executed 1,536 generated cases in
0.25 seconds.

Scheduled depth:

```powershell
$env:PROPTEST_CASES = "2048"
cargo test --locked -p sorotte-client-app `
  --test configuration_migration_properties -- --nocapture
Remove-Item Env:PROPTEST_CASES
```

Result: 3/3 tests passed; the properties executed 6,144 generated cases in
1.02 seconds.

Strict lint and source checks:

```powershell
cargo clippy --locked -p sorotte-client-app --all-targets --all-features `
  -- -D warnings
cargo fmt -p sorotte-client-app -- --check
git diff --check -- `
  crates/sorotte-client-app/tests/configuration_migration_properties.rs
```

All passed.

## Scope limits

This is a deterministic generated migration grammar, not arbitrary-byte parser
fuzzing. It does not mutate the real process environment, prove filesystem
atomicity, or replace the separate 30-field configuration-composition suite.
Controlled-room password derivation, CLI argument parsing, and future unknown
legacy container syntaxes remain separate boundaries.
