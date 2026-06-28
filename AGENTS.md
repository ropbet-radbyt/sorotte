# Repository Guidelines

## Project Structure & Module Organization
`sorotte` is a Rust workspace rooted in `crates/`. Core protocol and runtime crates live in `sorotte-protocol`, `sorotte-core`, `sorotte-client-core`, and `sorotte-server`. User-facing binaries live in `sorotte-cli` and `sorotte-gui`; player integration is split into `sorotte-player-api` and `sorotte-player-mpv`. Keep shared test data in `fixtures/`, repo docs in `docs/`, and automation in `scripts/` and `.github/workflows/`.

For detailed contributor workflow, test placement, and compatibility guidance, use `docs/DEVELOPMENT.md`. For parity work, treat the sibling Python checkout in `../syncplay/` as the behavioral reference.

## Build, Test, and Development Commands
- `cargo test --workspace` or `cargo test-all`: run the full Rust test suite.
- `cargo test -p sorotte-cli`: target one crate while iterating.
- `cargo fmt --all` and `cargo fmt-check`: format code or verify formatting.
- `cargo clippy --workspace --all-targets -- -D warnings` or `cargo lint`: fail on any lint warning.
- `cargo build -p sorotte-gui --bin sorotte-gui`: rebuild the desktop binary before native smoke tests.
- `powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json`: run GUI semantic scenarios.
- `powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 50000`: run Windows UI smoke coverage against the existing `target/debug/sorotte-gui.exe`.

## Coding Style & Naming Conventions
Use Rust `1.96.0` with edition `2024`. Follow `rustfmt` defaults: 4-space indentation, trailing commas where rustfmt adds them, and imports organized by the formatter. Use `snake_case` for modules, functions, and test names, `CamelCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Crate names stay `kebab-case`. Prefer small, test-backed vertical slices over large refactors.

## Testing Guidelines
Every behavior change should add coverage at the lowest sensible layer. Session/protocol changes belong in `sorotte-protocol`, `sorotte-client-core`, or `sorotte-server`. Config and startup changes belong in `sorotte-client-app` or `sorotte-cli`. GUI workflow changes should extend `crates/sorotte-gui/src/semantic_scenarios/*-flow.txt` plus semantic smoke; Windows rendering, accessibility, or startup changes should also run native smoke.

## Commit & Pull Request Guidelines
Recent commits use short imperative subjects such as `Fix playlist sync edge cases` or `Add playlist row keyboard shortcuts`. Keep commits focused on one slice of behavior. PRs should include the problem being solved, linked issue or compatibility gap, commands run, and any skipped validation. Add screenshots only for visible GUI changes, and update `README.md` or the relevant `docs/` guide when supported behavior changes.
