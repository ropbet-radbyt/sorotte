# syncplay-rs

Rust rewrite workspace for Syncplay.

## Current status

- Phase 0 bootstrap completed.
- Phase 1 protocol foundation started and passing checks.

Implemented:
- workspace crate layout and baseline CI/tooling
- typed protocol models for all top-level message families
- protocol fixtures for `Hello`, `Set`, `List`, `State`, `Chat`, `Error`, `TLS`
- typed decode integration in `syncplay-client-core` and fixture decoding in `syncplay-compat`

Detailed continuity checkpoint:
- `../NEXT_AGENT_HANDOFF.md`

## Quick commands

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
