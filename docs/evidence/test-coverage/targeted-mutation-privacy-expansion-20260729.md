# Targeted mutation proof: expanded privacy classifier

Date: 2026-07-29 (Australia/Sydney)

Branch: `codex/test-coverage-design`

Drift checkout commit: `941e737ec291cffe01a6e5a7f49e8cd2aa5f53a2`

Producer: `cargo-mutants 27.1.0`

Target: `sorotte-secret`, `crates/sorotte-secret/src/lib.rs`

## Claim

Replaying the required `privacy-secret` mutation shard found that the checked-in
100% policy had drifted after credential-diagnostic classification expanded.
The clean committed source failed at 119/153 viable mutants caught, with 29
missed and 5 timed out. Deterministic helper-level tests and bounded
semantics-preserving scans now catch 121/121 viable mutants with zero missed
or timed-out mutations. The one original const-context exception remains the
only unviable mutation.

No product defect was found or fixed. The red replay identified test-oracle
and mutation-testability debt in recently added privacy behavior.

## Why the earlier proof was no longer sufficient

The original privacy proof covered a 10,605-byte source and 44 generated
mutants. Later credential-classification work expanded the source to 20,053
bytes with URL/JSON escape projection, token-colon heuristics, key scanning,
and hex decoding. The policy still required 100%, but no current-run mutation
evidence had been generated for that larger behavior surface.

The schema-2 compatibility replay ran from a clean configured source. It was
therefore also a live audit of whether the existing scheduled gate still
matched repository reality.

## Experiment 1: reproduce policy drift

The checked-in wrapper selected the package and literal file, required all
features and `Cargo.lock`, and reconciled 162 mutants. It completed in 217.48
seconds:

| Outcome | Count |
|---|---:|
| Unmutated baseline | Passed |
| Caught | 119 |
| Missed | 29 |
| Timed out | 5 |
| Unviable | 9 |
| Viable kill rate | 77.78% |

The missed mutants clustered around:

- escaped diagnostic offsets and completeness;
- camel/punctuation word boundaries;
- exact numeric and alpha hex decoding;
- key trimming and identifier start boundaries;
- independent structured, quoted, and authentication-scheme token branches.

All five timeouts were mutation-created non-progressing index loops in
diagnostic projection or reverse key scanning. Six new unviable mutations
attempted to turn Rust `&& let` chains into invalid `|| let` expressions. Two
more were compile-time arithmetic overflows caused by operator replacement
precedence inside `hex_nibble`.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Configured Rust source | 20,053 | `6b979df743dae5356052332b34cb4afc9c8d136cf624cc7e82f5ff33733ea0af` |
| `mutants.json` | 254,089 | `575b97049f1d15a0713d8f952ec329052afe1a5d2d359972367de42c6b3a166e` |
| `outcomes.json` | 322,216 | `dbc1ae454d3fda04539a366a0623558db629948a9590981365043987e1d8d701` |
| Wrapper report | 69,009 | `daebfae1fa96bac851f591f07d9602c6553e633f5ce92b2a102a91317177e908` |

The normalized inventory digest was
`ae8f310bb0ad72b5ea0f61e3981fae3af07f520f739d68f304c045743cdff618`.

## Bounded implementation seams

Two loops were rewritten without changing their output:

1. Diagnostic escape projection now advances through successively shorter byte
   slices. Complete `%HH` and `\uHHHH` prefixes are consumed as units; invalid
   or non-ASCII escapes retain the original one-byte fallback.
2. Credential-key discovery now uses bounded reverse-position searches over
   the same delimiter prefix instead of decrementing mutable indices.

These structures cannot become non-progressing through a single arithmetic
operator mutation. The `&& let` chains disappeared as a consequence of
explicit slice-pattern matching, and parentheses make both alpha-hex
arithmetic mutants compile and face tests. This removes tooling noise while
retaining the behavioral branches. No unviable exception was added.

Deterministic tests now encode:

- exact word splitting across camel case, punctuation, uppercase runs, and
  empty inputs;
- exact ASCII projection for plain text, percent escapes, both Unicode marker
  cases, incomplete/invalid/non-ASCII escapes, invalid `\x` markers, and UTF-8
  bytes;
