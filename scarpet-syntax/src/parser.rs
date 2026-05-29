use crate::lexer::{Token, TokenKind};
use chumsky::{extra, prelude::*, recursive::Recursive};
use logosky::{Lexed, utils::Span};

type TokenStream<'s> = logosky::TokenStream<'s, Token<'s>>;
type Extra<'s> = extra::Err<Rich<'s, Lexed<'s, Token<'s>>, Span>>;
type BoxedP<'s, O> = Boxed<'s, 's, TokenStream<'s>, O, Extra<'s>>;

// ====================================================================
// CST (concrete syntax tree) — preserves comments and breaks as
// `leading` trivia attached to each node.
// ====================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct Cst<'s> {
    pub leading: Vec<Trivia<'s>>,
    pub kind: CstKind<'s>,
}

impl<'s> Cst<'s> {
    pub fn bare(kind: CstKind<'s>) -> Self {
        Self {
            leading: Vec::new(),
            kind,
        }
    }

    pub fn with_leading(mut self, mut leading: Vec<Trivia<'s>>) -> Self {
        if leading.is_empty() {
            return self;
        }
        leading.extend(self.leading);
        self.leading = leading;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trivia<'s> {
    Comment(&'s str),
    Break,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CstKind<'s> {
    Number(&'s str),
    Str(&'s str),
    Ident(&'s str),
    Call {
        callee: Box<Cst<'s>>,
        args: Vec<Cst<'s>>,
    },
    List(Vec<Cst<'s>>),
    Map(Vec<Cst<'s>>),
    Paren(Box<Cst<'s>>),
    Empty,
    Binary {
        op: BinOp,
        lhs: Box<Cst<'s>>,
        rhs: Box<Cst<'s>>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Cst<'s>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    Match,
    Get,
    Assign,
    AddAssign,
    Swap,
    Arrow,
    Semi,
    Comma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
    Not,
    Unpack,
}

// ====================================================================
// Front-end
// ====================================================================

#[derive(Debug, Clone, Copy)]
pub struct Code<'s> {
    source: &'s str,
}

impl<'s> Code<'s> {
    pub fn from_source(src: &'s str) -> Result<Self, LexError> {
        Ok(Self { source: src })
    }

    pub fn source(&self) -> &'s str {
        self.source
    }

    pub fn parse(&self) -> Result<Cst<'s>, ParseError> {
        parse_source(self.source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    /// Byte range of the offending token. Empty (`len..len`) at end of input.
    pub span: std::ops::Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    UnexpectedToken,
    UnexpectedEof,
    Trailing,
}

impl ParseErrorKind {
    /// Human-readable description of the error kind, shared by the corpus
    /// reporter and the CLI's ariadne diagnostics.
    pub fn message(&self) -> &'static str {
        match self {
            ParseErrorKind::UnexpectedToken => "unexpected token",
            ParseErrorKind::UnexpectedEof => "unexpected end of input",
            ParseErrorKind::Trailing => "trailing input",
        }
    }
}

pub fn parse_source(src: &str) -> Result<Cst<'_>, ParseError> {
    let stream = TokenStream::new(src);
    let len = src.len();
    match program_parser().parse(stream).into_result() {
        Ok(cst) => Ok(cst),
        Err(errs) => {
            let err = errs.into_iter().next().unwrap();
            let span = err.span().start()..err.span().end();
            let kind = if span.start >= len {
                ParseErrorKind::UnexpectedEof
            } else {
                ParseErrorKind::UnexpectedToken
            };
            Err(ParseError { kind, span })
        }
    }
}

/// Return a clone of `cst` with all leading trivia removed, recursively.
///
/// Useful for comparing two trees for structural equality while ignoring
/// comments and breaks — e.g. to assert a formatter is non-destructive.
pub fn strip_trivia<'s>(cst: &Cst<'s>) -> Cst<'s> {
    let kind = match &cst.kind {
        CstKind::Call { callee, args } => CstKind::Call {
            callee: Box::new(strip_trivia(callee)),
            args: args.iter().map(strip_trivia).collect(),
        },
        CstKind::List(items) => CstKind::List(items.iter().map(strip_trivia).collect()),
        CstKind::Map(items) => CstKind::Map(items.iter().map(strip_trivia).collect()),
        CstKind::Paren(inner) => CstKind::Paren(Box::new(strip_trivia(inner))),
        CstKind::Binary { op, lhs, rhs } => CstKind::Binary {
            op: *op,
            lhs: Box::new(strip_trivia(lhs)),
            rhs: Box::new(strip_trivia(rhs)),
        },
        CstKind::Unary { op, operand } => CstKind::Unary {
            op: *op,
            operand: Box::new(strip_trivia(operand)),
        },
        CstKind::Number(s) => CstKind::Number(s),
        CstKind::Str(s) => CstKind::Str(s),
        CstKind::Ident(s) => CstKind::Ident(s),
        CstKind::Empty => CstKind::Empty,
    };
    Cst {
        leading: Vec::new(),
        kind,
    }
}

// ====================================================================
// chumsky-based parser
// ====================================================================
//
// Precedence ladder (low → high):
//
//   program     = top
//   top         = comma_chain
//   comma_chain = seq_chain   (`,` seq_chain)*
//   seq_chain   = arrow_chain (`;` arrow_chain)*
//   arrow_chain = assign (`->` arrow_chain)?
//   assign      = lor    (`=` | `+=` | `<>` assign)?
//   lor         = land   (`||` land)*
//   land        = equality (`&&` equality)*
//   equality    = compare ((`==` | `!=`) compare)*
//   compare     = additive ((`<` | `<=` | `>` | `>=`) additive)*
//   additive    = multiplicative ((`+` | `-`) multiplicative)*
//   multiplicative = power ((`*` | `/` | `%`) power)*
//   power       = unary (`^` power)?
//   unary       = (`+` | `-` | `!` | `...`)* get
//   get         = primary ((`~` | `:`) primary)*
//   primary     = atom | `(` top `)` | `[` arg_list `]` | `{` arg_list `}` | ident `(` arg_list `)`
//
// Trivia (Break / Comment) is collected at every token consumer via
// `leading_trivia()`. Leaf and compound nodes alike carry their leading
// trivia in `Cst::leading`. Trivia that sits immediately before an
// operator token is treated as belonging to the operator's RHS — it is
// prepended to the RHS node's leading.

fn program_parser<'s>() -> impl Parser<'s, TokenStream<'s>, Cst<'s>, Extra<'s>> {
    top_parser()
        .then(leading_trivia())
        .map(|(mut cst, trailing)| {
            // Anchor any pure-trailing trivia (e.g. a comment after the final
            // expression) onto the root so it isn't silently dropped.
            cst.leading.extend(trailing);
            cst
        })
        .then_ignore(end())
}

fn top_parser<'s>() -> BoxedP<'s, Cst<'s>> {
    let mut seq_chain =
        Recursive::<chumsky::recursive::Indirect<TokenStream<'s>, Cst<'s>, Extra<'s>>>::declare();
    let mut top =
        Recursive::<chumsky::recursive::Indirect<TokenStream<'s>, Cst<'s>, Extra<'s>>>::declare();

    let arg_list = arg_list_parser(seq_chain.clone().boxed()).boxed();

    let primary = {
        let number = tok_matching(|t| match t {
            Token::Number(s) => Some(CstKind::Number(s)),
            _ => None,
        })
        .map(|(leading, kind)| Cst { leading, kind });

        let string = tok_matching(|t| match t {
            Token::String(s) => Some(CstKind::Str(s)),
            _ => None,
        })
        .map(|(leading, kind)| Cst { leading, kind });

        let ident_only = tok_matching(|t| match t {
            Token::Ident(s) => Some(s),
            _ => None,
        });

        let ident_or_call = ident_only
            .then(
                arg_list
                    .clone()
                    .delimited_by(kind(TokenKind::OpenParen), kind(TokenKind::CloseParen))
                    .or_not(),
            )
            .map(|((leading, name), args)| match args {
                Some(args) => Cst {
                    leading,
                    kind: CstKind::Call {
                        callee: Box::new(Cst::bare(CstKind::Ident(name))),
                        args,
                    },
                },
                None => Cst {
                    leading,
                    kind: CstKind::Ident(name),
                },
            });

        let paren = kind(TokenKind::OpenParen)
            .then(top.clone())
            .then_ignore(kind(TokenKind::CloseParen))
            .map(|(leading, inner)| Cst {
                leading,
                kind: CstKind::Paren(Box::new(inner)),
            });

        let list = kind(TokenKind::OpenBrack)
            .then(arg_list.clone())
            .then_ignore(kind(TokenKind::CloseBrack))
            .map(|(leading, args)| Cst {
                leading,
                kind: CstKind::List(args),
            });

        let map = kind(TokenKind::OpenBrace)
            .then(arg_list.clone())
            .then_ignore(kind(TokenKind::CloseBrace))
            .map(|(leading, args)| Cst {
                leading,
                kind: CstKind::Map(args),
            });

        choice((number, string, ident_or_call, paren, list, map)).boxed()
    };

    let get = primary
        .clone()
        .foldl(
            choice((
                kind(TokenKind::Tilde).map(|l| (l, BinOp::Match)),
                kind(TokenKind::Colon).map(|l| (l, BinOp::Get)),
            ))
            .then(primary.clone())
            .repeated(),
            |lhs, ((op_leading, op), rhs)| bin(op, lhs, rhs.with_leading(op_leading)),
        )
        .boxed();

    let unary_prefix = choice((
        kind(TokenKind::Sub).map(|l| (l, UnaryOp::Neg)),
        kind(TokenKind::Add).map(|l| (l, UnaryOp::Pos)),
        kind(TokenKind::Bang).map(|l| (l, UnaryOp::Not)),
        kind(TokenKind::Ellipsis).map(|l| (l, UnaryOp::Unpack)),
    ));
    let unary = unary_prefix
        .repeated()
        .collect::<Vec<_>>()
        .then(get)
        .map(|(prefixes, operand)| {
            // Innermost prefix wraps the operand; each outer prefix wraps
            // the result. Each prefix takes its own leading trivia.
            prefixes
                .into_iter()
                .rev()
                .fold(operand, |acc, (l, op)| Cst {
                    leading: l,
                    kind: CstKind::Unary {
                        op,
                        operand: Box::new(acc),
                    },
                })
        })
        .boxed();

    // power is right-associative.
    let power = unary
        .clone()
        .then(
            kind(TokenKind::Pow)
                .then(unary)
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(first, rest)| {
            if rest.is_empty() {
                return first;
            }
            let mut operands = Vec::with_capacity(rest.len() + 1);
            let mut op_leadings = Vec::with_capacity(rest.len());
            operands.push(first);
            for (op_leading, rhs) in rest {
                op_leadings.push(op_leading);
                operands.push(rhs);
            }
            let mut acc = operands.pop().unwrap();
            while let Some(lhs) = operands.pop() {
                let op_leading = op_leadings.pop().unwrap();
                acc = bin(BinOp::Pow, lhs, acc.with_leading(op_leading));
            }
            acc
        })
        .boxed();

    let multiplicative = power
        .clone()
        .foldl(
            choice((
                kind(TokenKind::Mul).map(|l| (l, BinOp::Mul)),
                kind(TokenKind::Div).map(|l| (l, BinOp::Div)),
                kind(TokenKind::Rem).map(|l| (l, BinOp::Rem)),
            ))
            .then(power)
            .repeated(),
            |lhs, ((op_leading, op), rhs)| bin(op, lhs, rhs.with_leading(op_leading)),
        )
        .boxed();

    let additive = multiplicative
        .clone()
        .foldl(
            choice((
                kind(TokenKind::Add).map(|l| (l, BinOp::Add)),
                kind(TokenKind::Sub).map(|l| (l, BinOp::Sub)),
            ))
            .then(multiplicative)
            .repeated(),
            |lhs, ((op_leading, op), rhs)| bin(op, lhs, rhs.with_leading(op_leading)),
        )
        .boxed();

    let compare = additive
        .clone()
        .foldl(
            choice((
                kind(TokenKind::LtEq).map(|l| (l, BinOp::LtEq)),
                kind(TokenKind::GtEq).map(|l| (l, BinOp::GtEq)),
                kind(TokenKind::Lt).map(|l| (l, BinOp::Lt)),
                kind(TokenKind::Gt).map(|l| (l, BinOp::Gt)),
            ))
            .then(additive)
            .repeated(),
            |lhs, ((op_leading, op), rhs)| bin(op, lhs, rhs.with_leading(op_leading)),
        )
        .boxed();

    let equality = compare
        .clone()
        .foldl(
            choice((
                kind(TokenKind::EqEq).map(|l| (l, BinOp::Eq)),
                kind(TokenKind::BangEq).map(|l| (l, BinOp::NotEq)),
            ))
            .then(compare)
            .repeated(),
            |lhs, ((op_leading, op), rhs)| bin(op, lhs, rhs.with_leading(op_leading)),
        )
        .boxed();

    let land = equality
        .clone()
        .foldl(
            kind(TokenKind::And).then(equality).repeated(),
            |lhs, (op_leading, rhs)| bin(BinOp::And, lhs, rhs.with_leading(op_leading)),
        )
        .boxed();

    let lor = land
        .clone()
        .foldl(
            kind(TokenKind::Or).then(land).repeated(),
            |lhs, (op_leading, rhs)| bin(BinOp::Or, lhs, rhs.with_leading(op_leading)),
        )
        .boxed();

    // assign is right-associative; per-link op may differ.
    let assign_op = choice((
        kind(TokenKind::Assign).map(|l| (l, BinOp::Assign)),
        kind(TokenKind::AddAssign).map(|l| (l, BinOp::AddAssign)),
        kind(TokenKind::Swap).map(|l| (l, BinOp::Swap)),
    ));
    let assign = lor
        .clone()
        .then(assign_op.then(lor).repeated().collect::<Vec<_>>())
        .map(|(first, rest)| {
            if rest.is_empty() {
                return first;
            }
            let mut operands: Vec<Cst<'s>> = Vec::with_capacity(rest.len() + 1);
            let mut links: Vec<(Vec<Trivia<'s>>, BinOp)> = Vec::with_capacity(rest.len());
            operands.push(first);
            for ((op_leading, op), rhs) in rest {
                links.push((op_leading, op));
                operands.push(rhs);
            }
            let mut acc = operands.pop().unwrap();
            while let Some(lhs) = operands.pop() {
                let (op_leading, op) = links.pop().unwrap();
                acc = bin(op, lhs, acc.with_leading(op_leading));
            }
            acc
        })
        .boxed();

    let arrow_chain = assign
        .clone()
        .then(
            kind(TokenKind::Arrow)
                .then(assign)
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(first, rest)| {
            if rest.is_empty() {
                return first;
            }
            let mut operands = Vec::with_capacity(rest.len() + 1);
            let mut op_leadings = Vec::with_capacity(rest.len());
            operands.push(first);
            for (op_leading, rhs) in rest {
                op_leadings.push(op_leading);
                operands.push(rhs);
            }
            let mut acc = operands.pop().unwrap();
            while let Some(lhs) = operands.pop() {
                let op_leading = op_leadings.pop().unwrap();
                acc = bin(BinOp::Arrow, lhs, acc.with_leading(op_leading));
            }
            acc
        })
        .boxed();

    seq_chain.define(seq_chain_inner(arrow_chain));
    top.define(comma_chain_inner(seq_chain.clone().boxed()));

    top.boxed()
}

fn seq_chain_inner<'s>(
    arrow_chain: BoxedP<'s, Cst<'s>>,
) -> impl Parser<'s, TokenStream<'s>, Cst<'s>, Extra<'s>> + Clone {
    let leading = leading_trivia();
    custom(move |inp| {
        let mut acc = inp.parse(arrow_chain.clone())?;
        loop {
            // The leading trivia for the next operator. If we end up at a
            // closer/EOF/comma, this trivia belongs to no specific child —
            // it gets handed back to the enclosing parser as the next
            // peeked-token's leading. Because chumsky doesn't let us
            // "un-collect" trivia, we have to be careful to NOT consume
            // it unless we know we're about to commit. Strategy: save a
            // checkpoint, collect trivia, peek; if the next token is `;`
            // commit, otherwise rewind and stop.
            let saved = inp.save();
            let trivia = inp.parse(leading.clone())?;
            if !peek_is(inp, TokenKind::SemiColon) {
                inp.rewind(saved);
                break;
            }
            // Eat one `;`, then any additional `;`s (Scarpet's preprocessor
            // strips runs of them). Keep the trivia gathered after the
            // FINAL `;` so it can attach to the next statement.
            let _ = inp.next();
            let mut post_semi_trivia = inp.parse(leading.clone())?;
            while peek_is(inp, TokenKind::SemiColon) {
                let _ = inp.next();
                post_semi_trivia = inp.parse(leading.clone())?;
            }
            if peek_is_closer_or_eof(inp) || peek_is(inp, TokenKind::Comma) {
                // Trailing `;`. Both the trivia before the first `;` and
                // any trivia after the final `;` would otherwise be lost —
                // anchor them onto the accumulator's leading.
                acc.leading.extend(trivia);
                acc.leading.extend(post_semi_trivia);
                break;
            }
            let rhs = inp.parse(arrow_chain.clone())?;
            // Trivia before the `;` (visually ends the LHS statement) and
            // trivia after the final `;` both flow into the next stmt's
            // leading, in source order.
            let mut combined = trivia;
            combined.extend(post_semi_trivia);
            acc = bin(BinOp::Semi, acc, rhs.with_leading(combined));
        }
        Ok(acc)
    })
}

fn comma_chain_inner<'s>(
    seq_chain: BoxedP<'s, Cst<'s>>,
) -> impl Parser<'s, TokenStream<'s>, Cst<'s>, Extra<'s>> + Clone {
    let leading = leading_trivia();
    custom(move |inp| {
        let mut acc = inp.parse(seq_chain.clone())?;
        loop {
            let saved = inp.save();
            let trivia = inp.parse(leading.clone())?;
            if !peek_is(inp, TokenKind::Comma) {
                inp.rewind(saved);
                break;
            }
            let _ = inp.next();
            let trivia2 = inp.parse(leading.clone())?;
            if peek_is_closer_or_eof(inp) {
                // Trailing `,`. Trivia between the previous expr and `,` is
                // attached to acc; trivia after `,` goes to acc too.
                let mut combined = trivia;
                combined.extend(trivia2);
                acc.leading.extend(combined);
                break;
            }
            let rhs = inp.parse(seq_chain.clone())?;
            // trivia (before `,`) attaches to rhs; trivia2 (after `,`) also
            // attaches to rhs's leading (already does via the seq parse).
            // Order: trivia, then trivia2, then rhs's own leading.
            let mut combined = trivia;
            combined.extend(trivia2);
            acc = bin(BinOp::Comma, acc, rhs.with_leading(combined));
        }
        Ok(acc)
    })
}

// arg_list (between `(`, `[`, `{`) — comma-separated `seq_chain`s, tolerating:
//   - empty list right before closer
//   - omitted entries: `f(a, , b)` → second arg is `CstKind::Empty`
//   - trailing comma: `(a, b,)` does NOT insert a phantom trailing Empty
//
// Trivia is preserved by attaching to each item's leading. Trivia before a
// phantom Empty is recorded on the Empty node.
fn arg_list_parser<'s>(
    seq: BoxedP<'s, Cst<'s>>,
) -> impl Parser<'s, TokenStream<'s>, Vec<Cst<'s>>, Extra<'s>> + Clone {
    let leading = leading_trivia();
    custom(move |inp| {
        let initial = inp.parse(leading.clone())?;
        let mut items: Vec<Cst<'s>> = Vec::new();
        if peek_is_closer_or_eof(inp) {
            // Trivia inside an empty `(... )`. Promote it onto a phantom-less
            // tail; we have nowhere natural to attach it, so re-attach to
            // the caller via the input rewind isn't possible. Stash on a
            // hidden Empty node so it's not lost.
            if !initial.is_empty() {
                items.push(Cst {
                    leading: initial,
                    kind: CstKind::Empty,
                });
            }
            return Ok(items);
        }
        let mut pending: Vec<Trivia<'s>> = initial;
        loop {
            if peek_is(inp, TokenKind::Comma) {
                // Omitted entry: synthesise an Empty carrying the pending
                // trivia (which would otherwise be lost).
                items.push(Cst {
                    leading: std::mem::take(&mut pending),
                    kind: CstKind::Empty,
                });
            } else if peek_is_closer_or_eof(inp) {
                if !pending.is_empty() {
                    items.push(Cst {
                        leading: std::mem::take(&mut pending),
                        kind: CstKind::Empty,
                    });
                }
                break;
            } else {
                let v = inp.parse(seq.clone())?;
                let leading_before = std::mem::take(&mut pending);
                items.push(v.with_leading(leading_before));
            }
            let trivia_after = inp.parse(leading.clone())?;
            if peek_is(inp, TokenKind::Comma) {
                let _ = inp.next();
                let trivia_post_comma = inp.parse(leading.clone())?;
                // trivia_after sits between the previous item and `,`;
                // trivia_post_comma sits between `,` and the next item.
                // Both flow into the next item's leading.
                pending = trivia_after;
                pending.extend(trivia_post_comma);
                if peek_is_closer_or_eof(inp) {
                    // Trailing comma — flush pending onto last item.
                    if !pending.is_empty() {
                        items.last_mut().unwrap().leading.extend(pending);
                    }
                    break;
                }
            } else {
                // No comma, so we're done. Trivia between last item and
                // closer attaches back onto the last item.
                if !trivia_after.is_empty() {
                    items.last_mut().unwrap().leading.extend(trivia_after);
                }
                break;
            }
        }
        Ok(items)
    })
}

