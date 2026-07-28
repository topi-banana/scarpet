//! The high-level intermediate representation: a flat arena of expressions.
//!
//! # Why an arena
//!
//! [`Hir`] stores expressions in a `Vec` addressed by [`ExprId`] rather than as
//! a recursive `Box`ed tree, for four reasons in descending order of force:
//!
//! 1. **No lifetime parameter.** `scarpet_syntax`'s [`Cst`] and `ast` both
//!    borrow `&'s str` out of the source, which is why the playground and the
//!    CLI REPL both `Box::leak` their input to get a `'static` tree. A HIR that
//!    owns its atoms lets a caller hold an [`Analysis`](crate::Analysis) in an
//!    `Rc` and drop the source.
//! 2. **`Drop` and `Debug` are stack-safe.** Scarpet's `;`, `,` and `+` all
//!    left-nest, so a long script is a very deep syntax tree. A `Box`ed HIR
//!    would recurse on drop and on `{:#?}`; a `Vec` cannot. All recursion risk
//!    is concentrated in lowering and
//!    [`Analysis::dump`](crate::Analysis::dump), where it is handled with an
//!    explicit stack.
//! 3. **Stable ids for side tables.** Inferred types live in a `Vec<Ty>`
//!    indexed by `ExprId`; a future semantic-token or const-value pass can add
//!    its own table the same way, without touching this module.
//! 4. **Cheap position queries.** [`Hir::expr_at`] is a scan over one flat
//!    `Vec`, which is what hover and inlay hints need.
//!
//! # Shape
//!
//! The HIR deliberately does *not* mirror the syntax tree:
//!
//! - `[a, b]` and `l(a, b)` become the same [`Expr::List`]; `{..}` and `m(..)`
//!   the same [`Expr::Map`]. The spelling is a formatting concern, and
//!   `scarpet-fmt` already has the lossless tree for that.
//! - Parentheses are transparent. A `ParenExpr` contributes its *span* to the
//!   expression it wraps but no node of its own, so hovering anywhere inside
//!   `(a + b)` selects the addition.
//! - `;` and `,` chains are flattened into one [`Expr::Seq`] instead of the
//!   left-nested `BinExpr` spine the parser produces. This is what keeps node
//!   depth proportional to expression nesting rather than to statement count.
//! - The ten-level precedence ladder of `scarpet_syntax::ast` collapses to a
//!   single [`Expr::Binary`] carrying a [`BinOp`].
//!
//! [`Cst`]: scarpet_syntax::cst::Cst

use crate::span::Span;

/// An identifier: a variable, a parameter, or a call target.
///
/// A `Box<str>` rather than an interned symbol. Interning would want an arena
/// threaded through every public accessor for a gain that has not been measured
/// yet; if name-heavy workloads ever show up in a profile, this alias is the
/// single place to change.
pub type Name = Box<str>;

/// A handle to an expression in a [`Hir`] arena.
///
/// Only meaningful together with the `Hir` that produced it. Mixing ids across
/// analyses is not detectable, but every accessor that takes one is total, so
/// the worst outcome is a wrong answer rather than a panic.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ExprId(u32);

impl ExprId {
    /// The underlying index, for a caller building its own side table.
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) const fn from_index(index: usize) -> Self {
        ExprId(index as u32)
    }
}

/// A handle to a function definition in a [`Hir`] arena.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FnId(u32);

impl FnId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) const fn from_index(index: usize) -> Self {
        FnId(index as u32)
    }
}

/// A binary operator, flattened out of the parser's precedence ladder.
///
/// Assignment (`=`, `+=`, `<>`) is deliberately *not* here: its left operand is
/// a place rather than a value, and it is the only construct that writes a
/// scope. See [`Expr::Assign`] and [`AssignOp`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    /// `%` — floor-mod for two integers, following the divisor's sign.
    Rem,
    /// `^` — always yields a double.
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    /// `:` — element access on a list or map; `null` on anything else.
    Get,
    /// `~` — index-of on a list, key lookup on a map, regex match otherwise.
    Match,
}

impl BinOp {
    /// The source spelling, for diagnostics and dumps.
    pub const fn as_str(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Pow => "^",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Get => ":",
            BinOp::Match => "~",
        }
    }
}

