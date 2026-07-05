<!--
  Thanks for contributing to scarpet!

  This template mirrors the structure most PRs in this repo already use.
  Fill in each section, then DELETE any section that does not apply to your
  change. The HTML comments (like this one) are guidance only — they are not
  rendered on the PR page, so you may leave them in or strip them out.

  Descriptions may be written in English or Japanese, whichever you prefer.

  Keep the title in Conventional Commits form, scoped by crate, e.g.:
    feat(scarpet-fmt): make indent width and max width configurable
    fix(scarpet-syntax): anchor trailing comments in empty arg lists
    feat(scarpet-vm): support destructuring assignment
    refactor(scarpet-cli): move TOML parsing into ConfigFile
-->

## Summary

<!--
  What does this PR do, and why? One or two short paragraphs.
  If it advances a README roadmap item, say which one. Link related issues:
  e.g. "Related issue: #123" or "Related issue: N/A".
-->

## What changed

<!--
  The concrete changes, ideally grouped by file or module, e.g.:
    - scarpet-syntax: `lexer.rs`, `parser.rs`, `cst.rs`, `nodes.rs`,
      `ast.rs`, `scarpet.ungram`
    - scarpet-fmt:    `doc.rs` (Doc IR), lowering, rendering, `Config`
    - scarpet-vm:     evaluator, builtins
    - scarpet-cli:    `scarpet format` / `scarpet repl`, `ConfigFile`
  A small table or bullet list per area works well. Include a short
  before/after example or `.sc` snippet when it clarifies the behavior.
-->

## Invariants

<!--
  REQUIRED when touching the lexer, parser, CST, or formatter; delete otherwise.

  State how this change relates to the invariants in CLAUDE.md:

    - Lossless tree — the rowan tree still reproduces the source byte-for-byte
      (`tree.text() == source`).
    - Trivia preservation — comments and line breaks are never silently
      dropped; explain any new trivia-anchoring (leading trivia, phantom
      `CstKind::Empty`) in `cst.rs`.
    - Formatter fidelity & idempotency — re-parsing formatted output yields a
      structurally equal CST (`strip_trivia(a) == strip_trivia(b)`), and
      formatting twice equals formatting once. The `corpus` round-trip test
      must stay green.
    - Precedence ladder — if you touch the ladder in `parser.rs`, update the
      ladder comment and the precedence tests together.
    - Grammar / generated nodes — `scarpet.ungram` and
      `src/nodes/generated.rs` move together (the sourcegen test regenerates
      and fails while stale).
    - wasm32 compat — `scarpet-syntax --lib` stays wasm-clean (no `std::fs`,
      threads, etc. in library code).

  If you relax a preservation guarantee, spell out exactly what is preserved
  and confirm idempotency and comment preservation still hold.
-->

## Testing

<!--
  How did you verify this? List new/updated tests (unit, corpus round-trip,
  sourcegen, precedence, trivia-preservation) and what they cover. Mention any
  manual / end-to-end checks (e.g. `scarpet format` on a sample `.sc` file, or
  a `scarpet repl` session).

  The CI gates, for reference (run before declaring work done):

    cargo fmt --all -- --check
    taplo fmt --check --diff
    typos
    cargo clippy --workspace --all-targets -- -D warnings
    cargo check --workspace --all-targets
    cargo machete
    cargo test --workspace --all-targets

  If your change touches scarpet-fmt, confirm `cargo test -p scarpet-fmt`
  passes (the corpus round-trip needs the `example/` submodule checked out:
  `git submodule update --init --recursive`).
-->
