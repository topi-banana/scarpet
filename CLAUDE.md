# CLAUDE.md

Guidance for Claude Code when working in this repository.

## What this is

Rust tooling for **Scarpet**, the scripting language of Minecraft's Carpet mod (`.sc` apps and `.scl` libraries). A Cargo workspace (edition 2024) of seven crates:

- `scarpet-syntax` — hand-written lexer + recursive-descent parser → a lossless `rowan` syntax tree (every byte of the source, including whitespace), lowered on demand to a compact CST that preserves comments and line breaks as **leading trivia** on each node (`cst.rs`). The tree's node shapes are specified in `scarpet.ungram`; the typed accessor layer (`nodes.rs`) is generated from it by the sourcegen test. Builds for `wasm32`.
- `scarpet-fmt` — formatter: lowers the CST to a Wadler/Lindig pretty-printing `Doc` IR, then renders it at a style set by a `Config` (indent width, max width).
- `scarpet-hir` — static analysis: lowers the **typed rowan layer** (not the CST — it has no spans) to a span-carrying arena HIR, then infers a type for every expression and reports diagnostics. `wasm`-clean, file-I/O-free, and public: the playground consumes it today, an LSP and a compiler are meant to. See `scarpet-hir/README.md`.
- `scarpet-vm` — tree-walking evaluator (early prototype): lowers the CST to an AST (`scarpet-syntax`'s `ast.rs`) and evaluates it — values, operators, assignment/destructuring, user-defined functions, and a few builtins. Driven by `scarpet repl`.
- `scarpet-cli` — `clap` CLI (`scarpet format`, `scarpet repl`, `scarpet lsp`). Built binary is `scarpet-cli`.
- `scarpet-lsp` — language server over stdio (`async-lsp`, not `tower-lsp`): formatting plus parse diagnostics.
- `scarpet-playground` — browser playground (Yew, **struct components**, Trunk + Tailwind 4). Two screens: a two-pane editor (Format / Syntax tree / AST / Types / Run) and a notebook over a persistent VM kernel.

Three paths share the syntax front end. Formatting is one-directional and non-destructive: `source → lexer → parser → rowan tree → CST (trivia) → lower → Doc → string`. Evaluation lowers the same CST to an AST and walks it (`scarpet repl`) — an early prototype. Analysis takes the *rowan tree* instead, because the CST and AST both drop spans: `source → parser → rowan tree → HIR → types + diagnostics`. The formatter remains the mature path.

## Commands

These mirror CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) — run them before declaring work done:

```sh
cargo fmt --all -- --check
taplo fmt --check --diff                          # TOML formatting (CI enforces it)
typos                                             # spell check (config: typos.toml; example/ excluded)
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

CI also builds `wasm32-unknown-unknown` (`scarpet-syntax` and `scarpet-hir`, `--lib` only); keep both crates `wasm`-clean (no `std::fs`, threads, etc. in library code).

## Invariants — do not break these

- **The formatter must stay non-destructive and idempotent.** Re-parsing formatted output must yield a structurally equal CST (`strip_trivia(a) == strip_trivia(b)`), and formatting twice must equal formatting once. The `corpus` test in `scarpet-fmt/src/lib.rs` enforces both across all corpus files. When changing `scarpet-fmt`, run `cargo test -p scarpet-fmt` — a corpus failure means a real regression, not a flaky test.
- **Trivia must never be silently dropped.** The rowan tree is lossless by construction (a test asserts `tree.text() == source`). The CST view attaches comments and breaks as `leading` trivia, and `cst.rs` goes to some length to anchor otherwise-orphaned trivia (trailing comments, comments in empty arg lists, around trailing commas) onto a node — often a phantom `CstKind::Empty`. Preserve this when touching `cst.rs` or `parser.rs`; the trivia-preservation tests in `parser.rs` are the spec, and the module comment in `cst.rs` documents the attachment rules (including a few deliberately preserved drop-quirks of the original parser that byte-identical formatting depends on).
- **The precedence ladder lives in `scarpet-syntax/src/parser.rs`** (documented as a comment above the `Parser` impl). It mirrors Scarpet's operator precedence. Changing it changes parse results — update the ladder comment and the precedence tests together.
- **`scarpet.ungram` and `src/nodes/generated.rs` move together.** The sourcegen test (`scarpet-syntax/tests/sourcegen.rs`) regenerates the typed node layer from the grammar and fails while it is stale — edit the grammar (and `SyntaxKind` in `syntax.rs` if node kinds change), run `cargo test -p scarpet-syntax`, commit the regenerated file.
- **`scarpet-hir` must never panic.** `analyze` is total — a parse error becomes a `Diagnostic`, and everything else degrades to `Ty::Unknown` rather than guessing. The corpus test in `scarpet-hir/src/lib.rs` runs every corpus file through it and checks span and id validity. Lowering, `dump`, and the playground's row flattening are iterative because `;`, `,` and `+` all left-nest; the inference walk is recursive but depth-capped.
- **`scarpet-hir` must stay `wasm`-clean.** The playground depends on it, and the `playground` CI job (`trunk build --release`) has no `continue-on-error`, so it is a hard gate regardless of the build matrix. No `std::fs`, threads, time, or env in library code — the corpus test's `std::fs` is `#[cfg(test)]`, as in `scarpet-fmt`.
- **`scarpet-hir` must not depend on `scarpet-vm`** — that would make a compiler depend on an interpreter. `scarpet-hir/src/infer/ops.rs` is a deliberate port of the VM's `value/*.rs` and each rule cites its source file; when the VM's operator semantics change, both move together.
- **The type lattice must stay finite.** `Ty` is recursive, so `Ty::list`/`Ty::map` truncate past `MAX_DEPTH` and `Ty::union` widens past `MAX_UNION`. That bound is not cosmetic: it is what makes `Drop` safe *and* what guarantees the inference fixpoint converges. Never construct `Ty::List`/`Ty::Map`/`Ty::Union` outside `ty.rs`.

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
- `scarpet-hir/src/builtins/names.rs` is **generated and committed**: `scarpet-hir/tests/sourcegen.rs` mines it from `example/gnembon/fabric-carpet/docs/scarpet/Full.md`. It is a test rather than a build script because the `build` and `playground` CI jobs check out without submodules. The test skips when `example/` is absent, so a green `cargo test` locally may mean it never ran.
- Two traps when walking `scarpet_syntax::nodes`: `Expr::can_cast(NAME_REF)` is `true`, so a hand-rolled `children().find_map(Expr::cast)` on a `DEFINE_FUNCTION` returns the *name* rather than the body (use the generated accessors); and `f(a, b)` does **not** produce a comma `BinExpr` — arguments are sibling children of `ARG_LIST`, so comma chains appear only under `ROOT` and inside `PAREN_EXPR`.
- `scarpet-fmt`'s `doc.rs` carries `#![allow(dead_code)]` because the `Doc` builder set is intentionally fuller than current usage; don't "clean up" unused builders.
