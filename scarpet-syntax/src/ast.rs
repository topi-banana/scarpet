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
//! - `->` → [`Expr`] — a function definition [`Expr::Def`] when the left of the
//!   arrow is a call, otherwise a generic [`Expr::Arrow`]
//! - `=` `+=` `<>` → [`Assign`] — an [`Assignable`] target with an [`AssignOp`]
//! - `||` `&&` `==`/`!=` `<`/`<=`/`>`/`>=` `+`/`-` `*`/`/`/`%` `^` → one enum
//!   per level ([`Lor`], [`Land`], [`Equality`], [`Compare`], [`Additive`],
//!   [`Mult`], [`Power`])
//! - prefix `-` `+` `!` `...` → [`Unary`]
//! - `~` `:` → [`Get`]
//! - atoms, calls, `[...]`, `{...}`, `(...)` → [`Primary`]
//!
//! Lowering is fallible only where an [`Assignable`] is required — the left of
//! `=`/`+=`/`<>` — so `1 = 2` or `a + b = c` yields [`LowerError`] rather than a
//! silently wrong tree. Function parameters, by contrast, are general
//! expressions (Scarpet dispatches on literal patterns like `f('add', x) -> …`),
//! so they never constrain to an assignable.
//!
//! Every borrowed `&'s str` points back into the original source, exactly as in
//! the [`Cst`]; lowering allocates only the `Vec`/`Box` spine.

use crate::parser::{BinOp, Cst, CstKind, UnaryOp};

// ====================================================================
// AST — one type per precedence level (low → high)
// ====================================================================

/// `,`-separated sequence: call arguments, list / map elements, a parenthesized
/// body, and the program root (the grammar's `top`). Each element is a full
/// [`Code`] (statement sequence), so `[a; b, c]` is two elements `a; b` and `c`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args<'s>(pub Vec<Code<'s>>);

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
    /// `params` are general expressions, not [`Assignable`]s: besides plain
    /// binders (`a`), rest binders (`...rest`), and `outer(x)` captures, Scarpet
    /// allows *literal pattern* parameters for value dispatch (`f('add', x) ->
    /// …`, `f(1) -> …`). It is the evaluator that classifies each by shape.
    Def {
        name: &'s str,
        params: Args<'s>,
        body: Box<Expr<'s>>,
    },
    /// `lhs -> body` where `lhs` is not a call — a map entry (`'k' -> v`) or
    /// any other arrow whose left side is an ordinary expression.
    Arrow {
        lhs: Assign<'s>,
        body: Box<Expr<'s>>,
    },
    /// No `->` at this level — an assignment-or-tighter expression.
    Assign(Assign<'s>),
}

