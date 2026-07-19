//! Abstract syntax tree — a structured, trivia-free lowering of the [`Cst`].
//!
//! Where the [`Cst`] is a single uniform `Binary { op, lhs, rhs }` / `Unary` /
//! atom shape that preserves comments and breaks, the AST here re-encodes the
//! precedence ladder *into the types*: one enum per level, mirroring
//! `parser.rs`'s ladder from `comma` (loosest) down to `primary` (tightest).
//! Comments and breaks are dropped — the AST is meant to be evaluated, not
//! re-printed (the formatter keeps using the trivia-bearing [`Cst`]).
//!
//! The shape, low → high:
//!
//! - `,` → [`Args`]`(Vec<Code>)` — argument / element lists, paren bodies, root
//! - `;` → [`Code`]`(Vec<Expr>)` — a statement sequence
//! - `->` → [`Expr`] — the parser tags every accepted `->` at this level a
//!   function definition ([`CstKind::DefineFunction`] → [`Expr::Def`]); a map
//!   item's key/value arrow never reaches this level (it lowers with the map,
//!   as a [`MapEntry`])
//! - `=` `+=` `<>` → [`Assign`] — an [`LValue`] target with an [`AssignOp`]
//! - `||` `&&` `==`/`!=` `<`/`<=`/`>`/`>=` `+`/`-` `*`/`/`/`%` `^` → one enum
//!   per level ([`Lor`], [`Land`], [`Equality`], [`Compare`], [`Additive`],
//!   [`Mult`], [`Power`])
//! - prefix `-` `+` `!` `...` → [`Unary`]
//! - `~` `:` → [`Get`]
//! - atoms, calls, `[...]`, `{...}`, `(...)` → [`Primary`]
//!
//! Lowering is fallible where a structured **target** or **parameter list** is
//! required, so a malformed one yields [`LowerError`] rather than a silently wrong
//! tree. The two are deliberately *separate* types, because they are different
//! little languages:
//!
//! - A function's parameters lower to [`Params`] — a flat list of plain binders,
//!   `outer(x)`-style [`Capture`]s (each a [`ParamWord`] reserved word), and at
//!   most one `...rest`. The parser only accepts the call-shaped signature
//!   `name(args) -> body`; lowering validates that every argument is a valid
//!   parameter (not a literal, index, or nested pattern).
//! - An assignment target (the left of `=`/`+=`/`<>`) lowers to [`LValue`] — a
//!   single [`Place`] (`x`, `var(e)`, `a:b:c`), a [`Destructure`](LValue::Destructure)
//!   list that may nest and carry at most one `...rest` per
//!   level, or a [`Computed`](LValue::Computed) call (`if(c, a, b) = …`) resolved
//!   at runtime. A shape that can never be a place (`1 = 2`, `a + b = c`,
//!   `a ~ b = c`) is a [`LowerError::NotAssignable`]. So the structured cases are
//!   executable by construction, unlike a general [`Lor`] expression.
//!
//! Every borrowed `&'s str` points back into the original source, exactly as in
//! the [`Cst`]; lowering allocates only the `Vec`/`Box` spine.

use std::collections::VecDeque;

use crate::parser::{BinOp, Cst, CstKind, UnaryOp};

// ====================================================================
// AST — one type per precedence level (low → high)
// ====================================================================

/// `,`-separated sequence: call arguments, list elements, a parenthesized
/// body, and the program root (the grammar's `top`). Each element is a full
/// [`Code`] (statement sequence), so `[a; b, c]` is two elements `a; b` and `c`.
/// (Map items are [`MapEntry`]s, not a plain `Args`.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args<'s>(pub VecDeque<Code<'s>>);

/// `;`-separated statement sequence; the value is the last [`Expr`]. A lone
/// expression is a one-element `Code`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code<'s>(pub Vec<Expr<'s>>);

/// The `->` level. Right-associative: `a -> b -> c` is `a -> (b -> c)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr<'s> {
    /// `name(params) -> body` — a function definition (the left of `->` is a
    /// call). The anonymous form `_(x) -> …` is just `name == "_"`.
    ///
    /// `params` are a [`Params`]: plain positional binders, `outer(x)`-style
    /// [`Capture`]s, and at most one `...rest`. Lowering has already classified and
    /// validated them, so the evaluator binds without re-checking.
    Def {
        name: &'s str,
        params: Params<'s>,
        body: Box<Expr<'s>>,
    },
    /// No `->` at this level — an assignment-or-tighter expression.
    Assign(Assign<'s>),
}

/// The assignment level (`=`, `+=`, `<>`). Right-associative, so `a = b = 5`
/// nests on the right. The left of the operator is an [`LValue`] target,
/// validated at lowering — the evaluator never sees a non-assignable shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assign<'s> {
    /// `target <op> value`. `target` is a structurally valid [`LValue`] (a
    /// [`Place`] or a destructuring list); a non-assignable left side is a
    /// [`LowerError::NotAssignable`] rather than a runtime error.
    Set {
        target: LValue<'s>,
        op: AssignOp,
        value: Box<Assign<'s>>,
    },
    /// No assignment at this level — a logical-or-or-tighter expression.
    Lor(Lor<'s>),
}

/// Which assignment operator joins a [`Assign::Set`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    /// `=`
    Assign,
    /// `+=`
    Add,
    /// `<>`
    Swap,
}

// ====================================================================
// Function parameters — a flat, faithful-to-Scarpet signature
// ====================================================================

/// A function's parameter list, lowered from `f(<params>) -> body`. Structured so
/// binding is direct: [`fixed`](Params::fixed) binders consume arguments by
/// position, [`captures`](Params::captures) (`outer(x)`) pull from the defining
/// scope *without* consuming a position, and the single optional
/// [`rest`](Params::rest) collects the trailing arguments (an empty list when none
/// remain). This mirrors fabric-carpet's `FunctionValue`: variables, `outer`, and
/// one vararg are the *only* legal parameters — a literal or nested pattern is a
/// [`LowerError::InvalidParameter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Params<'s> {
    /// Positional binders, in order — each consumes one argument.
    pub fixed: Vec<&'s str>,
    /// Reserved-word captures such as `outer(x)`; these do not consume a
    /// positional argument.
    pub captures: Vec<Capture<'s>>,
    /// The single optional vararg `...rest`. At most one per signature (a second
    /// is a [`LowerError::MultipleRest`]).
    pub rest: Option<&'s str>,
}

/// A reserved-word parameter capture, e.g. `outer(x)`. The [`word`](Capture::word)
/// is an enum so a new reserved binder is one variant away, with no reshaping of
/// [`Params`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture<'s> {
    /// Which reserved word wraps the binder.
    pub word: ParamWord,
    /// The captured variable name (the `x` in `outer(x)`).
    pub name: &'s str,
}

/// A reserved word that may wrap a parameter in a signature. Mirrors
/// fabric-carpet's `FunctionAnnotationValue.Type`; today only `outer` exists, but
/// adding a variant here (and a case in [`param_word`]) introduces a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamWord {
    /// `outer(x)` — capture `x` from the defining scope.
    Outer,
}

// ====================================================================
// Assignment targets (l-values) — a recursive, always-executable shape
// ====================================================================

