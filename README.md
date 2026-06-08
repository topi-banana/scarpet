# scarpet

[![CI](https://github.com/topi-banana/scarpet/actions/workflows/ci.yml/badge.svg)](https://github.com/topi-banana/scarpet/actions/workflows/ci.yml)

*English | [日本語](README_jp.md)*

Rust tooling for [Scarpet](https://github.com/gnembon/fabric-carpet/blob/master/docs/scarpet/Documentation.md), the scripting language embedded in the [Carpet](https://github.com/gnembon/fabric-carpet) mod for Minecraft. Scarpet scripts (`.sc` files) drive in-game apps and server extensions; this repository provides a lexer, a trivia-preserving parser, a code formatter, and an experimental evaluator for them.

> **Status:** early. The parser covers the full expression grammar and parses 98.6% of a 220-file real-world corpus; the formatter round-trips that corpus non-destructively. A tree-walking evaluator (`scarpet-vm`) is an early prototype, driven through the CLI's `repl`. APIs are unstable.

## Workspace layout

This is a Cargo workspace of four crates plus a test corpus:

| Crate | What it is |
| --- | --- |
| [`scarpet-syntax`](scarpet-syntax) | Lexer ([`logos`](https://crates.io/crates/logos)) and parser ([`chumsky`](https://crates.io/crates/chumsky) via [`logosky`](https://crates.io/crates/logosky)) producing a CST that preserves comments and line breaks. Also builds for `wasm32`. |
| [`scarpet-fmt`](scarpet-fmt) | Code formatter. Lowers the CST to a Wadler/Lindig pretty-printing IR and renders it at a configurable style. |
| [`scarpet-vm`](scarpet-vm) | Tree-walking evaluator — an early prototype. Lowers the CST to an AST and evaluates it: values, operators, assignment and destructuring, user-defined functions, and a few builtins (`type`, `str`, `print`, `call`, `if`, `range`). |
| [`scarpet-cli`](scarpet-cli) | Command-line front end (`scarpet`), built on `clap`. Exposes `format` and an interactive `repl`. |
| [`example/`](example) | Git submodules of community Scarpet scripts, used as a parse/format corpus. |

Two pipelines share the syntax front end. Formatting is one-directional and non-destructive:

```
source (.sc) → lexer → parser → CST (with trivia) → fmt lower → Doc IR → formatted text
                                  └─ scarpet-syntax ─┘   └──────── scarpet-fmt ────────┘
```

Evaluation (experimental) lowers the same CST to an AST and walks it:

```
source (.sc) → lexer → parser → CST → AST lower → evaluate → value
                                  └─ scarpet-syntax ─┘ └─ scarpet-vm ─┘
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

# Check formatting without writing — prints a diff for each unformatted file
cargo run -p scarpet-cli -- format --check src/*.sc

# Same, but exit non-zero when any file differs (for CI), like clippy's `-D warnings`
cargo run -p scarpet-cli -- format --check -D warnings src/*.sc

# Format with an explicit config file (otherwise scarpet-fmt.toml in the cwd is used)
cargo run -p scarpet-cli -- format --config scarpet-fmt.toml script.sc
```

Install it as a standalone binary:

```sh
cargo install --path scarpet-cli   # installs `scarpet-cli`
```

Exit codes: `0` success — including unformatted files under `--check`, which print a diff but do not fail by default; `1` a parse error, or a `--check` difference when `-D warnings` is set; `2` an I/O or configuration error.

### Formatting style

The style is configurable through a TOML file: `scarpet format` reads `scarpet-fmt.toml` from the current directory, or an explicit `--config <path>` (which takes precedence); with neither it falls back to the built-in defaults. Every key is optional.

```toml
# scarpet-fmt.toml
indent = 4                # indentation width, in spaces
max_width = 100           # line-length target before a group breaks
comment_width = -1        # comment line-length target; -1 disables wrapping
line_ending = "lf"        # newline style: "lf" (default), "crlf", "auto" (match source), or "native" (host OS)
brace_style = "same_line" # opening delimiter of a broken block: "same_line" (default) or "next_line"
```

Unknown keys, `max_width = 0`, `comment_width = 0` or less than `-1`, a `line_ending` other than `"lf"`, `"crlf"`, `"auto"`, or `"native"`, and a `brace_style` other than `"same_line"` or `"next_line"` are rejected. Beyond these knobs the layout is fixed. Highlights:

- Binary operators are spaced (`a + b`, `a -> b`), except `:` (get), which is tight: `a:b`. Unary prefixes hug their operand: `-x`, `!x`, `...xs`.
- `;` statement sequences are laid out one per line, each terminated with `;`. A parenthesized `;`-chain becomes an indented block.
- Lists, maps, and call arguments stay on one line when they fit, otherwise break to one item per line with a trailing comma.
- Comments are preserved. A comment on its own line stays on its own line; a trailing comment stays on the line it followed. When `comment_width` is positive, long `//` comments wrap to that width; `-1` leaves them unwrapped. Runs of blank lines collapse to a single blank line.
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

With `brace_style = "next_line"`, the opening delimiter of a broken call or function body instead starts its own line (Allman-style); a block that already fits on one line is unaffected:

```sc
foo() ->
(
    a;
    b;
)
```

The formatter is **non-destructive** (re-parsing its output yields a structurally identical tree) and **idempotent** (formatting twice is the same as once). Both properties are enforced against the whole corpus in CI — under both brace styles.

### The REPL (experimental)

`scarpet repl` starts an interactive read–eval–print loop backed by `scarpet-vm`, the prototype evaluator. Each submission is parsed, lowered to an AST, and evaluated in a session VM whose variables and function definitions persist across submissions; the resulting value is printed, or a rustc-style diagnostic on a parse, lowering, or evaluation error.

```sh
cargo run -p scarpet-cli -- repl
```

```
scarpet> 1 + 2 * 3
Single(Int(7))
scarpet> a = [1, 2, 3]
Single(List(ArrayList([Int(1), Int(2), Int(3)])))
scarpet> foo(x) -> x * x
Single(String("foo"))
scarpet> foo(4)
Single(Int(16))
```

A submission can span several lines: it stays open while a bracket is unclosed, and Shift+Enter (or Alt+Enter) forces a newline. Enter submits, Ctrl+C abandons the current submission, and Ctrl+D exits. With non-terminal input the prompt and banner are dropped and one statement is read per line, so `echo 'a = 5; a + 1' | scarpet repl` prints just the value.

The evaluator is an early prototype: it covers arithmetic, comparison and equality, unary and `match` operators, element access, list and map literals, assignment and destructuring, user-defined functions, and the `type`, `str`, `print`, `call`, `if`, and `range` builtins. Values currently print in their `Debug` form, and much of Scarpet's standard library is not yet implemented.

## The corpus

[`example/`](example) vendors nine community Scarpet repositories as git submodules — 220 `.sc` files in total. They are used two ways:

- **Parse rate.** CI's `format parse` step formats every file and reports how many parse. It is a progress metric, not a gate (it always succeeds; a few known upstream syntax errors are expected).

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