/// The assignment level (`=`, `+=`, `<>`). Right-associative, so `a = b = 5`
/// nests on the right. The left of the operator is an [`Assignable`] target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assign<'s> {
    /// `target <op> value`.
    Set {
        target: Assignable<'s>,
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

/// The left-hand side of an assignment (`=`, `+=`, `<>`). Beyond the bare
/// variable and destructuring list, this covers the target shapes that occur in
/// real Scarpet: indexed targets (`x:0`, `m:'k'`, `obj~'f'`), rest binders in a
/// destructure (`[a, ...rest]`), and call-shaped targets (`l(a, b)`,
/// `var('x' + i)`, `if(cond, a, b)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assignable<'s> {
    /// A plain variable: `x`.
    Var(&'s str),
    /// A destructuring list: `[a, b, c]`.
    List(Vec<Assignable<'s>>),
    /// An indexed target: `base:key` (`op == Get`) or `base~key` (`op == Match`).
    /// The key is a [`Primary`] (the grammar only allows a primary there).
    Index {
        base: Box<Assignable<'s>>,
        op: GetOp,
        key: Primary<'s>,
    },
    /// A rest binder / spread target: `...rest`.
    Rest(Box<Assignable<'s>>),
    /// A call-shaped target: a destructuring constructor (`l(a, b)`), a
    /// dynamic-variable access (`var('x' + i)`), or an l-value-returning call
    /// (`if(cond, a, b)`). The arguments are general expressions — the parser
    /// cannot tell a binder from a computed name — so the evaluator interprets
    /// them per the called function.
    Call { name: &'s str, args: Args<'s> },
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

/// Which operator joins a [`Get::Index`] / [`Assignable::Index`].
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
    Map(Args<'s>),
    /// A parenthesized body `( … )`; its contents are a full [`Args`] (`top`).
    Paren(Args<'s>),
}

// ====================================================================
// Errors
// ====================================================================

/// Why a [`Cst`] could not be lowered to an AST.
///
/// The [`Cst`] carries no source spans, so an error describes *what* was wrong
/// rather than *where*. The two cases are a non-assignable target where one was
/// required, and an internal shape that a well-formed parse never produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowerError {
    /// A construct appeared where an [`Assignable`] was required — the left of
    /// `=`/`+=`/`<>`, or a function parameter — but cannot be one (a literal, an
    /// operator expression, a map, …). The text names the offending node kind.
    NotAssignable(&'static str),
    /// A node shape no well-formed parse produces reached a position that only
    /// accepts a tighter level (e.g. a bare `,`/`;` chain where a primary was
    /// due). Indicates a malformed or hand-built CST. The text names the kind.
    Unexpected(&'static str),
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerError::NotAssignable(what) => {
                write!(f, "cannot assign to {what}")
            }
            LowerError::Unexpected(what) => {
                write!(f, "unexpected {what} where an expression was required")
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
        let mut codes = Vec::new();
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
        let mut codes = Vec::with_capacity(items.len());
        for item in items {
            if matches!(item.kind, CstKind::Empty) {
                continue;
            }
            codes.push(Code::try_from(item)?);
        }
        Ok(Args(codes))
    }
}

/// Walk a left-nested `,` chain left-to-right, lowering each operand to a
/// [`Code`]. A non-`,` node is the single operand.
fn collect_comma<'s>(cst: &Cst<'s>, out: &mut Vec<Code<'s>>) -> Result<(), LowerError> {
    if let CstKind::Binary {
        op: BinOp::Comma,
        lhs,
        rhs,
    } = &cst.kind
    {
        collect_comma(lhs, out)?;
        out.push(Code::try_from(rhs.as_ref())?);
    } else {
        out.push(Code::try_from(cst)?);
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

/// `arrow_chain`: a `->` whose left side is a call is a function definition; any
/// other `->` is a generic arrow; no `->` falls through to the assignment level.
impl<'a, 's> TryFrom<&'a Cst<'s>> for Expr<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        if let CstKind::Binary {
            op: BinOp::Arrow,
            lhs,
            rhs,
        } = &cst.kind
        {
            let body = Box::new(Expr::try_from(rhs.as_ref())?);
            if let CstKind::Call { callee, args } = &lhs.kind {
                Ok(Expr::Def {
                    name: ident_of(callee)?,
                    params: Args::try_from(args.as_slice())?,
                    body,
                })
            } else {
                Ok(Expr::Arrow {
                    lhs: Assign::try_from(lhs.as_ref())?,
                    body,
                })
            }
        } else {
            Ok(Expr::Assign(Assign::try_from(cst)?))
        }
    }
}

/// `assign` (`=`, `+=`, `<>`; right-associative). The left of the operator is an
/// [`Assignable`] target.
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
            target: Assignable::try_from(lhs.as_ref())?,
            op,
            value: Box::new(Assign::try_from(rhs.as_ref())?),
        })
    }
}

/// A node coerced into an [`Assignable`], or [`LowerError::NotAssignable`].
impl<'a, 's> TryFrom<&'a Cst<'s>> for Assignable<'s> {
    type Error = LowerError;
    fn try_from(cst: &'a Cst<'s>) -> Result<Self, LowerError> {
        match &cst.kind {
            CstKind::Ident(s) => Ok(Assignable::Var(s)),
            CstKind::List(items) => Ok(Assignable::List(assignable_items(items)?)),
            CstKind::Call { callee, args } => Ok(Assignable::Call {
                name: ident_of(callee)?,
                args: Args::try_from(args.as_slice())?,
            }),
            CstKind::Binary {
                op: op @ (BinOp::Get | BinOp::Match),
                lhs,
                rhs,
            } => Ok(Assignable::Index {
                base: Box::new(Assignable::try_from(lhs.as_ref())?),
                op: get_op(*op),
                key: Primary::try_from(rhs.as_ref())?,
            }),
            CstKind::Unary {
                op: UnaryOp::Unpack,
                operand,
            } => Ok(Assignable::Rest(Box::new(Assignable::try_from(
                operand.as_ref(),
            )?))),
            other => Err(LowerError::NotAssignable(describe(other))),
        }
    }
}

