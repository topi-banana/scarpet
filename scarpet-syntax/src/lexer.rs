use logos::Logos;

#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[logos(skip r"[ \t\r\f]+")]
pub enum Token<'a> {
    // ===== Trivia =====
    #[token("\n")]
    #[token("$")]
    Break,
    #[regex(r"//[^\n\r]*")]
    Comment(&'a str),

    // ===== Literals / Atoms =====
    #[regex(r"0[xX][0-9a-fA-F]+", priority = 3)]
    #[regex(r"\d+(\.\d+)?([eE][+-]?\d+)?", priority = 2)]
    Number(&'a str),
    #[regex(r"'(\\[\s\S]|[^'\\])*'")]
    String(&'a str),
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")]
    Ident(&'a str),

    // ===== Delimiters =====
    #[token("(")]
    OpenParen,
    #[token(")")]
    CloseParen,
    #[token("[")]
    OpenBrack,
    #[token("]")]
    CloseBrack,
    #[token("{")]
    OpenBrace,
    #[token("}")]
    CloseBrace,

    // ===== Comparison =====
    #[token("==")]
    EqEq,
    #[token("!=")]
    BangEq,
    #[token("<")]
    Lt,
    #[token("<=")]
    LtEq,
    #[token(">")]
    Gt,
    #[token(">=")]
    GtEq,

    // ===== Logical =====
    #[token("&&")]
    And,
    #[token("||")]
    Or,
    #[token("!")]
    Bang,

    // ===== Arithmetic =====
    #[token("+")]
    Add,
    #[token("-")]
    Sub,
    #[token("*")]
    Mul,
    #[token("/")]
    Div,
    #[token("%")]
    Rem,
    #[token("^")]
    Pow,

    // ===== Assignment =====
    #[token("=")]
    Assign,
    #[token("+=")]
    AddAssign,
    #[token("<>")]
    Swap,

    // ===== Match / Get =====
    #[token("~")]
    Tilde,
    #[token(":")]
    Colon,

    // ===== Misc punctuation =====
    #[token(".")]
    Dot,
    #[token("...")]
    Ellipsis,
    #[token(",")]
    Comma,
    #[token(";")]
    SemiColon,
    #[token("->")]
    Arrow,
}

impl<'a> Token<'a> {
    /// True if the token is whitespace-like (newline / dollar-line / comment) that
    /// the parser typically wants to skip.
    pub fn is_trivia(&self) -> bool {
        matches!(self, Token::Break | Token::Comment(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TokenKind {
    Break,
    Comment,
    Number,
    String,
    Ident,
    OpenParen,
    CloseParen,
    OpenBrack,
    CloseBrack,
    OpenBrace,
    CloseBrace,
    EqEq,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    Bang,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Assign,
    AddAssign,
    Swap,
    Tilde,
    Colon,
    Dot,
    Ellipsis,
    Comma,
    SemiColon,
    Arrow,
}

impl<'a> logosky::Token<'a> for Token<'a> {
    type Char = char;
    type Kind = TokenKind;
    type Logos = Self;

    fn kind(&self) -> Self::Kind {
        match self {
            Token::Break => TokenKind::Break,
            Token::Comment(_) => TokenKind::Comment,
            Token::Number(_) => TokenKind::Number,
            Token::String(_) => TokenKind::String,
            Token::Ident(_) => TokenKind::Ident,
            Token::OpenParen => TokenKind::OpenParen,
            Token::CloseParen => TokenKind::CloseParen,
            Token::OpenBrack => TokenKind::OpenBrack,
            Token::CloseBrack => TokenKind::CloseBrack,
            Token::OpenBrace => TokenKind::OpenBrace,
            Token::CloseBrace => TokenKind::CloseBrace,
            Token::EqEq => TokenKind::EqEq,
            Token::BangEq => TokenKind::BangEq,
            Token::Lt => TokenKind::Lt,
            Token::LtEq => TokenKind::LtEq,
            Token::Gt => TokenKind::Gt,
            Token::GtEq => TokenKind::GtEq,
            Token::And => TokenKind::And,
            Token::Or => TokenKind::Or,
            Token::Bang => TokenKind::Bang,
            Token::Add => TokenKind::Add,
            Token::Sub => TokenKind::Sub,
            Token::Mul => TokenKind::Mul,
            Token::Div => TokenKind::Div,
            Token::Rem => TokenKind::Rem,
            Token::Pow => TokenKind::Pow,
            Token::Assign => TokenKind::Assign,
            Token::AddAssign => TokenKind::AddAssign,
            Token::Swap => TokenKind::Swap,
            Token::Tilde => TokenKind::Tilde,
            Token::Colon => TokenKind::Colon,
            Token::Dot => TokenKind::Dot,
            Token::Ellipsis => TokenKind::Ellipsis,
            Token::Comma => TokenKind::Comma,
            Token::SemiColon => TokenKind::SemiColon,
            Token::Arrow => TokenKind::Arrow,
        }
    }
}

impl<'a> logosky::LosslessToken<'a> for Token<'a> {
    fn is_trivia(&self) -> bool {
        Token::is_trivia(self)
    }
}

#[cfg(test)]
mod tests {
    use super::Token;
    #[test]
    fn single_function_expr() {
        let tokens: Result<Vec<_>, _> =
            logos::Lexer::<Token>::new("println('Hello World!')").collect();
        assert_eq!(
            tokens.unwrap(),
            [
                Token::Ident("println"),
                Token::OpenParen,
                Token::String("'Hello World!'"),
                Token::CloseParen
            ]
        );
    }

    #[test]
    fn numeric_literals() {
        let tokens: Result<Vec<_>, _> =
            logos::Lexer::<Token>::new("0xff 1 1.5 1e-10 67E22 0.7").collect();
        assert_eq!(
            tokens.unwrap(),
            [
                Token::Number("0xff"),
                Token::Number("1"),
                Token::Number("1.5"),
                Token::Number("1e-10"),
                Token::Number("67E22"),
                Token::Number("0.7"),
            ]
        );
    }

    #[test]
    fn identifiers_with_digits() {
        let tokens: Result<Vec<_>, _> =
            logos::Lexer::<Token>::new("block1 __on_start global_color").collect();
        assert_eq!(
            tokens.unwrap(),
            [
                Token::Ident("block1"),
                Token::Ident("__on_start"),
                Token::Ident("global_color"),
            ]
        );
    }

    #[test]
    fn operators_and_punctuation() {
        let src = "-> <> ... ^ ! += == != <= >= && ||";
        let tokens: Result<Vec<_>, _> = logos::Lexer::<Token>::new(src).collect();
        assert_eq!(
            tokens.unwrap(),
            [
                Token::Arrow,
                Token::Swap,
                Token::Ellipsis,
                Token::Pow,
                Token::Bang,
                Token::AddAssign,
                Token::EqEq,
                Token::BangEq,
                Token::LtEq,
                Token::GtEq,
                Token::And,
                Token::Or,
            ]
        );
    }

    #[test]
    fn dollar_treated_as_break() {
        let tokens: Result<Vec<_>, _> = logos::Lexer::<Token>::new("a$b").collect();
        assert_eq!(
            tokens.unwrap(),
            [Token::Ident("a"), Token::Break, Token::Ident("b")]
        );
    }

    #[test]
    fn crlf_comment_excludes_carriage_return() {
        // A `\r` before the newline (CRLF source) is whitespace, not part of
        // the comment text, and the `\n` alone is the Break.
        let tokens: Result<Vec<_>, _> = logos::Lexer::<Token>::new("// hi\r\nx").collect();
        assert_eq!(
            tokens.unwrap(),
            [Token::Comment("// hi"), Token::Break, Token::Ident("x")]
        );
    }
}
