# Outstanding defect remediation design

Date: 2026-07-30

Branch: `codex/test-coverage-design`

Status: implementation-ready proposal; this slice adds characterization
coverage but intentionally does not change production behavior

## Decision summary

The current defect set contains seven independently testable contracts. The
reported Plex symptom is split into two defects because candidate selection
and retry/notification policy have different owners and failure modes.

| Defect | Recommended repair | Risk | Alternative |
|---|---|---:|---|
| `TC-CLIENT-002` | Clear both connection-scoped playback transactions and reset their health during reconnect reset | Low | Add generation IDs to every effect and completion |
| `TC-PROTOCOL-002` | Derive nested `Set` order from the last surviving top-level `Set` occurrence | Low | Reject duplicate JSON command members |
| `TC-PROTOCOL-003` | Render only exact known command names through `Debug`; replace unknown names with a fixed marker | Low | Store all untrusted identifiers in a redacted wrapper |
| `TC-GUI-001` | Strictly validate a tool-specific UTF-8 version banner | Low | Run a generated-media capability probe |
| `TC-GUI-002` | Drain both child pipes concurrently into bounded captures while supervising the process | Medium | Move probes to a shared async process runtime |
| `TC-PLEX-001` | Filter playable parts lexicographically by filename, size, then duration and fail only on a remaining tie | Medium | Add an explicit Plex version/part picker |
| `TC-GUI-003` | Carry typed retryability from Plex and make ambiguity terminal for the current resolution context | Medium | Deduplicate notifications while continuing retries |

The recommended repairs are compatible with valid existing traffic and saved
configuration. They should be implemented as four focused commits: client and
protocol state correctness; bounded media-tool probing; Plex part selection;
and GUI Plex failure classification.

## Shared implementation rules

1. Do not infer behavior from rendered error strings. Retryability, redaction,
   and candidate evidence must remain typed through their owning boundary.
2. Preserve established compatibility semantics unless the input is unsafe.
   In particular, duplicate protocol commands retain first-position,
   last-value semantics.
3. A fallback may select a Plex part only when the available evidence leaves
   one candidate. Stable vector order, rating-key order, or Plex response
   order is not evidence.
4. A terminal result is terminal only for its exact resolution context. A row,
   playlist generation, target, policy, server identity, credential, or
   explicit user retry change must rearm resolution.
5. Each fix removes its expected-failure registry entry in the same commit
   that converts the characterization into an ordinary positive regression.

## TC-CLIENT-002: reconnect transaction invalidation

### Contract

After `reset_sync_state_for_reconnect`, no completion issued for the previous
connection may mutate the new connection's local position, pause state,
readiness, or health projection.

### Recommended implementation

Add one `ClientModel` method owned by `model.rs`, for example
`cancel_connection_scoped_playback_transactions`:

```text
pending_local_pause_change = None
pending_room_pause_sync = None
local_pause_change_health = Healthy
```

Call it from `reset_sync_state_for_reconnect_with_attempt` before the fresh
connection projection is built. Do not emit rollback or compensating effects:
the old transport is gone, the current player state will be observed again,
and the new server snapshot is authoritative.

This is preferable to reaching into private transaction fields from the
session module. It gives every future disconnect path one named invariant and
keeps transaction ownership in `ClientModel`.

### Required proof

- Convert
  `known_defect_reconnect_reset_rejects_stale_reducer_completions` to a
  positive test.
- Remove the three defect normalizations from the complete 24-seed fresh-model
  comparison so it compares the entire projection exactly.
- Inject both success and failure completions for every old transaction stage
  after reset and require no state change and no follow-up effect.
- Start a new transaction after reset and prove that it still completes.
- Keep the complete reset idempotence test.

### Alternative and decision

Generation-tagged effect IDs are stronger if player/control effects become
asynchronous and can overlap a newly armed identical command. Current
`run_model_event` execution is synchronous, so generation plumbing through
every `ClientEffect` and completion is disproportionate today. Adopt it only
when effect execution becomes asynchronous or independently queued.

## TC-PROTOCOL-002: surviving duplicate `Set` order

### Contract

For duplicate JSON command members, Sorotte's established behavior is:

```text
execution position = first decoded-key occurrence
typed value         = last decoded-key occurrence
metadata for value  = same last occurrence as the typed value
```

