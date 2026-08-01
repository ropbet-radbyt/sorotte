# Pinned mpv version matrix — 2026-08-01

## Outcome and scope

Sorotte's focused Linux real-player lane now distinguishes the advertised
compatibility floor from reviewed post-release upstream behavior. Ordinary
pull-request and push runs keep the minimum endpoint on the required path.
Nightly and manually dispatched runs expand the same required job to both
endpoints:

| Identity | Exact official source | Role |
|---|---|---|
| `minimum` | `41f6a645068483470267271e1d09966ca3b9f413` | peeled commit for mpv `v0.41.0`; proves the support floor |
| `newest` | `d12f2ce19c918875981e00ed276f153bdf40a2ac` | reviewed post-release snapshot; proves forward behavior on Ubuntu 24.04's native dependency set |

The newest endpoint is 330 commits ahead of the minimum and zero commits
behind. It is intentionally immutable. The matrix never checks out a floating
tag or branch.

The matrix implementation source boundary is:

```text
64255fe97ccb126eb275074166b9d551dee306ce
```

The standalone version-validator correction boundary is:

```text
5a94562d18182058c5a322bbe0f627a15b6f1cc6
```

## Matrix contract

The `mpv-pr-semantics` job keeps one aggregate job identity, uses
`fail-fast: false`, and exposes the selected endpoint in its display name. A
selected endpoint must:

1. check out the exact source SHA into its isolated runner;
2. verify `HEAD^{commit}` against the selected SHA;
3. build mpv with the same headless feature set and Lua enabled;
4. validate a release or development version line as mpv `0.41.0` or newer;
5. require the minimum endpoint to report the exact minimum tuple; and
6. execute all four existing real-player contracts without a tolerated error.

The four contracts are:

```text
real_mpv_pause_seek_resume_semantics
real_mpv_cache_cap_drains_and_input_resumes
real_mpv_premature_http_disconnect_recovers_same_media_generation
real_mpv_clients_keep_seek_recovery_bounded_during_an_http_stall
```

The ordinary required aggregate still consumes `mpv-pr-semantics` once.
GitHub reports the matrix job successful only when every selected endpoint is
successful, so adding the second manual/nightly endpoint does not create an
optional result path.

## Upstream dependency boundary

The official latest release remained `v0.41.0` on 2026-08-01. Merely testing
the latest release would therefore duplicate the minimum endpoint. Initial
preflight selected official master
`1d15686142fd5d53c954aab7526cedab05ef9dc3`, 918 commits after the release.
Its Meson configuration failed before compilation because it requires
libplacebo `>=7.360.1`, while Ubuntu 24.04 provides `6.338.2`.

The requirement changed in mpv commit
`022fbd16b99187d51f1961da788c2720cf3036ec`. Its first parent, the selected
`d12f2ce19c918875981e00ed276f153bdf40a2ac`, is the final official snapshot
that retains the runner's `>=6.338.2` dependency floor. This keeps the lane
about Sorotte/mpv behavior rather than silently adding a second source-built
graphics dependency stack.

The rejected-master diagnostic is preserved at:

```text
target/verification/mpv-version-matrix-preflight/20260801-newest/source/build/meson-logs/meson-log.txt
bytes:   15,918
sha256:  0181c13852af1e25729487ad4e869cb1cf9e6e7de1fa96db61b5d260afb1d3dd
```

## Hosted validator diagnostic and correction

Manual exact-head run
[`30673144701`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30673144701)
expanded both endpoints on documentation head
`b0f4821934ca601ddea57428d3ebc7e83495bf14`. Minimum job `91294887252`
and newest job `91294887263` both checked out and verified their exact source,
installed dependencies, and built mpv. Both then failed only at `Verify
supported mpv version`, before any Sorotte semantic contract ran.

The embedded regular expression placed a word-boundary assertion immediately
between the optional `v` and the first digit. Both are word characters, so it
rejected the valid release and development forms `mpv v0.41.0` and `mpv
v0.41.0-dev-gd12f2ce19`. The failed run was cancelled only after both endpoint
results and the parser diagnostic were retained.

