# Live Python semantic playlist progress evidence — 2026-07-29

## Scope

This record closes TC-HARNESS-003. It covers the real Rust GUI runtime owner,
the receipt-owned TCP session transport, the legacy Python server and client,
and the semantic projection used by `live-python-peer-connect-flow`.

## Preserved failure

A complete semantic run reached both peers, room switching, and bidirectional
chat, then failed its first GUI-to-Python playlist assertion:

```text
scenario: live-python-peer-connect-flow
suite result: 13 passed / 1 failed
error:
  invalid python batch response: python live peer timed out waiting for the
  requested playlist state; observed=[]
```

The shell already projected the queued entry, so extending the Python timeout
or replaying the scenario could produce a pass without proving transport
delivery.

## Causal proof

The GUI queue helper first applies
`GuiShellAction::AppendSharedPlaylistEntries`, then enqueues
`GuiRuntimeRequest::QueuePlaylistEntry`. The subsequent GUI projection wait
calls `pump_and_apply` once before checking the already-optimistic shell state.

Production TCP delivery does not transfer an arbitrary batch in that pump.
`GuiSessionRuntimeAdapter::begin_outbound_protocol_delivery` stages at most one
protocol line and retains ownership until the transport reports a matching
receipt. A playlist queue emits a compound playlist-change/index batch. Once
the semantic flow entered the Python probe's blocking `wait_for_playlist`
command, it stopped pumping the GUI owner; the owner could neither acknowledge
the staged receipt nor advance the rest of the batch. The Python peer therefore
continued to report an empty playlist.

This explains both observations that a timing-only theory did not:

- the local shell could already show the expected playlist;
- the same scenario could pass when host scheduling happened to advance the
  receipt before the blocking wait began.

## Repair

`wait_for_peer_observed_playlist` and
`wait_for_peer_observed_playlist_index` now:

1. pump the real `GuiPersistedConfigRuntimeOwner`;
2. request an immediate correlated peer snapshot;
3. evaluate playlist or index state;
4. repeat under the original bounded deadline.

The repair does not add a retry, sleep-based grace period, or test-only
transport. It makes every required production progress engine run while the
oracle is waiting. Timeout errors retain the last observed playlist, index, and
room.

The Python reference peer now advertises `sharedPlaylists: true` and configures
`sharedPlaylistEnabled: true`, matching the behavior under test. A fast source
contract test makes those fixture capabilities mechanically reviewable.

## Verification

All commands were executed from the same worktree and no failed attempt was
converted into a pass.

| Command / boundary | Result |
|---|---:|
| `cargo test -p sorotte-compat live_python_peer_probe_advertises_the_playlist_behavior_it_exercises` | 1/1 |
| exact real-Python GUI chat/playlist regression, ten consecutive processes | 10/10 |
| all `gui_persisted_config_runtime_owner_projects_live_python_peer_` tests | 5/5 |
| focused `live-python-peer-connect-flow`, independent semantic processes | 3/3 |
| complete semantic suite, first post-fix run | 14/14 |
| complete semantic suite, second consecutive post-fix run | 14/14 |

Both full semantic runs produced only their structured success JSON. There was
no STARTTLS warning, unexpected stderr, retry, or skipped scenario.
