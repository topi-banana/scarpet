# CLAUDE.md

Guidance for Claude Code when working in this repository.

## What this is

Rust tooling for **Scarpet**, the scripting language of Minecraft's Carpet mod (`.sc` files). A Cargo workspace (edition 2024) of four crates:

- `scarpet-syntax` — lexer (`logos`) + parser (`chumsky` via the `logosky` bridge) → a CST that preserves comments and line breaks as **leading trivia** on each node. Builds for `wasm32`.
- `scarpet-fmt` — formatter: lowers the CST to a Wadler/Lindig pretty-printing `Doc` IR, then renders it at a style set by a `Config` (indent width, max width).
- `scarpet-vm` — tree-walking evaluator (early prototype): lowers the CST to an AST (`scarpet-syntax`'s `ast.rs`) and evaluates it — values, operators, assignment/destructuring, user-defined functions, and a few builtins. Driven by `scarpet repl`.
- `scarpet-cli` — `clap` CLI (`scarpet format`, `scarpet repl`). Built binary is `scarpet-cli`.

Data flow (formatting): `source → lexer → parser → CST (trivia) → lower → Doc → string` — one-directional and non-destructive. A second, experimental path evaluates rather than formats: `scarpet-vm` lowers the same CST to an AST and walks it (`scarpet repl`). The evaluator is an early prototype; the formatter remains the mature path.

## Commands

These mirror CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) — run them before declaring work done:

```sh
cargo fmt --all -- --check
taplo fmt --check --diff                          # TOML formatting (CI enforces it)
cargo clippy --workspace --all-targets -- -D warnings   # warnings are errors
cargo check --workspace --all-targets
cargo machete                                     # fails on unused dependencies
cargo test --workspace --all-targets
```

Other useful invocations:

```sh
cargo run -p scarpet-cli -- format <file>         # format to stdout (also: --in-place, --check, --config, stdin)
git submodule update --init --recursive           # fetch example/ corpus (needed for corpus tests)
```

CI also builds `wasm32-unknown-unknown` (only `scarpet-syntax --lib`); keep that crate `wasm`-clean (no `std::fs`, threads, etc. in library code).

## Invariants — do not break these

- **The formatter must stay non-destructive and idempotent.** Re-parsing formatted output must yield a structurally equal CST (`strip_trivia(a) == strip_trivia(b)`), and formatting twice must equal formatting once. The `corpus` test in `scarpet-fmt/src/lib.rs` enforces both across all ~220 corpus files. When changing `scarpet-fmt`, run `cargo test -p scarpet-fmt` — a corpus failure means a real regression, not a flaky test.
- **Trivia must never be silently dropped.** Comments and breaks are attached as `leading` trivia. The parser goes to some length to anchor otherwise-orphaned trivia (trailing comments, comments in empty arg lists, around trailing commas) onto a node — often a phantom `CstKind::Empty`. Preserve this when touching `parser.rs`; the trivia-preservation tests there are the spec.
- **The precedence ladder lives in `scarpet-syntax/src/parser.rs`** (documented as a comment above `top_parser`). It mirrors Scarpet's operator precedence. Changing it changes parse results — update the ladder comment and the precedence tests together.

## Conventions

- Edition 2024. Let-chains (`if x && let Some(y) = ...`) are used and expected to compile — needs a recent toolchain (developed against Rust 1.96).
- `clippy -D warnings`, `rustfmt`, and `taplo` (TOML) are hard gates. No unused dependencies (`cargo machete`).
- Shared dep versions live in the root `[workspace.dependencies]`; member crates reference them with `.workspace = true`.
- The corpus parse rate is a **metric, not a gate** — CI's `format parse` step formats every `example/` file and reports how many parse, always succeeding (a few known-broken upstream files are expected). The `scarpet-fmt` round-trip test, by contrast, *is* a gate, so it skips those ~3 files via a `KNOWN_BAD` list (`scarpet-fmt/src/lib.rs`); keep that list current as the corpus changes.
- Prefer adding tests next to the code (the crates use inline `#[cfg(test)] mod tests`). Match the existing style: small focused unit tests plus the corpus round-trip for breadth.
- Formatting style is a `scarpet_fmt::Config` (indent width, max width) threaded into rendering; its `Default` reproduces the original fixed style (4-space indent, 100 columns). **TOML parsing lives in `scarpet-cli`** (`ConfigFile` → `Config`) so `scarpet-fmt` stays file-I/O-free and `wasm`-clean — add new knobs to `Config` and the CLI's `ConfigFile` together, not by reading files in the library.

## Gotchas

- `example/` is git submodules. Tests that need it skip quietly when it is absent, so a passing `cargo test` locally may simply have skipped the corpus — confirm submodules are checked out when validating formatter changes.
- In the lexer, `$` lexes as a `Break` (Scarpet uses `$` as a newline stand-in in one-liners), and `//` runs to end of line.
- `scarpet-fmt`'s `doc.rs` carries `#![allow(dead_code)]` because the `Doc` builder set is intentionally fuller than current usage; don't "clean up" unused builders.
