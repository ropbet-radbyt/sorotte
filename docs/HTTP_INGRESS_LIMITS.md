# HTTP and update ingress limits

Plex metadata uses an explicit same-origin redirect policy. Origin means the
scheme, canonical host, and effective port. Relative redirects and HTTP
301/302/303/307/308 work within that origin, up to ten requests in a redirect
chain. Every hop is checked against the original origin. A different host,
port, HTTPS downgrade, or URL user information fails before forwarding the
credential. Errors omit redirect destinations. The token header is also
marked sensitive for diagnostics.

PIN authentication follows the same origin restriction on its configured
endpoint. Explicit LAN HTTP server configuration continues to work. Playback
URL construction remains a separate boundary: absolute media-part URLs must
match the configured server origin before the token is appended. These
changes preserve logical `plex://` room identities and do not change mpv's
playback ownership.

The application applies these resource budgets to successful response bytes,
including chunked or lengthless transfers. Non-success responses are rejected
without collecting their bodies.

| Input | Budget |
|---|---|
| Plex metadata response | 8 MiB |
| Plex section inventory plus one search's fallback queries | 32 MiB, 64 HTTP requests, 60 seconds |
| GUI public servers, GitHub release/artifact metadata and manifests | 2 MiB |
| Downloaded update archive, including an outer Actions artifact | 512 MiB |
| One uncompressed ZIP entry | 512 MiB |
| All extracted bytes, including outer and inner Actions archives | 2 GiB |
| All ZIP entries, including outer and inner Actions archives | 4,096 |
| One ZIP central directory or ZIP64 extension record | 8 MiB |

The metadata budgets allow large Plex library/search responses and release
inventories while limiting the amount parsed at once. The archive budgets
allow binary packages and nested CI artifacts with substantial headroom;
packages exceeding them require an explicit reviewed limit increase.

Downloads run on the existing update worker with separate 10-second connect,
30-second idle, and 30-minute overall deadlines. A progressing download can
outlive the metadata request timeout. Cancellation is polled every 50 ms
while awaiting network progress and between disk writes and extracted chunks.
Changing update settings or dropping the update owner signals that worker;
a cancelled generation cannot launch the updater. Ordinary CDN redirects use
reqwest's standard Authorization-header stripping rather than Plex's custom
credential header.

Each download gets a fresh directory created with a protected current-user
DACL on Windows or mode 0700 on Unix. Packages stream to disk while computing
SHA-256. A failed or cancelled attempt removes only its own partial stage,
preserving older staging/rollback material and the installed app.

ZIP end records are checked before the ZIP library allocates its inventory.
Update packages use ordinary ZIP or bounded ZIP64 offsets without executable
prefixes. Their directory/comments must contain one unambiguous set of end
records. Inventory construction cannot fall back to an earlier directory
embedded in a binary payload; extraction still reads the original payload
bytes unchanged. Index parsing has its own read budget.
Extraction checks declared sizes first and counts actual emitted bytes as
well. Path traversal, symbolic links, Windows device-name aliases, and
duplicate normalized names fail. Outer and inner archives share one quota.
The installed updater independently rechecks these archive limits; it retains
its bounded authenticated immutable package snapshot and existing journaled
replacement/recovery protocol.

The boundary tests live in `crates/sorotte-plex/src/tests/http_boundaries.rs`,
`crates/sorotte-gui/src/app/remote_services/ingress_tests.rs`,
`crates/sorotte-gui/src/app/remote_services/download.rs`,
`crates/sorotte-gui/src/update_limits.rs`, and
`crates/sorotte-gui/src/app/runtime_owner/updates/tests.rs`.

The Plex regression was first run against audit baseline
`8b9ee43b52d9f6049ff5d44a41161fde5b97529a`: the second loopback origin was
contacted. It passes with the policy fix. The matrix also uses a verified
fixture certificate to exercise a real HTTPS-to-HTTP redirect, without
disabling certificate checks. HTTP and ZIP fixtures use small limits or
synthetic declared sizes to exercise failure without exhausting the host.
