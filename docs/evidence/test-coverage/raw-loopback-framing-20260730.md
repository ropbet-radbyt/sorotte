# Raw loopback protocol framing evidence (2026-07-30)

## Scope

This slice exercises raw protocol bytes across real IPv4 loopback TCP sockets at both production
session boundaries:

- server receive path through
  `run_server_network_client_session_with_pre_hello_timeout`;
- CLI receive path through the full connected-session runner used by the existing test boundary
  (plaintext is selected deliberately so this matrix isolates protocol framing from STARTTLS).

No public interface is bound or contacted. Every accept, read, write, task join, and half-close
outcome is bounded by a three-second timeout. Tests use explicit channels, state queries, and
transport completion as barriers; there are no timing sleeps.

## Matrix and mechanical oracles

| Boundary | Raw byte history | Oracle |
| --- | --- | --- |
| Server | complete Hello written one application byte at a time | production session registers the exact username and emits Hello |
| Server | Hello payload, then `\r`, then `\n` | no actor session exists before `\n`; Hello commits after it |
| Server | Hello and List frames in one TCP write | Hello response precedes List response |
| Server | valid Hello followed in the same write by malformed JSON | valid prefix commits, typed legacy Error follows, connection closes |
| Server | valid Hello followed by invalid UTF-8 | valid prefix commits, decode Error follows, session ends with `InvalidData` |
| Server | valid Hello followed by a line over the server limit | valid prefix commits, limit Error follows, session ends with `InvalidData` |
| Server | malformed unterminated JSON followed by write half-close | final bytes are decoded and rejected before EOF |
| Server | valid unterminated Hello followed by write half-close | final frame commits before orderly EOF |
| Server | each faulty peer beside a healthy sentinel | faulty actor session is removed while the sentinel still answers List |
| CLI | Hello and Set-ready frames coalesced into one TCP write | assigned identity and ready state both commit in wire order |
| CLI | valid Hello followed by malformed JSON | Hello state remains committed and a typed `ProtocolError::InvalidJson` is returned |
| CLI | valid Hello followed by invalid UTF-8 | Hello state remains committed and `FromUtf8Error` remains downcastable |
| CLI | valid Hello followed by a line over the client limit | Hello state remains committed and the framing-limit error is returned |
| CLI | malformed unterminated JSON followed by server write half-close | EOF causes a typed JSON error and cannot activate the session |
| CLI | valid unterminated Hello followed by server write half-close | final frame commits and the runner returns `TransportClosed` |
| CLI | one-byte continuation after an observed partial read is cancelled | deterministic expected-panic characterization of TC-CLI-003 |
| CLI | line feed released after an observed payload-plus-CR read is cancelled | independent deterministic expected-panic characterization of TC-CLI-003 |

The loopback server keeps its receive half open after half-closing its write half and drains it
until the client closes. This is required on Windows: dropping a socket with unread peer bytes can
replace the intended orderly EOF with `WSAECONNABORTED`, which would test teardown behavior rather
than framing.

## Surfaced defect: TC-CLI-003

**Proposed title:** Connected-session select cancellation drops fragmented inbound protocol
prefixes.

**Exact characterization panic prefix:**

```text
TC-CLI-003: fragmented inbound protocol read lost bytes before the CRLF delimiter
```

### Observed behavior

A real loopback server writing a valid Hello one byte at a time produced nondeterministic JSON
locations such as `expected ident at line 1 column 2`, `trailing characters at line 1 column 5`,
and `expected value at line 1 column 1`. An initially positive split-CRLF test also failed on
stress iteration 12 when the client decoded an empty suffix (`EOF while parsing a value at line 1
column 0`).

### Root-cause proof

`read_inbound_protocol_line` accumulates bytes in a future-local `Vec` and consumes those bytes
from `BufReader`. The connected-session loop constructs that future directly inside its outer
`tokio::select!`. If an autoplay, player-input, local-input, or other ready branch wins before the
delimiter arrives, the read future is dropped. Its local buffer disappears even though the bytes
have already been consumed from the transport. The next read starts at the remaining suffix.

A `cfg(test)` task-local observation seam records only two facts without changing production
behavior:

1. a partial line has been consumed from the buffered socket;
2. that read future was dropped before completing a frame.

The reproducer gates the server after the prefix, waits for fact 1, closes a supplied local-input
channel to make a competing branch ready, waits for fact 2, and only then releases the remaining
bytes. One case releases the remaining frame one application byte at a time. The other places the
gate between `\r` and `\n`. Both are ordinary, non-ignored `#[should_panic]` tests with the exact
prefix above. A cancellation-safe implementation will stop panicking, which makes both
characterizations fail until their expected-panic annotations are removed or inverted.

No product fix is included in this slice.

## Files

- `crates/sorotte-server/src/tests/raw_protocol_framing_tests.rs`
- `crates/sorotte-server/src/tests.rs`
- `crates/sorotte-cli/src/tests/raw_protocol_framing.rs`
- `crates/sorotte-cli/src/tests.rs`
- `crates/sorotte-cli/src/protocol_io.rs` (`cfg(test)` observation only)

## Validation

Focused matrices:

```text
cargo test -p sorotte-server raw_protocol_framing_tests -- --nocapture
  3 passed; 0 failed

cargo test -p sorotte-cli raw_protocol_framing -- --nocapture --test-threads=1
  5 passed; 0 failed; 0 ignored
  (includes two expected-panic TC-CLI-003 characterizations)
```

Deterministic stress:

```text
server raw framing selector, serial, 50 iterations: 50/50 passed
CLI raw framing selector, serial, 50 iterations: 50/50 passed
```

Owning-crate gates:

```text
cargo test -p sorotte-cli --all-features
  lib: 356 passed; 0 failed; 8 ignored
  app_boundary_consumer: 2 passed; 0 failed

cargo test -p sorotte-server --all-features
  lib: 355 passed; 0 failed
  binary unit tests: 14 passed; 0 failed
  server_binary: 2 passed; 0 failed
  server_release_verify: 6 passed; 0 failed

cargo clippy -p sorotte-cli --all-targets --all-features -- -D warnings
  passed

cargo clippy -p sorotte-server --all-targets --all-features -- -D warnings
  passed

cargo fmt --all --check
  passed
```
