use crate::lexer::Token;
use logos::Logos;
use nom::{
    IResult, Parser,
    error::{Error as NomError, ErrorKind},
};

// ====================================================================
// CST (concrete syntax tree, AST-shaped) definition
// ====================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Cst<'s> {
    /// Numeric literal (integer / float / hex / scientific), as-written.
    Number(&'s str),
    /// String literal including the surrounding single quotes.
    Str(&'s str),
    /// Bare identifier (including `null`, `true`, `false`, `pi`, `_`, `_x`...).
    Ident(&'s str),

    /// Function-call style application: `callee(args...)`.
    Call {
        callee: Box<Cst<'s>>,
        args: Vec<Cst<'s>>,
    },
    /// List literal: `[args...]` (preprocessor desugars to `l(args...)`).
    List(Vec<Cst<'s>>),
    /// Map literal: `{args...}` (preprocessor desugars to `m(args...)`).
    Map(Vec<Cst<'s>>),
    /// Parenthesized expression: `(expr)`.
    Paren(Box<Cst<'s>>),
    /// Omitted argument in a call/list/map: `f(a, , b)` → second arg is `Empty`.
    Empty,

    /// Binary operator application.
    Binary {
        op: BinOp,
        lhs: Box<Cst<'s>>,
        rhs: Box<Cst<'s>>,
    },
    /// Prefix unary operator application.
    Unary {
        op: UnaryOp,
        operand: Box<Cst<'s>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    // Comparison
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    // Logical
    And,
    Or,
    // Match / Get
    Match,
    Get,
    // Assignment
    Assign,
    AddAssign,
    Swap,
    // Function definition / map kv
    Arrow,
    // Sequence operators
    Semi,
    Comma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,    // -
    Pos,    // +
    Not,    // !
    Unpack, // ...
}

// ====================================================================
// Front-end: token buffer + builder API expected by the CLI
// ====================================================================

/// Buffer of tokens that the parser will consume.
#[derive(Debug, Default, Clone)]
pub struct Code<'s> {
    tokens: Vec<Token<'s>>,
}

impl<'s> Code<'s> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_source(src: &'s str) -> Result<Self, LexError> {
        let mut tokens = Vec::new();
        for tok in Token::lexer(src) {
            match tok {
                Ok(t) => tokens.push(t),
                Err(()) => return Err(LexError),
            }
        }
        Ok(Self { tokens })
    }

    pub fn push(mut self: Box<Self>, token: Token<'s>) -> Box<Self> {
        self.tokens.push(token);
        self
    }

    pub fn tokens(&self) -> &[Token<'s>] {
        &self.tokens
    }

    pub fn parse(&self) -> Result<Cst<'s>, ParseError> {
        parse_tokens(&self.tokens)
    }
}

/// Builder that produces a [`Code`].
#[derive(Debug, Default, Clone, Copy)]
pub struct Builder;

impl Builder {
    pub fn new<'s>() -> Code<'s> {
        Code::new()
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

// ====================================================================
// nom-based parser
// ====================================================================

type In<'a, 's> = &'a [Token<'s>];
type PResult<'a, 's, O> = IResult<In<'a, 's>, O>;

pub fn parse_tokens<'s>(tokens: &[Token<'s>]) -> Result<Cst<'s>, ParseError> {
    let filtered: Vec<Token<'s>> = tokens.iter().copied().filter(|t| !t.is_trivia()).collect();
    let input: In<'_, 's> = &filtered;
    match parse_program(input) {
        Ok((rest, cst)) if rest.is_empty() => Ok(cst),
        Ok((rest, _)) => Err(ParseError {
            kind: ParseErrorKind::Trailing,
            at: filtered.len() - rest.len(),
        }),
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => Err(ParseError {
            kind: if e.input.is_empty() {
                ParseErrorKind::UnexpectedEof
            } else {
                ParseErrorKind::UnexpectedToken
            },
            at: filtered.len() - e.input.len(),
        }),
        Err(nom::Err::Incomplete(_)) => Err(ParseError {
            kind: ParseErrorKind::UnexpectedEof,
            at: filtered.len(),
        }),
    }
}

// --- token matchers -------------------------------------------------

fn err<'a, 's>(input: In<'a, 's>, kind: ErrorKind) -> nom::Err<NomError<In<'a, 's>>> {
    nom::Err::Error(NomError::new(input, kind))
}

macro_rules! tag_tok {
    ($name:ident, $pat:pat) => {
        fn $name<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, ()> {
            match input.split_first() {
                Some(($pat, rest)) => Ok((rest, ())),
                _ => Err(err(input, ErrorKind::Tag)),
            }
        }
    };
}

tag_tok!(t_open_paren, Token::OpenParen);
tag_tok!(t_close_paren, Token::CloseParen);
tag_tok!(t_open_brack, Token::OpenBrack);
tag_tok!(t_close_brack, Token::CloseBrack);
tag_tok!(t_open_brace, Token::OpenBrace);
tag_tok!(t_close_brace, Token::CloseBrace);
tag_tok!(t_comma, Token::Comma);
tag_tok!(t_semi, Token::SemiColon);
tag_tok!(t_arrow, Token::Arrow);
tag_tok!(t_eq, Token::Assign);
tag_tok!(t_add_eq, Token::AddAssign);
tag_tok!(t_swap, Token::Swap);
tag_tok!(t_or, Token::Or);
tag_tok!(t_and, Token::And);
tag_tok!(t_eqeq, Token::EqEq);
tag_tok!(t_neq, Token::BangEq);
tag_tok!(t_lt, Token::Lt);
tag_tok!(t_lteq, Token::LtEq);
tag_tok!(t_gt, Token::Gt);
tag_tok!(t_gteq, Token::GtEq);
tag_tok!(t_add, Token::Add);
tag_tok!(t_sub, Token::Sub);
tag_tok!(t_mul, Token::Mul);
tag_tok!(t_div, Token::Div);
tag_tok!(t_rem, Token::Rem);
tag_tok!(t_pow, Token::Pow);
tag_tok!(t_bang, Token::Bang);
tag_tok!(t_ellipsis, Token::Ellipsis);
tag_tok!(t_tilde, Token::Tilde);
tag_tok!(t_colon, Token::Colon);

fn t_number<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, &'s str> {
    match input.split_first() {
        Some((Token::Number(s), rest)) => Ok((rest, *s)),
        _ => Err(err(input, ErrorKind::Tag)),
    }
}