/// An assignment target: the left of `=`, `+=`, `<>`. Lowering rejects a shape
/// that can never be a place (`1 = 2`, `a + b = c`, `a ~ b = c`) as
/// [`LowerError::NotAssignable`]. The structured variants ([`Place`],
/// [`Destructure`](LValue::Destructure)) are executable by construction; the lone
/// dynamic variant ([`Computed`](LValue::Computed)) mirrors Scarpet's runtime
/// l-value model, where any call returning a *bound* value is a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LValue<'s> {
    /// A single writable place: `x`, `var(e)`, or `a:b:c`.
    Place(Place<'s>),
    /// A destructuring list, binding several places at once
    /// (see [`LPatterns`]).
    Destructure(LPatterns<'s>),
    /// A place produced by evaluating a call — `if(c, a, b) = …`, where the call
    /// returns one of its bound arguments. This is the one l-value resolved
    /// dynamically (Scarpet decides assignability at runtime by whether the value
    /// is bound), so only a call — never a literal or operator — reaches here.
    Computed(Box<Primary<'s>>),
}

/// A single writable place. The base of an [`Index`](Place::Index) is itself a
/// [`Place`], so "indexing a destructuring list" is unrepresentable by
/// construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place<'s> {
    /// A plain variable: `x`. A `global_*` name is still a [`Var`](Place::Var) —
    /// the global routing is by name prefix, decided when binding.
    Var(&'s str),
    /// A dynamically named variable: `var(<expr>)`. The argument's string value is
    /// the variable name.
    DynVar(Box<Code<'s>>),
    /// A container element: `base:key`, nestable as `a:b:c`. `key` is an ordinary
    /// expression evaluated to the address; only `:` (not `~`) reaches here.
    Index {
        base: Box<Place<'s>>,
        key: Primary<'s>,
    },
}

/// The elements of a destructuring [`LValue::Destructure`]: a fixed front, an
/// optional single rest binder, and a fixed tail after it. One rest per level
/// keeps the split unambiguous — `[a, ...mid, b]` binds `a` from the front, `b`
/// from the back, and `mid` the middle. A second `...` at this level is a
/// [`LowerError::MultipleRest`]; a nested list carries its own [`LPatterns`], so a
/// rest inside it is a different level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LPatterns<'s> {
    /// Elements before the rest binder — or all of them, when there is no rest.
    pub before: Vec<LValue<'s>>,
    /// The rest binder `...x` and the elements after it, when a rest is present.
    pub rest: Option<LRest<'s>>,
}

/// The rest binder `...x` of an [`LPatterns`], plus any elements after it (the
/// `b` in `[a, ...x, b]`). The binder is itself an [`LValue`], so `[a, ...[p, q]]`
/// — a rest into a nested destructure — is representable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LRest<'s> {
    /// The place(s) bound by `...` (the `x` in `...x`).
    pub binder: Box<LValue<'s>>,
    /// Elements after the rest binder.
    pub after: Vec<LValue<'s>>,
}

/// Logical or (`||`). Left-associative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lor<'s> {
    /// `lhs || rhs`
    Or { lhs: Box<Lor<'s>>, rhs: Land<'s> },
    /// passthrough
    Land(Land<'s>),
}

/// Logical and (`&&`). Left-associative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Land<'s> {
    /// `lhs && rhs`
    And {
        lhs: Box<Land<'s>>,
        rhs: Equality<'s>,
    },
    /// passthrough
    Equality(Equality<'s>),
}

/// Equality (`==`, `!=`). Left-associative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Equality<'s> {
    /// `lhs == rhs`
    Eq {
        lhs: Box<Equality<'s>>,
        rhs: Compare<'s>,
    },
    /// `lhs != rhs`
    Ne {
        lhs: Box<Equality<'s>>,
        rhs: Compare<'s>,
    },
    /// passthrough
    Compare(Compare<'s>),
}

/// Relational compare (`<`, `<=`, `>`, `>=`). Left-associative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compare<'s> {
    /// `lhs < rhs`
    Lt {
        lhs: Box<Compare<'s>>,
        rhs: Additive<'s>,
    },
    /// `lhs <= rhs`
    Le {
        lhs: Box<Compare<'s>>,
        rhs: Additive<'s>,
    },
    /// `lhs > rhs`
    Gt {
        lhs: Box<Compare<'s>>,
        rhs: Additive<'s>,
    },
    /// `lhs >= rhs`
    Ge {
        lhs: Box<Compare<'s>>,
        rhs: Additive<'s>,
    },
    /// passthrough
    Additive(Additive<'s>),
}

/// Additive (`+`, `-`). Left-associative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Additive<'s> {
    /// `lhs + rhs`
    Add {
        lhs: Box<Additive<'s>>,
        rhs: Mult<'s>,
    },
    /// `lhs - rhs`
    Sub {
        lhs: Box<Additive<'s>>,
        rhs: Mult<'s>,
    },
    /// passthrough
    Mult(Mult<'s>),
}

/// Multiplicative (`*`, `/`, `%`). Left-associative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mult<'s> {
    /// `lhs * rhs`
    Mul { lhs: Box<Mult<'s>>, rhs: Power<'s> },
    /// `lhs / rhs`
    Div { lhs: Box<Mult<'s>>, rhs: Power<'s> },
    /// `lhs % rhs`
    Rem { lhs: Box<Mult<'s>>, rhs: Power<'s> },
    /// passthrough
    Power(Power<'s>),
}

/// Power (`^`). Right-associative: `2 ^ 3 ^ 2` is `2 ^ (3 ^ 2)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Power<'s> {
    /// `base ^ exp`
    Pow {
        base: Unary<'s>,
        exp: Box<Power<'s>>,
    },
    /// passthrough
    Unary(Unary<'s>),
}

/// Prefix unary operators (`-`, `+`, `!`, `...`). Stackable: `!-x`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unary<'s> {
    /// `-x`
    Neg(Box<Unary<'s>>),
    /// `+x`
    Pos(Box<Unary<'s>>),
    /// `!x`
    Not(Box<Unary<'s>>),
    /// `...x`
    Unpack(Box<Unary<'s>>),
    /// passthrough
    Get(Get<'s>),
}

/// The get / match level (`:`, `~`). Left-associative: `a:b:c` is `(a:b):c`.
/// The right operand is always a [`Primary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Get<'s> {
    /// `base:key` (`op == Get`) or `base~key` (`op == Match`).
    Index {
        base: Box<Get<'s>>,
        op: GetOp,
        key: Primary<'s>,
    },
    /// passthrough
    Primary(Primary<'s>),
}

/// Which operator joins a [`Get::Index`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetOp {
    /// `:` — element / member access.
    Get,
    /// `~` — match / search.
    Match,
}

/// The tightest level: atoms, calls, and the bracketed forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Primary<'s> {
    /// A numeric literal, verbatim (`42`, `0xff`, `1e-10`).
    Number(&'s str),
    /// A string literal, including its quotes (`'hi'`).
    Str(&'s str),
    /// A bare identifier / variable reference.
    Ident(&'s str),
    /// A function call `name(args)`. The callee is always an identifier.
    Call { name: &'s str, args: Args<'s> },
    /// A list literal `[ … ]`.
    List(Args<'s>),
    /// A map literal `{ … }`.
    Map(Vec<MapEntry<'s>>),
    /// A parenthesized body `( … )`; its contents are a full [`Args`] (`top`).
    Paren(Args<'s>),
}

/// One item of a map literal, as tagged by the parser ([`CstKind::MapEntry`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapEntry<'s> {
    /// `key -> value` at the item's top level. The key sits at the assignment
    /// level (anything tighter than `->`); the value is a full expression.
    /// Boxed to keep the enum near [`Single`](Self::Single)'s size.
    Pair {
        key: Box<Assign<'s>>,
        value: Box<Expr<'s>>,
    },
    /// A bare item — a full `;`-statement sequence. The VM classifies its
    /// *value* at runtime: a two-element list is a key→value pair, any other
    /// list an error, anything else a key with a `null` value.
    Single(Code<'s>),
}

// ====================================================================
// Errors
// ====================================================================

/// Why a [`Cst`] could not be lowered to an AST.
///
/// The [`Cst`] carries no source spans, so an error describes *what* was wrong
/// rather than *where*. Beyond an internal shape no well-formed parse produces,
/// lowering enforces the executable shape of assignment targets, so a
/// non-assignable target or a second `...` at one destructuring level is rejected
/// here rather than at evaluation. Likewise, every argument in a function
/// signature must be a valid parameter; a malformed one is an
/// [`InvalidSignature`](Self::InvalidSignature).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowerError {
    /// A node shape no well-formed parse produces reached a position that only
    /// accepts a tighter level (e.g. a bare `,`/`;` chain where a primary was
    /// due). Indicates a malformed or hand-built CST. The text names the kind.
    Unexpected(&'static str),
    /// More than one rest binder (`...x`) appeared at the same level of a
    /// destructuring list ([`LPatterns`]).
    MultipleRest,
    /// The left of `=`/`+=`/`<>` is not a valid assignment target — a literal,
    /// operator expression, `~` match, or other non-place shape (`1 = 2`,
    /// `a + b = c`, `a ~ b = c`, `[a, 1] = …`).
    NotAssignable,
    /// A function parameter is not a variable, an `outer(x)` capture, or the
    /// single `...rest`.
    InvalidSignature,
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerError::Unexpected(what) => {
                write!(f, "unexpected {what} where an expression was required")
            }
            LowerError::MultipleRest => {
                write!(f, "multiple rest binders (`...`) at the same level")
            }
            LowerError::NotAssignable => {
                write!(f, "left-hand side is not a valid assignment target")
            }
            LowerError::InvalidSignature => {
                write!(f, "function definition has invalid parameters")
            }
        }
    }
}

impl std::error::Error for LowerError {}

// ====================================================================
// Lowering: Cst -> AST
// ====================================================================

// Each level type lowers from a borrowed CST node via `TryFrom<&Cst>`, so the
// whole tree converts with `Type::try_from(&cst)` / `cst.try_into()`. Two roots
// are useful: `Args::try_from(&cst)` keeps the grammar's `top` (comma) level,
// while `Code::try_from(&cst)` treats the program as a `;`-separated statement
// sequence (the natural root for a script or REPL line — a top-level `,` then
// has nowhere to go and is a `LowerError`). The conversion borrows `&Cst` rather
// than consuming it: the AST only borrows the source `&'s str`, so the CST stays
// intact for any other use (e.g. the formatter).

/// A `top` / paren body: a possibly-`,`-chained node flattened into an [`Args`].
/// A comma-free node is a one-element `Args` wrapping the statement [`Code`].
impl<'a, 's> TryFrom<&'a Cst<'s>> for Args<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        let mut codes = VecDeque::new();
        collect_comma(cst, &mut codes)?;
        Ok(Args(codes))
    }
}

/// Bracketed contents (`[...]`, `{...}`, call args) — already a slice of
/// per-element [`Code`]s. Phantom `Empty` nodes (trivia anchors and omitted
/// slots) carry no expression and are dropped.
impl<'a, 's> TryFrom<&'a [Cst<'s>]> for Args<'s> {
    type Error = LowerError;
    fn try_from(items: &'a [Cst<'s>]) -> Result<Self, LowerError> {
        let mut codes = VecDeque::with_capacity(items.len());
        for item in items {
            if matches!(item.kind, CstKind::Empty) {
                continue;
            }
            codes.push_back(Code::try_from(item)?);
        }
        Ok(Args(codes))
    }
}

/// Walk a left-nested `,` chain left-to-right, lowering each operand to a
/// [`Code`]. A non-`,` node is the single operand.
fn collect_comma<'s>(cst: &Cst<'s>, out: &mut VecDeque<Code<'s>>) -> Result<(), LowerError> {
    if let CstKind::Binary {
        op: BinOp::Comma,
        lhs,
        rhs,
    } = &cst.kind
    {
        collect_comma(lhs, out)?;
        out.push_back(Code::try_from(rhs.as_ref())?);
    } else {
        out.push_back(Code::try_from(cst)?);
    }
    Ok(())
}

/// A `seq_chain`: a left-nested `;` chain flattened into a [`Code`] — the
/// statement-sequence root for a script or REPL submission.
impl<'a, 's> TryFrom<&'a Cst<'s>> for Code<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        let mut exprs = Vec::new();
        collect_semi(cst, &mut exprs)?;
        Ok(Code(exprs))
    }
}

