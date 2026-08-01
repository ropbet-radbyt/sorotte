# Targeted mutation proof: protocol codec and diagnostic redaction

Date: 2026-07-29 (Australia/Sydney)

Branch: `codex/test-coverage-design`

Experiment checkout commit: `dbc4d68ece8b6fdcd3b4a1daeb960d73edf77b55`

Producer: `cargo-mutants 27.1.0`

Target:

- `sorotte-protocol`, `crates/sorotte-protocol/src/codec.rs`
- `sorotte-protocol`, `crates/sorotte-protocol/src/redacted_debug.rs`

Scheduled test scope: complete `sorotte-protocol` library target

## Claim

The third bounded mutation shard converts raw protocol command ordering,
protocol error chaining, and credential-safe diagnostic formatting into a
100% viable mutation ratchet. The unchanged-source baseline caught only 70 of
97 viable mutations. Seventeen deterministic tests and
semantics-preserving, bounded scanner seams now catch 80 of 80 viable
mutations with zero misses and zero timeouts.

Eight generated replacements cannot compile because they try to synthesize
`Default` values for types that deliberately have no meaningful default.
Each is matched by file, function, return type, mutation genre, and exact
replacement, and expires for review on 2026-10-31. A new or stale exception
fails the shard.

No application defect was discovered or fixed. The baseline survivors were
test-oracle gaps or mutation-testability noise. The proof did expose and fix
one defect in the mutation attestation wrapper: cargo-mutants represents a
top-level constant mutation with `"function": null`, which the earlier
single-file shards had not exercised.

## Why this target

The codec is a narrow but high-consequence protocol boundary. It owns:

- reconstruction of top-level command order that `serde_json` maps do not
  preserve;
- reconstruction of nested `Set` member order;
- structural scanning across escaped strings, arrays, nested objects, and
  whitespace;
- selection and decoding of coalesced protocol commands;
- preservation of per-command decoding errors;
- safe `Debug`, `Display`, and `Error::source` behavior.

The adjacent redaction module is the last diagnostic boundary for permissive
JSON and text carriers. A false negative can expose a password or token; an
over-broad true result can erase ordinary diagnostics that operators need.
The two files therefore form one behavioral shard even though the full
protocol crate contains many unrelated DTOs.

The complete library suite is an appropriate execution boundary. It passed 65
tests in approximately 0.01 seconds locally, while exercising public
decode/encode behavior in addition to the private scanner and formatter
oracles.

## Experiment 0: unchanged-source baseline

The initial command was:

```text
cargo mutants --package sorotte-protocol \
  --file crates/sorotte-protocol/src/codec.rs \
  --file crates/sorotte-protocol/src/redacted_debug.rs \
  --no-config --colors never --no-times --no-shuffle \
  --all-features --cargo-arg=--locked \
  --cargo-test-arg=--lib \
  --jobs 2 --timeout 60 --build-timeout 120 \
  --output target/mutation-protocol-codec-baseline
```

The unmutated baseline passed and all 106 generated mutations were
classified:

| Outcome | Count |
|---|---:|
| Caught | 70 |
| Missed | 27 |
| Timed out | 0 |
| Unviable | 9 |
| Viable kill rate | 72.16% |

The 27 survivors clustered around behavior that ordinary round-trip tests did
not distinguish:

- exact 64 KiB line-limit arithmetic;
- `ProtocolError` debug output and JSON error source chaining;
- depth/expect-key transitions in both raw scanners;
- whitespace after keys and colons;
- exclusive object-end offsets;
- direct versus nested `Set` selection;
- ordinary versus sensitive JSON strings and object keys;
- exact `Some`/`None` rendering for redaction wrappers.

The nine unviables comprised eight impossible generated default values and one
invalid cargo-mutants rewrite of a Rust `&& let` chain into `|| let`.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `mutants.json` | 191,621 | `0167832cc9c38cb9447af0f4a626c193e5b82c01d797a6bfde1ce4a7263dd5b1` |
| `outcomes.json` | 216,613 | `87056a7933364a21840dfe16944b702bf3bd13aab48b1fb2d7a524396fca199c` |

## Oracle and testability changes

### Bounded scanner progress

Both scanners previously advanced mutable whitespace indices with `+= 1`.
Replacing that operator can create a non-progressing mutation. The new
`next_non_whitespace_index` uses a bounded enumerated slice search and returns
the input length when no later non-whitespace byte exists. Its exact behavior
is tested at the beginning, middle, first non-whitespace byte, trailing
whitespace, and a start beyond the slice.

The scanner output is unchanged. This is a testability seam, not a parser
policy change.

### Explicit structural states