Commit `5a94562d18182058c5a322bbe0f627a15b6f1cc6` moves source and version
validation into `scripts/mpv_version_matrix.py`. It accepts a bounded optional
`v`, retains exact lowercase 40-character source SHA checks, rejects unknown,
floating, collapsed, and drifted identities, requires an exact three-part
minimum tuple, and preserves exact-minimum/newer-than-minimum semantics. Five
unit tests cover release, development, unprefixed, malformed, partial,
embedded, older, newer, and source-drift cases. The committed validator also
accepted the preserved local binary's exact development line and emitted its
source-bound JSON record.

Corrected exact-head run
[`30673650173`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30673650173)
was bound to that commit. Minimum job
[`91296358144`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30673650173/job/91296358144)
and newest job
[`91296358146`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30673650173/job/91296358146)
both passed source verification, build, version validation, and all four
real-player contracts.

## Canonical local campaign

The selected snapshot built locally as:

```text
mpv v0.41.0-dev-gd12f2ce19
```

The WSL image lacked only Lua 5.2 development headers and did not permit
noninteractive sudo. The matching Ubuntu packages were therefore downloaded
and extracted below the preserved preflight root without modifying the WSL
system. The exact build used `-Dlua=enabled`, and `ldd` resolved
`liblua5.2.so.0` from that root.

Artifact identities:

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Lua-enabled `build-lua/mpv` | 8,638,872 | `831ab9e1b77b67f1c7b9aaab7841f8c820e9f1e6f81128183d258f07f97db0d0` |
| `liblua5.2-dev_5.2.4-3build2_amd64.deb` | 1,815,372 | `56125948d28dbaefc920563adeaeee90599ea67f2960932b608c582a00ceddd2` |
| `liblua5.2-0_5.2.4-3build2_amd64.deb` | 123,358 | `b030bc0bea32fe27dfb70a31e14491b6ab767379811877e6497af28338eab5d8` |

An exploratory Lua-disabled build first passed three contracts and correctly
failed `real_mpv_cache_cap_drains_and_input_resumes`: without the Lua
hook/readback path, `verification_complete` remained false. No assertion was
weakened. The Lua-enabled implementation-source campaign then passed 4/4 in
74.91 seconds. After commit `64255fe97ccb126eb275074166b9d551dee306ce`,
the canonical committed-source rerun passed 4/4 in 75.07 seconds.

All generated source, build, dependency, and Cargo artifacts remain preserved
under:

```text
target/verification/mpv-version-matrix-preflight/
```

## Policy coverage

The CI policy suite binds the exact event matrix, both source SHAs, the
selected-source expression, checkout path, source verifier, minimum version,
and version-validation contract. Adversarial copies must fail when they:

- remove the newest endpoint;
- replace the newest SHA with floating `master`;
- collapse minimum and newest onto one source; or
- enable matrix fail-fast and erase the other endpoint's result.

Focused results:

```text
scripts.tests.test_mpv_version_matrix + scripts.tests.test_ci_policy: 21/21 passed
actionlint .github/workflows/rust-ci.yml: passed
git diff --check: passed
```

Final local gates:

```text
cargo fmt --all --check: passed
cargo test --locked --workspace --all-features: passed, including doctests
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings: passed
python -m unittest discover -s scripts/tests -p "test_*.py": 542/542 passed
```

## Limits

This closes the source-built Linux semantic/recovery matrix. It does not claim
that every intermediate mpv revision is supported, nor does it duplicate the
complete Rust suite per endpoint. The four tests use generated local media and
loopback fault servers; external YouTube remains outside required evidence.

The separate native Windows GUI vertical still has one locally supplied mpv
identity. Repeating its digest-, process-, Hello-, screenshot-, and artifact-
bound HTTP fault modes with distinct minimum/newest Windows executables
remains a separate operational investment.