/// Walk a left-nested `;` chain left-to-right, lowering each statement to an
/// [`Expr`].
fn collect_semi<'s>(cst: &Cst<'s>, out: &mut Vec<Expr<'s>>) -> Result<(), LowerError> {
    if let CstKind::Binary {
        op: BinOp::Semi,
        lhs,
        rhs,
    } = &cst.kind
    {
        collect_semi(lhs, out)?;
        out.push(Expr::try_from(rhs.as_ref())?);
    } else {
        out.push(Expr::try_from(cst)?);
    }
    Ok(())
}

/// `arrow_chain`: the parser tagged every accepted `->` at this level a function
/// definition ([`CstKind::DefineFunction`]) — a map item's key/value arrow
/// lowers with the map instead (see [`MapEntry`]). The parser has already
/// guaranteed the `name(args)` shape; lowering only validates the parameters.
/// A non-`->` node falls through to the assignment level.
impl<'a, 's> TryFrom<&'a Cst<'s>> for Expr<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        match &cst.kind {
            CstKind::DefineFunction { name, args, body } => Ok(Expr::Def {
                name,
                params: lower_params(args).ok_or(LowerError::InvalidSignature)?,
                body: Box::new(Expr::try_from(body.as_ref())?),
            }),
            _ => Ok(Expr::Assign(Assign::try_from(cst)?)),
        }
    }
}

/// `assign` (`=`, `+=`, `<>`; right-associative). The left of the operator is an
/// [`LValue`] target, validated by [`lower_lvalue`].
impl<'a, 's> TryFrom<&'a Cst<'s>> for Assign<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        let op = match &cst.kind {
            CstKind::Binary {
                op: BinOp::Assign, ..
            } => AssignOp::Assign,
            CstKind::Binary {
                op: BinOp::AddAssign,
                ..
            } => AssignOp::Add,
            CstKind::Binary {
                op: BinOp::Swap, ..
            } => AssignOp::Swap,
            _ => return Ok(Assign::Lor(Lor::try_from(cst)?)),
        };
        let CstKind::Binary { lhs, rhs, .. } = &cst.kind else {
            unreachable!("matched a Binary assign op above")
        };
        Ok(Assign::Set {
            target: lower_lvalue(lhs)?,
            op,
            value: Box::new(Assign::try_from(rhs.as_ref())?),
        })
    }
}

// --- lowering: assignment targets and function parameters -----------

/// Lower a CST node to an [`LValue`] assignment target. A list literal is a
/// destructuring list; a call other than `var(…)` is a dynamic
/// [`Computed`](LValue::Computed) place (`if(c, a, b) = …`); anything else must be
/// a single [`Place`] ([`lower_place`]), so a shape that can never be a place
/// (`1 = 2`, `a + b = c`, `a ~ b`) is a [`LowerError::NotAssignable`]. Also drives
/// the recursion for nested destructures, since each element is itself an [`LValue`].
fn lower_lvalue<'s>(cst: &Cst<'s>) -> Result<LValue<'s>, LowerError> {
    match &cst.kind {
        CstKind::List(items) => Ok(LValue::Destructure(lower_lpatterns(items)?)),
        // A call other than `var(…)` may still be a place at runtime —
        // `if(c, a, b) = …` returns one of its bound arguments.
        CstKind::Call { callee, .. } if !matches!(&callee.kind, CstKind::Ident("var")) => {
            Ok(LValue::Computed(Box::new(Primary::try_from(cst)?)))
        }
        // `x`, `var(e)`, `a:b:c` — a single statically-known place.
        _ => Ok(LValue::Place(lower_place(cst)?)),
    }
}