/// A prefix operator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PrefixOp {
    Neg,
    Pos,
    Not,
    /// `...` — the spread marker. In a signature it introduces the vararg
    /// binder; in an expression the VM tags the container and never reads the
    /// tag back, so the HIR types it as its operand.
    Spread,
}

impl PrefixOp {
    pub const fn as_str(self) -> &'static str {
        match self {
            PrefixOp::Neg => "-",
            PrefixOp::Pos => "+",
            PrefixOp::Not => "!",
            PrefixOp::Spread => "...",
        }
    }
}

/// An assignment operator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AssignOp {
    /// `=`
    Assign,
    /// `+=` — applies the `+` rule to the target's current value.
    Add,
    /// `<>` — swaps the two sides' bindings.
    Swap,
}

impl AssignOp {
    pub const fn as_str(self) -> &'static str {
        match self {
            AssignOp::Assign => "=",
            AssignOp::Add => "+=",
            AssignOp::Swap => "<>",
        }
    }
}

/// Which separator produced an [`Expr::Seq`].
///
/// Both evaluate every element and take the value of the last, but they sit at
/// different rungs of the precedence ladder, so keeping them apart lets a dump
/// (and a future pretty-printer) reproduce the program's grouping.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SeqSep {
    /// `;`
    Semi,
    /// `,`
    Comma,
}

impl SeqSep {
    pub const fn as_str(self) -> &'static str {
        match self {
            SeqSep::Semi => ";",
            SeqSep::Comma => ",",
        }
    }
}

/// One entry of a map literal.
///
/// `{k -> v}` is a [`value`](MapEntry::value)-carrying pair; a bare `{e}` has
/// none and the VM requires `e` to evaluate to a two-element list.
#[derive(Clone, PartialEq, Debug)]
pub struct MapEntry {
    pub key: ExprId,
    pub value: Option<ExprId>,
}

/// A user-defined function, `name(params) -> body`.
///
/// Scarpet has one flat, global function namespace: a definition nested inside
/// another function body is still visible everywhere, and a user definition
/// shadows a builtin of the same name. So every `FnDef` in a program lands in
/// the same [`Hir::fns`] list regardless of where it was written.
#[derive(Clone, PartialEq, Debug)]
pub struct FnDef {
    pub name: Name,
    /// Just the name token, for a "defined here" diagnostic label.
    pub name_span: Span,
    /// Positional binders, in order. Parameters are bound by value.
    pub params: Vec<Name>,
    /// `outer(x)` captures. These do *not* consume a positional slot, and they
    /// share the defining scope's storage slot in both directions.
    pub captures: Vec<Name>,
    /// The single `...rest` binder, which collects the surplus arguments.
    pub rest: Option<Name>,
    pub body: ExprId,
    /// The whole definition, name through body.
    pub span: Span,
}

impl FnDef {
    /// The number of arguments a call must supply, and whether more are
    /// allowed. Captures are excluded — they are bound from the defining scope,
    /// not from the call.
    pub fn arity(&self) -> (usize, bool) {
        (self.params.len(), self.rest.is_some())
    }

    /// Whether `count` arguments satisfy this signature.
    pub fn accepts(&self, count: usize) -> bool {
        let (fixed, variadic) = self.arity();
        if variadic {
            count >= fixed
        } else {
            count == fixed
        }
    }

    /// What a call must supply, phrased for a `wrong-arg-count` message.
    pub fn describe_arity(&self) -> String {
        match self.arity() {
            (fixed, true) => format!("at least {fixed} arguments"),
            (1, false) => "1 argument".to_owned(),
            (fixed, false) => format!("{fixed} arguments"),
        }
    }
}

/// An expression node.
///
/// Every variant's children are [`ExprId`]s into the same arena. Use
/// [`Hir::children`] to walk them generically rather than matching all twelve
/// variants.
#[derive(Clone, PartialEq, Debug)]
pub enum Expr {
    /// A hole where the syntax tree had no usable child — a binary operand the
    /// parser never produced, or an empty program. Types as
    /// `unknown` and is never itself diagnosed: whatever went wrong has already
    /// been reported as a parse error.
    Missing,

