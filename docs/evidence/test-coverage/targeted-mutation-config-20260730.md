# Targeted mutation proof: persisted runtime configuration decisions

Date: 2026-07-30 (Australia/Sydney)

Branch: `codex/test-coverage-design`

Experiment checkout commit: `c4ad56e0bd4bf363f0ec86b605326191bc8073b2`

Producer: `cargo-mutants 27.1.0`

Target: `sorotte-client-app`,
`crates/sorotte-client-app/src/legacy_runtime_config.rs`

Scheduled test scope: `--lib legacy_runtime_config::tests::`

## Claim

The `client-runtime-config` shard turns the persisted-settings-to-runtime
compatibility boundary into a zero-survivor mutation ratchet. The final
source-bound run inventoried 14 selected tests and 103 mutations. It caught
all 98 viable mutations, with no misses or timeouts. Five generated `|| let`
replacements cannot parse and collapse to three exact mutation identities;
all three are matched by minimal, expiring policy entries.

No product defect was found or fixed. The baseline survivors exposed missing
behavior oracles for:

- stored configuration versus environment-variable precedence across every
  supported runtime override;
- the independent validity predicates for controlled-room names and hashes;
- normalization of blank host, username, and password values;
- the empty controlled-room-password result after filtering unsupported
  characters.

One baseline survivor was strictly equivalent. After the parser's
`exactly one colon` branch returns on every path, testing whether the remaining
input has `more than one colon` or simply `contains a colon` is observationally
identical. The final source uses the simpler predicate. This is a
behavior-preserving simplification, not a product behavior change.

## Why this target

The preferred primary resolver,
`crates/sorotte-client-app/src/runtime_config.rs`, is an 88 KB source with 198
candidate mutations. The INI parser has 170. The selected 27 KB compatibility
boundary originally had 106 mutations and is the smaller, more coherent owner
of the decisions under test:

- explicit host and port parsing, including bracketed and unbracketed IPv6;
- safe public-server fallback and invalid-port filtering;
- controlled-room canonicalization and password normalization;
- whitespace normalization for identity and credential values;
- precedence between stored values and command-line environment presence;
- projection of every stored playback, synchronization, readiness, privacy,
  and interface override into the legacy runtime plan.

These decisions are dense with boolean gates. One-token changes can silently
reverse precedence, apply an absent setting's default as an explicit override,
or admit an invalid controlled-room identifier while all broad happy-path
tests still pass. A source-bound mutation ratchet gives stronger evidence than
line coverage alone.

## Experiment 1: unchanged-source baseline

The fail-closed wrapper selected the original ten owning tests and ran:

```text
python scripts/mutation_ci.py run \
  --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard client-runtime-config \
  --results-root target/mutation-ci/client-runtime-config-baseline-20260730 \
  --output target/verification/mutation-client-runtime-config-baseline-20260730.json
```

| Outcome | Count |
|---|---:|
| Selected tests | 10 |
| Total mutations | 106 |
| Caught | 52 |
| Missed | 49 |
| Timed out | 0 |
| Unviable | 5 |
| Viable kill rate | 51.49% |

The 49 survivors grouped into four actionable oracle gaps:

| Gap | Representative mutation | Baseline survivors |
|---|---|---:|
| Environment shadowing and explicit override projection | delete `!` or replace `&&` with `||` | 44 |
| Controlled-room base/hash validation | replace `||` with `&&` | 2 |
| Blank password and username filtering | delete `!` | 2 |
| Redundant remaining-colon boundary | replace `>` with `>=` | 1 |

The last mutation was equivalent because the preceding exactly-one-colon
branch always returns. The other 48 represented observable behavior that the
ten-test baseline did not distinguish. None demonstrated incorrect current
product behavior.

| Binding or artifact | Value |
|---|---|
| Original source SHA-256 before/after | `e65c3180e201626fe127c39e7de318649b64698dc72356aa08ff6d4f5f80859d` |
| Test inventory canonical SHA-256 | `bf1ce197e3f745c3e22da6955ac30b7028469bfe864f9f7a64656a78ea8b9eff` |
| Mutation inventory canonical SHA-256 | `dcb373040c1178ab004c63aa8f90962f52d3e0c045fe6266f63e2e24c482837d` |
| `missed.txt` | 6,524 bytes, `b846cdc025f803d08605b2a77148b06042a55280ff994229b9e8bc07692e582e` |
| `mutants.json` | 215,202 bytes, `8c9bfa5104ca718f5393ee0ab380385258ab78487e371bc0c59cba5c8923010b` |
| `outcomes.json` | 235,619 bytes, `69b4b662128f2d7813ae135fcc3010901ca451644f248760d901a0196ed9afb7` |

## Added behavior oracles

Four deterministic ordinary tests distinguish every observable survivor:

1. `controlled_room_normalization_rejects_each_invalid_canonical_component`
   independently supplies an empty base, an eleven-character hash, and a
   twelve-character hash containing punctuation. It also proves that a
   password containing only unsupported characters normalizes to absent.
2. `runtime_snapshot_discards_blank_optional_identity_values` proves that
   whitespace-only host, server password, and username values do not survive
   the runtime snapshot boundary.
3. `config_plan_applies_every_explicit_override_when_environment_is_absent`
   supplies all 31 persisted runtime decisions at once and compares the whole
   resulting plan, including normalized secrets, controlled-room identity,
   domain lists, thresholds, privacy modes, and false-valued overrides.
