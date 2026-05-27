use crate::lexer::{Token, TokenKind};
use chumsky::{extra, prelude::*, recursive::Recursive};
use logosky::{Lexed, Tokenizer, utils::Span};

type TokenStream<'s> = logosky::TokenStream<'s, Token<'s>>;
type Extra<'s> = extra::Err<Rich<'s, Lexed<'s, Token<'s>>, Span>>;
type BoxedP<'s, O> = Boxed<'s, 's, TokenStream<'s>, O, Extra<'s>>;

// ====================================================================
// CST (concrete syntax tree, AST-shaped) definition
// ====================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Cst<'s> {
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
    pub at: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    UnexpectedToken,
    UnexpectedEof,
    Trailing,
}

pub fn parse_source(src: &str) -> Result<Cst<'_>, ParseError> {
    let stream = TokenStream::new(src);
    let len = src.len();
    match program_parser().parse(stream).into_result() {
        Ok(cst) => Ok(cst),
        Err(errs) => {
            let err = errs.into_iter().next().unwrap();
            let at = err.span().start();
            let kind = if at >= len {
                ParseErrorKind::UnexpectedEof
            } else {
                ParseErrorKind::UnexpectedToken
            };
            Err(ParseError { kind, at })
        }
    }
}

// ====================================================================
// chumsky-based parser
// ====================================================================
//
// Precedence ladder (low → high), straight from docs/scarpet/language/Operators.md:
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
// Trivia (Break / Comment) is skipped between every token via the
// `kind` / `tok_matching` helpers, which prefix-trim with logosky's
// `skip_trivias()`.

fn program_parser<'s>() -> impl Parser<'s, TokenStream<'s>, Cst<'s>, Extra<'s>> {
    let trim = TokenStream::skip_trivias::<Extra<'s>>();
    top_parser().then_ignore(trim).then_ignore(end())
}