The final line is the missing invariant.

### Recommended implementation

Replace the first-match behavior of `top_level_object_value_span` with an
explicit last-match helper. `set_command_order` must scan the object belonging
to the last decoded top-level key equal to `Set`, including escaped spellings
such as `"\u0053et"`. Top-level command ordering remains deduplicated at the
first source position.

The scanner must continue to enforce its current structural boundaries:

- ignore `Set` text inside strings;
- ignore nested shadow objects;
- decode escaped keys before comparing;
- derive order only from direct members of the surviving `Set` object; and
- deduplicate nested command names at their first position within that
  surviving object while retaining serde's last nested value.

### Required proof

- Convert the exact characterization to a positive test.
- Cover both duplicate orders, escaped/unescaped combinations, whitespace,
  a discarded scalar or malformed typed payload followed by a valid `Set`,
  nested shadow `Set` keys, and three or more duplicates.
- Extend the generated duplicate-composite oracle to compare each surviving
  typed `Set` field with the order ledger from the same source occurrence.

### Alternative and decision

A custom serde visitor that rejects all duplicate members is simpler to
explain, but changes accepted wire behavior and may break Python Syncplay or
third-party peers. It should be a future protocol-version decision, not this
repair.

## TC-PROTOCOL-003: unknown command diagnostic redaction

### Contract

No untrusted unknown command name may appear in ordinary `Debug`, `Display`,
error, tracing, or panic output. Exact built-in protocol names are safe fixed
literals and may remain useful in diagnostics.

### Recommended implementation

Introduce one central exact command classifier for:

```text
Hello, Set, List, State, Chat, Error, TLS
```

Wrap `DecodedMessageLineItem.command` only while formatting. The public field
retains the original string for callers, but its custom `Debug` representation
is:

```text
known command   -> exact known literal
unknown command -> "<unknown-protocol-command>"
no command      -> None
```

Do not include a prefix, suffix, hash, or byte length from the unknown value;
none is necessary for diagnosis and all create avoidable reflection channels.
Keep the already-redacted payload and typed error formatting unchanged.

### Required proof

- Convert the credential-canary characterization to a positive test.
- Exercise query strings, bearer/token syntax, control characters, Unicode,
  very long names, and escaped JSON spellings.
- Assert that all seven exact known names remain visible.
- Format the item through direct `Debug`, nested collections, and a tracing-
  style wrapper to ensure the custom boundary is not bypassed.

### Alternative and decision

A redacted-string field type would make accidental direct formatting harder,
but changes a public data shape and complicates consumers that legitimately
inspect unknown extensions. The diagnostic-only wrapper closes the proven
boundary with less churn.

## TC-GUI-001 and TC-GUI-002: one bounded version-probe runner

These defects should be fixed together. Validating output without fixing pipe
drain order leaves a false timeout; draining without validation still blesses
the wrong executable.

### Process supervision contract

Immediately after spawn:

1. take child stdout and stderr;
2. start one drain worker for each pipe;
3. retain at most 64 KiB per stream while continuing to discard/drain excess;
4. record whether either capture was truncated;
5. poll child liveness until exit or the existing deadline;
6. on timeout or wait error, kill and reap before returning;
7. join both drain workers on every path; and
8. return status, bounded bytes, and truncation flags.

Use a private `ProbeOutputCapture` rather than `std::process::Output` so
truncation is explicit. A drain read error or worker panic is a probe failure,
but process reap still happens first. No captured tool text should be included
verbatim in an error.

This remains a small standard-library runner. Adding Tokio solely for two
startup probes is not justified.

### Version identity contract

Pass the expected `MediaMatchTool` into version parsing instead of inferring it
from a path. For the first nonempty stdout line:

- require strict UTF-8;
- reject an empty line set;
- cap the accepted line length;
- require the exact anchored prefix `ffmpeg version ` or
  `ffprobe version ` as appropriate; and
- require nonempty text after the prefix.

An unterminated finite final line remains valid if it was not truncated.
Nonzero exit status remains authoritative even if a plausible banner was
printed.

### Required proof

- Convert both characterizations to positive tests.
- Preserve nonzero status 23 and timeout kill/reap/image-release tests.
- Require a 512 KiB finite producer to exit successfully without retaining
  more than the capture limit.
