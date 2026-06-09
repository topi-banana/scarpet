# scarpet-fmt

The code formatter for [Scarpet](https://github.com/gnembon/fabric-carpet/blob/master/docs/scarpet/Documentation.md). It lowers the trivia-preserving CST produced by [`scarpet-syntax`](../scarpet-syntax) to a Wadler/Lindig pretty-printing IR and renders it at a configurable style.

> Part of the [`scarpet`](..) workspace. See the [root README](../README.md) for the CLI (`scarpet format`) and the project as a whole. This document covers the crate's internals and its roadmap toward rustfmt-level configurability.

## What it does

Formatting is one-directional and non-destructive:

```
CST (with trivia) → lower → Doc IR → render → formatted text
                    └──────────── scarpet-fmt ───────────┘
```

Two properties are guaranteed and enforced against the whole `example/` corpus in CI (under every supported style):

- **Non-destructive** — re-parsing the output yields a structurally equal CST (`strip_trivia(a) == strip_trivia(b)`). Comments and blank-line separators are preserved; only horizontal whitespace is normalized.
- **Idempotent** — formatting twice equals formatting once.

These invariants are the crate's hard constraints: every feature below must preserve them, and any new layout knob ships with a corpus round-trip test that proves it.

## Architecture

| File | Responsibility |
| --- | --- |
| `config.rs` | The `Config` style struct (`indent_width`, `max_width`, `comment_width`, `line_ending`, `brace_style`, `trailing_comma`, `overflow_delimited_expr`, `binop_separator`, `blank_lines_upper_bound`, `blank_lines_lower_bound`) and its `Default`. |
| `lower.rs` | Walks the CST, one arm per `CstKind`, emitting `Doc` nodes. Threads `Config` through so layout knobs are reachable everywhere. Handles trivia placement (own-line vs trailing comments, blank-line reconstruction). |
| `doc.rs` | The pretty-printing IR and its renderer. |
| `trivia.rs` | Helpers for laying out a node's leading trivia. |

### The Doc IR

`Doc` is a small Wadler/Lindig document. `render` lays it out at a target width, choosing for each `Group` whether to print flat (one line) or broken (its `Line`s become newlines).

| Primitive | Meaning |
| --- | --- |
| `Nil` / `Text` / `Comment` | Empty, literal text, a `//` comment (optionally wrapped). |
| `Line` / `SoftLine` | A space / nothing when flat; a newline + indent when broken. |
| `HardLine` / `BlankLine` | Always break (and force the enclosing group open); one / two newlines. |
| `IfBreak(broken, flat)` | Picks a branch by the enclosing group's mode (e.g. a trailing comma only when broken). |
| `Concat` / `Group` | Sequence; a break/flat decision point. |
| `Nest(n, doc)` | Indent by `n` **levels** (each level is `indent_width` spaces, applied at render time). |

The break/flat decision is **local**: a `Group` is laid out flat iff its *own* flat rendering fits the remaining width — what follows is deliberately not considered. Indentation never affects the fit check, which is what keeps formatting idempotent across indent widths.

### Known limitations of the IR

Three capabilities are intentionally absent today; several roadmap items below depend on adding them (a fourth — per-construct width caps — has since landed: a `Group` now carries an optional flat-width cap, powering `fn_call_width` / `array_width` / `struct_lit_width`):

- **No alignment.** `Nest` indents by whole levels only. Aligning a continuation to an opening delimiter (`indent_style = "Visual"`) or aligning map values needs an `Align(column, doc)` primitive.
- **No fill.** Items are laid out flat-or-one-per-line. A "pack as many as fit, then break" layout (`Compressed`) needs a `Fill` combinator.
- **Spaces only.** `render` emits indentation as spaces; `hard_tabs` needs tab output and a tab-width convention for the column math in `fits` and comment wrapping.

## Configuration today

Thirteen knobs, parsed from `scarpet-fmt.toml` by the CLI (TOML parsing lives in `scarpet-cli` so this crate stays file-I/O-free and `wasm`-clean):

| Config field | TOML key | Default | rustfmt analogue |
| --- | --- | --- | --- |
| `indent_width` | `indent` | `4` | `tab_spaces` |
| `max_width` | `max_width` | `100` | `max_width` |
| `comment_width` | `comment_width` (`-1` disables) | `none` | `comment_width` + `wrap_comments`, merged |
| `line_ending` | `line_ending` | `"lf"` | `newline_style` (`lf` / `crlf` / `auto` / `native`) |
| `brace_style` | `brace_style` | `"same_line"` | partial `brace_style` / `indent_style` |
| `trailing_comma` | `trailing_comma` | `"vertical"` | `trailing_comma` (`vertical` / `always` / `never`) |
| `overflow_delimited_expr` | `overflow_delimited_expr` | `false` | `overflow_delimited_expr` |
| `binop_separator` | `binop_separator` | `"back"` | `binop_separator` (`back` / `front`; rustfmt defaults `front`) |
| `blank_lines_upper_bound` | `blank_lines_upper_bound` | `1` | `blank_lines_upper_bound` |
| `blank_lines_lower_bound` | `blank_lines_lower_bound` | `0` | `blank_lines_lower_bound` |
| `fn_call_width` | `fn_call_width` (`0` = always break) | `none` | `fn_call_width` |
| `array_width` | `array_width` | `none` | `array_width` |
| `struct_lit_width` | `struct_lit_width` | `none` | `struct_lit_width` |

Everything else about the layout is currently fixed. The roadmap is to make the fixed choices configurable, matching rustfmt wherever an option has a Scarpet analogue.

## Roadmap: rustfmt parity

The goal is to cover **every rustfmt option that has a meaningful Scarpet analogue**. Scarpet has an unusually small syntactic surface — every `CstKind` is one of `Number`, `Str`, `Ident`, `Call`, `List`, `Map`, `Paren`, `Binary`, `Unary`, `Empty`. There are no items, types, attributes, macros, imports, `match`, or even control-flow keywords (`if`/`while`/`loop` are ordinary function calls). As a result roughly half of rustfmt's ~85 options describe Rust constructs that simply do not exist here, and are out of scope.

### Coverage map

Legend: ✅ done · 🟡 planned (has a Scarpet analogue) · ⬜ out of scope (Rust-specific) · ➖ deprecated upstream.

**Width & heuristics**

| rustfmt option | Status | Scarpet analogue / note |
| --- | --- | --- |
| `max_width` | ✅ | line-length target |
| `comment_width` | ✅ | merged with `wrap_comments` into one knob |
| `use_small_heuristics` | 🟡 | umbrella over the sub-widths below (deferred — set them individually for now) |
| `fn_call_width` | ✅ | call `f(...)` argument width |
| `array_width` | ✅ | list `[...]` width |
| `struct_lit_width` | ✅ | map `{...}` width |
| `chain_width` | 🟡 | `:` get-chains (weak analogue) |
| `short_array_element_width_threshold` | 🟡 | short-element fill (needs `Fill`) |

**Indentation & spacing**

| rustfmt option | Status | Scarpet analogue / note |
| --- | --- | --- |
| `tab_spaces` | ✅ | `indent_width` |
| `hard_tabs` | 🟡 | needs tab rendering |
| `indent_style` | 🟡 | `Block` done; `Visual` needs `Align` |
| `binop_separator` | ✅ | `back` (default) / `front`; assignment-likes always `back`; rustfmt defaults `front` |
| `space_after_colon` / `space_before_colon` | 🟡 | spacing around the `:` (get) operator; currently tight |
| `struct_field_align_threshold` | 🟡 | align map `->` values (needs `Align`) |

**Newlines & blank lines**

| rustfmt option | Status | Scarpet analogue / note |
| --- | --- | --- |
| `newline_style` | ✅ | `line_ending` — `lf` / `crlf` / `auto` / `native` |
| `blank_lines_upper_bound` | ✅ | max consecutive blank lines (default 1) |
| `blank_lines_lower_bound` | ✅ | min blank lines between statements (default 0) |

**Layout & delimiters**

| rustfmt option | Status | Scarpet analogue / note |
| --- | --- | --- |
| `brace_style` | 🟡 | partial via `brace_style` (same_line/next_line) |
| `trailing_comma` | ✅ | `vertical` / `always` / `never` |
| `fn_params_layout` | 🟡 | collection layout `Tall`/`Vertical`/`Compressed` (needs `Fill`) |
| `fn_single_line` | 🟡 | collapse `f(x) -> (expr)` to one line |
| `overflow_delimited_expr` | ✅ | last-arg block / `;`-chain hug (idiomatic in Scarpet); arrow-lambda & list/map last args deferred |
| `struct_lit_single_line` | 🟡 | map on one line (mostly already done) |

**Literals & strings**

| rustfmt option | Status | Scarpet analogue / note |
| --- | --- | --- |
| `hex_literal_case` | 🟡 | `0xff` / `0xFF` |
| `float_literal_trailing_zero` | 🟡 | `1.0` / `1.` |
| `format_strings` | 🟡 | string wrapping (risky — Scarpet has no implicit concat; likely deferred) |
| `wrap_comments` | ✅ | folded into `comment_width` |

**Other formatting**

| rustfmt option | Status | Scarpet analogue / note |
| --- | --- | --- |
| `remove_nested_parens` | 🟡 | strip redundant `((x))`; guarded — `(a;b)` is a semantic block, so default off |

**Tooling & diagnostics** (CLI-level)

| rustfmt option | Status | Note |
| --- | --- | --- |
| `error_on_line_overflow` / `error_on_unformatted` | 🟡 | error when output exceeds the target |
| `ignore` | 🟡 | gitignore-style file exclusion |
| `disable_all_formatting` | 🟡 | passthrough |
| `color` / `show_parse_errors` | 🟡 | CLI ergonomics |
| `required_version` / `unstable_features` | 🟡 | version pin / feature gate |

**Out of scope** (Rust-specific, no Scarpet analogue) ⬜

`edition`, `style_edition`, `version`, all imports options (`reorder_imports`, `imports_granularity`, `imports_indent`, `imports_layout`, `group_imports`, `merge_imports`, `merge_derives`), all `match_arm_*`, control-flow layout (`control_brace_style`, `combine_control_expr`, `single_line_if_else_max_width`, `single_line_let_else_max_width`, `force_multiline_blocks`, `where_single_line`, `empty_item_single_line`), types & items (`type_punctuation_density`, `spaces_around_ranges`, `enum_discrim_align_threshold`, `struct_variant_width`, `condense_wildcard_suffixes`, `use_field_init_shorthand`, `use_try_shorthand`, `force_explicit_abi`, `reorder_impl_items`, `reorder_modules`, `skip_children`, `trailing_semicolon`), attributes & macros (`attr_fn_like_width`, `inline_attribute_width`, `format_macro_bodies`, `format_macro_matchers`, `skip_macro_invocations`), doc comments (`format_code_in_doc_comments`, `doc_comment_code_block_width`, `normalize_doc_attributes`, `normalize_comments`), generated files (`format_generated_files`, `generated_marker_line_search_limit`). Deprecated upstream (➖): `fn_args_layout`, `hide_parse_errors`.

### Scarpet-native extras (beyond rustfmt)

A few useful knobs have no rustfmt counterpart because they assume Rust syntax that Scarpet lacks:

- **Operator spacing** as general knobs (today: `:` tight, other binops spaced, `,` as `, ` — all hardcoded).
- **Quote normalization** — if Scarpet accepts both `'...'` and `"..."`, pick a canonical form.
- **A skip directive** — a magic comment (`// fmt: skip`) as the analogue of `#[rustfmt::skip]`, since Scarpet has no attributes.
- **Range formatting** — format only selected lines (rustfmt's `--file-lines`).

## Implementation plan

### Phase 0 — Foundations

Enabling work the rest depends on:

1. **Sub-width `Group`** ✅ — `Group` carries an optional width cap; `fits` measures against `min(remaining, cap)`. Landed with `fn_call_width` / `array_width` / `struct_lit_width`; still the foundation for the remaining per-construct widths (`chain_width`, short-element packing).
2. **`Align` primitive** — set indent to the current column + n. Unlocks `indent_style = "Visual"` and map alignment.
3. **`Fill` combinator** — pack-then-break layout. Unlocks `Compressed` and short-element packing.
4. **Tab rendering** — `hard_tabs`, plus a tab-width convention for `fits` and comment wrapping.
5. **Test harness** — generalize the existing `roundtrip_next_line_braces` corpus test to run over a matrix of option combinations.

### Phase 1 — High-value width & comma knobs

`use_small_heuristics` + `fn_call_width` / `array_width` / `struct_lit_width`; `overflow_delimited_expr`; `blank_lines_upper_bound` / `blank_lines_lower_bound`. Direct analogues, common in the corpus, low risk.

### Phase 2 — Indentation & whitespace

`hard_tabs`; collection layout (`Tall` / `Vertical` / `Compressed`); `fn_single_line`.

### Phase 3 — Advanced layout (needs `Align` / `Fill`)

`indent_style = "Visual"`; map `->` alignment (`struct_field_align_threshold` analogue); `short_array_element_width_threshold`.

### Phase 4 — Token normalization

`hex_literal_case`; `float_literal_trailing_zero`; `:` spacing (`space_after_colon` / `space_before_colon`); operator-spacing knobs; `remove_nested_parens` (guarded, default off); optionally split `comment_width` / `wrap_comments` to match rustfmt's naming.

### Phase 5 — Tooling & diagnostics (CLI)

`ignore`; `error_on_line_overflow` / `error_on_unformatted`; `disable_all_formatting`; `color` / `show_parse_errors`; `required_version` / `unstable_features`; the `// fmt: skip` directive; range formatting.

## Constraints & risks

- **Idempotency and non-destructiveness are gates.** Each layout knob ships with a corpus round-trip test (`strip_trivia` equality + double-format equality), mirroring `roundtrip_next_line_braces`. Combinations multiply quickly — test representative pairs, not the full cross product.
- **Destructive knobs default off.** `remove_nested_parens` must respect that `(a;b)` is a semantic block; `format_strings` is risky because Scarpet has no implicit string concatenation. Both stay opt-in.
- **`hard_tabs` complicates column math.** `fits` and comment wrapping count display columns; settle on a tab width (e.g. `indent_width`).
- **Config plumbing is paired.** Every new knob touches both `Config` (here) and the CLI's `ConfigFile` — never read files in this crate; it must keep building for `wasm32`.

## Open design questions

1. **Default philosophy** — keep the current fixed style as each knob's default (corpus stays unchanged; recommended), or adopt rustfmt's defaults (e.g. `binop_separator = "Front"`)?
2. **Naming** — mirror rustfmt's option names for familiarity, or keep Scarpet-native names (`indent`, `brace_style = "same_line"`) with a documented mapping?
3. **Tooling scope** — how far to chase CLI-level options (`ignore`, `color`, `required_version`) versus focusing on the formatting knobs of Phases 1–4?