/// Lower a CST node to a single writable [`Place`]: a variable, `var(<expr>)`, or
/// a (possibly nested) `:` index whose base is itself a place. Any other shape is
/// a [`LowerError::NotAssignable`] — in particular `~` is a match, not a writable
/// address.
fn lower_place<'s>(cst: &Cst<'s>) -> Result<Place<'s>, LowerError> {
    match &cst.kind {
        CstKind::Ident(s) => Ok(Place::Var(s)),
        // `var(<expr>)` names a variable dynamically; it takes exactly one argument.
        CstKind::Call { callee, args } if matches!(&callee.kind, CstKind::Ident("var")) => {
            let arg = single_arg(args).ok_or(LowerError::NotAssignable)?;
            Ok(Place::DynVar(Box::new(Code::try_from(arg)?)))
        }
        // `base:key` — a writable element; `~` (Match) is not a target.
        CstKind::Binary {
            op: BinOp::Get,
            lhs,
            rhs,
        } => Ok(Place::Index {
            base: Box::new(lower_place(lhs)?),
            key: Primary::try_from(rhs.as_ref())?,
        }),
        _ => Err(LowerError::NotAssignable),
    }
}

/// Lower the `,`-separated elements of a destructuring list into an [`LPatterns`],
/// dropping phantom `Empty` slots. A `...x` element becomes
/// the single rest binder (a second is a [`LowerError::MultipleRest`]); elements
/// after it go into [`LRest::after`]. Each element is itself an [`LValue`], so a
/// nested destructure recurses.
fn lower_lpatterns<'s>(items: &[Cst<'s>]) -> Result<LPatterns<'s>, LowerError> {
    let mut before = Vec::new();
    let mut rest: Option<LRest<'s>> = None;
    for item in items {
        if matches!(item.kind, CstKind::Empty) {
            continue;
        }
        if let CstKind::Unary {
            op: UnaryOp::Unpack,
            operand,
        } = &item.kind
        {
            if rest.is_some() {
                return Err(LowerError::MultipleRest);
            }
            rest = Some(LRest {
                binder: Box::new(lower_lvalue(operand)?),
                after: Vec::new(),
            });
        } else {
            let elem = lower_lvalue(item)?;
            match &mut rest {
                None => before.push(elem),
                Some(r) => r.after.push(elem),
            }
        }
    }
    Ok(LPatterns { before, rest })
}

/// Try to lower the `,`-separated arguments of a definition's call LHS as a
/// function signature ([`Params`]), dropping phantom `Empty` slots. Each
/// parameter must be a plain binder, an `outer(x)`-style [`Capture`] (a
/// [`ParamWord`]), or the single `...rest`. Returns `None` if any of them is
/// not — a literal, an index, a nested pattern, an unknown reserved word, or a
/// second `...` — in which case the signature is invalid.
fn lower_params<'s>(items: &[Cst<'s>]) -> Option<Params<'s>> {
    let mut fixed = Vec::new();
    let mut captures = Vec::new();
    let mut rest: Option<&'s str> = None;
    for item in items {
        match &item.kind {
            CstKind::Empty => {}
            // `...rest` — the single vararg; its binder must be a plain name.
            CstKind::Unary {
                op: UnaryOp::Unpack,
                operand,
            } => {
                if rest.is_some() {
                    return None;
                }
                let CstKind::Ident(name) = &operand.kind else {
                    return None;
                };
                rest = Some(name);
            }
            // A plain positional binder.
            CstKind::Ident(s) => fixed.push(*s),
            // A reserved-word capture such as `outer(x)`: a known word wrapping one
            // plain name.
            CstKind::Call { callee, args } => {
                let CstKind::Ident(name) = &callee.kind else {
                    return None;
                };
                let word = param_word(name)?;
                let CstKind::Ident(bound) = &single_arg(args)?.kind else {
                    return None;
                };
                captures.push(Capture { word, name: bound });
            }
            // Literals, operator expressions, nested lists, … are not parameters.
            _ => return None,
        }
    }
    Some(Params {
        fixed,
        captures,
        rest,
    })
}

/// Map a signature reserved word to its [`ParamWord`]. Add a case here (and a
/// [`ParamWord`] variant) to introduce a new reserved binder.
fn param_word(name: &str) -> Option<ParamWord> {
    match name {
        "outer" => Some(ParamWord::Outer),
        _ => None,
    }
}

/// The sole non-`Empty` argument of a single-argument call (`var(e)`, `outer(x)`),
/// or `None` when there is not exactly one.
fn single_arg<'a, 's>(args: &'a [Cst<'s>]) -> Option<&'a Cst<'s>> {
    let mut it = args.iter().filter(|c| !matches!(c.kind, CstKind::Empty));
    match (it.next(), it.next()) {
        (Some(arg), None) => Some(arg),
        _ => None,
    }
}

/// `lor` (`||`; left-associative).
impl<'a, 's> TryFrom<&'a Cst<'s>> for Lor<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        if let CstKind::Binary {
            op: BinOp::Or,
            lhs,
            rhs,
        } = &cst.kind
        {
            Ok(Lor::Or {
                lhs: Box::new(Lor::try_from(lhs.as_ref())?),
                rhs: Land::try_from(rhs.as_ref())?,
            })
        } else {
            Ok(Lor::Land(Land::try_from(cst)?))
        }
    }
}

/// `land` (`&&`; left-associative).
impl<'a, 's> TryFrom<&'a Cst<'s>> for Land<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        if let CstKind::Binary {
            op: BinOp::And,
            lhs,
            rhs,
        } = &cst.kind
        {
            Ok(Land::And {
                lhs: Box::new(Land::try_from(lhs.as_ref())?),
                rhs: Equality::try_from(rhs.as_ref())?,
            })
        } else {
            Ok(Land::Equality(Equality::try_from(cst)?))
        }
    }
}

/// `equality` (`==`, `!=`; left-associative).
impl<'a, 's> TryFrom<&'a Cst<'s>> for Equality<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        match &cst.kind {
            CstKind::Binary {
                op: BinOp::Eq,
                lhs,
                rhs,
            } => Ok(Equality::Eq {
                lhs: Box::new(Equality::try_from(lhs.as_ref())?),
                rhs: Compare::try_from(rhs.as_ref())?,
            }),
            CstKind::Binary {
                op: BinOp::NotEq,
                lhs,
                rhs,
            } => Ok(Equality::Ne {
                lhs: Box::new(Equality::try_from(lhs.as_ref())?),
                rhs: Compare::try_from(rhs.as_ref())?,
            }),
            _ => Ok(Equality::Compare(Compare::try_from(cst)?)),
        }
    }
}

/// `compare` (`<`, `<=`, `>`, `>=`; left-associative).
impl<'a, 's> TryFrom<&'a Cst<'s>> for Compare<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        if let CstKind::Binary { op, lhs, rhs } = &cst.kind
            && matches!(op, BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq)
        {
            let lhs = Box::new(Compare::try_from(lhs.as_ref())?);
            let rhs = Additive::try_from(rhs.as_ref())?;
            return Ok(match op {
                BinOp::Lt => Compare::Lt { lhs, rhs },
                BinOp::LtEq => Compare::Le { lhs, rhs },
                BinOp::Gt => Compare::Gt { lhs, rhs },
                _ => Compare::Ge { lhs, rhs },
            });
        }
        Ok(Compare::Additive(Additive::try_from(cst)?))
    }
}

