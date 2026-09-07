# Persistence and HTTP fixture boundaries

The original main Rust run
[`34141647928`, attempt 1](https://github.com/ropbet-radbyt/sorotte/actions/runs/34141647928)
failed on `25a8c1bbbae273a7d8e5bb845cdecaf2c6c20f29`. Its tree was identical
to the reviewed PR #49 tree. Both failing tests had also executed successfully
before merging; these failures were not newly selected release-only tests.

| Original producer | Observation |
| --- | --- |
| Linux merged coverage, job `101809685326` | `release_verify_persistence_permanent_rooms_and_isolation` did not observe the expected playlist in its eight restart frames. The instrumented workspace exited 101. |
| Windows nextest, job `101805958900` | `disconnect_fault_advertises_full_body_but_closes_early` failed while writing its request with Windows error 10053. Its diagnostic retry passed, and the strict nextest gate correctly retained the failure. |

The main default Cargo suites passed on both platforms. Mutation and fuzzing
also passed. These component results do not qualify the failed Rust run or
authorize publication. GUI-dev stopped at ordinary-check readiness, before
minting an App token or starting its lifecycle and publication jobs. No hosted
retry was used to erase either primary failure.

## Persistence before a hard restart

The fixture acknowledged playlist, index and playback broadcasts, disconnected
its clients, slept 500 ms, and killed the server process. Normal server
transitions enqueue persistence without promising that a protocol response is
a database commit. The fixture's forced termination does not run the graceful
shutdown barrier.

A controlled Linux experiment used the exact main package's server binary,
SHA256 `bc9dbf750aa8b8c18919986474a6f4ced753149866bca884a22e67b9ea7173b1`:

1. Without interference, the database contained the playlist before the kill
   and the restarted server returned it.
2. An owned SQLite write transaction held through the original 500 ms delay
   prevented the commit. All watched broadcasts still arrived. After killing
   the server, the database lacked the row and the restart returned an empty
   playlist.
3. Releasing the same lock and observing the committed row before the same
   hard restart preserved the playlist.

This demonstrates the fixture's invalid timing assumption. It does not prove
the original coverage worker's scheduling or database state: that run retained
neither the database nor its eight received frames. The local experiment used
an uninstrumented release binary and is not hosted coverage evidence.

Both restart fixtures now observe the expected committed playlist and index
through a read-only SQLite connection within one five-second deadline. The
observer does not insert, commit, flush or checkpoint anything. Missing and
incorrect rows remain pending; malformed data and database errors are reported.
Each SELECT releases its read transaction, and SQLite's own busy wait cannot
extend the observation budget. The hard kill, restart, and eight-frame assertion
remain; failures now include the received frames. Real SQLite regressions reject
an existing but uncommitted row, incorrect contents, malformed data and a held
database lock, then verify success after an external commit or lock release.

## Accepted HTTP socket mode

The HTTP fixture used a nonblocking listener but performed synchronous reads on
the accepted socket. On Windows an accepted socket inherits the listener's
properties; setting a receive timeout does not make a nonblocking read wait.
See Microsoft's [accept contract](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-accept)
and [socket timeout contract](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-setsockopt).

A native Winsock probe consumed a partial request, observed immediate error
10035 (`WSAEWOULDBLOCK`), and reproduced client error 10053 after closing that
connection. Explicitly setting the accepted socket to blocking mode instead
waited for the remaining headers. Its two-second read timeout also remained
effective. This reproduces the mechanism; the original hosted run did not log
its server-side read sequence.

The connection worker now explicitly selects blocking mode before its existing
two-second read/write timeouts. The regression forces a nonblocking accepted
socket on every platform, withholds the final request fragment, and checks the
actual response and request record: 64 advertised body bytes, 12 transmitted
bytes, and an intentional early disconnect. Removing only the normalization
statement made this regression fail on Windows; restoring it passed all seven
network fixture tests. No socket-error retry or assertion tolerance was added.

## Diagnostic retention and validation

The coverage producers now upload `server-fixture-failures` beside their
original profile logs. The scheduled coverage upload also runs after a failed
producer. These records explain failures; they cannot satisfy a passing gate.
The original main logs, artifact ZIPs, failed-attempt records and controlled
experiments remain in the operator's verification evidence.

Windows passed the complete nine-test strict server matrix and all seven HTTP
fixture tests. The socket counterfactual failed as expected. Linux passed the
same nine server and seven HTTP tests under the pinned coverage instrumentation,
with the pinned live Python reference. Both platforms retained the original
assertions and used no test retries or ignored cases in these selections.
Formatting, affected-crate Clippy, workflow policy, actionlint/ShellCheck and
static preflight also passed. The initial local preflight's sandbox process-control
denial is retained separately from the subsequent normal-permission pass.

Fresh complete PR/main qualification, native lifecycle and stable publication
remain pending. These focused results do not establish a measured suite-wide
reliability or speed gain.