fn top_parser<'s>() -> BoxedP<'s, Cst<'s>> {
    let mut seq_chain =
        Recursive::<chumsky::recursive::Indirect<TokenStream<'s>, Cst<'s>, Extra<'s>>>::declare();
    let mut top =
        Recursive::<chumsky::recursive::Indirect<TokenStream<'s>, Cst<'s>, Extra<'s>>>::declare();

    let arg_list = arg_list_parser(seq_chain.clone().boxed()).boxed();

    let primary = {
        let number = tok_matching(|t| {
            if let Token::Number(s) = t {
                Some(Cst::Number(s))
            } else {
                None
            }
        });
        let string = tok_matching(|t| {
            if let Token::String(s) = t {
                Some(Cst::Str(s))
            } else {
                None
            }
        });
        let ident_only = tok_matching(|t| {
            if let Token::Ident(s) = t {
                Some(s)
            } else {
                None
            }
        });

        let ident_or_call = ident_only
            .then(
                arg_list
                    .clone()
                    .delimited_by(kind(TokenKind::OpenParen), kind(TokenKind::CloseParen))
                    .or_not(),
            )
            .map(|(name, args)| match args {
                Some(args) => Cst::Call {
                    callee: Box::new(Cst::Ident(name)),
                    args,
                },
                None => Cst::Ident(name),
            });

        let paren = top
            .clone()
            .delimited_by(kind(TokenKind::OpenParen), kind(TokenKind::CloseParen))
            .map(|inner| Cst::Paren(Box::new(inner)));

        let list = arg_list
            .clone()
            .delimited_by(kind(TokenKind::OpenBrack), kind(TokenKind::CloseBrack))
            .map(Cst::List);

        let map = arg_list
            .clone()
            .delimited_by(kind(TokenKind::OpenBrace), kind(TokenKind::CloseBrace))
            .map(Cst::Map);

        choice((number, string, ident_or_call, paren, list, map)).boxed()
    };

    let get = primary
        .clone()
        .foldl(
            choice((
                kind(TokenKind::Tilde).to(BinOp::Match),
                kind(TokenKind::Colon).to(BinOp::Get),
            ))
            .then(primary.clone())
            .repeated(),
            |lhs, (op, rhs)| bin(op, lhs, rhs),
        )
        .boxed();

    let unary_prefix = choice((
        kind(TokenKind::Sub).to(UnaryOp::Neg),
        kind(TokenKind::Add).to(UnaryOp::Pos),
        kind(TokenKind::Bang).to(UnaryOp::Not),
        kind(TokenKind::Ellipsis).to(UnaryOp::Unpack),
    ));
    let unary = unary_prefix
        .repeated()
        .collect::<Vec<_>>()
        .then(get)
        .map(|(prefixes, operand)| {
            prefixes
                .into_iter()
                .rev()
                .fold(operand, |acc, op| un(op, acc))
        })
        .boxed();

    // power is right-associative: collect `unary` separated by `^`, fold right.
    let power = unary
        .clone()
        .then(
            kind(TokenKind::Pow)
                .ignore_then(unary)
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(first, rest)| {
            let mut all = vec![first];
            all.extend(rest);
            all.into_iter()
                .rev()
                .reduce(|rhs, lhs| bin(BinOp::Pow, lhs, rhs))
                .unwrap()
        })
        .boxed();

    let multiplicative = power
        .clone()
        .foldl(
            choice((
                kind(TokenKind::Mul).to(BinOp::Mul),
                kind(TokenKind::Div).to(BinOp::Div),
                kind(TokenKind::Rem).to(BinOp::Rem),
            ))
            .then(power)
            .repeated(),
            |lhs, (op, rhs)| bin(op, lhs, rhs),
        )
        .boxed();

    let additive = multiplicative
        .clone()
        .foldl(
            choice((
                kind(TokenKind::Add).to(BinOp::Add),
                kind(TokenKind::Sub).to(BinOp::Sub),
            ))
            .then(multiplicative)
            .repeated(),
            |lhs, (op, rhs)| bin(op, lhs, rhs),
        )
        .boxed();

    let compare = additive
        .clone()
        .foldl(
            choice((
                kind(TokenKind::LtEq).to(BinOp::LtEq),
                kind(TokenKind::GtEq).to(BinOp::GtEq),
                kind(TokenKind::Lt).to(BinOp::Lt),
                kind(TokenKind::Gt).to(BinOp::Gt),
            ))
            .then(additive)
            .repeated(),
            |lhs, (op, rhs)| bin(op, lhs, rhs),
        )
        .boxed();

    let equality = compare
        .clone()
        .foldl(
            choice((
                kind(TokenKind::EqEq).to(BinOp::Eq),
                kind(TokenKind::BangEq).to(BinOp::NotEq),
            ))
            .then(compare)
            .repeated(),
            |lhs, (op, rhs)| bin(op, lhs, rhs),
        )
        .boxed();

    let land = equality
        .clone()
        .foldl(
            kind(TokenKind::And).ignore_then(equality).repeated(),
            |lhs, rhs| bin(BinOp::And, lhs, rhs),
        )
        .boxed();

    let lor = land
        .clone()
        .foldl(
            kind(TokenKind::Or).ignore_then(land).repeated(),
            |lhs, rhs| bin(BinOp::Or, lhs, rhs),
        )
        .boxed();

    // assign is right-associative: collect `lor` separated by an assign-op,
    // then fold right preserving the chosen op at each link.
    let assign_op = choice((
        kind(TokenKind::Assign).to(BinOp::Assign),
        kind(TokenKind::AddAssign).to(BinOp::AddAssign),
        kind(TokenKind::Swap).to(BinOp::Swap),
    ));
    let assign = lor
        .clone()
        .then(assign_op.then(lor).repeated().collect::<Vec<_>>())
        .map(|(first, rest)| {
            if rest.is_empty() {
                return first;
            }
            // rest: [(op_0, e_1), (op_1, e_2), ...]. Right-assoc means
            // e_0 `op_0` (e_1 `op_1` (... `op_{n-1}` e_n)).
            let mut operands: Vec<Cst<'s>> = Vec::with_capacity(rest.len() + 1);
            let mut ops: Vec<BinOp> = Vec::with_capacity(rest.len());
            operands.push(first);
            for (op, rhs) in rest {
                ops.push(op);
                operands.push(rhs);
            }
            let mut acc = operands.pop().unwrap();
            while let Some(lhs) = operands.pop() {
                let op = ops.pop().unwrap();
                acc = bin(op, lhs, acc);
            }
            acc
        })
        .boxed();

    // arrow_chain is right-associative; same trick.
    let arrow_chain = assign
        .clone()
        .then(
            kind(TokenKind::Arrow)
                .ignore_then(assign)
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(first, rest)| {
            let mut all = vec![first];
            all.extend(rest);
            all.into_iter()
                .rev()
                .reduce(|rhs, lhs| bin(BinOp::Arrow, lhs, rhs))
                .unwrap()
        })
        .boxed();

    seq_chain.define(seq_chain_inner(arrow_chain));
    top.define(comma_chain_inner(seq_chain.clone().boxed()));

    top.boxed()
}