/// `additive` (`+`, `-`; left-associative).
impl<'a, 's> TryFrom<&'a Cst<'s>> for Additive<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        match &cst.kind {
            CstKind::Binary {
                op: BinOp::Add,
                lhs,
                rhs,
            } => Ok(Additive::Add {
                lhs: Box::new(Additive::try_from(lhs.as_ref())?),
                rhs: Mult::try_from(rhs.as_ref())?,
            }),
            CstKind::Binary {
                op: BinOp::Sub,
                lhs,
                rhs,
            } => Ok(Additive::Sub {
                lhs: Box::new(Additive::try_from(lhs.as_ref())?),
                rhs: Mult::try_from(rhs.as_ref())?,
            }),
            _ => Ok(Additive::Mult(Mult::try_from(cst)?)),
        }
    }
}

/// `multiplicative` (`*`, `/`, `%`; left-associative).
impl<'a, 's> TryFrom<&'a Cst<'s>> for Mult<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        if let CstKind::Binary { op, lhs, rhs } = &cst.kind
            && matches!(op, BinOp::Mul | BinOp::Div | BinOp::Rem)
        {
            let lhs = Box::new(Mult::try_from(lhs.as_ref())?);
            let rhs = Power::try_from(rhs.as_ref())?;
            return Ok(match op {
                BinOp::Mul => Mult::Mul { lhs, rhs },
                BinOp::Div => Mult::Div { lhs, rhs },
                _ => Mult::Rem { lhs, rhs },
            });
        }
        Ok(Mult::Power(Power::try_from(cst)?))
    }
}

/// `power` (`^`; right-associative).
impl<'a, 's> TryFrom<&'a Cst<'s>> for Power<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        if let CstKind::Binary {
            op: BinOp::Pow,
            lhs,
            rhs,
        } = &cst.kind
        {
            Ok(Power::Pow {
                base: Unary::try_from(lhs.as_ref())?,
                exp: Box::new(Power::try_from(rhs.as_ref())?),
            })
        } else {
            Ok(Power::Unary(Unary::try_from(cst)?))
        }
    }
}

/// `unary` (prefix `-`, `+`, `!`, `...`).
impl<'a, 's> TryFrom<&'a Cst<'s>> for Unary<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        if let CstKind::Unary { op, operand } = &cst.kind {
            let inner = Box::new(Unary::try_from(operand.as_ref())?);
            return Ok(match op {
                UnaryOp::Neg => Unary::Neg(inner),
                UnaryOp::Pos => Unary::Pos(inner),
                UnaryOp::Not => Unary::Not(inner),
                UnaryOp::Unpack => Unary::Unpack(inner),
            });
        }
        Ok(Unary::Get(Get::try_from(cst)?))
    }
}

/// `get` (`:`, `~`; left-associative). The right operand is always a [`Primary`].
impl<'a, 's> TryFrom<&'a Cst<'s>> for Get<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        if let CstKind::Binary {
            op: op @ (BinOp::Get | BinOp::Match),
            lhs,
            rhs,
        } = &cst.kind
        {
            Ok(Get::Index {
                base: Box::new(Get::try_from(lhs.as_ref())?),
                op: get_op(*op),
                key: Primary::try_from(rhs.as_ref())?,
            })
        } else {
            Ok(Get::Primary(Primary::try_from(cst)?))
        }
    }
}

/// `primary`: an atom, a call, or a bracketed form.
impl<'a, 's> TryFrom<&'a Cst<'s>> for Primary<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        match &cst.kind {
            CstKind::Number(s) => Ok(Primary::Number(s)),
            CstKind::Str(s) => Ok(Primary::Str(s)),
            CstKind::Ident(s) => Ok(Primary::Ident(s)),
            CstKind::Call { callee, args } => Ok(Primary::Call {
                name: ident_of(callee)?,
                args: Args::try_from(args.as_slice())?,
            }),
            CstKind::List(items) => Ok(Primary::List(Args::try_from(items.as_slice())?)),
            CstKind::Map(items) => Ok(Primary::Map(lower_map_entries(items)?)),
            CstKind::Paren(inner) => Ok(Primary::Paren(Args::try_from(inner.as_ref())?)),
            other => Err(LowerError::Unexpected(describe(other))),
        }
    }
}

/// The items of a map literal: each is a [`CstKind::MapEntry`] tagged by the
/// parser, or a phantom `Empty` (a trivia anchor / omitted slot), dropped as
/// in [`Args`].
fn lower_map_entries<'s>(items: &[Cst<'s>]) -> Result<Vec<MapEntry<'s>>, LowerError> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match &item.kind {
            CstKind::Empty => {}
            CstKind::MapEntry {
                key,
                value: Some(value),
            } => out.push(MapEntry::Pair {
                key: Box::new(Assign::try_from(key.as_ref())?),
                value: Box::new(Expr::try_from(value.as_ref())?),
            }),
            CstKind::MapEntry { key, value: None } => {
                out.push(MapEntry::Single(Code::try_from(key.as_ref())?));
            }
            other => return Err(LowerError::Unexpected(describe(other))),
        }
    }
    Ok(out)
}

// --- small helpers --------------------------------------------------

/// Map the two get-level [`BinOp`]s onto a [`GetOp`].
fn get_op(op: BinOp) -> GetOp {
    match op {
        BinOp::Match => GetOp::Match,
        _ => GetOp::Get,
    }
}

/// Pull the identifier out of a call's callee. The parser only ever builds an
/// `Ident` callee; anything else is a malformed CST.
fn ident_of<'s>(cst: &Cst<'s>) -> Result<&'s str, LowerError> {
    match &cst.kind {
        CstKind::Ident(s) => Ok(s),
        other => Err(LowerError::Unexpected(describe(other))),
    }
}

