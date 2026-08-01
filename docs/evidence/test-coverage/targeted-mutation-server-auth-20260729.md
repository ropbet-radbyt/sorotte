# Targeted mutation proof: server-auth

Date: 2026-07-29 (Australia/Sydney)

Branch: `codex/test-coverage-design`

Experiment checkout commit: `941e737ec291cffe01a6e5a7f49e8cd2aa5f53a2`

Producer: `cargo-mutants 27.1.0`

Target: `sorotte-server`, `crates/sorotte-server/src/auth.rs`

Scheduled test scope: library target, `auth::tests::` namespace

## Claim

The second bounded mutation shard converts server controlled-room
authorization into a 100% viable mutation ratchet. A real baseline exposed one
missing negative authorization oracle, one nondeterministic salt-mapping
oracle, and an unsuitable package-wide execution boundary. Focused tests and
a semantics-preserving byte-mapping seam moved the result from 16/18 viable
mutants caught to 19/19, with no missed, timed-out, unviable, or accepted
mutants.

No product defect was discovered or fixed in this slice. The production salt
algorithm remains `A + (byte % 26)`; extracting that expression into a pure
function makes all 256 possible random inputs mechanically testable.

## Why this target

`auth.rs` owns security-sensitive, mostly pure decisions:

- controlled-room name recognition;
- legacy room-password grammar;
- salted controlled-room hashing and verification;
- controlled-room name construction;
- legacy-compatible random server salt encoding.

It is classified as a critical server-auth module in the coverage policy. Its
small source surface is appropriate for a bounded weekly shard, while the
entire server package includes persistence, networking, TLS, and external
release-verification tests that are not appropriate to rerun for every
authorization mutant.

## Experiment 0: reject the package-wide boundary

The unchanged-source probe initially used the package default test scope:

```text
cargo mutants --package sorotte-server \
  --file crates/sorotte-server/src/auth.rs \
  --no-config --colors never --no-times --no-shuffle \
  --all-features --cargo-arg=--locked \
  --jobs 2 --timeout 60 --build-timeout 120 \
  --output target/mutation-auth-baseline
```

The producer inventoried 18 mutants, but the unmutated baseline timed out
inside the external `server_release_verify` integration target. It therefore
tested zero mutants and failed closed after 93.53 seconds.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `mutants.json` | 29,417 | `3d32d7f31079b646938160a47c7419c639ed9da0e5fa2154cdbefecc88c6681c` |
| `outcomes.json` | 1,239 | `d7c6e9cf7df9eb024bdf72a3559930b98aa168e4967d20c4799d1d5c8ad1efc2` |

Increasing every mutant timeout would multiply unrelated integration cost and
would still couple a pure authorization gate to external Python and process
tests. The correct boundary is a focused library namespace.

## Experiment 1: expose the oracle gaps

The unchanged source was repeated with `--cargo-test-arg=--lib`. The unmutated
baseline passed, all 18 mutants ran, and the experiment completed in 150.87
seconds:

| Outcome | Count |
|---|---:|
| Caught | 16 |
| Missed | 1 |
| Timed out | 1 |
| Unviable | 0 |
| Viable kill rate | 88.89% |

The missed mutant was:

```text
replace % with / in generate_server_salt_legacy_compatible
```

The existing salt test asserted only ten uppercase characters. Division maps
every byte to `A` through `J`, which still satisfies that shape. Repeating a
random generator until a later letter appears would be probabilistic and was
rejected as a flaky oracle.

The timed-out mutant was:

```text
replace RoomPasswordProvider::is_controlled_room_name -> bool with true
```

The broad library suite did observe the corruption, but treating every
ordinary room as controlled caused failures and long-running cascades across
unrelated session, state, persistence, and network tests. The producer reached
its 60-second test timeout. This is useful evidence that mutation execution
scope is part of the assurance design, not merely a performance option.

The inventory was byte-identical to Experiment 0. Its outcomes artifact was
38,317 bytes with SHA-256
`7766c09ab677b9cc6021cd15680722a4b7d1a7457b037d644ec18855aecd299c`.

## Oracle and testability changes

