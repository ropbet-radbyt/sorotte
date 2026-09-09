# Settings replacement fixture and bounded contention

The default Windows workspace job in [stable run 34324960316](https://github.com/ropbet-radbyt/sorotte/actions/runs/34324960316/job/102380056878)
failed `cooperating_readers_observe_complete_documents_through_repeated_replacement`
on source `343933b2eaae26d26b60da0f779c22904583bc71`. The reader received the
documented `io::ErrorKind::WouldBlock` result after the settings lock's five-second
deadline. Its unconditional `unwrap()` turned that result into a test failure.
The same default workspace had passed on the PR candidate and merged source.

The fixture continuously reacquired the writer lock for twenty durable
replacements while another thread repeatedly opened shared read transactions.
It assumed that every competing acquisition would succeed within the production
deadline. The lock API promises a bounded busy result, not fairness across that
burst. The retained failure shows contention; it does not identify the hosted
scheduling or filesystem delays that exhausted the deadline.

The fixture now gives each publication a finite reader phase. Every successful
concurrent snapshot must still equal a complete before or after document. Only
the typed busy outcome is accepted during contention, and it is counted
separately from snapshots. After a publication attempt, the fixture stops and
joins the reader. A busy writer may then retry once with its competitor gone.
Each of the twenty rounds requires a fresh read containing exactly the expected
published document. Persistent contention, missing documents, torn contents and
other I/O failures remain test failures.

The raw-filesystem probes still reject every read error. The existing Windows
raw-open probe remains explicitly ignored for its recorded NTFS namespace gap;
this repair adds no ignore or relaxed production timeout. Existing deterministic
tests force shared/exclusive contention and require a busy reader to fail before
inspecting provisional settings:

- `shared_readers_overlap_while_excluding_a_writer`
- `busy_reader_errors_before_reading_instead_of_returning_missing_settings`

The failed release attempt was cancelled before the GUI, container or server
publishers ran. Its job log, cancellation snapshot and native cleanup receipt
are retained under `target/verification/v0.2.12-release/`.

Local validation passed the full default workspace (4,290 tests; 18 registered
ignores), client-app all-feature checks (276 passed; two registered ignores),
all-feature Clippy, formatting and static preflight. Eight simultaneous owned
fixture processes verified 160 published documents and 3,773 concurrent
snapshots, and all eight cleanup receipts passed. Those stress runs observed no
busy outcome; the original hosted trace and the existing forced-contention
regressions establish that part of the contract. Hosted qualification of this
repair remains required before merge and publication.