fn t_string<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, &'s str> {
    match input.split_first() {
        Some((Token::String(s), rest)) => Ok((rest, *s)),
        _ => Err(err(input, ErrorKind::Tag)),
    }
}

fn t_ident<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, &'s str> {
    match input.split_first() {
        Some((Token::Ident(s), rest)) => Ok((rest, *s)),
        _ => Err(err(input, ErrorKind::Tag)),
    }
}

// --- grammar --------------------------------------------------------
//
// Precedence ladder (low → high), straight from docs/scarpet/language/Operators.md:
//
//   program     = top
//   top         = comma_chain                         // outermost `,` operator
//   comma_chain = seq_chain   (`,` seq_chain)*        // left-assoc
//   seq_chain   = arrow_chain (`;` arrow_chain)*      // left-assoc
//   arrow_chain = assign (`->` arrow_chain)?          // right-assoc
//   assign      = lor    (`=` | `+=` | `<>` assign)?  // right-assoc
//   lor         = land   (`||` land)*
//   land        = equality (`&&` equality)*
//   equality    = compare ((`==` | `!=`) compare)*
//   compare     = additive ((`<` | `<=` | `>` | `>=`) additive)*
//   additive    = multiplicative ((`+` | `-`) multiplicative)*
//   multiplicative = power ((`*` | `/` | `%`) power)*
//   power       = unary (`^` power)?                  // right-assoc
//   unary       = (`+` | `-` | `!` | `...`) unary | get
//   get         = primary ((`~` | `:`) primary)*      // highest binary
//   primary     = atom | `(` top `)` | `[` arg_list `]` | `{` arg_list `}` | ident `(` arg_list `)`
//
// `arg_list` (between `(`, `[`, `{`) is `,`-separated `seq_chain`s — i.e. each
// argument may itself use `;`, but `,` is reserved as the separator.

fn parse_program<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    parse_top(input)
}

fn parse_top<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    parse_comma_chain(input)
}

fn parse_comma_chain<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    let (mut input, mut left) = parse_seq_chain(input)?;
    while let Ok((rest, _)) = t_comma(input) {
        // Trailing-comma tolerance inside parens: `(a, b,)`.
        if matches!(
            rest.first(),
            None | Some(Token::CloseParen | Token::CloseBrack | Token::CloseBrace)
        ) {
            input = rest;
            break;
        }
        let (rest, right) = parse_seq_chain(rest)?;
        left = bin(BinOp::Comma, left, right);
        input = rest;
    }
    Ok((input, left))
}