fn peek_is<'s>(
    inp: &mut chumsky::input::InputRef<'s, '_, TokenStream<'s>, Extra<'s>>,
    k: TokenKind,
) -> bool {
    match inp.peek() {
        Some(Lexed::Token(s)) => logosky::Token::kind(&s.data) == k,
        _ => false,
    }
}

fn peek_is_closer_or_eof<'s>(
    inp: &mut chumsky::input::InputRef<'s, '_, TokenStream<'s>, Extra<'s>>,
) -> bool {
    match inp.peek() {
        None => true,
        Some(Lexed::Token(s)) => matches!(
            logosky::Token::kind(&s.data),
            TokenKind::CloseParen | TokenKind::CloseBrack | TokenKind::CloseBrace
        ),
        _ => false,
    }
}

// --- token matching helpers ----------------------------------------

/// A parser that collects consecutive trivia tokens (Break / Comment)
/// from the head of the input without consuming any semantic token.
fn leading_trivia<'s>() -> BoxedP<'s, Vec<Trivia<'s>>> {
    custom(|inp| {
        let mut v = Vec::new();
        loop {
            let saved = inp.save();
            match inp.next() {
                Some(Lexed::Token(s)) => match s.data {
                    Token::Comment(c) => v.push(Trivia::Comment(c)),
                    Token::Break => v.push(Trivia::Break),
                    _ => {
                        inp.rewind(saved);
                        break;
                    }
                },
                _ => {
                    inp.rewind(saved);
                    break;
                }
            }
        }
        Ok(v)
    })
    .boxed()
}