fn seq_chain_inner<'s>(
    arrow_chain: BoxedP<'s, Cst<'s>>,
) -> impl Parser<'s, TokenStream<'s>, Cst<'s>, Extra<'s>> + Clone {
    // Mirrors the nom version: parse one arrow_chain, then while we see `;`
    // consume any run of them. If we're then at EOF / closer / `,`, the `;`
    // was a trailing separator and we stop. Otherwise parse another
    // arrow_chain and continue.
    let trim = TokenStream::skip_trivias::<Extra<'s>>();
    custom(move |inp| {
        let mut acc = inp.parse(arrow_chain.clone())?;
        loop {
            inp.parse(trim.clone())?;
            if !peek_is(inp, TokenKind::SemiColon) {
                break;
            }
            // Consume one or more `;`s.
            while peek_is(inp, TokenKind::SemiColon) {
                let _ = inp.next();
                inp.parse(trim.clone())?;
            }
            // Trailing `;` before EOF / closer / `,` is fine — stop.
            if peek_is_closer_or_eof(inp) || peek_is(inp, TokenKind::Comma) {
                break;
            }
            let rhs = inp.parse(arrow_chain.clone())?;
            acc = bin(BinOp::Semi, acc, rhs);
        }
        Ok(acc)
    })
}

fn comma_chain_inner<'s>(
    seq_chain: BoxedP<'s, Cst<'s>>,
) -> impl Parser<'s, TokenStream<'s>, Cst<'s>, Extra<'s>> + Clone {
    // Mirrors the nom version: trailing `,` immediately before EOF / closer
    // is tolerated (the parenthesised body `(a, b,)` is the canonical case).
    let trim = TokenStream::skip_trivias::<Extra<'s>>();
    custom(move |inp| {
        let mut acc = inp.parse(seq_chain.clone())?;
        loop {
            inp.parse(trim.clone())?;
            if !peek_is(inp, TokenKind::Comma) {
                break;
            }
            let _ = inp.next();
            inp.parse(trim.clone())?;
            if peek_is_closer_or_eof(inp) {
                break;
            }
            let rhs = inp.parse(seq_chain.clone())?;
            acc = bin(BinOp::Comma, acc, rhs);
        }
        Ok(acc)
    })
}