    Int(i64),
    Double(f64),
    /// A string literal with its quotes stripped and escapes expanded, matching
    /// `scarpet_vm::Value::from_string_literal`.
    Str(String),

    /// A bare identifier read.
    Name(Name),

    /// `name(args)`. Scarpet has no first-class callee expression — the callee
    /// is always an identifier resolved by name at run time, which is why
    /// `if`, `for` and friends are calls rather than syntax.
    Call {
        callee: Name,
        /// Just the callee token, so an `unknown function` diagnostic can point
        /// at the name instead of the whole call.
        callee_span: Span,
        args: Vec<ExprId>,
    },

    /// `[a, b]` or `l(a, b)`.
    List(Vec<ExprId>),

    /// `{k -> v, bare}` or `m(...)`.
    Map(Vec<MapEntry>),

    Prefix {
        op: PrefixOp,
        operand: ExprId,
    },

    Binary {
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
    },

    /// `target = value`, `target += value`, `target <> value`.
    Assign {
        op: AssignOp,
        target: ExprId,
        value: ExprId,
    },

    /// A flattened `;` or `,` chain. Evaluates every element and takes the
    /// value of the last; an empty sequence is `null`.
    Seq {
        sep: SeqSep,
        exprs: Vec<ExprId>,
    },

    /// `name(params) -> body`. Evaluates to the function's *name as a string* —
    /// not to a function value, which Scarpet's VM does not have.
    Define(FnId),
}

impl Expr {
    /// A short label naming the node kind, for dumps and tree views.
    pub fn label(&self) -> String {
        match self {
            Expr::Missing => "Missing".to_owned(),
            Expr::Int(v) => format!("Int {v}"),
            Expr::Double(v) => format!("Double {v}"),
            Expr::Str(s) => format!("Str {s:?}"),
            Expr::Name(n) => format!("Name {n}"),
            Expr::Call { callee, .. } => format!("Call `{callee}`"),
            Expr::List(items) => format!("List [{}]", items.len()),
            Expr::Map(entries) => format!("Map [{}]", entries.len()),
            Expr::Prefix { op, .. } => format!("Prefix `{}`", op.as_str()),
            Expr::Binary { op, .. } => format!("Bin `{}`", op.as_str()),
            Expr::Assign { op, .. } => format!("Assign `{}`", op.as_str()),
            Expr::Seq { sep, exprs } => format!("Seq `{}` [{}]", sep.as_str(), exprs.len()),
            Expr::Define(_) => "Define".to_owned(),
        }
    }
}

/// A lowered program: expressions, their spans, and the functions it defines.
#[derive(Clone, PartialEq, Debug)]
pub struct Hir {
    exprs: Vec<Expr>,
    /// Parallel to `exprs`. Kept out of [`Expr`] so the derived `Debug` of the
    /// arena stays readable and the variants stay small.
    spans: Vec<Span>,
    fns: Vec<FnDef>,
    root: ExprId,
}

impl Hir {
    pub(crate) fn new(exprs: Vec<Expr>, spans: Vec<Span>, fns: Vec<FnDef>, root: ExprId) -> Self {
        debug_assert_eq!(exprs.len(), spans.len());
        Hir {
            exprs,
            spans,
            fns,
            root,
        }
    }

    /// The whole program's expression.
    pub fn root(&self) -> ExprId {
        self.root
    }

    /// The expression `id` refers to, or [`Expr::Missing`] for an id from
    /// another arena.
    pub fn expr(&self, id: ExprId) -> &Expr {
        self.exprs.get(id.index()).unwrap_or(&Expr::Missing)
    }

    /// The source range `id` covers, or an empty span for a foreign id.
    pub fn span(&self, id: ExprId) -> Span {
        self.spans.get(id.index()).copied().unwrap_or_default()
    }

    /// The number of expressions in the arena.
    pub fn len(&self) -> usize {
        self.exprs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.exprs.is_empty()
    }