fn parse_seq_chain<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    let (mut input, mut left) = parse_arrow_chain(input)?;
    while let Ok((rest, _)) = t_semi(input) {
        // Scarpet's preprocessor strips redundant/trailing `;`s — swallow any
        // run of them.
        let mut rest = rest;
        while let Ok((r, _)) = t_semi(rest) {
            rest = r;
        }
        // A `;` directly followed by EOF or a closing bracket is a trailing
        // separator; nothing to chain.
        if matches!(
            rest.first(),
            None | Some(Token::CloseParen | Token::CloseBrack | Token::CloseBrace | Token::Comma)
        ) {
            input = rest;
            break;
        }
        let (after, right) = parse_arrow_chain(rest)?;
        left = bin(BinOp::Semi, left, right);
        input = after;
    }
    Ok((input, left))
}

fn parse_arrow_chain<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    let (input, lhs) = parse_assign(input)?;
    if let Ok((rest, _)) = t_arrow(input) {
        let (rest, rhs) = parse_arrow_chain(rest)?;
        Ok((rest, bin(BinOp::Arrow, lhs, rhs)))
    } else {
        Ok((input, lhs))
    }
}

fn parse_assign<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    let (input, lhs) = parse_lor(input)?;
    if let Ok((rest, _)) = t_eq(input) {
        let (rest, rhs) = parse_assign(rest)?;
        return Ok((rest, bin(BinOp::Assign, lhs, rhs)));
    }
    if let Ok((rest, _)) = t_add_eq(input) {
        let (rest, rhs) = parse_assign(rest)?;
        return Ok((rest, bin(BinOp::AddAssign, lhs, rhs)));
    }
    if let Ok((rest, _)) = t_swap(input) {
        let (rest, rhs) = parse_assign(rest)?;
        return Ok((rest, bin(BinOp::Swap, lhs, rhs)));
    }
    Ok((input, lhs))
}

fn parse_lor<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    left_assoc(input, parse_land, |i| {
        let (i, _) = t_or(i)?;
        Ok((i, BinOp::Or))
    })
}

fn parse_land<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    left_assoc(input, parse_equality, |i| {
        let (i, _) = t_and(i)?;
        Ok((i, BinOp::And))
    })
}

fn parse_equality<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    left_assoc(input, parse_compare, |i| {
        if let Ok((i, _)) = t_eqeq(i) {
            return Ok((i, BinOp::Eq));
        }
        let (i, _) = t_neq(i)?;
        Ok((i, BinOp::NotEq))
    })
}

fn parse_compare<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    left_assoc(input, parse_additive, |i| {
        // Order matters: `<=` and `>=` are matched as their own tokens by the
        // lexer, so we can probe them in any order here.
        if let Ok((i, _)) = t_lteq(i) {
            return Ok((i, BinOp::LtEq));
        }
        if let Ok((i, _)) = t_gteq(i) {
            return Ok((i, BinOp::GtEq));
        }
        if let Ok((i, _)) = t_lt(i) {
            return Ok((i, BinOp::Lt));
        }
        let (i, _) = t_gt(i)?;
        Ok((i, BinOp::Gt))
    })
}

fn parse_additive<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    left_assoc(input, parse_multiplicative, |i| {
        if let Ok((i, _)) = t_add(i) {
            return Ok((i, BinOp::Add));
        }
        let (i, _) = t_sub(i)?;
        Ok((i, BinOp::Sub))
    })
}

fn parse_multiplicative<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    left_assoc(input, parse_power, |i| {
        if let Ok((i, _)) = t_mul(i) {
            return Ok((i, BinOp::Mul));
        }
        if let Ok((i, _)) = t_div(i) {
            return Ok((i, BinOp::Div));
        }
        let (i, _) = t_rem(i)?;
        Ok((i, BinOp::Rem))
    })
}

fn parse_power<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    let (input, lhs) = parse_unary(input)?;
    if let Ok((rest, _)) = t_pow(input) {
        let (rest, rhs) = parse_power(rest)?;
        Ok((rest, bin(BinOp::Pow, lhs, rhs)))
    } else {
        Ok((input, lhs))
    }
}

