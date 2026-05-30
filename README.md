# scarpet

[![CI](https://github.com/topi-banana/scarpet/actions/workflows/ci.yml/badge.svg)](https://github.com/topi-banana/scarpet/actions/workflows/ci.yml)

*English | [日本語](README_jp.md)*

Rust tooling for [Scarpet](https://github.com/gnembon/fabric-carpet/blob/master/docs/scarpet/Documentation.md), the scripting language embedded in the [Carpet](https://github.com/gnembon/fabric-carpet) mod for Minecraft. Scarpet scripts (`.sc` files) drive in-game apps and server extensions; this repository provides a lexer, a trivia-preserving parser, and a code formatter for them.

> **Status:** early. The parser covers the full expression grammar and parses 98.6% of a 220-file real-world corpus; the formatter round-trips that corpus non-destructively. APIs are unstable.

## Workspace layout

This is a Cargo workspace of three crates plus a test corpus:

| Crate | What it is |
| --- | --- |
| [`scarpet-syntax`](scarpet-syntax) | Lexer ([`logos`](https://crates.io/crates/logos)) and parser ([`chumsky`](https://crates.io/crates/chumsky) via [`logosky`](https://crates.io/crates/logosky)) producing a CST that preserves comments and line breaks. Also builds for `wasm32`. |
| [`scarpet-fmt`](scarpet-fmt) | Code formatter. Lowers the CST to a Wadler/Lindig pretty-printing IR and renders it at a configurable style. |
| [`scarpet-cli`](scarpet-cli) | Command-line front end (`scarpet`), built on `clap`. Currently exposes `format`. |
| [`example/`](example) | Git submodules of community Scarpet scripts, used as a parse/format corpus. |

Data flows in one direction:

```
source (.sc) → lexer → parser → CST (with trivia) → fmt lower → Doc IR → formatted text
                                  └─ scarpet-syntax ─┘   └──────── scarpet-fmt ────────┘
```

## Getting started

Requires a recent stable Rust toolchain (edition 2024; developed against Rust 1.96).

```sh
# Clone with the corpus submodules (optional — only the corpus tests need them)
git clone --recurse-submodules git@github.com:topi-banana/scarpet.git
cd scarpet

# Or, if you already cloned without submodules:
git submodule update --init --recursive

cargo build --workspace
cargo test  --workspace
```

## Usage

The formatter reads from files or standard input. The binary is `scarpet-cli` (its `--help` calls itself `scarpet`).

```sh
# Format a file and print the result to stdout
cargo run -p scarpet-cli -- format script.sc

# Format from stdin
echo "print('hi')" | cargo run -p scarpet-cli -- format

# Rewrite files in place
cargo run -p scarpet-cli -- format --in-place src/*.sc

# Check formatting without writing (non-zero exit if any file differs)
cargo run -p scarpet-cli -- format --check src/*.sc

# Format with an explicit config file (otherwise scarpet-fmt.toml in the cwd is used)
cargo run -p scarpet-cli -- format --config scarpet-fmt.toml script.sc
```

Install it as a standalone binary:

```sh
cargo install --path scarpet-cli   # installs `scarpet-cli`
```

Exit codes: `0` success, `1` a parse error or a failed `--check`, `2` an I/O or configuration error.

### Formatting style

The style is configurable through a TOML file: `scarpet format` reads `scarpet-fmt.toml` from the current directory, or an explicit `--config <path>` (which takes precedence); with neither it falls back to the built-in defaults. Every key is optional.

```toml
# scarpet-fmt.toml
indent = 4               # indentation width, in spaces
max_width = 100          # line-length target before a group breaks
line_ending = "lf"       # newline style: "lf" (Unix, default) or "crlf" (Windows)
```

Unknown keys, `max_width = 0`, and a `line_ending` other than `"lf"` or `"crlf"` are rejected. Beyond these knobs the layout is fixed. Highlights:

- Binary operators are spaced (`a + b`, `a -> b`), except `:` (get), which is tight: `a:b`. Unary prefixes hug their operand: `-x`, `!x`, `...xs`.
- `;` statement sequences are laid out one per line, each terminated with `;`. A parenthesized `;`-chain becomes an indented block.
- Lists, maps, and call arguments stay on one line when they fit, otherwise break to one item per line with a trailing comma.
- Comments are preserved. A comment on its own line stays on its own line; a trailing comment stays on the line it followed. Runs of blank lines collapse to a single blank line.
- Output always ends in exactly one newline, with no trailing whitespace.

```sc
foo()->(a;b)
```

formats to

```sc
foo() -> (
    a;
    b;
)
```

The formatter is **non-destructive** (re-parsing its output yields a structurally identical tree) and **idempotent** (formatting twice is the same as once). Both properties are enforced against the whole corpus in CI.

## The corpus

[`example/`](example) vendors nine community Scarpet repositories as git submodules — 220 `.sc` files in total. They are used two ways:

- **Parse rate.** A standalone runner parses every file and reports how many succeed. It is a progress metric, not a gate (it always exits 0; the three known upstream syntax errors are listed in the runner).

  ```sh
  cargo run -p scarpet-syntax --bin corpus            # human-readable summary
  cargo run -p scarpet-syntax --bin corpus -- --markdown   # CI/PR Markdown report
  ```

- **Formatter safety.** A test (`scarpet-fmt`'s `corpus` module) formats every parseable file and asserts the result re-parses to a structurally equal tree and is idempotent. It skips quietly if the submodules are not checked out.

## Development

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs the following gates on every push and pull request, and builds for both `x86_64-unknown-linux-gnu` and `wasm32-unknown-unknown`. To reproduce locally:

```sh
cargo fmt --all -- --check                       # rustfmt
taplo fmt --check --diff                          # TOML formatting
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace --all-targets
cargo machete                                     # no unused dependencies
cargo test --workspace --all-targets
```

Dependency updates are managed by Dependabot. Results of each CI run are posted as a sticky summary comment on the pull request.

## License

Not yet specified. The repositories under [`example/`](example) are third-party submodules and retain their own licenses.
