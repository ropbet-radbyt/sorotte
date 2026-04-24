# Repository Guidelines

## Project Structure & Module Organization
`syncplay-rs` is a Rust workspace rooted in `crates/`. Core protocol and runtime crates live in `syncplay-protocol`, `syncplay-core`, `syncplay-client-core`, and `syncplay-server`. User-facing binaries live in `syncplay-cli` and `syncplay-gui`; player integration is split into `syncplay-player-api` and `syncplay-player-mpv`. Keep shared test data in `fixtures/`, repo docs in `docs/`, and automation in `scripts/` and `.github/workflows/`.

For parity work, treat the sibling Python checkout in `../syncplay/` as the behavioral reference, not the archived notes in `../old-docs/`.

## Build, Test, and Development Commands
- `cargo test --workspace` or `cargo test-all`: run the full Rust test suite.
- `cargo test -p syncplay-cli`: target one crate while iterating.
- `cargo fmt --all` and `cargo fmt-check`: format code or verify formatting.
- `cargo clippy --workspace --all-targets -- -D warnings` or `cargo lint`: fail on any lint warning.
- `cargo build -p syncplay-gui --bin syncplay-gui`: rebuild the desktop binary before native smoke tests.
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`: run GUI semantic scenarios.
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`: run Windows UI smoke coverage against the existing `target/debug/syncplay-gui.exe`.

## Coding Style & Naming Conventions
Use Rust `1.95.0` with edition `2024`. Follow `rustfmt` defaults: 4-space indentation, trailing commas where rustfmt adds them, and imports organized by the formatter. Use `snake_case` for modules, functions, and test names, `CamelCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Crate names stay `kebab-case`. Prefer small, test-backed vertical slices over large refactors.

## Testing Guidelines
Every behavior change should add coverage at the lowest sensible layer. Session/protocol changes belong in `syncplay-protocol`, `syncplay-client-core`, or `syncplay-server`. Config and startup changes belong in `syncplay-client-app` or `syncplay-cli`. GUI workflow changes should extend `crates/syncplay-gui/src/semantic_scenarios/*-flow.txt` plus semantic smoke; Windows rendering, accessibility, or startup changes should also run native smoke.

## Commit & Pull Request Guidelines
Recent commits use short imperative subjects such as `Fix playlist sync edge cases` or `Add playlist row keyboard shortcuts`. Keep commits focused on one slice of behavior. PRs should include the problem being solved, linked issue or parity gap, commands run, and any skipped validation. Add screenshots only for visible GUI changes, and update `README.md` or `PROJECT_STATUS.md` when supported behavior changes.