- Add simultaneous large stdout and stderr, infinite output until timeout,
  child close-order permutations, invalid UTF-8 in the first line, a valid
  prefix only on a later line, a wrong-tool banner, and prefix-only output.
- Probe real configured ffmpeg/ffprobe binaries when available in the manual
  capability lane; unit tests continue to use copied test executables.

### Stronger alternative

A generated-media capability probe verifies more than identity: ffprobe must
parse a fixture and ffmpeg must decode it. That is appropriate for install
verification and CI, but is too expensive for every GUI health refresh.

## TC-PLEX-001: evidence-ranked playable-part selection

### Confirmed failure

The characterization exercises candidate permutations rather than depending
on Plex response order. Across 20 deterministic cases, current production
behavior is wrong in every case:

- 16 return ambiguity even though filename or size uniquely identifies a
  candidate; and
- 4 select the wrong part because nearest duration overrides stronger exact
  filename/size evidence.

The complete test schedule and stress counts are retained in
[`plex-part-selection-retry-20260730.md`](evidence/test-coverage/plex-part-selection-retry-20260730.md).

The metadata item match itself succeeds, so this is not a connectivity,
authorization, or search-cache defect.

### Selection contract

Create an internal `PlexPartSelectionHint` containing:

```text
file_name:      optional basename
size_bytes:     optional exact byte count
duration_ms:    optional duration
```

Selection is a sequence of narrowing stages:

1. Start with every part having a nonempty stream key.
2. If any candidate exactly matches the hinted basename, retain only exact
   matches.
3. Otherwise, if any candidate has an ASCII-case-folded basename match,
   retain all folded matches. Exact case always outranks folded case.
4. If more than one remains and any candidate exactly matches the hinted byte
   size, retain only exact-size matches.
5. If more than one remains and at least one has a known duration, retain only
   candidates with the smallest absolute difference from the duration hint.
6. Return the part only when one candidate remains. Otherwise return a typed
   ambiguity carrying the rating key and remaining candidate count.

If a stage has usable hint data but no candidate matches it, leave the current
set unchanged and continue. This preserves fallback for renamed/copied media
without treating a mismatch as positive evidence. Missing candidate duration
cannot beat a known duration, and response order is never a tie-breaker.

### Hint propagation

- A direct `plex://` target supplies filename, size, and duration from
  `PlexPlaylistUri`.
- A plain shared filename supplies its basename before metadata lookup; any
  available `LocalFileUpdate` size and duration supplement it.
- A locally published Plex URI records the selected part's filename, size, and
  duration so another peer repeats the same evidence decision.
- A manual rating-key selection with no part evidence continues to fail closed
  when the item has multiple candidates.

Keep the existing public `playlist_uri_for_metadata` compatibility wrapper,
but route internal file-backed callers through a new hint-aware helper. This
avoids forcing unrelated callers to manufacture evidence.

### Required proof

- Candidate-order reversal and full permutation invariance.
- Multiple Plex versions, optimized copies, duplicate library paths, genuine
  multipart media, equal durations, missing durations, missing sizes, basename
  paths, case-only differences, and duplicate exact metadata.
- Exact filename outranks contradictory duration.
- Exact size breaks a filename tie and outranks contradictory duration.
- Duration is used only as the final discriminator.
- Candidates indistinguishable under all available evidence still return the
  typed ambiguity.
- The chosen part key, playback URL, logical file, and formatted playlist URI
  all refer to the same part.

### Alternative and decision

Some items are genuinely ambiguous because they expose multiple cuts,
resolutions, or multipart layouts with identical evidence. The complete
solution is a version/part picker showing filename, size, duration, container,
resolution, and media attributes, then storing a stable part identity in the
playlist URI. That requires protocol/UI design and is not necessary for the
reported case. Implement evidence ranking first and preserve the picker as the
fallback for genuine ties.

## TC-GUI-003: terminal Plex ambiguity

### Confirmed failure

For one unchanged automatic playlist-resolution key, two deterministic
resolution cycles currently produce:

```text
attempts = 2
warning notifications = 2
system chat announcements = 2
```

Both repeated messages are byte-identical. The 2/5/15/30-second backoff limits
frequency but cannot make a deterministic ambiguity resolve.