Redundant boolean guards were expressed as tuple patterns for:

- top-level object entry;
- depth-one key completion;
- depth-one comma transitions.

For valid JSON, changing one of the old `&&` or match guards could not escape a
second depth invariant, making six generated mutations behaviorally
equivalent. Tuple-pattern transitions retain the same valid-input state
machine without manufacturing meaningless boolean alternatives.

The wanted-key parse now uses a total parsed-key predicate rather than an
`&& let` chain. The equality remains a viable tested mutation, while the
syntactically impossible `|| let` mutation disappears.

### Seventeen deterministic oracles

Nine codec tests now pin:

- the exact 65,536-byte default limit;
- safe `ProtocolError` debug variants and JSON source chaining;
- bounded whitespace indices;
- direct key order across nested objects, arrays, escaped quotes, structural
  lookalikes, and non-object roots;
- Unicode-escaped key decoding and key/colon whitespace;
- exclusive object ends at nonzero offsets, nested objects, string-contained
  braces, non-object starts, and unterminated objects;
- direct escaped `Set` selection when a nested lookalike precedes it;
- rejection of nested-only, scalar, and missing wanted values;
- direct `Set` member order while nested and string lookalikes are ignored.

Eight redaction tests now pin:

- ordinary string preservation and credential-bearing string redaction;
- recursive key- and value-based redaction;
- exact optional JSON `Some` and `None` output;
- always-value-free optional text;
- classified optional sensitive text;
- per-element text-list classification;
- exact ordered-map ordinary/sensitive key output;
- both boolean outcomes of the shared classifier forwarding boundary.

The protocol library suite grew from 48 to 65 tests. No production result was
changed to satisfy an assertion.

## Experiment 1: independent residual guard noise

The first repeat inventoried 102 mutations:

| Outcome | Count |
|---|---:|
| Caught | 88 |
| Missed | 6 |
| Timed out | 0 |
| Unviable | 8 |
| Viable kill rate | 93.62% |

All six survivors were the paired redundant valid-JSON depth guards described
above: three in top-level key discovery and three in top-level object-span
selection. Direct malformed-state tests could have killed them, but that
would encode inputs the private scanner never receives after JSON decoding.
Expressing the actual state machine without redundant boolean mutation sites
was the stronger solution.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `mutants.json` | 182,950 | `9af6baf65560a8269e788949595b82b734bf3a369bc0ad7bb5f82e5086d3d9bf` |
| `outcomes.json` | 209,363 | `18f1b796156c33ab7925f11bc8062c7af724b738d2988f336614aa2cfc1ff4a7` |

## Experiment 2: zero-survivor producer proof

After the structural-state refactor, a direct producer repeat classified 88
mutations:

| Outcome | Count |
|---|---:|
| Caught | 80 |
| Missed | 0 |
| Timed out | 0 |
| Unviable | 8 |
| Viable kill rate | 100.00% |

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `mutants.json` | 162,196 | `d29cf319b6311614a6a8e9e2a5c9be48e2b6bd5f03d0c89e15b034c4eb63ca65` |
| `outcomes.json` | 180,764 | `dde7fb903638424857eda79d7e4ac75059b28827675abbe32c670fc8fbabf76c` |

The smaller final inventory is not itself the success metric. The evidence is
that every original survivor received an exact oracle or was shown to be a
redundant mutation site, the invalid `|| let` mutation was made viable as an
ordinary equality decision, progress is bounded, and the final viable
inventory has no survivor or timeout.

## Attestation-wrapper defect and regression

The first policy-driven run failed closed before starting the producer:

```text
mutation evidence error:
pre-run cargo-mutants inventory[0].function must be an object
```

The first item is the global line-limit arithmetic mutation. cargo-mutants
27.1.0 emits a complete function object for mutations inside functions but
emits JSON `null` for this top-level constant. The strict parser incorrectly
assumed the former shape was universal.

The wrapper now:

- accepts exactly `null` or the existing strict function object;
- continues to reject booleans, strings, arrays, and malformed objects;
- preserves `null` during pre/post inventory reconciliation;
- maps top-level identity to the reserved `<top-level>` / `<none>` sentinels
  if an unviable top-level mutation ever needs exact policy review.

Focused regression and adversarial tests exercise both the valid null shape
and invalid non-object shapes. The initial failure root and stderr log remain
preserved; the successful proof used a fresh results root.

## Experiment 3: exact fail-closed scheduled contract

The checked-in wrapper constructed and attested:

```text
cargo mutants --package sorotte-protocol \
  --file crates/sorotte-protocol/src/codec.rs \
  --file crates/sorotte-protocol/src/redacted_debug.rs \
  --no-config --colors never --no-times --no-shuffle \
  --all-features --cargo-arg=--locked \
  --cargo-test-arg=--lib \
  --jobs 2 --timeout 60 --build-timeout 120
```

The producer ran from 11:14:16Z through 11:15:46Z, 89.73 seconds:

| Outcome | Count |
|---|---:|
| Unmutated baseline | Passed |
| Caught | 80 |
| Missed | 0 |
| Timed out | 0 |
| Accepted unviable | 8 |
| Viable kill rate | 100.00% |

The pre-run and result inventories were exactly equal. Their canonical digest
was `dd2f4ad5997e505903647eb2ed0c9eca0d324188c69909abdc3417389e7b643c`.
Both configured source files had identical before/after hashes:

| Configured source | Bytes | SHA-256 |
|---|---:|---|
| `codec.rs` | 17,639 | `f6012abe4c7414c6d316adb8b090af8fd6313f1fd01671e8a1dc97a0e9413e7d` |
| `redacted_debug.rs` | 8,016 | `1a39c651af008b63ca3eefe901988ea34309dd0a2084fffa15ffe90f167abcf1` |

| Evidence artifact | Bytes | SHA-256 |
|---|---:|---|
| Pre-run normalized inventory | 162,197 | `0192e4f0550ca407a57c5163ec35f7a54a2901f2f30c66c5a7940d456563ce24` |
| `mutants.json` | 162,196 | `d29cf319b6311614a6a8e9e2a5c9be48e2b6bd5f03d0c89e15b034c4eb63ca65` |
| `outcomes.json` | 180,741 | `112cbfa95316cffd7e48e081a97b2ff83c555826a49163ce8f38a5c49cf965de` |
| `caught.txt` | 8,746 | `07af20fd1856953d46e961ee2f520860aa1c89aa4148da9ba51fa71153b46296` |
| `unviable.txt` | 1,242 | `baad04351db13555510aaf025b7e62c3c29799141866fd3f3184efbd715c7f65` |
| Wrapper report | 41,164 | `127c41b02309b2173a34cb853d108fc96acd2ed431bd6a3a5669376f2eb59fcc` |

The local report records `configured_sources_dirty: true` because the new
oracles had not yet been committed. The source hashes before and after,
reconciled inventories, and raw artifact hashes bind the proof. Scheduled CI
runs from a fresh checkout.

## Exact unviable policy

The eight generated compiler failures are:

1. construction of a default dynamic `Error` source;
2. construction of a default `ProtocolError`;
3. construction of a default `ProtocolMessage` after order decoding;
4. construction of a default `ProtocolMessage` vector element;
5. construction of a default `DecodedMessageLineItem` vector element;
6. construction of a default single `ProtocolMessage`;
7. construction of a default `HelloPayload` from JSON;
8. construction of a default `HelloPayload` from a typed message.

These are not counted as caught and do not inflate the viable score. The
wrapper requires each exact identity to appear as unviable, rejects any new
unviable identity, and rejects a listed identity that becomes stale. Adding
`Default` implementations solely to make these synthetic substitutions
compile would create misleading protocol values and is not justified.

## Mechanical enforcement

The weekly matrix now runs `privacy-secret`, `server-auth`, and
`protocol-codec` independently with matrix fail-fast disabled. The new shard
requires:

- the literal two-file source set and `sorotte-protocol` package;
- the library test target and no hidden selector;
- all features and the locked dependency graph;
- two workers, a 60-second mutant timeout, and a 120-second build timeout;
- an unmutated baseline;
- 100.00% viable kill rate;
- zero missed and zero timed-out mutations;
- exact, non-expired matching for all eight unviables.

CI policy tests assert the full workflow matrix, complete TOML object, and
exception identities. Wrapper tests preserve the newly observed top-level
inventory schema and continue to reject malformed alternatives.

## Finding disposition and limits

No application finding is added. During construction, four initial test
expectations were corrected because `serde_json::Value` uses Rust enum-style
`Debug` syntax such as `String("ready")`; the implementation already behaved
as designed.

This shard proves generated mutations in two protocol diagnostic files. It
does not prove:

- completeness of the credential vocabulary;
- memory or CPU bounds for arbitrarily large allowed lines;
- transport framing, reconnect, TLS, or session-state behavior;
- Unicode normalization beyond JSON escape decoding;
- semantic equivalence to every Python Syncplay ordering edge case;
- defects outside cargo-mutants' generated operator and return-value set.

Those remain covered by privacy corpora, protocol permutation tests, IPC fault
harnesses, compatibility lanes, and future transport/session mutation shards.