/// Match a token kind, returning its leading trivia.
fn kind<'s>(k: TokenKind) -> BoxedP<'s, Vec<Trivia<'s>>> {
    leading_trivia()
        .then(
            any().try_map(move |tok: Lexed<'s, Token<'s>>, span: Span| match tok {
                Lexed::Token(t) if logosky::Token::kind(&t.data) == k => Ok(()),
                _ => Err(Rich::custom(span, "unexpected token")),
            }),
        )
        .map(|(leading, _)| leading)
        .boxed()
}

/// Match a token and project a value from it, returning the leading trivia
/// alongside.
fn tok_matching<'s, T: 's, F>(f: F) -> BoxedP<'s, (Vec<Trivia<'s>>, T)>
where
    F: Fn(Token<'s>) -> Option<T> + Clone + 's,
{
    leading_trivia()
        .then(
            any().try_map(move |tok: Lexed<'s, Token<'s>>, span: Span| match tok {
                Lexed::Token(t) => f(t.data).ok_or_else(|| Rich::custom(span, "unexpected token")),
                _ => Err(Rich::custom(span, "unexpected token")),
            }),
        )
        .boxed()
}

// --- helpers --------------------------------------------------------

fn bin<'s>(op: BinOp, lhs: Cst<'s>, rhs: Cst<'s>) -> Cst<'s> {
    Cst::bare(CstKind::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    })
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Cst<'_> {
        parse_source(src).expect("parse error")
    }

    // Constructors with empty leading trivia, for terser assertions.
    fn num(s: &str) -> Cst<'_> {
        Cst::bare(CstKind::Number(s))
    }
    fn str_(s: &str) -> Cst<'_> {
        Cst::bare(CstKind::Str(s))
    }
    fn id(s: &str) -> Cst<'_> {
        Cst::bare(CstKind::Ident(s))
    }
    fn call<'s>(name: &'s str, args: Vec<Cst<'s>>) -> Cst<'s> {
        Cst::bare(CstKind::Call {
            callee: Box::new(id(name)),
            args,
        })
    }
    fn list(args: Vec<Cst<'_>>) -> Cst<'_> {
        Cst::bare(CstKind::List(args))
    }
    fn map(args: Vec<Cst<'_>>) -> Cst<'_> {
        Cst::bare(CstKind::Map(args))
    }
    fn paren(inner: Cst<'_>) -> Cst<'_> {
        Cst::bare(CstKind::Paren(Box::new(inner)))
    }
    fn un(op: UnaryOp, operand: Cst<'_>) -> Cst<'_> {
        Cst::bare(CstKind::Unary {
            op,
            operand: Box::new(operand),
        })
    }

    #[test]
    fn hello_world() {
        assert_eq!(
            parse("print('Hello World!')"),
            call("print", vec![str_("'Hello World!'")])
        );
    }

    #[test]
    fn arithmetic_precedence() {
        assert_eq!(
            parse("2 + 3 * 4"),
            bin(BinOp::Add, num("2"), bin(BinOp::Mul, num("3"), num("4")))
        );
    }

    #[test]
    fn additive_left_assoc() {
        assert_eq!(
            parse("2 + 3 - 1"),
            bin(BinOp::Sub, bin(BinOp::Add, num("2"), num("3")), num("1"))
        );
    }

    #[test]
    fn power_right_assoc() {
        assert_eq!(
            parse("2 ^ 3 ^ 2"),
            bin(BinOp::Pow, num("2"), bin(BinOp::Pow, num("3"), num("2")))
        );
    }

    #[test]
    fn unary_minus_then_get() {
        assert_eq!(
            parse("-foo:0"),
            un(UnaryOp::Neg, bin(BinOp::Get, id("foo"), num("0")))
        );
    }

    #[test]
    fn match_and_get_chain() {
        assert_eq!(
            parse("a:b:c"),
            bin(BinOp::Get, bin(BinOp::Get, id("a"), id("b")), id("c"))
        );
        assert_eq!(parse("a~b"), bin(BinOp::Match, id("a"), id("b")));
    }

    #[test]
    fn function_definition() {
        assert_eq!(
            parse("foo(a, b) -> a + b"),
            bin(
                BinOp::Arrow,
                call("foo", vec![id("a"), id("b")]),
                bin(BinOp::Add, id("a"), id("b")),
            )
        );
    }

    #[test]
    fn list_and_map_literals() {
        assert_eq!(parse("[1, 2, 3]"), list(vec![num("1"), num("2"), num("3")]));
        assert_eq!(
            parse("{'a' -> 1, 'b' -> 2}"),
            map(vec![
                bin(BinOp::Arrow, str_("'a'"), num("1")),
                bin(BinOp::Arrow, str_("'b'"), num("2")),
            ])
        );
    }

    #[test]
    fn semi_binds_looser_than_arrow_in_map() {
        // `;` (seq_chain) sits outside `->` (arrow_chain), so a map entry
        // `{1+2 ; 'a'->3*4}` groups as `{(1+2) ; ('a'->(3*4))}`. This mirrors
        // Scarpet, where `->` (precedence 2) binds tighter than `;` (1).
        assert_eq!(
            parse("{1+2;'a'->3*4}"),
            map(vec![bin(
                BinOp::Semi,
                bin(BinOp::Add, num("1"), num("2")),
                bin(
                    BinOp::Arrow,
                    str_("'a'"),
                    bin(BinOp::Mul, num("3"), num("4"))
                ),
            )])
        );
    }

    #[test]
    fn arrow_right_assoc() {
        // `->` is right-associative, so `{f()->g()->h()}` groups as
        // `{f() -> (g() -> h())}`.
        assert_eq!(
            parse("{f()->g()->h()}"),
            map(vec![bin(
                BinOp::Arrow,
                call("f", vec![]),
                bin(BinOp::Arrow, call("g", vec![]), call("h", vec![])),
            )])
        );
    }

    #[test]
    fn assignment_right_assoc() {
        assert_eq!(
            parse("a = b = 5"),
            bin(
                BinOp::Assign,
                id("a"),
                bin(BinOp::Assign, id("b"), num("5"))
            )
        );
    }

    #[test]
    fn semi_and_comma_sequence() {
        assert_eq!(
            parse("a; b; c"),
            bin(BinOp::Semi, bin(BinOp::Semi, id("a"), id("b")), id("c"))
        );
    }

    #[test]
    fn unpacking_in_call() {
        assert_eq!(
            parse("f(...xs)"),
            call("f", vec![un(UnaryOp::Unpack, id("xs"))])
        );
    }

    #[test]
    fn nested_function_call() {
        assert_eq!(
            parse("print(format('f » ', 'g hi'))"),
            call(
                "print",
                vec![call("format", vec![str_("'f » '"), str_("'g hi'")])]
            )
        );
    }

    #[test]
    fn lenient_trailing_semicolon() {
        assert_eq!(parse("a;"), id("a"));
    }

    #[test]
    fn anonymous_function_in_call() {
        assert_eq!(
            parse("map([1,2,3], _(x) -> x * x)"),
            call(
                "map",
                vec![
                    list(vec![num("1"), num("2"), num("3")]),
                    bin(
                        BinOp::Arrow,
                        call("_", vec![id("x")]),
                        bin(BinOp::Mul, id("x"), id("x")),
                    ),
                ],
            )
        );
    }

    #[test]
    fn full_source_from_compdisplay() {
        let src = "toggle() -> (\n    print(player(), 'hi');\n);";
        let expected = bin(
            BinOp::Arrow,
            call("toggle", vec![]),
            paren(call("print", vec![call("player", vec![]), str_("'hi'")])),
        );
        // Trivia is preserved, so equality after stripping leading lets us
        // verify the structural shape without enumerating every break.
        assert_eq!(strip_trivia(&parse(src)), expected);
    }

    // ----- trivia-preservation tests ------------------------------------

    #[test]
    fn comments_attach_as_leading_trivia() {
        let cst = parse("// hello\n  a + b\n");
        // Top is Binary(Add, lhs=a, rhs=b). The leading trivia from the
        // comment and the newline lives on lhs (the first token).
        match &cst.kind {
            CstKind::Binary {
                op: BinOp::Add,
                lhs,
                rhs,
            } => {
                assert_eq!(
                    lhs.leading,
                    vec![Trivia::Comment("// hello"), Trivia::Break]
                );
                assert_eq!(lhs.kind, CstKind::Ident("a"));
                // The trailing `\n` after `b` is anchored on the root or
                // on `b`'s leading via the comma/semi paths; here there's
                // no operator after `b`, so the trailing Break flows up
                // to the root's leading.
                assert!(rhs.leading.is_empty());
                assert_eq!(rhs.kind, CstKind::Ident("b"));
            }
            other => panic!("expected Add(a, b), got {other:?}"),
        }
        // Trailing newline anchored at the root.
        assert!(cst.leading.contains(&Trivia::Break));
    }

    #[test]
    fn break_inside_call_args_attaches_to_next_arg() {
        let cst = parse("f(a,\n b)");
        match &cst.kind {
            CstKind::Call { args, .. } => {
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].kind, CstKind::Ident("a"));
                assert!(args[0].leading.is_empty());
                assert_eq!(args[1].kind, CstKind::Ident("b"));
                assert_eq!(args[1].leading, vec![Trivia::Break]);
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn comment_between_operator_and_rhs_attaches_to_rhs() {
        let cst = parse("a + // mid\n b");
        match &cst.kind {
            CstKind::Binary {
                op: BinOp::Add,
                lhs,
                rhs,
            } => {
                assert!(lhs.leading.is_empty());
                assert_eq!(lhs.kind, CstKind::Ident("a"));
                assert_eq!(rhs.leading, vec![Trivia::Comment("// mid"), Trivia::Break]);
                assert_eq!(rhs.kind, CstKind::Ident("b"));
            }
            other => panic!("expected Add(a, b), got {other:?}"),
        }
    }

    #[test]
    fn semicolon_trivia_flows_to_next_statement() {
        let cst = parse("a;\n// note\n b");
        // (a ; b) where b carries the inter-statement Break + comment.
        match &cst.kind {
            CstKind::Binary {
                op: BinOp::Semi,
                lhs,
                rhs,
            } => {
                assert_eq!(lhs.kind, CstKind::Ident("a"));
                assert_eq!(rhs.kind, CstKind::Ident("b"));
                assert!(rhs.leading.contains(&Trivia::Comment("// note")));
                // At least one Break between `;` and the comment.
                assert!(rhs.leading.contains(&Trivia::Break));
            }
            other => panic!("expected Semi(a, b), got {other:?}"),
        }
    }

    #[test]
    fn trailing_comment_anchored_on_root() {
        // No operator/comma follows `a`, so trivia after the final token
        // would otherwise be dropped — `program_parser` anchors it on the
        // root node's leading instead.
        let cst = parse("a\n// trailing");
        assert_eq!(cst.kind, CstKind::Ident("a"));
        assert_eq!(
            cst.leading,
            vec![Trivia::Break, Trivia::Comment("// trailing")]
        );
    }

    #[test]
    fn comment_inside_empty_parens_becomes_phantom_empty() {
        // The arg list is otherwise empty, but the comment needs an anchor.
        // `arg_list_parser` synthesises a single `Empty` to hold the trivia.
        let cst = parse("f(// note\n)");
        match &cst.kind {
            CstKind::Call { args, .. } => {
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].kind, CstKind::Empty);
                assert_eq!(
                    args[0].leading,
                    vec![Trivia::Comment("// note"), Trivia::Break]
                );
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn comment_attaches_to_omitted_empty_arg() {
        // `f(a, , b)` already synthesises an Empty between the commas; a
        // comment that sits where that Empty would be needs to ride on it.
        let cst = parse("f(a, // gap\n , b)");
        match &cst.kind {
            CstKind::Call { args, .. } => {
                assert_eq!(args.len(), 3);
                assert_eq!(args[0].kind, CstKind::Ident("a"));
                assert!(args[0].leading.is_empty());
                assert_eq!(args[1].kind, CstKind::Empty);
                assert_eq!(
                    args[1].leading,
                    vec![Trivia::Comment("// gap"), Trivia::Break]
                );
                assert_eq!(args[2].kind, CstKind::Ident("b"));
                assert!(args[2].leading.is_empty());
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn comment_after_last_arg_in_call_attaches_back() {
        // No comma follows `a`; the trailing trivia would otherwise be
        // stranded between the last item and `)`. It re-attaches onto `a`.
        let cst = parse("f(a // tail\n)");
        match &cst.kind {
            CstKind::Call { args, .. } => {
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].kind, CstKind::Ident("a"));
                assert_eq!(
                    args[0].leading,
                    vec![Trivia::Comment("// tail"), Trivia::Break]
                );
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn comment_around_trailing_comma_in_list_flushes_onto_last() {
        // The trailing-comma branch must flush both trivia bands (pre- and
        // post-comma) onto the last item rather than dropping them.
        let cst = parse("[1, // tail\n]");
        match &cst.kind {
            CstKind::List(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].kind, CstKind::Number("1"));
                assert_eq!(
                    items[0].leading,
                    vec![Trivia::Comment("// tail"), Trivia::Break]
                );
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn comment_inside_paren_attaches_to_inner_first_token() {
        // `paren` delegates the body to `top`, so trivia immediately after
        // `(` becomes the leading of the first inner atom.
        let cst = parse("(// note\n a + b)");
        match &cst.kind {
            CstKind::Paren(inner) => {
                assert!(cst.leading.is_empty());
                match &inner.kind {
                    CstKind::Binary {
                        op: BinOp::Add,
                        lhs,
                        rhs,
                    } => {
                        assert_eq!(lhs.leading, vec![Trivia::Comment("// note"), Trivia::Break]);
                        assert_eq!(lhs.kind, CstKind::Ident("a"));
                        assert!(rhs.leading.is_empty());
                        assert_eq!(rhs.kind, CstKind::Ident("b"));
                    }
                    other => panic!("expected Add(a, b), got {other:?}"),
                }
            }
            other => panic!("expected Paren, got {other:?}"),
        }
    }

    #[test]
    fn comment_between_unary_prefix_and_operand() {
        // The unary prefix consumes its own leading (here empty); trivia
        // between the prefix and its operand must ride on the operand, not
        // on the Unary node.
        let cst = parse("! // note\n x");
        match &cst.kind {
            CstKind::Unary {
                op: UnaryOp::Not,
                operand,
            } => {
                assert!(cst.leading.is_empty());
                assert_eq!(
                    operand.leading,
                    vec![Trivia::Comment("// note"), Trivia::Break]
                );
                assert_eq!(operand.kind, CstKind::Ident("x"));
            }
            other => panic!("expected Unary(Not, _), got {other:?}"),
        }
    }
}