// arg_list (between `(`, `[`, `{`) — comma-separated `seq_chain`s, tolerating:
//   - empty list right before closer
//   - omitted entries: `f(a, , b)` → second arg is `Cst::Empty`
//   - trailing comma: `(a, b,)` does NOT insert a phantom trailing Empty
fn arg_list_parser<'s>(
    seq: BoxedP<'s, Cst<'s>>,
) -> impl Parser<'s, TokenStream<'s>, Vec<Cst<'s>>, Extra<'s>> + Clone {
    let trim = TokenStream::skip_trivias::<Extra<'s>>();
    custom(move |inp| {
        inp.parse(trim.clone())?;
        let mut items: Vec<Cst<'s>> = Vec::new();
        if peek_is_closer_or_eof(inp) {
            return Ok(items);
        }
        loop {
            inp.parse(trim.clone())?;
            if peek_is(inp, TokenKind::Comma) {
                items.push(Cst::Empty);
            } else if peek_is_closer_or_eof(inp) {
                break;
            } else {
                let v = inp.parse(seq.clone())?;
                items.push(v);
            }
            inp.parse(trim.clone())?;
            if peek_is(inp, TokenKind::Comma) {
                let _ = inp.next();
                inp.parse(trim.clone())?;
                if peek_is_closer_or_eof(inp) {
                    break;
                }
            } else {
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

fn kind<'s>(k: TokenKind) -> BoxedP<'s, ()> {
    let trim = TokenStream::skip_trivias::<Extra<'s>>();
    trim.ignore_then(
        any().try_map(move |tok: Lexed<'s, Token<'s>>, span: Span| match tok {
            Lexed::Token(t) if logosky::Token::kind(&t.data) == k => Ok(()),
            _ => Err(Rich::custom(span, "unexpected token")),
        }),
    )
    .boxed()
}

fn tok_matching<'s, T: 's, F>(f: F) -> BoxedP<'s, T>
where
    F: Fn(Token<'s>) -> Option<T> + Clone + 's,
{
    let trim = TokenStream::skip_trivias::<Extra<'s>>();
    trim.ignore_then(
        any().try_map(move |tok: Lexed<'s, Token<'s>>, span: Span| match tok {
            Lexed::Token(t) => f(t.data).ok_or_else(|| Rich::custom(span, "unexpected token")),
            _ => Err(Rich::custom(span, "unexpected token")),
        }),
    )
    .boxed()
}

// --- helpers --------------------------------------------------------

fn bin<'s>(op: BinOp, lhs: Cst<'s>, rhs: Cst<'s>) -> Cst<'s> {
    Cst::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    }
}

fn un<'s>(op: UnaryOp, operand: Cst<'s>) -> Cst<'s> {
    Cst::Unary {
        op,
        operand: Box::new(operand),
    }
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

    #[test]
    fn hello_world() {
        assert_eq!(
            parse("print('Hello World!')"),
            Cst::Call {
                callee: Box::new(Cst::Ident("print")),
                args: vec![Cst::Str("'Hello World!'")],
            }
        );
    }

    #[test]
    fn arithmetic_precedence() {
        assert_eq!(
            parse("2 + 3 * 4"),
            bin(
                BinOp::Add,
                Cst::Number("2"),
                bin(BinOp::Mul, Cst::Number("3"), Cst::Number("4")),
            )
        );
    }

    #[test]
    fn additive_left_assoc() {
        assert_eq!(
            parse("2 + 3 - 1"),
            bin(
                BinOp::Sub,
                bin(BinOp::Add, Cst::Number("2"), Cst::Number("3")),
                Cst::Number("1"),
            )
        );
    }

    #[test]
    fn power_right_assoc() {
        assert_eq!(
            parse("2 ^ 3 ^ 2"),
            bin(
                BinOp::Pow,
                Cst::Number("2"),
                bin(BinOp::Pow, Cst::Number("3"), Cst::Number("2")),
            )
        );
    }

    #[test]
    fn unary_minus_then_get() {
        assert_eq!(
            parse("-foo:0"),
            un(
                UnaryOp::Neg,
                bin(BinOp::Get, Cst::Ident("foo"), Cst::Number("0")),
            )
        );
    }

    #[test]
    fn match_and_get_chain() {
        assert_eq!(
            parse("a:b:c"),
            bin(
                BinOp::Get,
                bin(BinOp::Get, Cst::Ident("a"), Cst::Ident("b")),
                Cst::Ident("c"),
            )
        );
        assert_eq!(
            parse("a~b"),
            bin(BinOp::Match, Cst::Ident("a"), Cst::Ident("b"))
        );
    }

    #[test]
    fn function_definition() {
        let cst = parse("foo(a, b) -> a + b");
        assert_eq!(
            cst,
            bin(
                BinOp::Arrow,
                Cst::Call {
                    callee: Box::new(Cst::Ident("foo")),
                    args: vec![Cst::Ident("a"), Cst::Ident("b")],
                },
                bin(BinOp::Add, Cst::Ident("a"), Cst::Ident("b")),
            )
        );
    }

    #[test]
    fn list_and_map_literals() {
        assert_eq!(
            parse("[1, 2, 3]"),
            Cst::List(vec![Cst::Number("1"), Cst::Number("2"), Cst::Number("3")])
        );
        assert_eq!(
            parse("{'a' -> 1, 'b' -> 2}"),
            Cst::Map(vec![
                bin(BinOp::Arrow, Cst::Str("'a'"), Cst::Number("1")),
                bin(BinOp::Arrow, Cst::Str("'b'"), Cst::Number("2")),
            ])
        );
    }

    #[test]
    fn assignment_right_assoc() {
        assert_eq!(
            parse("a = b = 5"),
            bin(
                BinOp::Assign,
                Cst::Ident("a"),
                bin(BinOp::Assign, Cst::Ident("b"), Cst::Number("5")),
            )
        );
    }

    #[test]
    fn semi_and_comma_sequence() {
        assert_eq!(
            parse("a; b; c"),
            bin(
                BinOp::Semi,
                bin(BinOp::Semi, Cst::Ident("a"), Cst::Ident("b")),
                Cst::Ident("c"),
            )
        );
    }

    #[test]
    fn unpacking_in_call() {
        assert_eq!(
            parse("f(...xs)"),
            Cst::Call {
                callee: Box::new(Cst::Ident("f")),
                args: vec![un(UnaryOp::Unpack, Cst::Ident("xs"))],
            }
        );
    }

    #[test]
    fn nested_function_call() {
        let cst = parse("print(format('f » ', 'g hi'))");
        assert_eq!(
            cst,
            Cst::Call {
                callee: Box::new(Cst::Ident("print")),
                args: vec![Cst::Call {
                    callee: Box::new(Cst::Ident("format")),
                    args: vec![Cst::Str("'f » '"), Cst::Str("'g hi'")],
                }],
            }
        );
    }

    #[test]
    fn comments_and_newlines_are_skipped() {
        let cst = parse("// hello\n  a + b\n");
        assert_eq!(cst, bin(BinOp::Add, Cst::Ident("a"), Cst::Ident("b")));
    }

    #[test]
    fn lenient_trailing_semicolon() {
        assert_eq!(parse("a;"), Cst::Ident("a"));
    }

    #[test]
    fn anonymous_function_in_call() {
        let cst = parse("map([1,2,3], _(x) -> x * x)");
        assert_eq!(
            cst,
            Cst::Call {
                callee: Box::new(Cst::Ident("map")),
                args: vec![
                    Cst::List(vec![Cst::Number("1"), Cst::Number("2"), Cst::Number("3")]),
                    bin(
                        BinOp::Arrow,
                        Cst::Call {
                            callee: Box::new(Cst::Ident("_")),
                            args: vec![Cst::Ident("x")],
                        },
                        bin(BinOp::Mul, Cst::Ident("x"), Cst::Ident("x")),
                    ),
                ],
            }
        );
    }

    #[test]
    fn full_source_from_compdisplay() {
        let src = "toggle() -> (\n    print(player(), 'hi');\n);";
        let cst = parse(src);
        let head = bin(
            BinOp::Arrow,
            Cst::Call {
                callee: Box::new(Cst::Ident("toggle")),
                args: vec![],
            },
            Cst::Paren(Box::new(Cst::Call {
                callee: Box::new(Cst::Ident("print")),
                args: vec![
                    Cst::Call {
                        callee: Box::new(Cst::Ident("player")),
                        args: vec![],
                    },
                    Cst::Str("'hi'"),
                ],
            })),
        );
        assert_eq!(cst, head);
    }
}