fn parse_unary<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    if let Ok((rest, _)) = t_sub(input) {
        let (rest, operand) = parse_unary(rest)?;
        return Ok((rest, un(UnaryOp::Neg, operand)));
    }
    if let Ok((rest, _)) = t_add(input) {
        let (rest, operand) = parse_unary(rest)?;
        return Ok((rest, un(UnaryOp::Pos, operand)));
    }
    if let Ok((rest, _)) = t_bang(input) {
        let (rest, operand) = parse_unary(rest)?;
        return Ok((rest, un(UnaryOp::Not, operand)));
    }
    if let Ok((rest, _)) = t_ellipsis(input) {
        let (rest, operand) = parse_unary(rest)?;
        return Ok((rest, un(UnaryOp::Unpack, operand)));
    }
    parse_get(input)
}

fn parse_get<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    let (mut input, mut left) = parse_primary(input)?;
    loop {
        if let Ok((rest, _)) = t_tilde(input) {
            let (rest, right) = parse_primary(rest)?;
            left = bin(BinOp::Match, left, right);
            input = rest;
            continue;
        }
        if let Ok((rest, _)) = t_colon(input) {
            let (rest, right) = parse_primary(rest)?;
            left = bin(BinOp::Get, left, right);
            input = rest;
            continue;
        }
        break;
    }
    Ok((input, left))
}

fn parse_primary<'a, 's>(input: In<'a, 's>) -> PResult<'a, 's, Cst<'s>> {
    // Literals
    if let Ok((rest, s)) = t_number(input) {
        return Ok((rest, Cst::Number(s)));
    }
    if let Ok((rest, s)) = t_string(input) {
        return Ok((rest, Cst::Str(s)));
    }

    // Identifier (possibly followed by a call)
    if let Ok((rest, name)) = t_ident(input) {
        if let Ok((rest, _)) = t_open_paren(rest) {
            let (rest, args) = parse_arg_list(rest)?;
            let (rest, _) = t_close_paren(rest)?;
            return Ok((
                rest,
                Cst::Call {
                    callee: Box::new(Cst::Ident(name)),
                    args,
                },
            ));
        }
        return Ok((rest, Cst::Ident(name)));
    }

    // Parenthesized expression
    if let Ok((rest, _)) = t_open_paren(input) {
        let (rest, inner) = parse_top(rest)?;
        let (rest, _) = t_close_paren(rest)?;
        return Ok((rest, Cst::Paren(Box::new(inner))));
    }

    // List literal `[a, b, c]` (preprocessor: `l(a, b, c)`)
    if let Ok((rest, _)) = t_open_brack(input) {
        let (rest, args) = parse_arg_list(rest)?;
        let (rest, _) = t_close_brack(rest)?;
        return Ok((rest, Cst::List(args)));
    }

    // Map literal `{a -> b, c}` (preprocessor: `m(a -> b, c)`)
    if let Ok((rest, _)) = t_open_brace(input) {
        let (rest, args) = parse_arg_list(rest)?;
        let (rest, _) = t_close_brace(rest)?;
        return Ok((rest, Cst::Map(args)));
    }

    Err(err(input, ErrorKind::Alt))
}

/// Comma-separated list of arguments (each is a `seq_chain`).
/// Used for function-call args, list contents, and map contents.
///
/// Tolerates omitted entries (e.g. `f(a, , b)`) by inserting [`Cst::Empty`].
fn parse_arg_list<'a, 's>(mut input: In<'a, 's>) -> PResult<'a, 's, Vec<Cst<'s>>> {
    let is_closer = |inp: In<'_, 's>| {
        matches!(
            inp.first(),
            None | Some(Token::CloseParen | Token::CloseBrack | Token::CloseBrace)
        )
    };
    if is_closer(input) {
        return Ok((input, Vec::new()));
    }
    let mut items: Vec<Cst<'s>> = Vec::new();
    loop {
        // Try to parse one argument. A comma or closer here means an omitted arg.
        if matches!(input.first(), Some(Token::Comma)) {
            items.push(Cst::Empty);
        } else if is_closer(input) {
            // Trailing comma immediately before closer: stop (don't insert
            // a phantom trailing empty).
            break;
        } else {
            let (rest, arg) = parse_seq_chain(input)?;
            items.push(arg);
            input = rest;
        }
        match t_comma(input) {
            Ok((rest, _)) => input = rest,
            Err(_) => break,
        }
    }
    Ok((input, items))
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

