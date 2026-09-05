# Server framing, capacity, and time

## Framing

Limits count UTF-8 bytes of the encoded JSON object. The terminating LF or CRLF
is excluded from the line limit and included in queued-byte accounting. Rust
server ingress and 0.2.9 GUI/CLI readers accept at most 512 KiB. The generic
protocol decoder retains its existing 64 KiB default for API compatibility;
transport owners select the shared explicit transport constant.

| Recipient | Maximum outbound JSON line |
|---|---:|
| Advertises `sorotteLargeProtocolFramesV1` | 512 KiB |
| Earlier Rust readiness, barrier, participant-status, or Media Match extension | 64 KiB |
| Legacy/no recognized framing capability | 16 KiB |

The legacy limit matches the pinned Python LineReceiver contract. Advertising
the new capability is a promise to accept the larger frame; the server echoes
support in Hello. Unknown metadata does not imply support.

List remains a complete replacement of the visible roster. Optional Media Match
discovery signatures may be omitted when their aggregate would exceed the
recipient limit or transient fanout budget. Member identities, ordinary file
metadata, and unknown extensions are retained. Empty permanent rooms use unique
short whitespace identities accepted by legacy readers, avoiding quadratic
padding. Room isolation and recipient capabilities determine each view.

Hello, reconnect, file/room changes, controller authentication (including room
creation and generated-name responses), capability changes, playlist changes, and
visibility/permanent-room configuration are preflighted before committing
unreadable shared state. Multi-field Set messages preserve wire order while
preflighting the whole batch. Unrepresentable growth returns a redacted protocol
error. No unnegotiated List chunking or silent roster truncation occurs. The
fallible `try_set_isolate_rooms` and `try_set_permanent_rooms` APIs preserve prior
configuration on rejection; their legacy void setters panic on invalid operator
configuration. Configure resources before spawning the actor.

Closed readiness/barrier schemas reserve space for future participant state,
maximally escaped identifiers, and retained disconnected participants. This
conservative bound can reject room growth before the current minimal snapshot
fills the line. Transient roster, playlist, and coordination fanout are each
bounded against a quarter of the configured global queue-byte budget, leaving
room for transition messages. A final encoding check defends every output path.

## Network resources

Set these positive integers in the server process environment before startup:

| Variable | Default |
|---|---:|
| `SOROTTE_SERVER_MAX_CONNECTIONS` | 1024 |
| `SOROTTE_SERVER_MAX_UNAUTHENTICATED_CONNECTIONS` | 128 |
| `SOROTTE_SERVER_MAX_CONNECTIONS_PER_ADDRESS` | 128 |
| `SOROTTE_SERVER_MAX_QUEUED_BYTES_PER_PEER` | 4194304 |
| `SOROTTE_SERVER_MAX_QUEUED_BYTES_TOTAL` | 67108864 |

Byte limits require 1024 <= per-peer <= total. Invalid values fail startup.
Outbound admission also caps every recipient's JSON line at the per-peer byte
ceiling minus two CRLF bytes. A small queue setting can reject a capability or
room before its framing limit is reached. Changing limits on a populated runtime
rejects incompatible settings and preserves the previous limits and room state.
Admission precedes TLS/Hello workers. An authenticated connection releases its
unauthenticated permit but retains its total/address permit until socket cleanup.
IPv4 and mapped IPv6 share an address bucket; empty buckets are removed. The
per-address default allows multiple clients behind NAT; operators can adjust it.

Reliable messages preserve order. Only eligible periodic state may coalesce;
replacement accounts for its byte-size delta and retains the prior state if
growth cannot be reserved. A queued/in-flight write owns its bytes until the
write completes, fails, or is cancelled. Receiver drop, disconnect, and unwinding
release permits. `ServerRuntimeActorHandle::resource_snapshot()` reports active,
unauthenticated, address, rejected, current queued, and peak queued counts.
Limits count owned encoded payload bytes, not total allocator overhead or OS
socket buffers. Scaling measurements report those distinct observations.

## Clocks and ping estimates

Local elapsed time is nondecreasing and based on `Instant`. It drives reconnect
TTL and maintenance pruning, pending readiness transport evidence, protocol
liveness, periodic scheduling, buffering freshness/debounce/hysteresis, and
barrier prepare/started expiry. Wall-clock corrections cannot revive an expired
membership or move those deadlines backwards. The bounded reconnect cache holds
at most 4096 detached memberships and expires them at the exact 180-second edge.

Wire timestamps, cross-process playback anchors, persisted room activity, and
statistics retain their existing wall-clock meaning. Wire deadlines are formed
from the wall clock plus remaining elapsed duration. Participant-status age also
retains its conservative nondecreasing-age guard. These are separate contracts;
this change does not reinterpret every protocol number as monotonic time.
`set_clock_overrides_seconds` lets tests vary wall and elapsed clocks independently.
Elapsed suspend behavior follows Rust `Instant` on the host OS; no cross-platform
promise that suspended time is included is made. On resume, normal liveness and
fresh-report checks still apply.

Each connection retains at most 64 outstanding server-issued ping identities.
An echo must match exactly, be at most 90 elapsed seconds old, and is consumed
once. Negative/nonfinite/unissued/retired/replayed echoes and invalid client RTTs
do not change the timing estimate. RTT is measured from the retained local send
instant, never calculated by subtracting a supplied timestamp. Estimated forward
delay is bounded to 90 seconds and expires without a fresh accepted sample. This
keeps normal legacy echo behavior while rejecting fabricated timing authority.

## Persistence shutdown

`shutdown()` uses one five-second persistence budget;
`shutdown_with_timeout(Duration)` selects a bounded budget (minimum 100 ms).
Room and statistics workers share the deadline across flush acknowledgement,
SQLite busy retries, transactions, and joining. Blocking waits execute outside
the async actor runtime. Wake coalescing retains the latest desired state.
The terminal deadline is installed before actor queue admission and shortens
already queued or running Flush barriers. A flush cannot clear or extend it.
The caller's budget also covers sending Shutdown and joining the actor. If that
budget expires, an observable cleanup task retains the actor and worker owners
until they finish; timeout is always a failure.

Durable completion returns success. Contention that exhausts the flush budget
returns a durability failure even if workers subsequently join; it is never a
successful save. The held-lock regression keeps `BEGIN IMMEDIATE` held through
shutdown, checks unrelated async progress, then reopens old-or-new complete rows.
Drop does not repeat work after explicit shutdown consumes the worker handles.

Arbitrary uninterruptible filesystem calls cannot be cancelled by a thread
abstraction. An exceptional worker that misses joining remains owned in an
observable registry. `persistence_workers_awaiting_join()` reaps completed handles
and reports unresolved workers and actor cleanup owners; such an outcome is a lifecycle fault and is
not attested as durable shutdown.

## Executable proof

Server `frame_capacity_tests`, `ping_timing_tests`, readiness clock-boundary
tests, resource unit/loopback tests, and held-lock persistence subprocesses own
these contracts. GUI transport and CLI reader tests consume actual server-built
large rosters. The compatibility suite independently exercises the pinned Python
peer. See the current architecture index and 0.2.9 implementation ledger for
candidate identity, commands, and executed platform evidence.
