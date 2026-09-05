# Sorotte post-v0.2.8 code audit and agent task plan

Audit date: 5 September 2026. Intended audience: the maintainer and agents implementing the next improvement cycle.

## 1. Recommendation

Start the next cycle with the reproduced credential, persistence, and protocol-boundary defects. Then harden resource ownership and time-dependent behavior. Preserve the substantial verification work already on the release-closure branch. The highest return is in making adjacent components agree about limits, ownership, and successful completion, followed by reducing the cost of changing those components.

This report contains 22 bounded tasks. It distinguishes reproduced defects, defects established by control flow, hardening opportunities, and maintenance work. It does not claim that every proposed failure was observed in a running GUI, or that this audit proves the release safe to merge. There is enough concrete evidence to begin implementation without another general audit.

The first assignments should be A01 (Plex credentials), A02 (protocol frame sizes), and the configuration stream A03/A05/A06. A04 and A10 deserve early fault characterization because they affect owned-process cleanup and public-server resource use. Small independent fixes A07 and A14 can proceed alongside those streams.

### Baseline and source identity

- Requested checkout: C:/tmp/sorotte-v0.2.8-release-closure.
- Initial branch: codex/v0.2.8-release-closure; initial clean HEAD: **6858d67d393a2a2699fdd3f1acb96a51f4fc643f**.
- The branch advanced during the audit to **8b9ee43b52d9f6049ff5d44a41161fde5b97529a**, “Require explicit latest container promotion.”
- That intervening commit changes only the container publication workflow and its policy test: six inserted lines in two files. All audited Rust product sources and finding anchors are unchanged. The updated container policy suite was run separately.
- Source links in this document identify the refreshed commit. Recheck the eventual merged baseline before starting a task; reconcile subsequent changes before applying a proposed fix.
- The audited checkout was read without production edits. Reproductions used isolated scratch files, synthetic credentials, loopback servers, and dedicated build output. No live Sorotte, Plex account, mpv session, release, or remote configuration was modified.

### What the audit covered

Four reviewers covered the protocol/server/client runtime; player integration and lifecycle; GUI/CLI/settings/Plex/media matching; and verification infrastructure, dependency policy, architecture, and documentation. Findings were checked against callers and existing tests. New probes exercised public APIs and actual HTTP requests; the strongest claims received a separate skeptical review.

The baseline contains 15 crates, 817 Rust files, and 479,184 physical Rust lines under crates. There are 62 Python files under scripts, totaling 61,397 lines at the initial commit. These are repository-size observations, including tests and test tools; they are not measures of production complexity or test quality. A lexical count found 4,146 Rust test attributes, which is not the number of tests runnable on any one platform or feature combination.

Coverage is deliberately risk-focused. The audit is not an exhaustive examination of every line, a new full-workspace release qualification, a live dependency vulnerability scan, or a penetration test.

| Area | Inspection focus | Main next work |
|---|---|---|
| protocol, core | Wire framing, extension preservation, identity and snapshot semantics | A02; preserve existing codec/property/fuzz coverage |
| server | Admission, fanout, timing, readiness, persistence, shutdown | A02, A08–A10, A22 |
| client-core | Ordered player consumption, canonical authority, timing, large coordination modules | A02, A09, A16, A19 |
| client-app, secret | Settings parsing/writing, credential storage, redaction boundaries | A03, A05, A06; no new defect asserted in SecretValue itself |
| player-api, player-mpv | Attachment/retry, process and IPC cleanup, executable resources, evidence | A04, A11, A13, A14 |
| GUI, CLI | Transport limits, worker ownership, shared persistence, updater, native assurance | A02–A07, A15, A21 |
| Plex, media-match | Credentialed HTTP, metadata bounds, probe cancellation, extraction budgets | A01, A07, A15, A18 |
| compat, sim | Existing proof scope, legacy behavior preservation, generated schedules | Cross-layer acceptance for A02/A08/A09; no blanket replacement of current harnesses |
| lifecycle-evidence and scripts | Producer loss visibility, artifact parsing, policy risk map, dependency checks | A11, A12, A16, A17, A20 |

### Existing strengths to preserve

The current code already has ordered acknowledged player delivery, attachment/load-attempt fencing, replay and compaction tests, lifecycle model/oracle checks, real-mpv recovery coverage, source-bound behavior evidence, two-platform changed-line coverage, targeted mutation shards, fuzz targets, fail-on-flaky nextest policy, strict live Python compatibility, native GUI contracts, durable updater transactions, and immutable release-artifact checks.

Specific stale recommendations were rejected: adding generic poisoned legacy getters, another generic EOF recovery suite, treating participant status as playback authority, and assuming container signing or mutation testing are absent. The current implementation already addresses those areas. The empty known-defect registry describes the registered historical characterizations; it does not establish that the newly reproduced issues below are absent.

## 2. How to use the task plan

Each task is suitable for a focused implementation assignment. A task's acceptance criteria are part of its scope. Existing tests mentioned below were inspected; only tests explicitly listed in the audit evidence section were run in this audit.

Priority meanings: **P1** means address early because of credential exposure, substantial correctness impact, or public-service resilience; **P2** means the next planned improvement cycle; **P3** means follow-on maintainability or expanded assurance. No P0 incident is asserted.

Evidence classes: **R** = reproduced using current product APIs or a platform fixture; **C** = established by inspected control flow, with the complete user-visible scenario still to be reproduced; **H** = confirmed protection/test gap, with conditional impact; **M** = design or maintenance improvement. A task can combine a reproduced defect with related hardening, but the distinction is stated.

Effort is relative: **S** is a small focused change; **M** crosses several functions or one integration boundary; **L** crosses protocols, platforms, or several ownership boundaries. It is not a calendar estimate.