/// A short, static name for a [`CstKind`], used in [`LowerError`] messages.
fn describe(kind: &CstKind<'_>) -> &'static str {
    match kind {
        CstKind::Number(_) => "a number",
        CstKind::Str(_) => "a string",
        CstKind::Ident(_) => "an identifier",
        CstKind::Call { .. } => "a call",
        CstKind::List(_) => "a list",
        CstKind::Map(_) => "a map",
        CstKind::Paren(_) => "a parenthesized expression",
        CstKind::Empty => "an empty slot",
        CstKind::DefineFunction { .. } => "a function definition",
        CstKind::MapEntry { .. } => "a map entry",
        CstKind::Unary { .. } => "a unary expression",
        CstKind::Binary { .. } => "an operator expression",
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;

    /// Parse and lower, expecting success.
    fn ast(src: &str) -> Args<'_> {
        Args::try_from(&parse_source(src).expect("parse error")).expect("lower error")
    }

    /// Parse and lower, expecting a lowering failure.
    fn lower_err(src: &str) -> LowerError {
        Args::try_from(&parse_source(src).expect("parse error")).expect_err("expected lower error")
    }

    // --- Primary constructors / passthrough lifters (build expected trees) ---

    fn num(s: &str) -> Primary<'_> {
        Primary::Number(s)
    }
    fn p_mult(p: Primary<'_>) -> Mult<'_> {
        Mult::Power(Power::Unary(Unary::Get(Get::Primary(p))))
    }
    fn p_add(p: Primary<'_>) -> Additive<'_> {
        Additive::Mult(p_mult(p))
    }

    // --- descend a single-expression program to a given level ----------------

    fn only_expr<'a, 's>(a: &'a Args<'s>) -> &'a Expr<'s> {
        let Args(codes) = a;
        assert_eq!(codes.len(), 1, "expected one top-level statement-list");
        let Code(exprs) = &codes[0];
        assert_eq!(exprs.len(), 1, "expected a single statement");
        &exprs[0]
    }

    fn as_additive<'a, 's>(e: &'a Expr<'s>) -> &'a Additive<'s> {
        let Expr::Assign(Assign::Lor(Lor::Land(Land::Equality(Equality::Compare(
            Compare::Additive(a),
        ))))) = e
        else {
            panic!("not a bare additive: {e:?}");
        };
        a
    }

    fn as_unary<'a, 's>(e: &'a Expr<'s>) -> &'a Unary<'s> {
        let Additive::Mult(Mult::Power(Power::Unary(u))) = as_additive(e) else {
            panic!("not a bare unary: {e:?}");
        };
        u
    }

    fn as_power<'a, 's>(e: &'a Expr<'s>) -> &'a Power<'s> {
        let Additive::Mult(Mult::Power(p)) = as_additive(e) else {
            panic!("not a bare power: {e:?}");
        };
        p
    }

    fn as_get<'a, 's>(e: &'a Expr<'s>) -> &'a Get<'s> {
        let Unary::Get(g) = as_unary(e) else {
            panic!("not a bare get: {e:?}");
        };
        g
    }

    fn prim_of_expr<'a, 's>(e: &'a Expr<'s>) -> &'a Primary<'s> {
        let Get::Primary(p) = as_get(e) else {
            panic!("not a bare primary: {e:?}");
        };
        p
    }

    /// The variable name of a plain-variable [`LValue::Place`], for inspecting an
    /// assignment target or a destructure element.
    fn place_var_name<'s>(lv: &LValue<'s>) -> &'s str {
        let LValue::Place(Place::Var(name)) = lv else {
            panic!("not a variable place: {lv:?}");
        };
        name
    }

    // --- Code root (statement-sequence) --------------------------------------

    #[test]
    fn code_roots_a_program_at_the_statement_sequence() {
        // `Code::try_from` skips the `Args` wrapper: the root is the `;`-chain.
        let cst = parse_source("a; b; c").expect("parse error");
        let Code(exprs) = Code::try_from(&cst).expect("lower error");
        assert_eq!(exprs.len(), 3);
        assert_eq!(prim_of_expr(&exprs[0]), &Primary::Ident("a"));
        assert_eq!(prim_of_expr(&exprs[2]), &Primary::Ident("c"));

        // A lone expression is a one-statement Code.
        let cst = parse_source("42").expect("parse error");
        let Code(exprs) = Code::try_from(&cst).expect("lower error");
        assert_eq!(exprs.len(), 1);
    }

    #[test]
    fn code_rejects_a_top_level_comma() {
        // A `Code` has no comma level, so a top-level `,` cannot be lowered.
        let cst = parse_source("a, b").expect("parse error");
        assert_eq!(
            Code::try_from(&cst).unwrap_err(),
            LowerError::Unexpected("an operator expression")
        );
    }

    // --- atoms / passthrough -------------------------------------------------

    #[test]
    fn bare_atoms_lower_through_every_level() {
        assert_eq!(prim_of_expr(only_expr(&ast("42"))), &Primary::Number("42"));
        assert_eq!(prim_of_expr(only_expr(&ast("'hi'"))), &Primary::Str("'hi'"));
        assert_eq!(prim_of_expr(only_expr(&ast("foo"))), &Primary::Ident("foo"));
    }

    // --- arithmetic precedence / associativity -------------------------------

    #[test]
    fn additive_over_multiplicative() {
        // `2 + 3 * 4` → Add(2, Mul(3, 4))
        let a = ast("2 + 3 * 4");
        assert_eq!(
            as_additive(only_expr(&a)),
            &Additive::Add {
                lhs: Box::new(p_add(num("2"))),
                rhs: Mult::Mul {
                    lhs: Box::new(p_mult(num("3"))),
                    rhs: Power::Unary(Unary::Get(Get::Primary(num("4")))),
                },
            }
        );
    }

    #[test]
    fn additive_is_left_associative() {
        // `2 + 3 - 1` → Sub(Add(2, 3), 1)
        let a = ast("2 + 3 - 1");
        assert_eq!(
            as_additive(only_expr(&a)),
            &Additive::Sub {
                lhs: Box::new(Additive::Add {
                    lhs: Box::new(p_add(num("2"))),
                    rhs: p_mult(num("3")),
                }),
                rhs: p_mult(num("1")),
            }
        );
    }

    #[test]
    fn power_is_right_associative() {
        // `2 ^ 3 ^ 2` → Pow(2, Pow(3, 2))
        let a = ast("2 ^ 3 ^ 2");
        assert_eq!(
            as_power(only_expr(&a)),
            &Power::Pow {
                base: Unary::Get(Get::Primary(num("2"))),
                exp: Box::new(Power::Pow {
                    base: Unary::Get(Get::Primary(num("3"))),
                    exp: Box::new(Power::Unary(Unary::Get(Get::Primary(num("2"))))),
                }),
            }
        );
    }

    #[test]
    fn unary_wraps_a_get_chain() {
        // `-foo:0` → Neg(foo:0)
        let a = ast("-foo:0");
        let Unary::Neg(inner) = as_unary(only_expr(&a)) else {
            panic!("expected Neg");
        };
        let Unary::Get(Get::Index { base, op, key }) = inner.as_ref() else {
            panic!("expected a get under the negation");
        };
        assert_eq!(base.as_ref(), &Get::Primary(Primary::Ident("foo")));
        assert_eq!(*op, GetOp::Get);
        assert_eq!(key, &Primary::Number("0"));
    }

    #[test]
    fn get_chain_is_left_associative() {
        // `a:b:c` → (a:b):c
        let a = ast("a:b:c");
        let Get::Index { base, op, key } = as_get(only_expr(&a)) else {
            panic!("expected an index");
        };
        assert_eq!(*op, GetOp::Get);
        assert_eq!(key, &Primary::Ident("c"));
        assert_eq!(
            base.as_ref(),
            &Get::Index {
                base: Box::new(Get::Primary(Primary::Ident("a"))),
                op: GetOp::Get,
                key: Primary::Ident("b"),
            }
        );
    }

    #[test]
    fn match_operator_lowers_to_match_get() {
        let a = ast("a~b");
        let Get::Index { op, .. } = as_get(only_expr(&a)) else {
            panic!("expected an index");
        };
        assert_eq!(*op, GetOp::Match);
    }

    // --- sequences -----------------------------------------------------------

    #[test]
    fn semicolons_build_a_statement_code() {
        let a = ast("a; b; c");
        let Args(codes) = &a;
        assert_eq!(codes.len(), 1);
        let Code(exprs) = &codes[0];
        assert_eq!(exprs.len(), 3);
        assert_eq!(prim_of_expr(&exprs[0]), &Primary::Ident("a"));
        assert_eq!(prim_of_expr(&exprs[2]), &Primary::Ident("c"));
    }

    #[test]
    fn top_level_commas_build_args() {
        // `,` is looser than `;`, so `a; b , c` is two args: `a; b` and `c`.
        let a = ast("a; b, c");
        let Args(codes) = &a;
        assert_eq!(codes.len(), 2);
        assert_eq!(codes[0].0.len(), 2); // a ; b
        assert_eq!(codes[1].0.len(), 1); // c
        assert_eq!(prim_of_expr(&codes[1].0[0]), &Primary::Ident("c"));
    }

    #[test]
    fn parens_lower_to_a_primary_args_body() {
        let a = ast("(a + b)");
        let Primary::Paren(Args(codes)) = prim_of_expr(only_expr(&a)) else {
            panic!("expected a paren");
        };
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].0.len(), 1);
        // the body is `a + b`
        as_additive(&codes[0].0[0]);
    }

    // --- function definitions ------------------------------------------------

    #[test]
    fn function_definition_with_call_lhs() {
        let a = ast("foo(a, b) -> a + b");
        let Expr::Def { name, params, body } = only_expr(&a) else {
            panic!("expected a Def");
        };
        assert_eq!(*name, "foo");
        // params: two positional binders, no captures, no rest.
        assert_eq!(params.fixed, ["a", "b"]);
        assert!(params.captures.is_empty());
        assert!(params.rest.is_none());
        // body is `a + b`
        as_additive(body);
    }

    #[test]
    fn anonymous_function_is_a_def_named_underscore() {
        let a = ast("_(x) -> x * x");
        let Expr::Def { name, params, .. } = only_expr(&a) else {
            panic!("expected a Def");
        };
        assert_eq!(*name, "_");
        assert_eq!(params.fixed, ["x"]);
        assert!(params.rest.is_none());
    }

    #[test]
    fn rest_parameter_lands_in_the_rest_slot() {
        // `...rest` is the signature's single vararg.
        let a = ast("f(a, ...rest) -> rest");
        let Expr::Def { params, .. } = only_expr(&a) else {
            panic!("expected a Def");
        };
        assert_eq!(params.fixed, ["a"]);
        assert_eq!(params.rest, Some("rest"));
    }

    #[test]
    fn invalid_function_parameters_are_an_error() {
        // Scarpet allows only variables / `outer` / `...rest` in a signature,
        // so a literal arg (`str('add', _)`), an index arg
        // (`f(a:0)`), or an unknown reserved word (`f(inner(x))`) all fail.
        for src in ["str('add', _) -> x", "f(a:0) -> x", "f(inner(x)) -> x"] {
            assert_eq!(lower_err(src), LowerError::InvalidSignature, "{src}");
        }
    }

    #[test]
    fn outer_capture_parameter_lowers_to_a_capture() {
        // `outer(x)` is a reserved-word capture; it does not occupy a position.
        let a = ast("_(a, outer(x)) -> x");
        let Expr::Def { params, .. } = only_expr(&a) else {
            panic!("expected a Def");
        };
        assert_eq!(params.fixed, ["a"]);
        assert_eq!(
            params.captures,
            vec![Capture {
                word: ParamWord::Outer,
                name: "x"
            }]
        );
        assert!(params.rest.is_none());
    }

    // --- assignments ---------------------------------------------------------

    #[test]
    fn assignment_is_right_associative() {
        // `a = b = 5` → Set(a, Set(b, 5))
        let a = ast("a = b = 5");
        let Expr::Assign(Assign::Set { target, op, value }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        assert_eq!(place_var_name(target), "a");
        assert_eq!(*op, AssignOp::Assign);
        let Assign::Set {
            target: t2,
            op: op2,
            ..
        } = value.as_ref()
        else {
            panic!("expected a nested Set");
        };
        assert_eq!(place_var_name(t2), "b");
        assert_eq!(*op2, AssignOp::Assign);
    }

    #[test]
    fn add_assign_and_swap_operators() {
        let a = ast("a += 1");
        let Expr::Assign(Assign::Set { op, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        assert_eq!(*op, AssignOp::Add);

        let a = ast("a <> b");
        let Expr::Assign(Assign::Set { op, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        assert_eq!(*op, AssignOp::Swap);
    }

    #[test]
    fn indexed_assignment_target() {
        // `x:0 = 5` → an indexed place rooted at the variable `x`.
        let a = ast("x:0 = 5");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let LValue::Place(Place::Index { base, key }) = target else {
            panic!("expected an indexed place: {target:?}");
        };
        assert_eq!(base.as_ref(), &Place::Var("x"));
        assert_eq!(key, &Primary::Number("0"));
    }

    #[test]
    fn nested_index_assignment_target() {
        // `a:b:c = 5` → Index(Index(a, b), c): the base of an index is itself a place.
        let a = ast("a:b:c = 5");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let LValue::Place(Place::Index { base, key }) = target else {
            panic!("expected an indexed place");
        };
        assert_eq!(key, &Primary::Ident("c"));
        let Place::Index { base, key } = base.as_ref() else {
            panic!("expected a nested index");
        };
        assert_eq!(base.as_ref(), &Place::Var("a"));
        assert_eq!(key, &Primary::Ident("b"));
    }

    #[test]
    fn destructuring_list_assignment_target() {
        // `[a, b] = t` — a destructure of two variable places.
        let a = ast("[a, b] = t");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let LValue::Destructure(pats) = target else {
            panic!("expected a destructure: {target:?}");
        };
        assert!(pats.rest.is_none());
        let names: Vec<_> = pats.before.iter().map(place_var_name).collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn destructuring_call_assignment_target() {
        // `l(x, y, z) = p` — the `l(...)` constructor is the same destructure as `[…]`.
        let a = ast("l(x, y, z) = p");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let LValue::Destructure(pats) = target else {
            panic!("expected a destructure: {target:?}");
        };
        let names: Vec<_> = pats.before.iter().map(place_var_name).collect();
        assert_eq!(names, ["x", "y", "z"]);
    }

    #[test]
    fn dynamic_var_assignment_target() {
        // `var(<expr>) = …` lowers to a `DynVar` place holding the name expression.
        let a = ast("var('global_' + s) = 1");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let LValue::Place(Place::DynVar(arg)) = target else {
            panic!("expected a DynVar place: {target:?}");
        };
        // the argument is `'global_' + s`, an additive expression
        let Code(exprs) = arg.as_ref();
        assert_eq!(exprs.len(), 1);
        as_additive(&exprs[0]);
    }

    #[test]
    fn rest_in_a_list_target_binds_a_rest() {
        // `[a, ...r, b] = t` — `...r` is the single rest binder; `a` is bound from
        // the front and `b` from the back, the one-rest-per-level rule keeping the
        // split unambiguous.
        let a = ast("[a, ...r, b] = t");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let LValue::Destructure(pats) = target else {
            panic!("expected a destructure");
        };
        assert_eq!(pats.before.len(), 1);
        assert_eq!(place_var_name(&pats.before[0]), "a");
        let rest = pats.rest.as_ref().expect("expected a rest binder");
        assert_eq!(place_var_name(&rest.binder), "r");
        assert_eq!(rest.after.len(), 1);
        assert_eq!(place_var_name(&rest.after[0]), "b");
    }

    #[test]
    fn nested_destructure_target() {
        // `[a, [...b, c]] = t` — a destructure element is itself an `LValue`, so a
        // nested destructure (with its own rest) recurses.
        let a = ast("[a, [...b, c]] = t");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let LValue::Destructure(pats) = target else {
            panic!("expected a destructure");
        };
        assert_eq!(pats.before.len(), 2);
        assert_eq!(place_var_name(&pats.before[0]), "a");
        let LValue::Destructure(inner) = &pats.before[1] else {
            panic!("expected a nested destructure");
        };
        assert!(inner.before.is_empty());
        let rest = inner.rest.as_ref().expect("expected an inner rest");
        assert_eq!(place_var_name(&rest.binder), "b");
        assert_eq!(place_var_name(&rest.after[0]), "c");
    }

    #[test]
    fn rest_binder_into_a_nested_destructure() {
        // `[a, ...[p, q]] = t` — the rest binder is itself an `LValue`, here a
        // nested destructure.
        let a = ast("[a, ...[p, q]] = t");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let LValue::Destructure(pats) = target else {
            panic!("expected a destructure");
        };
        let rest = pats.rest.as_ref().expect("expected a rest binder");
        let LValue::Destructure(inner) = rest.binder.as_ref() else {
            panic!("expected the rest binder to be a destructure");
        };
        let names: Vec<_> = inner.before.iter().map(place_var_name).collect();
        assert_eq!(names, ["p", "q"]);
    }

    #[test]
    fn computed_call_assignment_target() {
        // `if(c, a, b) = …` — a call that may return a bound place at runtime
        // lowers to a `Computed` target (Scarpet's dynamic l-value model).
        let a = ast("if(c, a, b) = 1");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let LValue::Computed(call) = target else {
            panic!("expected a computed target: {target:?}");
        };
        let Primary::Call { name, .. } = call.as_ref() else {
            panic!("expected a call");
        };
        assert_eq!(*name, "if");
    }

    #[test]
    fn multiple_rest_binders_in_a_destructure_are_an_error() {
        // Two `...` at the same destructuring level cannot be represented. In a
        // *signature* a second `...` likewise invalidates the definition.
        assert_eq!(lower_err("[...a, ...b] = t"), LowerError::MultipleRest);
        assert_eq!(
            lower_err("f(...a, ...b) -> 0"),
            LowerError::InvalidSignature
        );
    }

    #[test]
    fn non_assignable_targets_are_rejected_at_lowering() {
        // The target is an `LValue` now, so a shape that can never be a place fails
        // at lowering rather than evaluation: a literal, an operator or
        // parenthesised expression, `~` match, and a literal destructure element.
        // (A call like `if(…)` is the one exception — it may be a place at runtime,
        // so it lowers to `LValue::Computed`.)
        for src in ["1 = 2", "a + b = c", "(a) = b", "a ~ b = c", "[a, 1] = t"] {
            assert_eq!(lower_err(src), LowerError::NotAssignable, "{src}");
        }
    }

    // --- primaries: call / list / map ----------------------------------------

    #[test]
    fn call_list_and_map_primaries() {
        let a = ast("print('hi')");
        let Primary::Call { name, args } = prim_of_expr(only_expr(&a)) else {
            panic!("expected a call");
        };
        assert_eq!(*name, "print");
        assert_eq!(args.0.len(), 1);

        let a = ast("[1, 2, 3]");
        let Primary::List(Args(codes)) = prim_of_expr(only_expr(&a)) else {
            panic!("expected a list");
        };
        assert_eq!(codes.len(), 3);

        let a = ast("l(1, 2, 3)");
        let Primary::List(Args(codes)) = prim_of_expr(only_expr(&a)) else {
            panic!("expected a list");
        };
        assert_eq!(codes.len(), 3);

        let a = ast("{'a' -> 1}");
        let Primary::Map(entries) = prim_of_expr(only_expr(&a)) else {
            panic!("expected a map");
        };
        assert_eq!(entries.len(), 1);
        // the single entry is a key/value pair
        assert!(matches!(entries[0], MapEntry::Pair { .. }));
    }

    #[test]
    fn map_entry_forms_lower_as_tagged() {
        // `k -> v` is a pair; a list (`[k, v]`, `l(k, v)`) or bare key is a
        // `Single` whose *value* the VM classifies at runtime.
        let a = ast("{'a' -> 1, [1, 2], l(1, 2), 'k'}");
        let Primary::Map(entries) = prim_of_expr(only_expr(&a)) else {
            panic!("expected a map");
        };
        assert_eq!(entries.len(), 4);
        assert!(matches!(entries[0], MapEntry::Pair { .. }));
        for entry in &entries[1..] {
            assert!(matches!(entry, MapEntry::Single(_)));
        }
    }

    #[test]
    fn map_entry_with_statement_chain_is_a_single() {
        // `{f(x) -> v; w}` — the `;` makes the whole item a bare key, whose
        // chain holds a definition followed by `w`.
        let a = ast("{f(x) -> v; w}");
        let Primary::Map(entries) = prim_of_expr(only_expr(&a)) else {
            panic!("expected a map");
        };
        let [MapEntry::Single(Code(exprs))] = entries.as_slice() else {
            panic!("expected one Single entry: {entries:?}");
        };
        assert_eq!(exprs.len(), 2);
        assert!(matches!(exprs[0], Expr::Def { .. }));

        // A non-call arrow in this position is rejected by the parser rather
        // than reaching AST lowering.
        assert!(parse_source("{'k' -> v; w}").is_err());
    }

    // --- trivia / empties ----------------------------------------------------

    #[test]
    fn trivia_is_dropped() {
        // Comments and breaks don't survive lowering: the AST matches the
        // comment-free source exactly.
        assert_eq!(ast("// header\n a + b\n"), ast("a + b"));
    }

    #[test]
    fn empty_call_with_only_a_comment_has_no_args() {
        // `f(// note\n)` parses to a phantom `Empty` arg; lowering drops it.
        let a = ast("f(// note\n)");
        let Primary::Call { args, .. } = prim_of_expr(only_expr(&a)) else {
            panic!("expected a call");
        };
        assert_eq!(args.0.len(), 0);
    }

    #[test]
    fn omitted_argument_slot_is_dropped() {
        // `f(a, , b)` has a phantom `Empty` middle arg; lowering drops it, so
        // the call carries two arguments.
        let a = ast("f(a, , b)");
        let Primary::Call { args, .. } = prim_of_expr(only_expr(&a)) else {
            panic!("expected a call");
        };
        assert_eq!(args.0.len(), 2);
    }
}

/// Lower every `example/` corpus file to prove the AST covers real Scarpet:
/// every file that parses must also lower without a [`LowerError`]. Skips
/// quietly when the `example/` submodule isn't checked out.
#[cfg(test)]
mod corpus {
    use super::*;
    use crate::parser::parse_source;
    use std::path::{Path, PathBuf};

    /// Files whose Scarpet source is not accepted by the parser, so there is no
    /// CST to lower. Mirrors the list in `scarpet-fmt`.
    const KNOWN_BAD: &[&str] = &[
        "gnembon/scarpet/programs/survival/portalorient.sc",
        "gnembon/scarpet/programs/survival/rifts/rifts.sc",
        "Ghoulboy78/Scarpet-edit/se.sc",
        "51mayday/ScarpetScripts/geo_v0.2.1_dev.sc",
        "CommandLeo/scarpet/programs/getallitems.sc",
        "CommandLeo/scarpet/programs/randomizer.sc",
        "CommandLeo/scarpet/programs/stx.sc",
    ];

    fn corpus_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("example")
    }

    fn walk_scripts(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk_scripts(&p, out);
            } else if matches!(
                p.extension().and_then(|extension| extension.to_str()),
                Some("sc" | "scl")
            ) {
                out.push(p);
            }
        }
    }

    #[test]
    fn every_parseable_corpus_file_lowers() {
        let root = corpus_root();
        if !root.is_dir() {
            eprintln!(
                "skipping AST corpus test: {} absent (run `git submodule update --init`)",
                root.display()
            );
            return;
        }
        let mut files = Vec::new();
        walk_scripts(&root, &mut files);
        files.sort();

        let mut failures = Vec::new();
        for f in &files {
            let rel = f
                .strip_prefix(&root)
                .unwrap_or(f)
                .to_string_lossy()
                .replace('\\', "/");
            if KNOWN_BAD.contains(&rel.as_str()) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(f) else {
                continue;
            };
            let Ok(cst) = parse_source(&src) else {
                // Parse failures are the formatter's concern, not the AST's.
                continue;
            };
            if let Err(e) = Args::try_from(&cst) {
                failures.push(format!("{rel}: {e}"));
            }
        }
        assert!(
            failures.is_empty(),
            "AST lowering failures ({}):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
