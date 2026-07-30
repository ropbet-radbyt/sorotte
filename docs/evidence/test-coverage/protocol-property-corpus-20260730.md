# Protocol parser property and corpus proof

Date: 2026-07-30 (Australia/Sydney)

Branch: `codex/test-coverage-design`

Scope:

- public `sorotte-protocol` byte-to-UTF-8 boundary;
- raw JSON and typed protocol decode entrypoints;
- top-level command ordering and duplicate-command payload selection;
- raw and typed encode/decode stability;
- deterministic checked-in parser corpus.

## Result

The new ordinary integration suite passed at both its default fixed-seed
budget and the repository's scheduled-depth budget. It found no product
defect.

Resolution update: after coverage-guided testing found and the product fix
resolved `TC-PROTOCOL-004`, its minimized raw and typed inputs were added to
this corpus. The explicit manifest now contains 16 files, exact raw and typed
roundtrip remains unconditional, and the renewed 50-run stress passed 800/800
file replays. Full resolution evidence is in
[`outstanding-defect-resolution-20260730.md`](outstanding-defect-resolution-20260730.md).

The implementation deliberately uses the existing `proptest` dependency and
ordinary `cargo test`. No separate runner or networked test environment is
required. The existing `nightly-deep` job already sets
`PROPTEST_CASES=2048` for locked, all-feature workspace tests, so this suite is
automatically included without another workflow.

## Independent oracle

The test treats `sorotte-protocol` as a public black box. For syntactically
valid top-level objects, a serde streaming `MapAccess` visitor records keys in
source order while consuming values as `IgnoredAny`. This does not call or
copy the production codec's handwritten order scanner.

The oracle checks that:

1. the line-item decoder accepts exactly the inputs accepted by raw JSON
   decoding;
2. every valid input yields at least one diagnostic item;
3. object items preserve first-occurrence, unique top-level source order;
4. each item carries the surviving JSON value for its command, including
   duplicate keys;
5. aggregate and singular typed decoding succeed exactly when every item
   type-checks;
6. aggregate typed message order matches item order;
7. successful raw and typed decodes are semantically stable after encoding
   and decoding again;
8. non-object JSON remains one diagnostic item with no command.

A panic or assertion failure is an ordinary test failure and is shrunk by
proptest. The generator seed is fixed at `0x5A170FFEC0DE2026`, so a given case
budget has the same inputs on every run.

## Generated input families

Three complementary properties run under the same fail-closed case budget:

- arbitrary byte vectors from 0 through 4,096 bytes, entering the string codec
  only after successful strict UTF-8 validation;
- arbitrary Unicode strings of up to 512 scalar values, driven through all
  four public decoding entrypoints;
- mutations of checked-in corpus entries using bounded insert, replace,
  delete, and truncate edits, with no output above 4,096 bytes.

The default is 512 cases per property, or 1,536 generated cases per test
binary invocation. `PROPTEST_CASES=2048` raises this to 6,144 generated cases.
Zero, malformed, and negative-looking budgets fail the test rather than
silently reducing it; excessive positive values are capped at 100,000.

## Checked-in corpus

The explicit corpus manifest contains 16 UTF-8 files:

| Expected boundary | Files | Representative shapes |
|---|---:|---|
| fully typed protocol | 6 | escaped command key, composite order, duplicate `Set`, nested structural lookalikes, Unicode, surrounding whitespace, exact float roundtrip |
| valid JSON with a typed rejection allowed | 6 | unknown command after a valid command, empty object, array, scalar null, duplicate unknown command, exact float roundtrip |
| invalid JSON | 4 | truncated object, malformed escape, trailing token, unclosed nested array |

Replay rejects an absent corpus, missing or unexpected entries, non-UTF-8
filenames, symlinks, non-regular files, unreadable inputs, and inputs above the
production 64 KiB line constant. Valid corpus JSON is additionally replayed
with LF, CRLF, and leading/trailing JSON whitespace to pin the line-framing
contract.

New minimized parser failures should be added as a named corpus entry with an
explicit expected boundary. The manifest makes an accidental corpus deletion
or unreviewed addition fail visibly.

## Executed experiments

### Default deterministic run

```text
cargo test --locked -p sorotte-protocol \
  --test protocol_parser_robustness -- --nocapture
```

Result:

- 6/6 ordinary integration tests passed;
- 1,536 fixed-seed generated cases passed;
- 16/16 checked-in corpus entries replayed;
- elapsed test time: 0.27 seconds.

### Scheduled-depth deterministic run

```text
PROPTEST_CASES=2048 cargo test --locked -p sorotte-protocol \
  --test protocol_parser_robustness -- --nocapture
```

Result:

- 6/6 ordinary integration tests passed;
- 6,144 fixed-seed generated cases passed;
- elapsed test time: 1.08 seconds.

### Corpus stability stress

The exact corpus replay test was executed 50 consecutive times.

Result:

- 50/50 invocations passed;
- 800/800 individual corpus-file replays passed.

### Owning crate and lint gates

```text
cargo test --locked -p sorotte-protocol --all-features
cargo clippy --locked -p sorotte-protocol \
  --all-targets --all-features -- -D warnings
rustfmt --edition 2024 --check \
  crates/sorotte-protocol/tests/protocol_parser_robustness.rs
```

Result:

- 88/88 protocol unit tests passed;
- 6/6 protocol parser integration tests passed;
- documentation test target passed with zero tests;
- all-target, all-feature Clippy passed with warnings denied;
- scoped formatting check passed.

## Boundaries of the proof

This slice covers deterministic parser robustness at the public
`sorotte-protocol` API. It does not model socket scheduling, transport
fragmentation, or concurrent session state. Random vectors are bounded to
4,096 bytes for fast ordinary CI; the existing protocol suite separately pins
behavior at and beyond the 64 KiB line constant and the JSON recursion limit.

The result is a reproducible, low-latency parser guard that runs in every
ordinary workspace test and at higher depth in the existing scheduled job.