- numeric, lowercase, and uppercase hex boundaries plus invalid nibbles;
- ASCII Unicode decoding at `0x00`, `0x7f`, `0x80`, and invalid input;
- empty, quoted, whitespace-trimmed, prefixed, and missing credential keys;
- structured-key, quoted-value, Bearer, Basic, Digest, and plain-prose token
  branches independently;
- public classifier offsets for exact `token: EOF` preservation.

The ordinary crate suite passes 20/20.

## Experiment 2: one independent survivor

The first repeat completed in 58.90 seconds with 121 caught, one missed, zero
timeouts, and only the original accepted unviable mutation. The survivor
replaced the Unicode marker guard with `true`; the corpus covered valid `u` and
`U` markers but not a same-length invalid marker.

Adding `\x0041 -> \x0041` as an exact projection case closed that independent
oracle. The failed repeat and first green repeat inventories were
byte-identical:

| Artifact | Repeat bytes | Repeat SHA-256 |
|---|---:|---|
| `mutants.json` | 198,003 | `d06642fc97b5b74ef998e01f80327c71e2a26f630514364933a13213c1eb38cd` |
| `outcomes.json` | 248,212 | `d407baa60757055d4c3d4a628d48e0d8376aac5963a460b01508d6d611f2f398` |
| Wrapper report | 50,224 | `862e1bfd305182787d69be68f61b18dace7a275456e318de3777c6de0a2a3a94` |

## Experiment 3: exact Clippy-clean required ratchet

The first green repeat caught 122/122 against the same 123-mutant inventory.
Workspace Clippy then correctly rejected its separate Unicode marker guard as
redundant with a slice pattern. Expressing `u | U` directly in the pattern is
equivalent, leaves the invalid-marker oracle in place, and removes that one
guard mutation from the exact source.

The final Clippy-clean wrapper run completed in 58.37 seconds:

| Outcome | Count |
|---|---:|
| Unmutated baseline | Passed |
| Caught | 121 |
| Missed | 0 |
| Timed out | 0 |
| Accepted unviable | 1 |
| Viable kill rate | 100.00% |

The pre-run and result inventories were exactly equal, with canonical digest
`91cb9ac19f7abb2c6847319388bf151713fbcd69fadaf8ff8bb98bb616465ebb`.
The configured source hash was identical before and after execution.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Configured Rust source | 25,232 | `2805c14727aeb85070c3fc59c4d6ba00ed5538cc5ce232c9b2fc738bad825dcd` |
| `mutants.json` | 196,365 | `0a47590972cbf5ec46953967459c1dc0c63463d117d0de276475a176c85a5b29` |
| `outcomes.json` | 246,199 | `a1eaec80d959fb12cd4d2c9f45299609dfa090554069fcfcf30eead12264eb4d` |
| Wrapper report | 49,605 | `f9a15fee1012612c23feec8c2cdff49e1a9c6f9802248c660c5501b399ebe8ce` |

The final local report records a dirty configured source because the new
oracles had not yet been committed. Source hashes before and after, inventory
reconciliation, and raw artifact hashes bind the proof.

## Inventory reduction is not the score

The final inventory is smaller than the drifted inventory because bounded
slice traversal replaces mutation-sensitive index bookkeeping. The claim is
not that 122 is inherently better than 162. The evidence for improvement is:

- all 29 observed survivors receive deterministic behavioral oracles;
- all five observed timeouts are replaced by bounded execution;
- eight new compiler-infeasible mutants disappear without policy exceptions;
- both formerly unviable alpha-hex arithmetic changes become executable;
- the final producer has zero survivor or timeout and one unchanged accepted
  const exception;
- the failed and first-green 123-mutant inventories are byte-identical, the
  sole survivor is killed by one test-only input, and the exact Clippy-clean
  source is separately proven at 121/121.

## Finding disposition and limits

No application finding is added. During test construction, two hand-written
fixture offsets and one assumption about suffix-sensitive key classification
were corrected; the implementation conformed to its documented policy.

This shard tests generated mutations in one privacy crate. It does not prove
that its credential vocabulary is complete, that every future encoding is
recognized, or that downstream code consistently invokes the classifier.
Taint corpora, domain-carrier tests, protocol/system boundaries, review, and
the separate `server-auth` shard remain necessary independent controls.
