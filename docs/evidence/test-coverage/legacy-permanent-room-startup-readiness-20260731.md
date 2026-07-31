# Legacy permanent-room startup readiness

Date: 2026-07-31

Finding: `TC-HARNESS-021`

Scope: live Syncplay v1.7.5 permanent-room compatibility oracle

## Hosted RED

Workflow run
[`30610965479`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30610965479),
Ubuntu job
[`91093403065`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30610965479/job/91093403065),
failed the strict live legacy compatibility step in:

```text
legacy_server_fanout_roundtrip_matches_server_runtime_on_permanent_rooms_file_scenario
```

The failure was the step-zero connect-time snapshot. In the assertion,
`left` was the normalized live legacy output and `right` was the Sorotte
runtime output:

```text
left:  {"Set":{"playlistIndex":{}}}
right: {"Set":{"playlistIndex":{"index":0}}}
```

The empty normalized object represents the legacy server's null
`playlistIndex`; the shared cross-implementation normalizer removes null
members. The committed legacy trace and the Sorotte runtime both use index
zero for a newly configured permanent room.

## Root cause

This was an oracle-harness startup race, not a Sorotte server parity defect.
The pinned Syncplay v1.7.5 `RoomDBManager.connect` starts Twisted `adbapi`
schema creation and room loading asynchronously. The server begins accepting
TCP connections without awaiting the `loadRooms` callback. On the Ubuntu
runner, the first scenario `Hello` entered `permanent-room` before that
callback had installed the configured room with its seeded
`playlistIndex=0`. The legacy server therefore created a transient ordinary
room whose default index is null. Local Windows scheduling happened to finish
the callback first, which explained both the local pass and the committed
index-zero trace.

## Correction

The live legacy runner now establishes a causal readiness barrier whenever a
scenario configures permanent rooms:

1. connect a GUI probe in a collision-safe room ending in `-temp`;
2. poll the public legacy `List` response;
3. proceed only after every configured permanent-room key is observable;
4. half-close the probe and wait for peer EOF, proving the server processed
   its disconnect before any scenario client connects; and
5. fail closed if either room readiness or probe cleanup does not complete
   within the existing six-second startup bound.

The barrier observes the protocol state the scenario depends on. It does not
add a timing sleep, alter the reference checkout, canonicalize null to zero,
weaken the parity assertion, or change Sorotte product behavior. A focused
unit regression rejects malformed, non-`List`, and partial snapshots and
accepts only a snapshot containing every expected permanent room.

## Validation

Passed locally against pinned oracle commit
`d1c5f85af377c960c5a940707c4d01bc84fd9c3f`:

```text
cargo test -p sorotte-compat `
  permanent_room_startup_requires_a_gui_list_snapshot_with_every_room `
  -- --nocapture
```

Result: 1 passed, 0 failed.

```text
$env:SYNCPLAY_LEGACY_ROOT=(Resolve-Path '.interop-cache/syncplay-legacy').Path
$env:SYNCPLAY_REQUIRE_LIVE_INTEROP='1'
$env:SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY='1'
cargo test -p sorotte-compat --all-features `
  legacy_server_fanout_roundtrip_matches_server_runtime_on_permanent_rooms_file_scenario `
  -- --nocapture
```

Result: 1 passed, 0 failed.

```text
cargo test -p sorotte-compat
cargo test -p sorotte-compat --all-features legacy_server_ -- --nocapture
cargo clippy -p sorotte-compat --all-targets --all-features -- -D warnings
rustfmt --edition 2024 --check `
  crates/sorotte-compat/src/lib.rs `
  crates/sorotte-compat/src/legacy_process.rs `
  crates/sorotte-compat/src/legacy_server.rs
git diff --check -- `
  crates/sorotte-compat/src/lib.rs `
  crates/sorotte-compat/src/legacy_process.rs `
  crates/sorotte-compat/src/legacy_server.rs
rg -n " +$" `
  docs/evidence/test-coverage/legacy-permanent-room-startup-readiness-20260731.md
```

Results: 138/138 default compatibility tests passed; 20/20 strict live legacy
tests passed; strict Clippy and formatting passed; the tracked diff check was
clean and the evidence trailing-whitespace search returned no matches.