/// Lower the elements of a `[...]` destructuring target to [`Assignable`]s,
/// dropping phantom `Empty` slots. A free helper, not a `TryFrom` impl: the
/// orphan rule forbids implementing the foreign `TryFrom` for `Vec`.
fn assignable_items<'s>(items: &[Cst<'s>]) -> Result<Vec<Assignable<'s>>, LowerError> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if matches!(item.kind, CstKind::Empty) {
            continue;
        }
        out.push(Assignable::try_from(item)?);
    }
    Ok(out)
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
            CstKind::Map(items) => Ok(Primary::Map(Args::try_from(items.as_slice())?)),
            CstKind::Paren(inner) => Ok(Primary::Paren(Args::try_from(inner.as_ref())?)),
            other => Err(LowerError::Unexpected(describe(other))),
        }
    }
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
        // params are general expressions; here two bare identifiers.
        let Args(codes) = params;
        assert_eq!(codes.len(), 2);
        assert_eq!(prim_of_expr(&codes[0].0[0]), &Primary::Ident("a"));
        assert_eq!(prim_of_expr(&codes[1].0[0]), &Primary::Ident("b"));
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
        assert_eq!(params.0.len(), 1);
    }

    #[test]
    fn rest_parameter_is_an_unpack_expression() {
        // `...rest` is a parameter expression, lowered as a unary unpack.
        let a = ast("f(a, ...rest) -> rest");
        let Expr::Def { params, .. } = only_expr(&a) else {
            panic!("expected a Def");
        };
        assert_eq!(params.0.len(), 2);
        assert!(matches!(
            as_unary(&params.0[1].0[0]),
            Unary::Unpack(inner) if matches!(inner.as_ref(), Unary::Get(Get::Primary(Primary::Ident("rest")))),
        ));
    }

    #[test]
    fn literal_pattern_parameter_lowers() {
        // Scarpet dispatches on literal parameters: `f('add', x) -> …`. The
        // string parameter is a plain expression, not an assignable.
        let a = ast("f('add', x) -> x");
        let Expr::Def { params, .. } = only_expr(&a) else {
            panic!("expected a Def");
        };
        assert_eq!(params.0.len(), 2);
        assert_eq!(prim_of_expr(&params.0[0].0[0]), &Primary::Str("'add'"));
    }

    #[test]
    fn outer_capture_parameter_lowers_to_a_call_expression() {
        let a = ast("_(outer(x)) -> x");
        let Expr::Def { params, .. } = only_expr(&a) else {
            panic!("expected a Def");
        };
        assert_eq!(params.0.len(), 1);
        let Primary::Call { name, args } = prim_of_expr(&params.0[0].0[0]) else {
            panic!("expected outer(...) call");
        };
        assert_eq!(*name, "outer");
        assert_eq!(args.0.len(), 1);
    }

    #[test]
    fn arrow_with_non_call_lhs_is_a_generic_arrow() {
        // A map entry `'k' -> v` keeps its arrow as `Expr::Arrow`.
        let a = ast("'k' -> v");
        let Expr::Arrow { lhs, body } = only_expr(&a) else {
            panic!("expected an Arrow");
        };
        assert!(matches!(lhs, Assign::Lor(_)));
        assert_eq!(prim_of_expr(body), &Primary::Ident("v"));
    }

    // --- assignments ---------------------------------------------------------

    #[test]
    fn assignment_is_right_associative() {
        // `a = b = 5` → Set(a, Set(b, 5))
        let a = ast("a = b = 5");
        let Expr::Assign(Assign::Set { target, op, value }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        assert_eq!(target, &Assignable::Var("a"));
        assert_eq!(*op, AssignOp::Assign);
        let Assign::Set {
            target: t2,
            op: op2,
            ..
        } = value.as_ref()
        else {
            panic!("expected a nested Set");
        };
        assert_eq!(t2, &Assignable::Var("b"));
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
        // `x:0 = 5` → the target is an indexed assignable.
        let a = ast("x:0 = 5");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        assert_eq!(
            target,
            &Assignable::Index {
                base: Box::new(Assignable::Var("x")),
                op: GetOp::Get,
                key: Primary::Number("0"),
            }
        );
    }

    #[test]
    fn destructuring_list_assignment_target() {
        let a = ast("[a, b] = t");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        assert_eq!(
            target,
            &Assignable::List(vec![Assignable::Var("a"), Assignable::Var("b")])
        );
    }

    #[test]
    fn destructuring_call_assignment_target() {
        // `l(x, y, z) = pos()` — the `l(...)` list-constructor destructure.
        // Call-target args are general expressions (here three identifiers).
        let a = ast("l(x, y, z) = p");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let Assignable::Call { name, args } = target else {
            panic!("expected a call target");
        };
        assert_eq!(*name, "l");
        assert_eq!(args.0.len(), 3);
        assert_eq!(prim_of_expr(&args.0[1].0[0]), &Primary::Ident("y"));
    }

    #[test]
    fn lvalue_returning_call_assignment_targets() {
        // `var(<expr>) = …` assigns to a dynamically-named variable; the
        // computed name is an arbitrary expression, not an assignable.
        let a = ast("var('global_' + s) = 1");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let Assignable::Call { name, args } = target else {
            panic!("expected a call target");
        };
        assert_eq!(*name, "var");
        assert_eq!(args.0.len(), 1);

        // `if(cond, a, b) = …` assigns through an l-value-selecting call.
        let a = ast("if(c, a, b) = 1");
        let Expr::Assign(Assign::Set { target, .. }) = only_expr(&a) else {
            panic!("expected a Set");
        };
        let Assignable::Call { name, args } = target else {
            panic!("expected a call target");
        };
        assert_eq!(*name, "if");
        assert_eq!(args.0.len(), 3);
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

        let a = ast("{'a' -> 1}");
        let Primary::Map(Args(codes)) = prim_of_expr(only_expr(&a)) else {
            panic!("expected a map");
        };
        assert_eq!(codes.len(), 1);
        // the single entry is an arrow
        assert!(matches!(codes[0].0[0], Expr::Arrow { .. }));
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

    // --- errors --------------------------------------------------------------

    #[test]
    fn assigning_to_a_literal_is_an_error() {
        assert_eq!(lower_err("1 = 2"), LowerError::NotAssignable("a number"));
    }

    #[test]
    fn assigning_to_an_operator_expression_is_an_error() {
        assert_eq!(
            lower_err("a + b = c"),
            LowerError::NotAssignable("an operator expression")
        );
    }

    #[test]
    fn assigning_to_a_paren_is_an_error() {
        // A parenthesized expression is not a valid top-level target shape.
        assert_eq!(
            lower_err("(a) = b"),
            LowerError::NotAssignable("a parenthesized expression")
        );
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

    /// Files whose Scarpet source doesn't parse (upstream typos), so there is no
    /// CST to lower. Mirrors the list in `scarpet-fmt`.
    const KNOWN_BAD: &[&str] = &[
        "gnembon/scarpet/programs/survival/portalorient.sc",
        "gnembon/scarpet/programs/survival/rifts/rifts.sc",
        "Ghoulboy78/Scarpet-edit/se.sc",
    ];

    fn corpus_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("example")
    }

    fn walk_sc(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk_sc(&p, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some("sc") {
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
        walk_sc(&root, &mut files);
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
