# Unix socket mpv IPC kernel coverage — 2026-07-31

## Scope and source state

This slice adds the Unix-domain-socket kernel equivalent of the existing Windows
named-pipe mpv IPC fault coverage. The audit began on
`codex/test-coverage-design` at
`c594c290308bf3c0b381a255d75179c7cb177c10`. The implementation remained
uncommitted while this draft was written so the parent test-coverage pass can
review and commit the four slices independently.

Files in this slice:

- `crates/sorotte-player-mpv/src/tests/ipc_unix_socket_fault_tests.rs`
- `crates/sorotte-player-mpv/src/tests.rs`
- `crates/sorotte-player-mpv/src/ipc.rs`

The only implementation seam change widens two pre-existing `#[cfg(test)]`
connection helpers from Windows to Unix-or-Windows. Runtime behavior is
unchanged.

## Safety boundary

- The fixture binds only `std::os::unix::net::UnixListener` paths beneath a
  process-and-counter-owned temporary directory.
- The fixture does not create an IP socket, contact a network target, launch
  mpv, read credentials, persist state, elevate privileges, or inspect another
  process.
- All payloads are synthetic JSON created in the test process.
- Peer reads and writes have one-second bounds. Production commands use
  300-millisecond ordinary deadlines, with a dedicated 70-millisecond timeout
  case.
- RAII cleanup removes the owned socket file and temporary directory. A
  dedicated test also proves that dropping an idle production client closes its
  worker-owned kernel stream and that the fixture leaves neither path behind.

## Covered behavior

Nine Rust tests execute fourteen deterministic kernel schedules:

1. Every response byte is sent in its own socket write, including splits inside
   UTF-8 and JSON tokens.
2. Two events and their response are sent in one coalesced write.
3. Stale, future, and duplicate response IDs fail correlation exactly once and
   terminally fence reuse.
4. Newline-complete malformed JSON, a truncated final frame, and EOF before any
   response fail within the bounded command budget.
5. A peer disconnect before the first request terminally fences the client
   whether the Unix kernel reports the boundary on write or read.
6. A server write-half close preserves the valid event delivered before EOF,
   then fails the still-unanswered command.
7. A withheld response reaches the production Unix stream deadline and emits
   exactly one command failure, timeout, and disconnect sequence.
8. A replacement client reconnects through a freshly rebound socket at the same
   owned path, receives a new logical generation, and restarts request IDs.
9. Request IDs correlate across `u64::MAX`, zero, and one.
10. Dropping an idle client joins the production worker, releases the accepted
    stream, and permits complete fixture path cleanup.

The tests enter through `MpvJsonIpcClient::connect_with_command_timeout`, so the
production `MpvPipeTransport`, Unix read/write deadlines, buffered line reader,
worker thread, response classifier, event queues, connection events, and client
drop path all execute. This is intentionally separate from the in-memory framed
transcript fuzz target.

## Validation

Host and guest toolchains were both Rust `1.97.1`. The Unix execution
environment was Ubuntu WSL2 on
`Linux 6.6.87.2-microsoft-standard-WSL2 x86_64`.

Windows compile:

```text
cargo test --locked -p sorotte-player-mpv --all-features --no-run
```

Result: passed; both the library unit-test executable and
`repro_acknowledged_cache_pause` integration-test executable were built.

Focused Ubuntu execution:

```text
CARGO_TARGET_DIR=/tmp/sorotte-player-mpv-unix-c594c290 \
  cargo test --locked -p sorotte-player-mpv --all-features unix_socket_ -- --nocapture
```

Result: 9 passed, 0 failed, 0 ignored; 410 unit tests and 2 integration tests
were filtered; the focused tests completed in 0.08 seconds.

Complete Ubuntu player crate:

```text
CARGO_TARGET_DIR=/tmp/sorotte-player-mpv-unix-c594c290 \
  cargo test --locked -p sorotte-player-mpv --all-features
```

Result: 418 passed, 0 failed, 1 explicitly opt-in real-mpv test ignored; both
integration regressions then passed. Unit tests completed in 8.81 seconds.

Warning-denied checks:

```text
cargo clippy --locked -p sorotte-player-mpv --all-targets --all-features -- -D warnings

CARGO_TARGET_DIR=/tmp/sorotte-player-mpv-unix-c594c290 \
  cargo clippy --locked -p sorotte-player-mpv --all-targets --all-features -- -D warnings
```

Result: both Windows and Ubuntu checks passed. Scoped `rustfmt --check` and
`git diff --check` also passed.

## Findings and limitations

No product or test-harness defect was found in this slice.

- The kernel campaign ran on Linux under Ubuntu WSL2. It did not execute on
  macOS, BSD, or another Unix implementation.
- It deliberately uses a synthetic mpv peer rather than a real mpv process;
  real-mpv lifecycle coverage remains in the separate opt-in smoke and native
  GUI campaigns.
- Unix stream writes do not promise that one userspace `write_all` call maps to
  exactly one underlying packet. The schedules prove how bytes are presented
  across real socket operations and how the production stream reader observes
  them, not packet boundaries.
- The pre-request disconnect assertion accepts a write-side or read-side kernel
  error because Unix kernels may surface a closed peer at either operation.
- The server owns and unlinks its socket namespace entry. The production client
  is correctly responsible only for closing its stream and worker resources.
