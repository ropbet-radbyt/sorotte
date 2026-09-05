# Scaling workloads

Run from the repository root with Rust 1.97.1 and Python. The command builds a
headless example over production server, media-index, GUI reducer, and mpv
lifecycle APIs. All network traffic uses a disposable loopback listener. No
external media, Plex account, running player, or interactive desktop is needed.

```powershell
python scripts/scaling_workloads.py --name windows-dev --output target/scaling/windows-dev.json --verify-clone-sensitivity
python scripts/scaling_workloads.py --name candidate --output target/scaling/windows-candidate.json --baseline target/scaling/windows-dev.json --baseline-name windows-dev
```

The same commands run on Linux; use a separate `linux-dev` baseline. Native GUI
build prerequisites remain the ones in [DEVELOPMENT.md](DEVELOPMENT.md).
`--target-dir` honors an isolated build directory. `--profile release` selects an
optimized build; it requires a separate baseline from the default `dev` profile.
`--skip-build` explicitly labels the prebuilt binary's source binding unverified.
A normal build refuses to label a binary if source changes during compilation.
Reports include commit SHA, working source digest, dirty state, binary SHA256,
rustc identity, features, platform, CPU, memory, fixture version, sample count,
warmup count, raw observations, and min/median/p95/max/mean/standard deviation.
Allocation counters observe Rust global-allocator requests; native SQLite and
GUI-library allocations are outside that counter. Database bytes and OS handle
counts are separate measurements rather than estimates of total process memory.

The first command records a named baseline; the second produces a comparison
without modifying that baseline. Hardware, profile, features, case selection,
fixture dimensions and metric inventory must match. Source and binary identities
may differ because comparisons are intended to measure code changes. Comparison
rows include absolute and percentage changes; zero denominators are explicit.
Elapsed-time observations are advisory. No p95 threshold is a correctness gate,
and thresholds require stable-worker noise measurements first.

| Fixture | Normal | Large |
|---|---:|---:|
| Roster members | 4 | 64 |
| Empty permanent rooms | 8 | 512 |
| Metadata bytes per member | 128 | 1,024 |
| Accepted server playlist entries | 16 | 250 |
| GUI playlist projection entries | 16 | 2,048 |
| Inventory/fingerprint rows | 64 | 1,024 |
| Audio anchors per row | 32 | 32 |
| GUI projection pumps | 16 | 32 |
| Reconnect/recovery cycles | 32 | 256 |

Both fixtures run by default with one discarded warmup and three measured
samples. `--churn-cycles 10000 --samples 1 --warmup 0` expands a bounded churn
campaign without changing other dimensions. Record a new baseline for that
fixture. `--timeout` bounds each subprocess; reported failures return nonzero.

Fixture version 2 separates the server's legacy-compatible 250-entry/10,000-character
playlist limit from larger local GUI projections. Each server recipient must
receive the exact generated playlist; correction responses fail the workload.
Server workloads populate actual complete rosters and empty rooms, apply legal
metadata/playlist changes, measure encoded fanout bytes and allocations, and
perform repeated join/leave operations. Large fixtures retain headroom under
the production frame and aggregate fanout limits. The TCP workload keeps one
peer unread while measuring a healthy peer's round trips, records queue bytes,
depth, overload disconnects and coalescing, then repeats real connect/Hello/drop
cycles. Checkpoints require zero retained connections, unauthenticated permits,
address buckets and queued bytes. Queue peaks must stay within the configured
4 MiB aggregate cap. Network and actor tasks must join. OS handles (Windows) or
file descriptors (Linux) are observed before/after and through the campaign;
their wall-clock-dependent counts are not equated with actor task counts.

The real mpv adapter verification harness executes tracked loads and same-media
recovery, raw mpv event ingress, authoritative reconciliation, delivery
acknowledgement and attachment replacement. Checkpoints count retained attempts,
events and semantic outcomes; fully acknowledged cycles may retain at most two
attempts and no pending events/outcomes. This workload uses deterministic player
ingress and does not claim a live-mpv benchmark.

Media fixtures create a real SQLite inventory and fingerprint/anchor index.
Four files share each anchor family, so searches must return useful matches
with bounded candidate/hit work. The report separates first lookup, repeated
warm lookup, and a newly opened SQLite connection. Reopening does **not** evict
the OS page cache. Cancellation interrupts a rebuild transaction halfway through
inventory mutation; the live identities/fingerprints must remain intact and the
staging directory must disappear. Database and audio-blob bytes make index growth
visible alongside build/lookup allocation and elapsed-time measurements.

GUI pump measurements apply a runtime snapshot and build the production widget
projection on one initialized semantic driver. Parsing and initialization are
outside each timed pump; allocation totals explicitly include setup. These
measurements cover the shell projection and exclude native rendering/startup.
Reuse [gui-startup-bench.ps1](../scripts/gui-startup-bench.ps1) for startup, then
pass its existing schema-v2 artifact through `--startup-report`. The report binds
that artifact by digest and keeps its original source identity and measurements
separate. Omitted startup artifacts are labelled unavailable.

`--verify-clone-sensitivity` repeats a normal List dispatch with an additional
full-roster clone. Both allocation calls and allocated bytes must increase;
the normal encoded dispatch remains identical. The example's ordinary Rust
test checks this same deliberate regression. Python tests also tamper with
successful reports to prove that resource bounds are checked independently of
the producer's success label.

```powershell
cargo test --locked -p sorotte-gui --example scaling_workloads --features gui-semantic-smoke,live-python-interop
cargo test --locked -p sorotte-gui --lib --features gui-semantic-smoke,live-python-interop semantic_driver::scaling
python -m unittest discover -s scripts/tests -p test_scaling_workloads.py
```
