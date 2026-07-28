# scarpet-hir

Static analysis for Scarpet: a high-level IR, and a type for every expression in it.

```
source → scarpet-syntax::parse → rowan tree → lower → Hir → infer → Ty + Diagnostic
                                              └──────────── scarpet-hir ───────────┘
```

Sibling to `scarpet-fmt`, not built on it. The formatter consumes the lossless `Cst`;
this crate consumes the typed rowan layer (`scarpet_syntax::nodes`), because that is
the only view of a program that carries source positions — `Cst` and `ast` both drop
them, and a located diagnostic cannot be built without them.

No dependency on `scarpet-vm`: a compiler must not depend on an interpreter. The
operator rules in `infer/ops.rs` are a deliberate port of the VM's `value/*.rs`, each
rule naming its source file, so the two are kept honest by review rather than by a
dependency edge.

`wasm`-clean and free of file I/O, so the playground, a language server, and a future
compiler can all depend on it. Any `.sc` loading or multi-file resolution belongs in
the caller.

## What it is for

Scarpet is very nearly total. `1 + [1, 2]` is `"1[1, 2]"`, `'hello' - 1` is
`"hello"`, `1 / [1, 2]` is `"1/[1, 2]"`, and indexing a non-container yields `null`
rather than failing. A checker built to find type *errors* would find almost none.
So, in order of value:

1. **Describe.** Say that an expression is a `string`. That powers the playground's
   type view and, later, editor hover and inlay hints. This is the product.
2. **Check names and arities.** Real, common, statically decidable, no lattice
   needed.
3. **Warn about the totality.** Point at the arithmetic that silently becomes text.

## Architecture

| File | What it does |
| --- | --- |
| `span.rs` | `Span` — the bridge between `rowan::TextRange` (what the tree gives) and `Range<usize>` (what `ariadne` and the LSP take). |
| `hir.rs` | The arena: `Hir`, `ExprId`, `Expr`, `FnDef`, and the walking API. |
| `lower.rs` | rowan tree → arena. Iterative, reserve-then-fill. |
| `ty.rs` | The lattice: `Ty`, `join`, and the bounds that keep it finite. |
| `builtins.rs` | Arities and return types for the core builtins; `builtins/names.rs` is generated from the upstream reference. |
| `infer.rs` | The inference walk, scopes, special forms, and the fixpoint. |
| `infer/ops.rs` | Operator type rules, ported from `scarpet-vm`. |
| `diagnostic.rs` | `Diagnostic`, `Severity`, `LintCode` — one shape every front end adapts. |
| `dump.rs` | A text tree for tests and terminals; `walk` for graphical ones. |

### The arena

Expressions live in a `Vec` addressed by `ExprId` rather than in a `Box`ed tree:

- **No lifetime parameter.** `Cst` and `ast` borrow `&'s str` out of the source, which
  is why the playground and the CLI REPL both `Box::leak` their input. An owning HIR
  lets a caller hold an `Analysis` in an `Rc` and drop the source.
- **`Drop` and `Debug` are stack-safe.** Scarpet's `;`, `,` and `+` all left-nest, so
  a long script is a very deep syntax tree. All recursion risk is concentrated in
  lowering and the dump, where it is handled with an explicit stack.
- **Stable ids for side tables.** Types are a `Vec<Ty>` indexed by `ExprId`; a future
  semantic-token pass can add its own the same way.

The HIR deliberately does not mirror the syntax tree. `[a, b]` and `l(a, b)` become
the same node, parentheses are transparent, `;`/`,` chains flatten into one `Seq`, and
the ten-level precedence ladder collapses to one `Binary` with a `BinOp`.

### The lattice

```
unknown                             top: no information, and the widening sink
never                               bottom: no value arrives here
bool  int  double  number  string   number is int | double, what type() calls "number"
list<T>  map<K, V>
block entity nbt screen text task   known by name, not by structure
undef                               read before anything wrote it
null
A | B                               a normalised union, at most 4 members
```

`join` is the least upper bound. There is deliberately no `meet` and no narrowing:
narrowing refines a type inside a branch guarded by a predicate, and Scarpet has no
syntactic control flow, no type guards, and no exhaustiveness to check.