### Recommended implementation

Add a distinct `PlexError::AmbiguousPlayableParts` (or an equally typed
resolver outcome). At the GUI worker boundary, map it to
`PermanentForContext`; do not parse `Display` text. Retain existing retryable
classification for a cache miss or transport failure.

Extend the existing Plex miss state with a typed disposition and optional
deadline:

```text
Retryable           -> next_retry_at = Some(backoff deadline)
PermanentForContext -> next_retry_at = None
```

On the first permanent failure:

- consume the worker result;
- retain a terminal failure for the exact `PlexResolutionMissKey`;
- enqueue one redacted warning and one system-chat event;
- project the provider as `Failed`, not `Resolving` or a miss that “will retry
  automatically”; and
- do not start another worker from timer ticks.

Clear the terminal state when the row, playlist generation, target, provider
policy, selected server, relevant credentials, or plugin enablement changes.
An explicit Retry/Force Plex gesture must also clear it. This makes
“permanent” contextual rather than eternal.

The successful fix for `TC-PLEX-001` will make this path rare, but it remains
required for truly indistinguishable parts.

### Required proof

- Convert the retry/warning characterization to a positive test requiring one
  attempt, one warning, and one chat event across every simulated backoff
  deadline.
- Preserve retries at 2, 5, 15, then 30 seconds for retryable `None`,
  network, and worker-disconnect outcomes.
- Prove target, row, generation, server context, and explicit retry changes
  each rearm one attempt.
- Prove stale worker results cannot overwrite a newer terminal or successful
  context.
- Prove a later success clears both retryable and permanent failure state.

### Rejected lean-looking alternative

Notification deduplication alone hides the user-visible spam but continues
network requests, cache work, and failed resolution forever. It also leaves
the UI claiming another automatic retry will occur. Typed terminal state is
only slightly larger and fixes the behavior rather than its presentation.

## Implementation order

1. Land `TC-CLIENT-002`, `TC-PROTOCOL-002`, and `TC-PROTOCOL-003`. They are
   small, isolated state/codec corrections.
2. Land `TC-GUI-001` and `TC-GUI-002` together because they share one process
   boundary.
3. Land `TC-PLEX-001` and convert all part-ranking characterizations.
4. Land `TC-GUI-003` using the typed ambiguity produced by step 3.
5. Run the locked all-feature workspace, warning-denied all-target Clippy,
   Python policy suite, GUI semantic suite, and Windows native smoke.

### Production change map

| Repair | Primary production files |
|---|---|
| reconnect invalidation | `crates/sorotte-client-core/src/model.rs`, `crates/sorotte-client-core/src/session/reconnect.rs` |
| duplicate order and Debug redaction | `crates/sorotte-protocol/src/codec.rs` |
| bounded version probe | `crates/sorotte-gui/src/app/media_match_support.rs` |
| evidence-ranked Plex part selection | `crates/sorotte-plex/src/lib.rs` |
| typed terminal Plex result | `crates/sorotte-plex/src/lib.rs`, `crates/sorotte-gui/src/app/runtime_owner.rs`, `crates/sorotte-gui/src/app/runtime_owner/player/media_search.rs`, `crates/sorotte-gui/src/app/runtime_owner/player/plex_miss.rs`, `crates/sorotte-gui/src/app/runtime_owner/player/resolution_attempt.rs` |

### Merge gates

The focused positive selectors must run before the broad gates so a failure is
owned precisely. The final tree then requires:

```text
cargo fmt --all -- --check
cargo test --locked -p sorotte-client-core
cargo test --locked -p sorotte-protocol
cargo test --locked -p sorotte-plex
cargo test --locked -p sorotte-gui --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
python -m unittest discover -s scripts/tests -p "test_*.py" -v
python scripts/known_defect_policy.py validate \
  --registry coverage/known-defects.toml \
  --catalog coverage/behaviors.toml \
  --repo-root .
```

Run the GUI semantic and Windows native suites after the Plex/GUI changes
because automatic playlist activation and transient notifications cross both
surfaces. Do not accept a retry of a failed first attempt as replacement
evidence.

The final merge must have no expected-failure entry for these seven IDs and no
unexplained change to the existing 23-test ignored registry.
