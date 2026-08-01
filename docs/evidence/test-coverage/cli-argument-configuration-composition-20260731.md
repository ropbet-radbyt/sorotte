# CLI argument/configuration composition evidence — 2026-07-31

## Result

The bounded CLI-layer campaign found and fixed two distinct defect classes at
the real legacy argument parser boundary:

- `TC-CLI-004`: one parser-state/composition defect with four manifestations:
  attached `--option=value` syntax was rejected, later duplicate values could
  not clear an earlier CLI-layer override, a duplicate host without a port
  retained the earlier CLI port, and missing required host/name values were
  silently accepted.
- `TC-CLI-005`: unknown attached option diagnostics reproduced their raw
  values and could expose a credential-shaped value.

The focused regression module and its 256-case deterministic composition
campaign pass after the fixes. The owning crate's complete all-feature suite
and strict all-target Clippy gate also pass.

## 2026-08-01 review addendum

The original 2026-07-31 result was incomplete at one grammar boundary. Its
self-authored renderer treated `-x=value` as the attached short spelling and
therefore did not exercise argparse's canonical `-xVALUE` form or known flag
clusters. In particular, it did not prove `-pSECRET`, `-aexample:8999`,
`-nAlice`, `-rroom`, `-dg`, or `-gd`. The original raw-token representation
also meant an accepted `-pSECRET` could bypass the equals-sign-only diagnostic
redactor. Those are product defects, not merely omissions in this report.

The follow-up temporarily reopened `TC-CLI-004` and `TC-CLI-005` at commit
`db4c929`, retaining exact expected-failure characterizations before the fix.
Commit `cbdb870` replaces the short-option parser with explicit
character-by-character grammar, stores only structural issue data, validates
the composed endpoint before settings/player/network side effects, and
converts both characterizations to ordinary positive regressions.

The new differential invokes the actual pinned Python
`ConfigurationGetter.getConfiguration()` from commit
`d1c5f85af377c960c5a940707c4d01bc84fd9c3f`; it does not compare against a
second handwritten grammar. Side-effecting Python collaborators are stubbed,
and passwords project only as the fixed string `<redacted>`. It proves
canonical short-attached forms, separated forms, clusters, optional-value
fall-through, and exact `-p`/`-psn` precedence. A second differential covers
the final endpoint boundary. The pinned Python implementation accepts
`[::1]:notaport` by falling back to the previous/default port; Sorotte
deliberately remains stricter and rejects that explicit malformed port. The
report therefore records this one fail-closed delta instead of claiming false
parity.

Actual-binary integration tests independently inspect stdout, stderr, and the
returned error for accepted and rejected password canaries. A malformed final
endpoint is also run with an absent config root and a canary player path; the
process must exit without creating the config root or reporting a player
launch. Complete follow-up evidence is in
[`pro-review-remediation-20260801.md`](pro-review-remediation-20260801.md).

The slice began from committed branch base
`2e6746b4a0ec4fdee2bbe09328161f064d5ca772`. Two unrelated focused slices were
committed concurrently, so final focused validation ran with shared worktree
HEAD `a3e4d065ba41fdc397f0b9ea825574beeb540d2b`; the CLI changes remained
uncommitted and isolated to the files listed below during this report.

## Strategy gap and boundary

`docs/TEST_COVERAGE_STRATEGY.md` already records a lower-layer
`sorotte-client-app` generated campaign over all 30 environment-overridable
stored fields. That suite deliberately models environment presence through a
pure projection and does not cross the CLI parser.

This slice covers the separate parser boundary in `sorotte-cli`. Each valid
generated case invokes the same production parser and composition entrypoints
used by startup. Listed in configuration-precedence order, those entrypoints
are:

1. `build_client_loop_config_from_env`;
2. `apply_stored_client_settings_mvp_if_env_absent`;
3. `parse_legacy_client_arg_overrides`; and
4. `apply_legacy_client_arg_overrides`.

Startup parses arguments before building configuration; because parsing is
pure, the campaign builds the lower-layer projection before invoking that same
parser and applying its output. Invalid cases stop at the production
parser/diagnostic boundary, as startup does. The campaign projects
representative CLI-composable values: host and its coupled port, username,
room, controlled-room password, plus the lower-layer server password. It
combines real process-environment values, a production stored-settings DTO,
and rendered long/short CLI arguments.

The pinned Syncplay reference in
`.interop-cache/syncplay-legacy/syncplay/ui/ConfigurationGetter.py` defines
host/name as required-value argparse options and room/password with
`nargs='?'`. Argparse accepts attached `--option=value` syntax, and its
truthy-value override loop means an empty or missing optional duplicate reveals
the lower configuration layer. The Rust fixes preserve those composition
semantics while failing closed for missing required values.

## Deterministic campaign and independent oracle

The test
`generated_cli_configuration_composition_matches_independent_precedence_oracle`
uses:

- fixed seed `0xC11A_5E7C_0F1A_2026`;
- fixed budget 256;
- 16 explicit scenario patterns, each exercised 16 times;
- 208 valid compositions and 48 invalid parser cases;
- 64 cases containing a clear operation; and
- 112 cases containing duplicate field operations.

The patterns cover no CLI override, separated and attached long/short forms,
duplicate host/name/room/password operations, a later host without a port,
empty attached duplicates, missing required host/name values, missing optional
room/password values, controlled-room embedded passwords, explicit password
precedence, and an unknown attached credential-shaped value.

The oracle does not reuse the production parser, host parser, controlled-room
normalizer, or configuration application helper. It independently:

- renders an operation model into CLI tokens;
- models invalid-option identity and order;
- models stored-versus-environment precedence;
- parses only the generated canonical host shapes;
- recognizes only the generated canonical controlled-room password shape; and
- applies unchanged/replace/clear CLI transitions to a small expected
  projection.

Production and oracle projections are compared exactly. Secret-bearing fields
use `SecretValue` only as the comparison carrier so assertion/debug failures
remain redacted. Every case also checks that generated server passwords,
controlled-room passwords, explicit CLI passwords, and unknown attached values
are absent from production `Debug` text and bounded invalid-option
diagnostics.

## Retained RED and defect classification

Before the fixes, this exact command selected five narrow regression tests:

```powershell
cargo test --locked -p sorotte-cli --all-features `
  cli_argument_configuration_composition -- --nocapture
```

Result: **0 passed, 5 failed, 0 ignored, 368 filtered out**.

The failures proved:

1. `--api-token=CLI_UNKNOWN_OPTION_SECRET_CANARY` appeared verbatim in the
   diagnostic (`TC-CLI-005`);
2. attached host/name/room/password forms were all classified as unknown;
3. `--host first.example:1111 --host second.example` retained CLI port `1111`;
4. attached empty duplicates did not clear prior CLI overrides; and
5. missing required host/name values were silently accepted.

Items 2–5 share the same `TC-CLI-004` root cause: the hand-written parser had
no explicit per-occurrence unchanged/replace/clear/invalid transition and
updated the host and port as separable accumulated values. Item 1 is separate
because it crosses the user-visible diagnostic boundary and requires
fail-closed value redaction regardless of parser state.

After the focused production fixes, the same command's first GREEN result was
**5 passed, 0 failed, 0 ignored, 368 filtered out**. The deterministic campaign
was then added to exercise the generalized behavior rather than only the five
examples.

## Implemented source changes

- `crates/sorotte-cli/src/client_args/parser.rs`
  - accepted long attached values and the campaign's `-x=value` short
    spelling; canonical `-xVALUE` support is the follow-up described above;
  - replaces host and port as one CLI-layer occurrence;
  - permits empty attached values to clear an earlier CLI-layer occurrence;
  - preserves optional missing room/password behavior; and
  - records missing required host/name options as invalid.
- `crates/sorotte-cli/src/client_args/apply.rs`
  - builds bounded unknown-option diagnostics and redacts every attached value
    after the first `=`.
- `crates/sorotte-cli/src/client_args.rs` and
  `crates/sorotte-cli/src/lib.rs`
  - route the production startup diagnostic through that redaction boundary.
- `crates/sorotte-cli/src/tests.rs` and
  `crates/sorotte-cli/src/tests/cli_argument_configuration_composition.rs`
  - add the five retained regressions and deterministic independent-oracle
    campaign.

No client-app source or central documentation was changed by the original
2026-07-31 slice. The review follow-up updates the central documents and the
evidence record.

## Executed proof

Generated campaign:

```powershell
cargo test --locked -p sorotte-cli --all-features `
  generated_cli_configuration_composition_matches_independent_precedence_oracle `
  -- --nocapture
```

Result: **1 passed, 0 failed, 0 ignored, 373 filtered out**. All 256 generated
cases completed in 0.02 seconds.

Complete focused module:

```powershell
cargo test --locked -p sorotte-cli --all-features `
  cli_argument_configuration_composition -- --nocapture
```

Result: **6 passed, 0 failed, 0 ignored, 368 filtered out**.

Owning-crate all-feature gate:

```powershell
cargo test --locked -p sorotte-cli --all-features
```

Result: **366 library tests passed, 0 failed, 8 ignored**, plus **2/2
integration tests passed** and doc tests passed.

Strict lint:

```powershell
cargo clippy --locked -p sorotte-cli --all-targets --all-features `
  -- -D warnings
```

Result: passed with no warnings.

Scoped formatting and whitespace validation:

```powershell
rustfmt --check --edition 2024 `
  crates/sorotte-cli/src/client_args.rs `
  crates/sorotte-cli/src/client_args/apply.rs `
  crates/sorotte-cli/src/client_args/parser.rs `
  crates/sorotte-cli/src/lib.rs `
  crates/sorotte-cli/src/tests.rs `
  crates/sorotte-cli/src/tests/cli_argument_configuration_composition.rs

git diff --check -- `
  crates/sorotte-cli/src/client_args.rs `
  crates/sorotte-cli/src/client_args/apply.rs `
  crates/sorotte-cli/src/client_args/parser.rs `
  crates/sorotte-cli/src/lib.rs `
  crates/sorotte-cli/src/tests.rs
```

Both scoped checks passed. A concurrent repository-wide
`cargo fmt --all -- --check` stopped only on the other agent's uncommitted
`crates/sorotte-gui/src/bin/sorotte-gui-native-smoke/native_smoke_runner/real_mpv_vertical.rs`;
it reported no CLI-file diff. The final branch-wide validation should rerun
that gate after the GUI slice is formatted.

## Safety and scope limits

All generated argument/configuration input remains in memory. The campaign
uses only argument vectors, configuration structs, and six process-environment
keys. Environment mutation is serialized through the existing shared test
lock and restored by `TestEnvGuard`. It opens no sockets, starts no process,
writes no configuration file, targets no network service, and performs no
reconnaissance, credential access, persistence, privilege work, or
exploitation.

This is representative CLI-layer coverage, not an exhaustive parser grammar.
It does not duplicate the lower `sorotte-client-app` 30-field composition
campaign, and it does not cover unrelated startup/player-path, playlist, GUI,
or diagnostics flags. It uses canonical valid lower-layer values; malformed
environment and stored values remain covered at their owning boundaries.