`Ty` is recursive, so nesting is truncated past a fixed depth and unions widen past a
fixed arity. That is what makes the lattice finite — and therefore what makes the
inference fixpoint guaranteed to converge.

### The fixpoint

A function's return type depends on its parameters, and its parameters come from its
call sites — which for a recursive function include its own body. Both are seeded at
`never` and raised together:

```
fib(n) -> if(n < 2, n, fib(n-1) + fib(n-2)); print('fib = ' + fib(10))

round 1  n from `fib(10)` is int; body = if(bool, int, never + never)
         = join(int, never) = int          -> fib returns int
round 2  body = if(bool, int, int + int) = int, unchanged -> converged
```

Raising from the bottom is what makes this work: `unknown` is absorbing, so seeding
there would leave every recursive function unknown forever.

## Lints

| Code | Severity | Fires on |
| --- | --- | --- |
| `parse` | error | a syntax error, forwarded from the parser |
| `wrong-arg-count` | error | a call that cannot satisfy the callee's signature |
| `unknown-function` | error | a name that is neither defined, imported, nor documented |
| `expected-number` | error | `%` `^` unary `±` on a provably non-numeric operand |
| `map-entry-not-pair` | error | a bare map entry that is a literal list of length ≠ 2 |
| `string-fallback` | warning | arithmetic that silently joins its operands as text |
| `read-before-write` | warning | a variable read with no prior write |

`string-fallback` is narrow on purpose. Scarpet's `+` between a string and a number
is the *idiomatic* way to build a message — `print('x = ' + x)` is everywhere — so
flagging it would bury the signal. What it reports is arithmetic where **neither**
side is a string and the result is one anyway: `1 + [1, 2]` is `"1[1, 2]"`,
`count + null` is `"5null"`.

Over the whole `example/` corpus (about 108 000 HIR nodes) the set produces roughly
130 findings — under one per file.

## Known imprecision

Recorded as decisions, not bugs:

- **Slot aliasing is not modelled.** `b = a = 1` gives `b` *`a`'s storage slot* in the
  VM, so a later write to one is visible through the other. Tracking that needs
  union-find over slots and would change essentially no inferred type.
- **Branches are not merged.** Assignment is flow-sensitive and last-write-wins, so
  after `if(c, a = 1, a = 'x')` the analysis reports `a` as `string` rather than
  `int | string`.
- **`outer(x)` captures are `unknown`.** A capture shares its slot with the defining
  scope in both directions, resolved when the definition runs.
- **`global_` variables are `unknown`** and never diagnosed.
- **A lazy `range` is a `list`.** The VM keeps them apart only so `type()` can answer
  `iterator`; nothing else in the language treats them differently.
- **`var(<non-literal>)` poisons its scope.** Any name could have been written, so
  unresolved reads widen rather than being accused of being uninitialised.
- **`unknown-function` tracks the pinned docs submodule.** A builtin added upstream
  after that snapshot reads as unknown until the submodule moves.

## The generated name list

`src/builtins/names.rs` holds every function name `docs/scarpet/Full.md` documents,
regenerated by `tests/sourcegen.rs` and **committed**. It is a test rather than a
build script because most CI legs check out without submodules; a `build.rs` reading
`example/` would hard-fail them. When the submodule is absent the test skips quietly,
so a green `cargo test` locally does not prove it ran — confirm with
`git submodule update --init --recursive`.

Names only: the reference records signatures like `min(arg, ...)` but no types.
Anything with a return type is hand-written in `builtins.rs`.

## Roadmap

1. **Language server.** `Analysis::ty_at` is the hover and inlay-hint entry point;
   `Diagnostic` maps onto `publishDiagnostics` directly.
2. **`scarpet check`.** A CLI subcommand rendering findings through the `ariadne`
   reporter the crate already has, with `--deny` per `LintCode`.
3. **Deeper inference.** Narrowing on `type(x) == 'list'`, per-index element types,
   `Ty::Fn` once the VM grows first-class functions.
4. **The rest of the builtin table.** About 45 of the 320 documented functions are
   typed; the Minecraft API in `docs/scarpet/api/*.md` is the bulk of the remainder.
