# Plex part-selection and retry characterization evidence

Date: 2026-07-30

Branch: `codex/test-coverage-design`

Starting commit: `e114e3afdfa4ce6417169aad4ccc88dd39f231bb`

Platform: Windows, Rust 1.97.1 workspace toolchain

## Objective

Determine whether a successful Plex metadata match can still fail automatic
playback when the item has multiple playable parts, and whether that failure
is retried as if it were a transient network miss. This slice changes no
production behavior. It retains both results as exact expected-failure
characterizations.

## Findings

The reported symptom contains two independent defects:

1. `TC-PLEX-001` — playable-part selection ignores available filename and
   size evidence and compares candidates only by duration.
2. `TC-GUI-003` — a deterministic ambiguity is collapsed to an error string,
   recorded as a transient miss, retried, and announced repeatedly.

Plex search, cache lookup, metadata retrieval, and authorization can all
succeed before either defect occurs.

## TC-PLEX-001 experiment

The test drives the production `PlexMediaResolver::resolve_stream_target`
path with a fake transport. It observes the part passed to the production
stream-URL builder, rather than calling `choose_playable_part` directly.

An independent test-only model narrows candidates by:

```text
exact-case basename
ASCII-folded basename
exact byte size
smallest duration difference
```

The model has its own basename parsing and ASCII folding and does not call the
production path/normalization or selection helpers.

Ten source cases are each run with candidates in forward and reverse order:

- plain shared filename after successful metadata search;
- equal-duration multiple versions;
- optimized/original copies separated by exact size;
- missing durations with an exact filename;
- path basename and ASCII-case normalization;
- exact-case filename versus a folded alias;
- explicitly named multipart part;
- exact filename versus a closer-duration wrong version;
- exact filename versus a conflicting exact-size wrong version; and
- exact size versus a closer-duration wrong version.

All 20 production observations disagree with the independent oracle:

- 16 return `contains ambiguous playable parts` despite one evidence-selected
  candidate; and
- 4 select the duration-nearest wrong part even though filename or size is
  stronger evidence.

Two positive controls retain fail-closed behavior for candidates genuinely
indistinguishable under all available hints and for unidentified multipart
media. A third proves duration remains a valid final discriminator after
filename and size tie.

Exact characterization:

```text
tests::part_selection_adversarial::known_defect_resolver_ignores_filename_and_size_evidence
TC-PLEX-001: Plex part selection must use filename and size evidence
```

## TC-GUI-003 experiment

The GUI test drives
`sync_selected_shared_playlist_media_to_attached_player_impl`, the production
Plex miss state, retry-due decision, stream-result consumption, and feedback
queue. It supplies an already completed worker result whose message is
constructed through the production `PlexError::InvalidResponse` formatting
boundary, then advances the existing miss deadline without sleeping.

For one unchanged row, playlist generation, target, policy, Plex operation
context, and trigger key, current behavior is:

```text
resolution attempts = 2
warning notifications = 2
system-chat announcements = 2
repeated message bytes = identical
```

Exact characterization:

```text
app::runtime_owner::tests::player_runtime_tests::plex_ambiguity_retry::known_defect_permanent_plex_ambiguity_retries_and_repeats_warning
TC-GUI-003: permanent Plex ambiguity must warn once without automatic retry
```

The GUI fixture deliberately injects the completed worker value instead of
calling a live Plex server. The owning GUI state machine is real; HTTP,
authentication, and server metadata parsing are outside this specific retry
oracle and already precede the resolver boundary.

## Stress evidence

The Plex matrix passed 50/50 complete repetitions, running 150 Rust tests:

- 1,000 expected production/oracle mismatches from the 20-case
  `TC-PLEX-001` matrix;
- 100 fail-closed positive-control cases;
- 50 duration-final-discriminator cases.

That is 1,150 part-selection evidence cases. Separately, the GUI
characterization passed 50/50 repetitions: 50 tests, 100 retry attempts, and
200 feedback actions across warning and system-chat delivery. No case depends
on Plex response order, wall-clock sleeps, a network service, or filesystem
timestamp movement.

## Validation

```text
cargo test -p sorotte-plex --all-features --quiet
  68 passed, 0 failed, 0 ignored; 0.457s

cargo test -p sorotte-gui --all-features --quiet
  1,191 passed, 0 failed, 2 ignored; 64.199s
```

Warning-denied all-target/all-feature Clippy passed for both owning crates.
Workspace formatting, `git diff --check`, and the exact 7/7 known-defect
registry policy also passed.

## Registry and solution

`coverage/known-defects.toml` contains exactly one characterization for each
new ID, with an expiry of 2026-09-30. The current registry validates as seven
defects and seven exact characterizations.

The implementation-ready solution—including filename/size/duration priority,
typed terminal ambiguity, rearm rules, alternatives, and positive acceptance
tests—is in `docs/OUTSTANDING_DEFECT_REMEDIATION_DESIGN.md`.

## Boundary limitations

- This is deterministic resolver/state-machine proof, not a capture from the
  reporter's Plex server.
- It does not assert that all Plex `Media`/`Part` layouts can be selected
  automatically. Genuine ties continue to require fail-closed behavior or a
  future explicit version picker.
- It does not classify every Plex error. The proposed production change makes
  ambiguous parts terminal for the same context while preserving the existing
  backoff for ordinary misses and transport failures.
- Expected-failure tests prove the defects exist; they are not green product
  behavior. Both must become ordinary positive tests when the fixes land.