| ID | Task | Priority | Evidence | Effort | Main owner / dependency |
|---|---|---|---|---|---|
| [A01](#a01) | Fence credentialed Plex redirects by origin | P1 | R | S–M | Plex; coordinate A15 |
| [A02](#a02) | Make server output fit supported client frame limits | P1 | R | L | Protocol/server/CLI/GUI |
| [A03](#a03) | Preserve Windows credential-file security during replacement | P1 | R | M | Settings persistence |
| [A04](#a04) | Guarantee owned-player cleanup after bounded GUI shutdown | P2 | C/H | L | GUI/process/player |
| [A05](#a05) | Make INI save/read semantics agree on duplicates | P2 | R | S | Settings; same owner as A03/A06 |
| [A06](#a06) | Make settings read-modify-write safe across writers | P2 | R | M | Settings; sequence after A05 |
| [A07](#a07) | Honor cancellation throughout media probing and pipe cleanup | P2 | R/H | M | Media Match |
| [A08](#a08) | Use monotonic time for server expiry and local deadlines | P2 | R/H | L | Server/readiness |
| [A09](#a09) | Correlate and bound ping-derived timing estimates | P2 | R | M | Server timing; coordinate A08 |
| [A10](#a10) | Add admission limits and queued-byte budgets | P1* | H | L | Server/network; coordinate A02 |
| [A11](#a11) | Make lifecycle recording failures sticky and visible | P2 | H | M | Evidence producer |
| [A12](#a12) | Give verification artifacts consistent strict parsing | P2 | R/H | M | Python verification; coordinate A11 |
| [A13](#a13) | Put executable Lua resources in a trusted private cache | P2 | H | M | Player/platform filesystem |
| [A14](#a14) | Start reconnect backoff after initialization finishes | P2 | C | S | mpv adapter |
| [A15](#a15) | Bound HTTP bodies and update extraction | P2 | H | M | Plex/GUI remote services |
| [A16](#a16) | Align critical coverage and mutation selection with current risks | P2 | R/H | M | Verification policy |
| [A17](#a17) | Add dependency advisory and source-policy automation | P2 | H | M | CI/release |
| [A18](#a18) | Establish reproducible scaling and latency workloads | P2 | M | M | Server/GUI/media performance |
| [A19](#a19) | Split coordination code along existing ownership boundaries | P2 | M | L | Client-core/player; after fixes |
| [A20](#a20) | Publish a compact current architecture and verification index | P3 | M | S–M | Documentation; after task decisions |
| [A21](#a21) | Expand native GUI accessibility and display-condition assurance | P3 | H | M | GUI/native tests |
| [A22](#a22) | Bound server persistence shutdown under SQLite contention | P2 | H | M–L | Server/storage |

**A10 priority:** P1 for an exposed public server; a private trusted deployment may reasonably schedule it as P2. No exhaustion attack was run against a live service.

## 3. Task specifications

<a id="a01"></a>

### A01 — Fence credentialed Plex redirects by origin

**Problem and evidence.** PlexHttpClient constructs a reqwest client with its default redirect policy, then attaches X-Plex-Token as a custom request header. A loopback test returned a redirect from one origin to another; the second origin received the synthetic token and the API call succeeded. This used the product HTTP client, not an imitation of its header logic. The existing absolute media-part URL origin check is useful but does not protect these HTTP redirects.

Sources: [crates/sorotte-plex/src/lib.rs:863](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-plex/src/lib.rs#L863), [crates/sorotte-plex/src/lib.rs:1118](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-plex/src/lib.rs#L1118), [crates/sorotte-plex/src/lib.rs:1322](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-plex/src/lib.rs#L1322), [crates/sorotte-plex/src/lib.rs:1402](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-plex/src/lib.rs#L1402).

**Implementation plan.**

1. Give authenticated metadata/server requests an explicit redirect policy. Prefer rejecting cross-origin redirects; if redirect support is required, reevaluate the credential decision for every hop.
2. Define origin as scheme, canonical host, and effective port. Reject HTTPS-to-HTTP credential downgrades. Preserve explicitly configured LAN HTTP behavior separately.
3. Keep authentication/PIN and playback URL behavior explicit; do not assume a global redirect change has identical compatibility consequences for each.
4. Return a useful redacted error and keep the token out of logs, debug output, and tests' failure messages.

**Acceptance.** A different host, port, or scheme never receives the credential; same-origin redirects either work under a bounded policy or produce a documented failure. Cover relative redirects, redirect chains/loops, supported redirect statuses, an HTTPS downgrade, discovery, metadata lookup, selected-server operations, and report_timeline at [crates/sorotte-plex/src/lib.rs:1454](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-plex/src/lib.rs#L1454). Re-run existing same-origin stream-part validation and token-redaction tests. Marking the header sensitive alone is insufficient: the redirect policy must prevent forwarding. The demonstrated case is a redirecting endpoint, not theft by any arbitrary room participant.

**Validation and handoff.** Add a two-listener HTTP regression in sorotte-plex, using only canary credentials. Run that crate and affected GUI Plex worker tests. A15 should reuse the resulting HTTP construction policy. Do not disable certificate verification to make fixtures convenient.

<a id="a02"></a>

### A02 — Make server output fit every supported client framing contract

**Problem and evidence.** The server accepts frames up to 512 KiB. The CLI reader allows 64 KiB and the GUI reader 512 KiB. List serialization aggregates room/member metadata without a total encoded-size bound. The audit reproduced:

- A 66,851-byte server-accepted metadata update generates a 66,895-byte peer update, exceeding the CLI limit.
- A signature accepted by the actual Media Match validator, in a 9,859-byte input per provider, produces a 566,863-byte List with 57 providers plus the reader.
- Two accepted 300,058-byte extension-bearing file updates produce a 600,747-byte List.
- 1,024 configured permanent rooms produce a 623,255-byte List without large media metadata.

The first input is described as server-accepted, not asserted to satisfy the separate 32 KiB Media Match wire contract. The validator-approved signature case establishes that malformed fingerprints are not required for aggregate overflow.

Sources: [crates/sorotte-server/src/lib.rs:288](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/lib.rs#L288), [crates/sorotte-server/src/runtime_maintenance.rs:2056](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_maintenance.rs#L2056), [crates/sorotte-server/src/runtime_handlers.rs:703](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_handlers.rs#L703), [crates/sorotte-cli/src/protocol_io.rs:15](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-cli/src/protocol_io.rs#L15), [crates/sorotte-gui/src/app/runtime_stack/transport/tcp.rs:29](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-gui/src/app/runtime_stack/transport/tcp.rs#L29), [crates/sorotte-server/src/tests/session_tests.rs:269](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/tests/session_tests.rs#L269), [crates/sorotte-client-core/src/session/apply.rs:874](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-core/src/session/apply.rs#L874).

Capability sanitization removes Media Match metadata for recipients without that capability, and room isolation changes the aggregate scope. The permanent-room case uses the GUI-style empty-room projection. Ordinary List messages replace the client's roster, so sending several unnegotiated chunks would discard earlier entries.

**Implementation plan.**

1. Write one inbound/outbound frame-limit contract for Rust clients, the Rust server, and supported legacy peers, including delimiters and encoded byte length.
2. Align Rust limits where appropriate, but also bound the server's aggregate output. Raising the CLI limit alone does not solve the reproduced List failure.
3. Decide which optional metadata may be omitted or compacted per capability. Do not silently truncate an authoritative roster or split List into chunks unless replacement/merge semantics are explicitly supported.
4. If a full legacy snapshot cannot fit, reject the triggering growth or negotiate a supported representation before committing state that recipients cannot consume.
5. Check encoded size before queue admission and before large fanout allocation. Coordinate with A10.

**Acceptance.** Every accepted state mutation can be represented within each intended recipient's framing contract, or fails explicitly without corrupting room state. Cover LF/CRLF boundaries, Unicode expansion, large extension data, valid signatures, empty/permanent rooms, late joins, reconnect, isolated rooms, GUI/CLI peers, and a pinned Python peer. A large client must not disconnect healthy small clients or silently hide members.

**Validation.** Add real loopback server-to-CLI and server-to-GUI-reader tests, plus protocol properties for the exact boundary and one-byte-over cases. Preserve the existing legal-large-payload tests by documenting the intended changed behavior. Run server, CLI framing, compatibility, and GUI transport suites.

<a id="a03"></a>

### A03 — Preserve Windows credential-file security during atomic replacement

**Problem and evidence.** The settings writer preserves Rust Permissions, which does not preserve a Windows DACL, then replaces the destination using MoveFileExW. Its non-Unix owner-only enforcement is a no-op. A fixture with a protected owner-only DACL became inheriting after the product write: explicit rules changed from 1 to 0; inherited rules from 0 to 6.

This demonstrates loss of the explicit protection. Whether another user gains access depends on the containing directory's ACL. The test did not expose real stored credentials or assert that every default profile is broadly readable.

Sources: [crates/sorotte-client-app/src/sorotte_ini/paths.rs:105](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-app/src/sorotte_ini/paths.rs#L105), [crates/sorotte-client-app/src/sorotte_ini/paths.rs:205](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-app/src/sorotte_ini/paths.rs#L205), [crates/sorotte-client-app/src/sorotte_ini/paths.rs:232](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-app/src/sorotte_ini/paths.rs#L232), [crates/sorotte-client-app/src/sorotte_ini/paths.rs:279](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-app/src/sorotte_ini/paths.rs#L279).

**Implementation plan.**

1. Define the Windows security-descriptor contract for an existing settings file and a newly created file.
2. Apply an appropriate protected descriptor to the temporary file before writing secrets; preserve a stricter existing descriptor rather than merely its read-only bit.
3. Use handle-based APIs and a reviewed replacement sequence; fail explicitly if required security cannot be applied.
4. Preserve atomicity, rollback-on-failure, and Unix owner-only behavior. Keep this change focused on settings and its temporary files.

**Acceptance.** Restricted fixtures retain protection and expected principals after save; new files and temporary files do not transiently inherit broader access; permissive-parent fixtures remain safe; failures before replacement leave original bytes and ACL intact. Test read-only destinations, existing inheritance, missing parents, and error cleanup. Never test against the user's real configuration.

**Validation.** Add a Windows DACL integration test that inspects both bytes and security descriptors. Retain the existing atomic-save regressions and Unix permission tests. Coordinate all edits to paths.rs with A06.

<a id="a04"></a>

### A04 — Guarantee owned-player cleanup when GUI shutdown detaches a blocked worker

**Problem and evidence.** The GUI allows two seconds for runtime shutdown, then detaches the worker so process exit can continue. The managed mpv Child and its cleanup guard live on that worker. IPC operations can take five seconds, and terminal cleanup can first wait for an outstanding command. The existing blocked-owner test explicitly proves the guard has not been dropped when the deadline expires, then manually releases the worker.

This establishes a cleanup-ownership gap. An mpv process surviving actual parent exit is inferred, not reproduced in this audit. Healthy Exit coverage does not close the stalled-I/O case.

Sources: [crates/sorotte-gui/src/app/runtime_queue.rs:607](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-gui/src/app/runtime_queue.rs#L607), [crates/sorotte-gui/src/app/runtime_queue.rs:731](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-gui/src/app/runtime_queue.rs#L731), [crates/sorotte-gui/src/app/runtime_queue/tests.rs:567](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-gui/src/app/runtime_queue/tests.rs#L567), [crates/sorotte-gui/src/app/mpv_launch.rs:72](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-gui/src/app/mpv_launch.rs#L72), [crates/sorotte-player-mpv/src/ipc.rs:242](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/ipc.rs#L242).

**Implementation plan.**

1. First add an isolated process fixture: owned child plus blocked player command, followed by actual parent exit without releasing the worker.
2. Keep owned-child lifetime containment effective even when that worker cannot finish. Choose a reviewed platform mechanism, such as Windows job containment or independent termination ownership.
3. Make terminal IPC cleanup cancellation-first with its own overall deadline.
4. Preserve the distinction between Sorotte-launched and externally attached players. Review the analogous CLI ownership path before introducing a shared helper.

**Acceptance.** Sorotte exits within its intended bound; its owned child is gone and owned IPC resources can be reused; an external player survives. Cover blocked polling, pending heartbeat/readback, replacement during exit, launch failure, and normal shutdown. Tests must observe process exit, not merely a mock Drop flag.

**Validation.** Run runtime-queue and managed-player lifecycle tests, then the isolated process regression and real-mpv recovery/Exit contracts. Use an isolated interactive desktop for strict native smoke.

<a id="a05"></a>

### A05 — Make INI save/read semantics agree on duplicate keys and sections

**Problem and evidence.** The parser takes the last recognized assignment. The writer updates the first matching key in the first matching section. Both a duplicate key and duplicate section reproduce “save name=new; reload name=last.”

Sources: [crates/sorotte-client-app/src/sorotte_ini/helpers.rs:105](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-app/src/sorotte_ini/helpers.rs#L105), [crates/sorotte-client-app/src/sorotte_ini/parser.rs:20](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-app/src/sorotte_ini/parser.rs#L20), [crates/sorotte-client-app/src/sorotte_ini/writer.rs:16](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-app/src/sorotte_ini/writer.rs#L16).

**Implementation plan.** Preserve the documented parser's compatibility semantics. Either update the effective final occurrence or normalize all recognized duplicates to a single effective value. Preserve comments, unknown keys, unrelated sections, escaping, and intentional credential clearing. Do not broadly replace the INI format as part of this fix.

**Acceptance.** Parsing a saved document returns the requested recognized values for duplicate keys, repeated sections, mixed case, whitespace, BOM, escaped values, and combinations of these. Explicitly test sensitive settings, not just username. Unknown content remains intact according to the existing contract; saving twice is idempotent.

**Validation.** Add table-driven and generated parser/writer round-trip properties to sorotte_ini tests, then run shared client-app configuration tests and affected CLI/GUI persistence tests. Use the public writer/parser reproduction as the initial failing regression.

<a id="a06"></a>

### A06 — Make settings read-modify-write safe across independent writers

**Problem and evidence.** The public update function loads, invokes a callback, and writes a whole settings snapshot without a transaction lock or stale-version check. Two deliberately interleaved updates to different fields lost the earlier username change while retaining the later room change. Atomic file replacement protects complete bytes, not the read-modify-write transaction.

Source: [crates/sorotte-client-app/src/sorotte_ini/paths.rs:309](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-app/src/sorotte_ini/paths.rs#L309). The reproduction establishes the public persistence API race. A routine GUI-only race is not asserted; multiple processes or independent callers must overlap.

**Implementation plan.**

1. Define concurrency semantics for update, full replacement, clear, migration, and credential clearing.
2. Prefer a per-path cross-process lock covering read through commit, with a bounded wait and useful busy error. If optimistic conflict detection is chosen, expose a retryable conflict rather than silently overwriting.
3. Account for canonical path aliases and lock-file lifetime when the destination is replaced atomically.
4. Do not automatically retry a FnOnce callback whose side effects may not be repeatable.
5. For full-snapshot saves, detect stale revisions or merge only intended changes. Serializing writers alone cannot prevent a previously captured snapshot from restoring credentials cleared by another writer.

**Acceptance.** Two-process disjoint updates preserve both changes; conflicting updates have a documented order or explicit conflict; crash/stale-lock recovery is bounded; readers never see partial content; clear cannot race a stale save into recreating credentials. Preserve A03's security contract.

**Validation.** Add deterministic barriers to a real two-process fixture rather than sleep-based probability tests. An external controller must release the first writer independently while verifying the second waits or conflicts; reusing the diagnostic callback-blocking schedule unchanged would deadlock after adding a transaction lock. Run the existing fault-injected atomic-write tests and generated config composition/migration tests. Keep this task in the same ownership stream as A03/A05.

<a id="a07"></a>

### A07 — Honor cancellation throughout media probing and pipe cleanup

**Problem and evidence.** Cancellable fingerprinting calls the non-cancellable ffprobe path before forwarding the flag to audio extraction. A flag already set before invocation still caused a synthetic ffprobe process to run and wait roughly 1.25 seconds. The production ffprobe timeout is 15 seconds. Tool pipe readers also accumulate output and join reader threads after child completion; cancellation and output limits need one end-to-end contract.

Sources: [crates/sorotte-media-match/src/extraction.rs:256](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-media-match/src/extraction.rs#L256), [crates/sorotte-media-match/src/extraction.rs:280](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-media-match/src/extraction.rs#L280), [crates/sorotte-media-match/src/extraction.rs:358](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-media-match/src/extraction.rs#L358), [crates/sorotte-media-match/src/extraction.rs:713](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-media-match/src/extraction.rs#L713), [crates/sorotte-media-match/src/extraction.rs:949](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-media-match/src/extraction.rs#L949).

Only the ffprobe cancellation delay was reproduced. Descendant-held pipes and retained reader threads are source-backed test gaps. Define a measurable cancellation/drain deadline from an after-launch handshake; do not reuse the 15-second probe timeout as the total budget for the supported audio-extraction workload.

**Implementation plan.** Check cancellation before filesystem/probe work and immediately before process launch. Add an internal cancellable probe while preserving the public convenience API. Carry a single operation deadline through execution and pipe draining. Bound textual probe output and stderr; derive PCM bounds from requested sample windows. Handle a descendant retaining an inherited pipe without an unbounded reader join.

**Acceptance.** Pre-cancel starts no child; cancellation during probe/audio work terminates owned work promptly; no result is committed after cancellation; large diagnostics and a pipe-holding descendant cannot defeat cleanup bounds. Preserve useful bounded error tails and do not turn cancellation into a successful degraded fingerprint.

**Validation.** Add deterministic fixture modes for pre-cancel, cancellation after launch, endless stdout/stderr, silent timeout, and inherited-pipe lifetime. Re-run the generated-media capability tests and GUI worker-fencing tests.

<a id="a08"></a>

### A08 — Use monotonic time for server expiry and local deadlines

**Problem and evidence.** A readiness reconnect membership detached at wall time 100 was still restored with its original epoch after maintenance observed time 400, then time rolled back to 150. The reconnect TTL is 180 seconds and pruning occurs at attach using wall-clock subtraction. Thus a deadline already passed in an observed history can become live again. Adjacent readiness pairing, buffering freshness, and barrier deadlines also require an explicit clock audit.

Sources: [crates/sorotte-server/src/runtime_readiness.rs:224](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_readiness.rs#L224), [crates/sorotte-server/src/runtime_readiness.rs:450](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_readiness.rs#L450), [crates/sorotte-server/src/runtime_readiness.rs:1021](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_readiness.rs#L1021), [crates/sorotte-server/src/runtime_readiness.rs:2076](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_readiness.rs#L2076), [crates/sorotte-server/src/lib.rs:557](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/lib.rs#L557).

**Implementation plan.** Separate externally meaningful protocol timestamps from internal elapsed time. Introduce an injectable monotonic clock/deadline type for reconnect TTL, pending request pairing, preparation timeouts, and freshness. Make expiry irreversible within an owned lifetime, including maintenance pruning. Keep wall-clock protocol compatibility and persisted timestamps explicit; do not substitute Instant everywhere.

**Acceptance.** Independent wall-clock forward/backward changes never revive an expired token or local deadline. Cover exact TTL, pruning without a reconnect, token reuse, capacity eviction, room/connection reset, and barriers that cross a clock adjustment. Advance wall and monotonic clocks independently in tests. Document suspend/resume expectations.

**Validation.** Convert the public-API rollback reproduction into a positive regression, then run readiness, buffering/barrier, reconnect, participant-status non-interference, and live compatibility tests. This is not a claim that the token can be guessed or used without possession.

<a id="a09"></a>

### A09 — Correlate and bound ping-derived timing estimates

**Problem and evidence.** With server time 100, a State containing latencyCalculation=-1000, clientRtt=0 and an unpaused seek to 30 generated canonical position 1680. The estimate derived from an unmatched ancient timestamp flows into synchronization. The advisory participant-status path applies a sanity bound that the canonical forward-delay path does not apply equivalently.

Sources: [crates/sorotte-server/src/runtime_maintenance.rs:1692](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_maintenance.rs#L1692), [crates/sorotte-server/src/runtime_maintenance.rs:1740](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_maintenance.rs#L1740), [crates/sorotte-server/src/runtime_handlers.rs:1669](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_handlers.rs#L1669), [crates/sorotte-server/src/runtime_handlers.rs:1752](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_handlers.rs#L1752).

**Implementation plan.** Correlate echoes to bounded outstanding server challenges in the current connection epoch, or define a compatible bounded estimator when legacy protocol constraints prevent strict correlation. Reject non-finite, negative, future, duplicate, retired, and implausibly old samples before they update timing state. Preserve useful previous estimates or fall back safely.

**Acceptance.** The ancient-echo reproduction cannot amplify the requested position. Cover replay after reconnect, wall-clock changes, legitimate large RTT, jitter, missing clientRtt, and rejected echoes accompanying otherwise valid State. Invalid timing must not suppress legitimate authority handling or poison future estimates.

**Validation.** Add deterministic timestamp schedules, a raw-loopback malformed echo case, and Rust/Python compatibility checks for ordinary echo behavior. This is a robustness defect, not an asserted authorization bypass: a peer permitted to seek may already choose a different position directly.

<a id="a10"></a>

### A10 — Add server admission limits and queued-byte budgets

**Problem and evidence.** Existing queues and overload policies are bounded by item count. The live accept path can still create unbounded sessions, including peers waiting for Hello or holding partial frames. Each encoded outbound String may itself be large; a 256-item queue does not establish a memory budget. A02 shows that the large-output path is reachable.

Sources: [crates/sorotte-server/src/network.rs:153](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/network.rs#L153), [crates/sorotte-server/src/network.rs:1474](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/network.rs#L1474), [crates/sorotte-server/src/backpressure.rs:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/backpressure.rs#L1), [crates/sorotte-server/src/lib.rs:288](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/lib.rs#L288).

**Implementation plan.**

1. Define configurable ceilings for active connections, unauthenticated connections, per-address admission, and queued bytes per peer and globally.
2. Reserve capacity before spawning/allocating; release permits on every failure and disconnect path.
3. Account for replacement/coalescing by subtracting retired bytes, and avoid materializing unlimited fanout before accounting.
4. Preserve fairness for legitimate users behind shared NATs, IPv6 normalization, and the current slow-peer isolation semantics.
5. Expose bounded counters and redacted rejection reasons for operators.

**Acceptance.** Excess connections are rejected without disturbing admitted peers; slowloris-style partial input and unread large fanout remain within configured limits; counters return to baseline after disconnects and panics; replaceable snapshots cannot discard receipt-owned authoritative work. Defaults must support the documented normal workload.

**Validation.** Add loopback stress/fault tests with deterministic capacity assertions, not fragile RSS thresholds. Use A18 to measure actual memory and latency on larger workloads. No live public-service exhaustion test is required.

<a id="a11"></a>

### A11 — Make lifecycle recording failures sticky and visible at finalization

**Problem and evidence.** Recorder emission can return validation or I/O errors, several product callers discard those results, and finalization performs a fresh flush without remembering earlier failures. A later successful flush therefore does not establish that every attempted event was recorded.

Sources: [crates/sorotte-lifecycle-evidence/src/lib.rs:299](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-lifecycle-evidence/src/lib.rs#L299), [crates/sorotte-lifecycle-evidence/src/lib.rs:409](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-lifecycle-evidence/src/lib.rs#L409), [crates/sorotte-lifecycle-evidence/src/lib.rs:463](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-lifecycle-evidence/src/lib.rs#L463), [crates/sorotte-player-mpv/src/adapter.rs:903](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/adapter.rs#L903).

The Python verifier already rejects malformed records, sequence gaps, unknown predecessors, and missing required scenario evidence. This is a producer loss-reporting gap, not a demonstrated false-green complete release gate.

**Implementation plan.** Introduce an injectable writer behind the file-backed API. Latch the first failure once recording is enabled; finalization must retain that failure even if later writes succeed. Decide whether subsequent events are refused or retained for diagnosis. Serialize a complete bounded record before writing, preserve the privacy-safe schema, and keep the disabled recorder cheap.

**Acceptance.** Inject failures before writing, midway through a record, at newline/flush, and after a valid prefix. A transient error can never be forgotten by finalization. Concurrent emitters see a consistent failed state. Invalid token/role attempts do not leak their content. Healthy sequence, causal chain, and digest behavior remain unchanged.

**Validation.** Run recorder tests and Python lifecycle-evidence tests; add a producer-to-consumer fault regression. Preserve required transition/oracle checks rather than replacing them with recorder health.

<a id="a12"></a>

### A12 — Give verification artifacts consistent strict parsing

**Problem and evidence.** The release-gate loader accepts duplicate JSON fields using ordinary json.loads; the behavior-evidence loader rejects them. A direct probe supplied a failed status followed by a duplicate passed status. The release loader returned passed, while the behavior loader rejected the input. This is a reproduced parser discrepancy; a complete gate bypass was not demonstrated.

Sources: [scripts/playback_release_gate.py:147](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/scripts/playback_release_gate.py#L147), [scripts/playback_lifecycle_evidence.py:119](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/scripts/playback_lifecycle_evidence.py#L119), [scripts/behavior_evidence.py:76](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/scripts/behavior_evidence.py#L76), [scripts/diff_coverage.py:1689](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/scripts/diff_coverage.py#L1689).

**Implementation plan.**

1. Define a small shared artifact-parsing utility with duplicate-key rejection, finite-number requirements, actual integer-vs-boolean validation, bounded byte/record counts, and consistent error categories.
2. Adopt it incrementally across lifecycle/release/coverage inputs. Keep each domain's closed schema and version policy separate.
3. Share only stable primitives such as bounded reads, strict JSON, and digest checks. Do not merge independent behavioral oracles into one self-confirming implementation.
4. Make malformed-artifact fixtures reusable across all public verification entrypoints.

**Acceptance.** Duplicate keys, NaN/Infinity, boolean schema versions, oversized input, invalid UTF-8, and trailing garbage are rejected consistently before attestation. A malformed artifact produces a failed report/nonzero result with an actionable explanation. Valid historical schemas remain accepted only where explicitly supported. Existing wrong-SHA, wrong-platform, missing-proof, duplicate-proof, and tampered-digest tests continue to pass.

**Validation.** Run the Python policy/oracle suites and a table-driven parser matrix through actual CLI entrypoints. Add one valid-to-malformed artifact mutation per boundary. This task complements A11; neither substitutes for the other's producer/consumer checks.

<a id="a13"></a>

### A13 — Put executable Lua resources in a trusted private cache

**Problem and evidence.** When XDG_CACHE_HOME is absent on non-Windows, bundled Lua resources default to the predictable shared temporary path sorotte/mpv-bridge. Materialization compares the canonical bytes and uses atomic replacement, but follows unverified directories/links and returns a pathname that mpv opens later. Another local user controlling an ancestor can change that path after the comparison. Static modified bytes are already rejected or repaired.

Sources: [crates/sorotte-player-mpv/src/bridge_resource.rs:44](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/bridge_resource.rs#L44), [crates/sorotte-player-mpv/src/bridge_resource.rs:86](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/bridge_resource.rs#L86), [crates/sorotte-player-mpv/src/bridge_resource.rs:145](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/bridge_resource.rs#L145), [crates/sorotte-player-mpv/src/adapter.rs:2647](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/adapter.rs#L2647), [crates/sorotte-player-mpv/src/adapter.rs:4113](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/adapter.rs#L4113).

**Implementation plan.** Use a trusted per-user cache, with a private random fallback when necessary. Check relevant ownership, permissions, and link/reparse-point boundaries before accepting an executable resource store. Bound comparison reads to expected length plus one. Preserve content-addressed reuse and atomic repair. Document explicit caller-supplied roots separately.

**Acceptance.** XDG-unset operation does not load executable Lua from a directory another user can pre-own. Unsafe ancestor/link fixtures fail safely; oversized files do not cause unbounded reads; concurrent canonical repair still works. A controlled replacement hook between materialization and load-script cannot redirect execution outside the trusted store.

**Validation.** Add platform filesystem tests and an isolated real-mpv materialization-to-load fixture. No arbitrary code execution was demonstrated in this audit; this is conditional local-user integrity hardening.

<a id="a14"></a>

### A14 — Start reconnect backoff after attachment initialization finishes

**Problem and evidence.** Reconnection samples completed_at after connecting but before initialize_json_ipc_attachment. The latter makes a fallible version query. If initialization consumes the one-second retry interval, the stored failure deadline is already expired. The existing initialization-failure test uses an immediately responding transport and a constant clock value, so it misses this ordering.

Sources: [crates/sorotte-player-mpv/src/adapter/reconnection.rs:49](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/adapter/reconnection.rs#L49), [crates/sorotte-player-mpv/src/adapter.rs:1474](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/adapter.rs#L1474), [crates/sorotte-player-mpv/src/adapter.rs:10807](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/adapter.rs#L10807), [crates/sorotte-player-mpv/src/ipc.rs:38](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/ipc.rs#L38).

**Implementation plan.** Treat connect plus initialization as one attempt and sample the finish clock after whichever operation fails. Keep live-connection, simulation, old-attachment retention, and successful initialization behavior unchanged. Extend the scripted transport to advance a controlled clock during its version query.

**Acceptance.** No retry occurs until a full interval after failure, whether initialization lasts less than, equal to, or longer than the interval. Successful initialization clears the deadline; failed replacement does not retire the previous valid attachment.

**Validation.** Run explicit_json_ipc_retry and version/replacement-fencing tests plus the existing player-mpv-explicit-ipc-retry mutation shard. A regression must fail when the completion clock is sampled before initialization. This small task should remain independent of a broader async-player rewrite.

<a id="a15"></a>

### A15 — Bound HTTP bodies and update extraction, with a separate download policy

**Problem and evidence.** Plex responses and GUI remote metadata/packages are read in full without application byte budgets. Update archives are extracted without per-entry, aggregate-uncompressed, or entry-count quotas. Packages also share the ten-second total request timeout used for small API calls. Path traversal, checksum validation, and updater rollback are already tested; those checks do not bound memory, disk output, or time for a legitimate large download.

Sources: [crates/sorotte-plex/src/lib.rs:885](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-plex/src/lib.rs#L885), [crates/sorotte-plex/src/lib.rs:1030](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-plex/src/lib.rs#L1030), [crates/sorotte-gui/src/app/remote_services.rs:775](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-gui/src/app/remote_services.rs#L775), [crates/sorotte-gui/src/app/remote_services.rs:1392](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-gui/src/app/remote_services.rs#L1392), [crates/sorotte-gui/src/app/remote_services.rs:1509](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-gui/src/app/remote_services.rs#L1509), [crates/sorotte-gui/src/app/remote_services.rs:1820](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-gui/src/app/remote_services.rs#L1820).

**Implementation plan.** Define distinct metadata, archive, decompressed-output, entry-count, and multi-request search budgets based on supported workloads. Enforce actual streamed bytes even when Content-Length is missing or wrong; avoid buffering irrelevant error bodies. Download into private staging while hashing, using separate connect/idle/overall deadlines and cancellation. Check ZIP metadata and actual extraction counters, including nested Actions archives.

**Acceptance.** Oversized, chunked, expanding, and many-entry inputs fail with bounded allocation/output and no updater launch. A slow steadily progressing valid transfer can complete under the download policy. Quota failure cleans only its partial stage and preserves the installed app and rollback material. Duplicate normalized archive entries and nested limits are covered.

**Validation.** Use local HTTP and ZIP fixtures; run Plex/remote-service/update-worker tests and existing updater recovery process tests. Do not induce machine-wide memory or disk exhaustion. The task can be delivered as two child slices—Plex metadata and update ingress—after agreeing shared HTTP policy with A01.

<a id="a16"></a>

### A16 — Align critical coverage and mutation selection with current risks

**Problem and evidence.** A probe of the actual changed-line policy classified lifecycle.rs as critical, but the mpv adapter, client playback coordination/coordinator, server network/backpressure, Plex implementation, and GUI player telemetry as ordinary. Critical files use a 90% minimum. Existing base/head policy merging protects listed paths against removal, but does not discover newly critical modules.

The mutation workflow's required PR matrix centers on ten participant-status-related shards, while older privacy/auth/codec/configuration shards run in the broader schedule/manual matrix. That is an explicit historical policy, not absence of mutation testing. It no longer closely matches the full set of current risk boundaries.

Sources: [coverage/diff-coverage-policy.toml:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/coverage/diff-coverage-policy.toml#L1), [scripts/diff_coverage.py:250](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/scripts/diff_coverage.py#L250), [scripts/diff_coverage.py:2646](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/scripts/diff_coverage.py#L2646), [coverage/mutation-policy.toml:4](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/coverage/mutation-policy.toml#L4), [.github/workflows/rust-mutation.yml:3](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/.github/workflows/rust-mutation.yml#L3), [scripts/tests/test_ci_policy.py:2439](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/scripts/tests/test_ci_policy.py#L2439).

**Implementation plan.**

1. Reassess critical modules by authority, privacy, persistence, and resource ownership; add narrow rules for agreed production boundaries.
2. Add a semantic inventory check linking those boundaries to policy entries so a module extraction cannot silently demote them.
3. Keep the mandatory report set, but also select relevant existing mutation shards from changed production files and their declared dependencies. Avoid running every expensive shard for unrelated changes.
4. Add focused mutation coverage for the newly fixed predicates in A01/A05/A08/A09/A14 where useful.
5. Retain exact source binding, accepted-unviable evidence, immutable base/head union, and fail-closed missing/duplicate artifact checks.

**Acceptance.** A representative new critical module cannot fall to ordinary policy unnoticed; changing secret/config/timing code triggers its agreed checks; unrelated docs do not trigger unnecessary mutation work. Changes to test selectors, features, and relevant lockfiles are accounted for. A report set cannot pass by silently omitting a selected shard.

**Validation.** Extend policy tests with semantic before/after examples and one intentional omitted-rule/omitted-shard failure. Run the affected mutation shard; a manifest or YAML assertion alone is not proof its selector executes useful tests. Coordinate A19's moves with this task.

<a id="a17"></a>

### A17 — Add dependency advisory and source-policy automation

**Problem and evidence.** The reviewed workflows pin actions and tools, enforce API compatibility, and build/sign an SBOM-bound server container. No checked-in cargo-audit/cargo-deny advisory gate, dependency source policy, or scheduled dependency update configuration was found. Neither scanner was installed for this audit, and no claim about current vulnerable versions is made.

Sources: [Cargo.toml:29](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/Cargo.toml#L29), [.github/workflows/rust-ci.yml:150](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/.github/workflows/rust-ci.yml#L150), [.github/workflows/publish-server-container.yml:116](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/.github/workflows/publish-server-container.yml#L116), [requirements/ci-policy.txt:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/requirements/ci-policy.txt#L1), [requirements/legacy-python-interop.txt:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/requirements/legacy-python-interop.txt#L1).

**Implementation plan.**

1. Add a pinned advisory/source checker for the locked dependency graph on relevant PRs and on a schedule. Cargo-deny or cargo-audit can provide the RustSec integration; choose one primary policy rather than duplicate overlapping gates.
2. Cover production Rust dependencies and build dependencies; separately account for Python verification tooling and bundled native tools.
3. Store any exception with advisory identity, rationale, owner, and expiry. Treat inability to fetch a required database as unavailable evidence, not zero findings.
4. Produce a reviewed dependency/third-party notice inventory for distributed GUI/server artifacts. Extend the existing SBOM contract; do not invent a parallel server-container signing system.
5. If automated update PRs are adopted, group changes conservatively and require the existing compatibility/behavior gates.

**Acceptance.** A fixture or deliberately selected known advisory causes a failing check; expired exceptions fail; unapproved sources are detected; the release dependency inventory corresponds to the actual artifact inputs. A clean result records checker/database identity. Establish the current baseline before choosing policy exceptions.

**Validation.** Test policy behavior without introducing a vulnerable dependency into production. Run the selected scanner live during implementation and report actual findings. Tool capabilities were checked against [RustSec](https://rustsec.org/) and the [cargo-deny check documentation](https://embarkstudios.github.io/cargo-deny/checks/index.html); this report does not prescribe an unverified latest tool version or make a legal license-compliance conclusion.

<a id="a18"></a>

### A18 — Establish reproducible scaling and latency workloads

**Problem and evidence.** Behavioral verification is extensive, and a GUI startup benchmark already exists. The reviewed tree does not have an equivalent reusable workload suite for roster/fanout growth, event-loop latency under slow consumers, or Media Match index scale. The output amplification in A02 is a concrete reason to measure these workloads. No general performance regression is asserted without measurement.

Sources: [scripts/gui-startup-bench.ps1:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/scripts/gui-startup-bench.ps1#L1), [crates/sorotte-server/src/runtime_maintenance.rs:2090](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_maintenance.rs#L2090), [crates/sorotte-server/src/actor.rs:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/actor.rs#L1), [crates/sorotte-media-match/src/media_index.rs:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-media-match/src/media_index.rs#L1), [crates/sorotte-client-core/src/runtime/playback_coordination.rs:932](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-core/src/runtime/playback_coordination.rs#L932).

**Implementation plan.** Build a small deterministic workload runner covering:

- Small normal rooms, a larger roster, many empty rooms, frequent joins/leaves, and a slow/unreading peer.
- Large legal metadata and playlist changes, with encoded bytes, queue occupancy, fanout allocation, and dispatch latency.
- Long-running reconnect/recovery churn, recording retained attempts, workers, sockets, and handles.
- Generated media-index inventories at several sizes, cancellation during rebuild, and bounded warm/cold search measurements.
- GUI projection of large rosters/playlists, recording pump responsiveness separately from startup.

Record source SHA, build profile, platform, hardware, fixture identity, warmup, sample count, and distributions. Prefer allocation/count invariants in ordinary CI; use stable workers for wall-clock trends. Profile measured hot paths before changing collections, caching, or actor topology.

**Acceptance.** One command produces reproducible machine-readable results and a comparison to a named baseline. Workload growth and retained-resource bounds are visible. A deliberately introduced extra full-roster clone or missed cleanup changes an appropriate measure. Define thresholds only after obtaining baseline noise data.

**Validation.** Run at least normal and large cases on Windows and Linux, plus a bounded churn test. Reuse the existing startup benchmark. Keep timing observations separate from correctness gates and avoid flaky p95 assertions on heterogeneous developer machines.

<a id="a19"></a>

### A19 — Split coordination code along existing ownership boundaries

**Problem and evidence.** The client playback-coordination module is 20,197 lines, including an inline test module beginning around line 6,887. The mpv adapter is 12,901 lines; playback_coordinator is 8,745. The important concern is how many unrelated invariants a change requires a reviewer to hold at once. Raw size alone is not evidence of a bug.

Sources: [crates/sorotte-client-core/src/runtime/playback_coordination.rs:42](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-core/src/runtime/playback_coordination.rs#L42), [crates/sorotte-client-core/src/runtime/playback_coordination.rs:932](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-core/src/runtime/playback_coordination.rs#L932), [crates/sorotte-client-core/src/runtime/playback_coordination.rs:6887](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-core/src/runtime/playback_coordination.rs#L6887), [crates/sorotte-client-core/src/playback_coordinator.rs:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-client-core/src/playback_coordinator.rs#L1), [crates/sorotte-player-mpv/src/adapter.rs:694](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-player-mpv/src/adapter.rs#L694).

**Implementation plan.**

1. First move existing inline test groups into domain-named test modules while retaining scenario identities and policy selectors.
2. Extract the ordered player-event consumer and its pure state transitions behind a narrow internal interface.
3. Separately extract participant-status reporting state and barrier/local-intent coordination where ownership is already explicit.
4. In the adapter, isolate network-option supervision and recovery bookkeeping after its pending bug fixes settle.
5. Use typed identity/deadline wrappers at the resulting seams where they prevent accidental substitution; do not perform a workspace-wide numeric-type rename.

**Acceptance.** Each extracted module documents its owned state, accepted inputs, outputs, and reset rules. Behavioral output and public API stay unchanged unless a separate reviewed task explicitly changes them. Tests still execute rather than disappearing because of moved selectors/features. No abstraction combines advisory status with canonical authority or GUI presentation with physical ownership.

**Validation.** Preserve existing lifecycle model, generated history, adapter-to-consumer, and real-player tests. Update A16's path policies and mutation selectors in the same change. Land a sequence of reviewable extractions rather than one giant formatting/refactor diff. Do not start overlapping extraction while A02/A04/A08/A09 are changing the same state contracts.

<a id="a20"></a>

### A20 — Publish a compact current architecture and verification index

**Problem and evidence.** The authoritative language and ADRs are useful, but current guarantees and historical implementation narratives are spread across long files. TEST_COVERAGE_FINDINGS is 4,239 lines, TEST_COVERAGE_STRATEGY 2,803, and coverage/README 1,232 at the initial audit commit. Several old “remaining” recommendations are now implemented. This raises the chance an agent repeats completed work or mistakes old evidence for current proof.

Sources: [CONTEXT.md:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/CONTEXT.md#L1), [docs/DEVELOPMENT.md:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/docs/DEVELOPMENT.md#L1), [docs/TEST_COVERAGE_FINDINGS.md:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/docs/TEST_COVERAGE_FINDINGS.md#L1), [docs/TEST_COVERAGE_STRATEGY.md:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/docs/TEST_COVERAGE_STRATEGY.md#L1), [coverage/behaviors.toml:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/coverage/behaviors.toml#L1), [coverage/playback-lifecycle.toml:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/coverage/playback-lifecycle.toml#L1).

**Implementation plan.** Add one concise current index mapping each important invariant to owner modules, normative document, executable proof, required environment, and last evidence identity. Label historical ledgers clearly and link them rather than duplicating their prose. Include crate responsibilities and the actual client/server/player authority flow. Mark each completed task with its fixing SHA, positive regression, and remaining limits.

**Acceptance.** A new agent can answer “who owns this state?”, “which test proves this boundary?”, and “what must I run?” without scanning chronological ledgers. The index distinguishes implemented capability, locally executed proof, hosted proof, and still-pending infrastructure. A lightweight checker catches broken source/test/catalog references.

**Validation.** Test the index against representative changes: a protocol field, GUI worker, persistence transaction, and real-mpv recovery. Preserve historical evidence; do not rewrite it as though it describes the new head. Use current DEVELOPMENT as the contributor entrypoint.

<a id="a21"></a>

### A21 — Expand native GUI accessibility and display-condition assurance

**Problem and evidence.** The existing strict native suite has meaningful UIA, physical input, and Exit contracts, plus a fixed-scenario Settings capture packet. DEVELOPMENT explicitly records that theme, DPI, and font inputs remain environmental. That is a test limitation; this audit did not observe a screen-reader or rendering defect.

Sources: [docs/DEVELOPMENT.md:123](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/docs/DEVELOPMENT.md#L123), [docs/DEVELOPMENT.md:165](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/docs/DEVELOPMENT.md#L165), [scripts/gui-visual-suite.ps1:1](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/scripts/gui-visual-suite.ps1#L1).

**Implementation plan.** Define a small risk-based display matrix: supported DPI scales, light/dark theme, constrained window size, long labels, and large roster/playlist content. Capture semantic/UIA trees alongside images. Exercise keyboard focus/activation, error presentation, and modal dismissal. Add deterministic theme/size inputs where feasible; document environmental controls that still require the runner.

**Acceptance.** Essential controls remain reachable, labeled, and correctly focused across the selected conditions; content is not clipped beyond recovery by scrolling; screenshots can be attributed to exact display settings. Exercise at least one actual screen-reader interaction if accessibility claims depend on it. Preserve the distinction between UiaOnly development evidence and StrictPhysical authoritative evidence.

**Validation.** Run the existing visual and semantic suites and the selected matrix on an isolated interactive desktop. Use geometry/semantics for robust assertions and visual review for rendering; avoid brittle whole-screen pixel equality. This is subordinate to the reproduced fixes and does not require automating the user's active desktop.

<a id="a22"></a>

### A22 — Bound server persistence shutdown under SQLite contention

**Problem and evidence.** Network teardown has a five-second grace, but actor shutdown subsequently performs synchronous persistence flush and worker joins without one total budget. SQLite's busy timeout can be consumed repeatedly across pending rooms. Ordinary runtime mutations are already asynchronous; the gap concerns explicit flush/shutdown. Existing contention coverage releases the database lock before shutdown.

Sources: [crates/sorotte-server/src/network.rs:1505](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/network.rs#L1505), [crates/sorotte-server/src/network.rs:1557](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/network.rs#L1557), [crates/sorotte-server/src/actor.rs:286](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/actor.rs#L286), [crates/sorotte-server/src/runtime_api.rs:450](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/runtime_api.rs#L450), [crates/sorotte-server/src/persistence_actor.rs:285](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/persistence_actor.rs#L285), [crates/sorotte-server/src/persistence.rs:488](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/persistence.rs#L488), [crates/sorotte-server/src/tests/network_tests.rs:598](https://github.com/ropbet-radbyt/sorotte/blob/8b9ee43b52d9f6049ff5d44a41161fde5b97529a/crates/sorotte-server/src/tests/network_tests.rs#L598).

**Implementation plan.** Define distinct graceful-durability and forced-termination outcomes. Move blocking acknowledgement waits off the async actor's execution path and add a deadline-bearing flush/shutdown contract. Check budget/cancellation between transactions and integrate SQLite busy/interruption handling for the supported contention case. Coalesce stale wake work without dropping latest desired state. Ensure Drop does not repeat an unbounded flush/join after the explicit path times out.

**Acceptance.** With a disposable database held in BEGIN IMMEDIATE throughout shutdown and several pending rooms, shutdown either completes durably or returns an explicit durability-timeout outcome within the supported budget; unrelated async tasks remain runnable. Reopening the database yields old-or-new-complete state. A timeout cannot be reported as successful persistence or hidden by dropping ownership of a live worker.

**Validation.** Add the held-lock subprocess regression on Windows and Linux; retain arbitration, crash, full/read-only, filesystem-fault, and actor responsiveness coverage. A Tokio timeout around synchronously blocking code is not sufficient. No production shutdown hang was reproduced here, and arbitrary uninterruptible filesystem behavior cannot be guaranteed cancellable by a Rust thread abstraction.

## 4. Integration order and parallel assignments

The unit of handoff is a task plus its evidence and acceptance criteria. Do not assign several agents overlapping files without a designated integration owner.

| Stream | Assignment sequence | Safe parallelism / merge boundary |
|---|---|---|
| HTTP/Plex | A01, then A15's Plex portion | Independent of settings/server. Coordinate one owner for Plex client construction. |
| Settings | A05, A03, then A06 | Same owner or sequential PRs; helpers/tests/paths overlap. A03 can lead if the protected-file risk is immediately relevant. |
| Server/protocol | A02 + A10 contract, then their implementations; A08/A09 | Assign a coordinator for network/maintenance/readiness edits. Separate parser/transport tests can be prepared independently. |
| Player/process | A14; A04; A13 | The small retry fix can land first. Process ownership and filesystem cache work need separate fixtures and clear file ownership. |
| Media tools | A07 | Independent until a shared process-supervision abstraction is deliberately agreed with A04. |
| Server storage | A22 | Independent of the settings writer; coordinate server actor/network teardown edits with A10. |
| Evidence/policy | A11/A12, then A16; A17 | Coordinate script/CI edits. Preserve oracle independence; source-path changes must update risk policy. |
| Follow-on quality | A18, A19, A20, A21 | Capture A18 baselines before optimizations. Defer A19 until behavioral fixes stabilize; update A20 throughout. |

A practical first dispatch with four agents is: Plex A01; settings A05/A03; protocol frame contract A02; and media cancellation A07. Give the server and player ownership investigations to the next free agents. Reserve an integration/review pass for the combined source SHA.

## 5. Common definition of done

1. **Rebase the diagnosis.** Start from the merged release-closure state, record the exact base SHA, and verify that the stated trigger still exists. A superseded finding should be closed with current evidence, not “fixed” again.
2. **Preserve the failing case.** For R/C tasks, add a regression that fails for the demonstrated reason before the change and passes afterward. For H/M tasks, first specify the missing observable contract.
3. **Implement the complete boundary.** Fix callers, consumers, failure handling, configuration/documentation, and any compatibility negotiation needed for the task. Avoid a helper-only change that leaves the end-user path unprotected.
4. **Verify negative behavior.** Include stale, cancelled, malformed, overflow, and failure cases relevant to that boundary. Tests must observe authority, bytes, processes, or durable state, not just mirror helper implementation.
5. **Use the existing gates.** Run focused tests while iterating, then repository-required formatting, strict Clippy, workspace tests, and affected semantic/process/compatibility checks. Public API changes need the pinned semver checker against the exact base.
6. **Review one coherent candidate.** An independent reviewer checks the final SHA, regression usefulness, compatibility, and the failure/cleanup path. Integrate first, then collect the combined candidate's required evidence.
7. **Hand back evidence.** Record changed behavior, commands/results, retained failing fixture, unexecuted environments, residual risks, and documentation/catalog changes. Do not turn a missing external capability into a green result.

Suggested standard commands, run from the implementing checkout:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
```

Use the current DEVELOPMENT guide for the affected native/semantic, real-mpv, compatibility, mutation, and release-artifact commands. Documentation-only tasks should record why code gates were skipped. Tests that manipulate the desktop or kill processes must use isolated fixtures and verified ownership.

### Copy-ready agent assignment

```text
Implement task Axx from docs/audits/sorotte-post-v0.2.8-audit-2026-09-05.md.
Use the merged release-closure baseline; record and verify its exact SHA.
Recheck the cited source and existing tests before changing behavior.
Complete the task's implementation, acceptance criteria, and relevant
cross-layer validation. Preserve compatibility and existing evidence gates.
Work in an isolated branch/worktree. Coordinate the files listed in the
integration stream; do not modify another agent's working changes.
Deliver the fix, useful regression coverage, updated docs/catalogs where
needed, and a concise evidence report including skipped environments.
Do not expand this into a general refactor or change unrelated product scope.
```

## 6. Audit evidence and limitations

### Executed evidence

The following are audit results, not future acceptance commands:

| Check | Observed result | Scope |
|---|---|---|
| Public API application harness, baseline dependency versions, locked/offline | Redirect credential forwarded; duplicate saves reload old values; overlapping update loses unrelated value; pre-cancel still launches/waits for probe | Synthetic token, local files, two loopback listeners, test child executable |
| Windows protected-DACL fixture using product atomic writer | Protected true→false; explicit rules 1→0; inherited rules 0→6 | Isolated fixture only |
| Public API server runtime probe | Oversized List/update output, ancient-ping amplification, reconnect expiry rollback reproduced | In-memory actual ServerRuntime; serializer byte counts; injectable time |
| Server library tests | 425 passed | Fresh locked/offline build in a separate target directory |
| Critical-path policy probe | Adapter/coordinators/network/Plex/telemetry classified ordinary | Actual policy loader and matcher |
| JSON loader probe | Release loader accepts duplicate status; behavior loader rejects it | Direct current-code loader calls; not a complete gate bypass |
| Updated container policy tests | 47 passed | Covers the intervening latest-promotion commit |
| Selected Python policy/oracle suites | 312 passed in 8.975 seconds | Immutable export of 8b9ee43 with writable target; command listed below |

The initial in-place Python run stalled while setting up a target-directory fixture; a focused diagnostic also failed to complete. Those attempts are retained as incomplete environment diagnostics. Re-running the unmodified tests from an immutable Git archive in the writable workspace completed successfully. No product change or weakened assertion was needed. This is not evidence of a source-code test failure.

Exact focused Python command, run from the immutable export:

```powershell
python -B -m unittest scripts.tests.test_ci_policy scripts.tests.test_mutation_ci scripts.tests.test_ignored_test_policy scripts.tests.test_known_defect_policy scripts.tests.test_diff_coverage scripts.tests.test_diff_coverage_map scripts.tests.test_playback_lifecycle_model scripts.tests.test_playback_lifecycle_oracle scripts.tests.test_playback_release_gate scripts.tests.test_playback_lifecycle_evidence scripts.tests.test_playback_lifecycle_release_gate scripts.tests.test_protocol_fuzz_policy scripts.tests.test_nextest_ci scripts.tests.test_behavior_evidence -v
```

The separate updated-container check was python -B -m unittest scripts.tests.test_server_container_verification -q. The server check was cargo test --locked --offline -p sorotte-server --lib, with the audit's separate target directory. Full-workspace Clippy/tests, fresh coverage generation, mutation campaigns, live Python interop, native GUI, and real-mpv system suites were not re-run as part of this document-only audit.

Representative probe output:

```text
plex_request_success=true cross_origin_received_credential=true
precancelled_fingerprint success=false elapsed_ms=1249
overlapping_persistence requested_name=new-name requested_room=new-room
  reloaded_name=old-name reloaded_room=new-room
duplicate_key requested_name=new reloaded_name=last
duplicate_section requested_name=new reloaded_name=last

validated_signature input_bytes=9859 providers=57 list_bytes=566863
extension input_bytes=300058 three_member_list_bytes=600747
permanent_rooms=1024 list_bytes=623255
ancient_ping now=100 latencyCalculation=-1000
  requested_position=30 canonical_position=1680
reconnect detach_wall=100 observed_wall=400 rollback_wall=150
  ttl_seconds=180 original_epoch=1 restored_epoch=1
```

The frame probe observes actual product serialization; it does not itself run a complete GUI/CLI disconnect. Those consumers' inspected limits make the incompatibility explicit, and A02 requires the real transport regression. The token rollback preserves a readiness membership epoch only when the caller still possesses the token and matches username/room; restored technical readiness is reset. It is not an account-authentication takeover.

### Retained local reproduction material

The [durable evidence packet](2026-09-05-evidence/README.md) includes both Rust reproduction sources, observed outputs, policy-probe results, and the checked source-anchor fingerprints.

Audit scratch root: C:/Users/shaun/Documents/workspace/sorotte/target/audit-2026-09-05.

- app-harness/replay.ps1, src/main.rs, Cargo.toml, Cargo.lock, harness-output.txt, and acl-output.json: public API/HTTP/cancellation/persistence reproductions and Windows ACL fixture.
- runtime_probe.rs, runtime_probe.exe, runtime_probe_commands.ps1, and runtime_probe_output.txt: server serialization/timing/reconnect probes.
- inventory.py and inventory.json: tracked-file counts and source fingerprints.
- policy_probe.py and policy_probe.json: risk classification and parser discrepancy.
- python-audit-tests-writable-export.log and container-policy-tests.log: completed validation output; earlier incomplete diagnostics remain separately named.

These harnesses are diagnostic reproductions, not production fixes or substitutes for repository regressions. Their baseline paths must be changed deliberately if the checkout moves. They contain synthetic test values, not account credentials.

### Findings deliberately not asserted

- A successful release-gate bypass from a dropped event or duplicate JSON field.
- An observed orphaned mpv after parent exit, or arbitrary Lua code execution through a cache swap.
- A fresh dependency vulnerability inventory, a license-compliance certification, or a new weakness in existing container signature verification.
- A general GUI accessibility defect, bad Media Match autoplay policy, uncontrolled filesystem traversal, or broad stale-worker resurrection.
- That every old manual/maintenance test must become a blocking PR check, or that increasing coverage percentages alone will prevent the reproduced defects.

The remaining small lifecycle presentation concern—an old queued same-target failure arriving before a successor gets its attempt binding—was not reproduced. Keep it as a narrow characterization experiment if that code changes; do not let it justify a speculative ownership rewrite.

### Optional architecture decision after the concrete fixes

The GUI's installation documentation already calls out the absence of a pinned signing trust anchor for privileged updates. If independent GUI publisher authentication is desired, commission a separate design for key custody, rotation/revocation, signed metadata, rollback policy, and artifact verification. Existing checksums and container Cosign attestations should retain their current roles. This is a future trust-model decision, not an additional confirmed exploit or a prerequisite for all tasks in this report.