/// Generic left-associative chain: `lower (op lower)*`.
fn left_assoc<'a, 's, F>(
    input: In<'a, 's>,
    mut lower: impl FnMut(In<'a, 's>) -> PResult<'a, 's, Cst<'s>>,
    mut op_parser: F,
) -> PResult<'a, 's, Cst<'s>>
where
    F: FnMut(In<'a, 's>) -> PResult<'a, 's, BinOp>,
{
    let (mut input, mut left) = lower(input)?;
    while let Ok((rest, op)) = op_parser(input) {
        let (rest, right) = lower(rest)?;
        left = bin(op, left, right);
        input = rest;
    }
    Ok((input, left))
}

// Keep nom's `Parser` import referenced (helps if downstream uses it).
#[allow(dead_code)]
fn _nom_parser_ref<P: Parser<()>>(_: P) {}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(src: &str) -> Vec<Token<'_>> {
        Token::lexer(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lex error")
    }

    fn parse(src: &str) -> Cst<'_> {
        let tokens = lex(src);
        parse_tokens(&tokens).expect("parse error")
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
        // 2 + 3 * 4  =>  Add(2, Mul(3, 4))
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
        // 2 + 3 - 1  =>  Sub(Add(2, 3), 1)
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
        // 2 ^ 3 ^ 2  =>  Pow(2, Pow(3, 2))
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
        // -foo:0  =>  Neg(Get(foo, 0))   (`:` binds tighter than unary `-`)
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
        // a:b:c  =>  Get(Get(a, b), c)
        assert_eq!(
            parse("a:b:c"),
            bin(
                BinOp::Get,
                bin(BinOp::Get, Cst::Ident("a"), Cst::Ident("b")),
                Cst::Ident("c"),
            )
        );
        // a~b  =>  Match(a, b)
        assert_eq!(
            parse("a~b"),
            bin(BinOp::Match, Cst::Ident("a"), Cst::Ident("b"))
        );
    }

    #[test]
    fn function_definition() {
        // foo(a, b) -> a + b
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
            Cst::List(vec![Cst::Number("1"), Cst::Number("2"), Cst::Number("3"),])
        );
        // Map with `->` key-value pairs
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
        // a = b = 5  =>  Assign(a, Assign(b, 5))
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
        // a; b; c  =>  Semi(Semi(a, b), c)  (left-assoc)
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
        // f(...xs)
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
        // print(format('f » ', 'g hi'))
        let cst = parse("print(format('f » ', 'g hi'))");
        assert_eq!(
            cst,
            Cst::Call {
                callee: Box::new(Cst::Ident("print")),
                args: vec![Cst::Call {
                    callee: Box::new(Cst::Ident("format")),
                    args: vec![Cst::Str("'f » '"), Cst::Str("'g hi'"),],
                },],
            }
        );
    }

    #[test]
    fn comments_and_newlines_are_skipped() {
        let cst = parse("// hello\n  a + b\n");
        assert_eq!(
            cst,
            bin(BinOp::Add, Cst::Ident("a"), Cst::Ident("b"))
        );
    }

    #[test]
    fn lenient_trailing_semicolon() {
        // Scarpet's preprocessor strips redundant `;`s; we tolerate one.
        assert_eq!(parse("a;"), Cst::Ident("a"));
    }

    #[test]
    fn anonymous_function_in_call() {
        // map([1,2,3], _(x) -> x * x)
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
    fn code_builder_round_trip() {
        // Smoke test for the CLI-facing Code/Builder API.
        let code: Box<Code<'_>> = Box::new(Code::new())
            .push(Token::Ident("println"))
            .push(Token::OpenParen)
            .push(Token::CloseParen);
        assert_eq!(
            code.parse().unwrap(),
            Cst::Call {
                callee: Box::new(Cst::Ident("println")),
                args: vec![],
            }
        );
    }

    #[test]
    fn full_source_from_compdisplay() {
        // toggle() -> (
        //   print(player(), 'hi');
        // );
        let src = "toggle() -> (\n    print(player(), 'hi');\n);";
        let cst = parse(src);
        // outermost is `Arrow(toggle(), Paren(Semi(print(...), ?)))` — the
        // trailing `;` before `)` is tolerated by parse_seq_chain.
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