    /// Every id in the arena, in lowering order — a pre-order walk of the
    /// program, so a parent always precedes its children.
    pub fn ids(&self) -> impl ExactSizeIterator<Item = ExprId> + '_ {
        (0..self.exprs.len()).map(ExprId::from_index)
    }

    /// A short label naming the node, for dumps and tree views. Unlike
    /// [`Expr::label`] it can name the function a [`Define`](Expr::Define)
    /// introduces, because it can reach the definition table.
    pub fn label(&self, id: ExprId) -> String {
        match self.expr(id) {
            Expr::Define(def) => match self.fn_def(*def) {
                Some(def) => format!("Define `{}`/{}", def.name, def.params.len()),
                None => "Define".to_owned(),
            },
            other => other.label(),
        }
    }

    /// The direct children of `id`, in source order.
    ///
    /// A [`Define`](Expr::Define) yields its body, so a walk from
    /// [`root`](Self::root) reaches every expression in the program exactly
    /// once.
    pub fn children(&self, id: ExprId) -> Children<'_> {
        Children(match self.expr(id) {
            Expr::Missing | Expr::Int(_) | Expr::Double(_) | Expr::Str(_) | Expr::Name(_) => {
                ChildrenKind::Pair(None, None)
            }
            Expr::Define(def) => ChildrenKind::Pair(self.fn_def(*def).map(|def| def.body), None),
            Expr::Prefix { operand, .. } => ChildrenKind::Pair(Some(*operand), None),
            Expr::Binary { lhs, rhs, .. } => ChildrenKind::Pair(Some(*lhs), Some(*rhs)),
            Expr::Assign { target, value, .. } => ChildrenKind::Pair(Some(*target), Some(*value)),
            Expr::Call { args, .. } => ChildrenKind::Ids(args.iter()),
            Expr::List(items) => ChildrenKind::Ids(items.iter()),
            Expr::Seq { exprs, .. } => ChildrenKind::Ids(exprs.iter()),
            Expr::Map(entries) => ChildrenKind::Entries {
                entries: entries.iter(),
                pending: None,
            },
        })
    }

    /// Every function the program defines, in lowering order.
    ///
    /// Flat regardless of nesting, because Scarpet's function namespace is: a
    /// definition written inside another body is still visible everywhere, and
    /// a later definition of the same name replaces an earlier one. Inference
    /// walks bodies from this list — each in its own scope — rather than from
    /// wherever they happen to appear.
    pub fn fns(&self) -> &[FnDef] {
        &self.fns
    }

    /// The definition `id` refers to, or `None` for an id from another arena.
    pub fn fn_def(&self, id: FnId) -> Option<&FnDef> {
        self.fns.get(id.index())
    }

    /// The innermost expression whose span covers `offset` — the node a cursor
    /// at that byte is "on". The entry point for hover and inlay hints.
    ///
    /// Ties are broken by narrowest span, then by latest id, so the deepest
    /// node wins when a parent and child share a range (a transparent
    /// parenthesis, or a `Seq` with a single element).
    pub fn expr_at(&self, offset: u32) -> Option<ExprId> {
        let mut best: Option<(ExprId, u32)> = None;
        for (index, span) in self.spans.iter().enumerate() {
            if !span.contains(offset) {
                continue;
            }
            let width = span.len();
            if best.is_none_or(|(_, best_width)| width <= best_width) {
                best = Some((ExprId::from_index(index), width));
            }
        }
        best.map(|(id, _)| id)
    }
}

/// Iterator over the direct children of an expression. See [`Hir::children`].
#[derive(Clone, Debug)]
pub struct Children<'h>(ChildrenKind<'h>);

#[derive(Clone, Debug)]
enum ChildrenKind<'h> {
    /// Up to two fixed slots, consumed left to right.
    Pair(Option<ExprId>, Option<ExprId>),
    Ids(std::slice::Iter<'h, ExprId>),
    /// Map entries yield the key, then the value when there is one.
    Entries {
        entries: std::slice::Iter<'h, MapEntry>,
        pending: Option<ExprId>,
    },
}

impl Iterator for Children<'_> {
    type Item = ExprId;

    fn next(&mut self) -> Option<ExprId> {
        match &mut self.0 {
            ChildrenKind::Pair(first, second) => first.take().or_else(|| second.take()),
            ChildrenKind::Ids(iter) => iter.next().copied(),
            ChildrenKind::Entries { entries, pending } => {
                if let Some(value) = pending.take() {
                    return Some(value);
                }
                let entry = entries.next()?;
                *pending = entry.value;
                Some(entry.key)
            }
        }
    }
}