4. `config_plan_suppresses_every_explicit_override_when_environment_is_present`
   supplies the same settings while marking every corresponding environment
   value present, then proves the stored plan is completely empty.

The last two tests deliberately use complementary inputs. They distinguish
both deletion of an environment-negation gate and replacement of
`stored-is-present && environment-is-absent` with `||`. Whole-structure
equality prevents a newly added plan field from silently escaping the
precedence matrix.

## Final attested run

```text
python scripts/mutation_ci.py run \
  --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard client-runtime-config \
  --results-root target/mutation-ci/client-runtime-config-20260730 \
  --output target/verification/mutation-client-runtime-config-20260730.json
```

Result:

```text
Found 103 mutants to test
ok       Unmutated baseline
103 mutants tested: 98 caught, 5 unviable
mutation shard client-runtime-config: 98/98 viable mutants caught (100.00%)
```

| Attestation field | Value |
|---|---|
| Status | `passed` |
| Checkout HEAD | `c4ad56e0bd4bf363f0ec86b605326191bc8073b2` |
| Configured source dirty | `true` |
| Source SHA-256 before/after | `19e9e99f00ece9fc75dbc378b8e841353c9009ef385d79606a63333eeb64ee74` |
| Selected tests | 14 |
| Test inventory canonical SHA-256 | `ec6811f371f12e1ccbc84a2016b41e81282cc37bf1b40f9ac2709c48313829ab` |
| Mutation inventory | 103 |
| Mutation inventory canonical SHA-256 | `a799d8aedb5fb722a640e5ab0eaa6223378bf071b7b9f8cdf867b2890b56143a` |
| Viable mutants | 98 |
| Caught | 98 |
| Missed | 0 |
| Timed out | 0 |
| Unviable | 5 |
| Viable kill rate | `100.00%` |

The source is reported dirty because the final attestation intentionally ran
against the uncommitted test slice. The wrapper binds both the pre-run and
post-run bytes, and their hashes are identical. A source edit during the run
would fail the attestation.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Attestation report | 50,737 | `3ea34a1191331c25ff5c7fdd5139c4a889bd6a355e289b659e5bb0ee93054b93` |
| `test-inventory.json` | 1,582 | `07c2198483a2cedfa3547f016a1e280d0243f531128a172835a569b4888a1a2e` |
| Pre-run `inventory.list.json` | 210,237 | `40aac196b4f112d1cfdb30b1c062c359e3a9dc1584f954cc08bd519119e75ee7` |
| Producer `mutants.json` | 210,236 | `cbc4359fd79761d1ea82f41ea2bad75ba34d3a64543505e29b400069e1d5d2c3` |
| Producer `outcomes.json` | 230,255 | `18558bcd7932f29bbe41de68fd800290cfd359d68ccd557b393be97544cf49ab` |

## Compiler-unviable exceptions

The five unviable sites reduce to three exact identities:

- two let-chain sites in
  `parse_host_and_optional_port_from_host_arg_legacy_compatible`;
- two let-chain sites in
  `normalize_controlled_room_input_legacy_compatible`;
- one public-server fallback let-chain in
  `stored_client_settings_runtime_snapshot_legacy_compatible`.

At each site cargo-mutants replaces `&&` with `||`. Rust permits a let
expression only in an `&&` chain, so the generated form fails to parse before
tests can run. Each policy exception is bound to shard, file, function, return
type, mutation genre, and exact replacement. All expire for review on
2026-10-31. A changed identity or newly unviable mutation fails closed.

## Scheduled enforcement

`.github/workflows/rust-mutation.yml` schedules `client-runtime-config`
alongside the existing bounded shards. Its policy is:

- one source file;
- the `sorotte-client-app` library target;
- exactly the `legacy_runtime_config::tests::` namespace;
- two mutation jobs;
- 60-second per-test timeout;
- 120-second build timeout;
- required unmutated baseline;
- 100% viable kill threshold;
- zero misses and zero timeouts.

This remains a scheduled ratchet rather than a pull-request latency gate. Any
new survivor, timeout, empty test selection, empty mutation inventory,
selector escape, stale exception, source drift, or contradictory producer
artifact fails closed. The evidence artifact is uploaded even when the
producer or policy fails.

## Validation

The completed slice passed:

- the independently named final `cargo-mutants` attestation: 98 of 98 viable
  mutations caught;
- `cargo test --package sorotte-client-app --locked --all-features --lib
  legacy_runtime_config::tests:: -- --nocapture`: 14 tests passed;
- `cargo test --package sorotte-client-app --locked --all-features`: 185 unit
  tests and the crate doc-test target passed;
- `cargo clippy --package sorotte-client-app --all-targets --all-features
  --locked -- -D warnings`: passed;
- `python -m unittest scripts.tests.test_mutation_ci
  scripts.tests.test_ci_policy -v`: 50 tests passed;
- `python scripts/mutation_ci.py validate --repo-root . --policy
  coverage/mutation-policy.toml --shard client-runtime-config`: 5 shards and
  14 exact accepted-unviable entries validated;
- `C:\Users\shaun\go\bin\actionlint.exe
  .github/workflows/rust-mutation.yml`: passed;
- `cargo fmt --all -- --check`: passed;
- `git diff --check` over the five owned files: passed.