The existing authorization tests were moved beside `auth.rs` under
`auth::tests`, preserving their behavior while giving the shard a stable
namespace. Two deterministic additions close the observed gaps:

1. A negative controlled-room grammar table distinguishes an ordinary room,
   missing prefix, short/long hash, and non-word hash from a valid controlled
   name. It also asserts `NotControlledRoom` for an ordinary room.
2. `legacy_salt_character(u8) -> char` isolates the unchanged legacy mapping.
   Boundary anchors prove `0 -> A`, `25 -> Z`, `26 -> A`, and `255 -> V`.
   These assertions kill modulo/division and wraparound changes without
   depending on random output.

The focused unmutated suite passes 7/7 in approximately 0.02 seconds. The pure
seam adds one new generated `char` return mutant, so the final inventory grows
from 18 to 19 rather than shrinking. The four arithmetic mutations remain on
the same mapping expression, now in the deterministic helper.

## Experiment 2: source-bound scheduled contract

The checked-in wrapper constructed and attested this command:

```text
cargo mutants --package sorotte-server \
  --file crates/sorotte-server/src/auth.rs \
  --no-config --colors never --no-times --no-shuffle \
  --all-features --cargo-arg=--locked \
  --cargo-test-arg=--lib \
  --cargo-test-arg=auth::tests:: \
  --jobs 2 --timeout 60 --build-timeout 120
```

The exact-final wrapper run completed in 113.36 seconds:

| Outcome | Count |
|---|---:|
| Unmutated baseline | Passed |
| Caught | 19 |
| Missed | 0 |
| Timed out | 0 |
| Unviable | 0 |
| Viable kill rate | 100.00% |

The pre-run and result inventories were exactly equal. Their canonical digest
was `7521098e46dbb09fc1867a5e6d67cc2e745da680f462e3eec75e83d24e16fb0d`.
The configured source hash was identical before and after execution.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Configured Rust source | 6,689 | `feebbd968bd532ba2e089ac913b9b1af17c914c1b2c9c0297a8c7ebd5d56a5fa` |
| `mutants.json` | 31,497 | `3d5a5678f287257570f3363d9eb086d3d55cf1d910b8f23778937081ba776787` |
| `outcomes.json` | 40,879 | `0fa205e89b3d5966320f81acddec77c123d1c26e709c1d550ba968e1a04b0523` |
| Wrapper report | 10,371 | `c550ff28f2a8b63d062042c98c009f3e2dedfe165f301d5cecab96fd689b3c00` |

The local report records `configured_sources_dirty: true` because proof ran
before this slice was committed. The exact source binding above, equal
before/after hashes, reconciled inventories, and artifact hashes bind the
experiment. Scheduled CI runs from a fresh checkout.

## Mechanical enforcement

Mutation policy schema 2 makes test execution scope explicit. Each shard owns:

- a literal package and source-file set;
- either package or library test target;
- an optional safe Rust module namespace rather than arbitrary Cargo text;
- jobs and build/test timeouts;
- viable kill, missed, timeout, baseline, and accepted-unviable policy.

The wrapper emits the scope through `--cargo-test-arg`, then validates every
producer build and test phase against the exact configured argument multiset.
Dropping the library target or namespace, adding an unapproved argument, or
running a different package makes the evidence invalid even if the summary
claims success.

The weekly workflow now uses a fail-closed matrix for `privacy-secret` and
`server-auth`. Matrix fail-fast is disabled so one failing shard does not erase
the other shard's diagnostic evidence. Each shard retains its own 120-minute
job ceiling and uploads its compact report and raw producer tree on success or
failure.

## Finding disposition and limits

No application finding is added: both survivors were coverage-design gaps,
and the deliberately corrupted controlled-room predicate is not evidence that
the unmutated product is wrong.

This shard proves only mutations generated for `auth.rs` and observed through
the focused unit namespace. It does not prove cryptographic strength,
operating-system entropy availability, constant-time comparison, network
admission wiring, persistence authorization, concurrency schedules, or
equivalent defects that cargo-mutants does not generate. Those remain separate
unit, integration, system, and review boundaries.
