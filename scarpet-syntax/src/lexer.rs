//! Hand-written lexer.
//!
//! Lossless: every byte of the source lands in exactly one token, including
//! whitespace and comments, so the parser can build a full-fidelity `rowan`
//! tree. A sequence the lexer cannot type becomes a [`SyntaxKind::ERROR`]
//! token rather than a failure — the parser reports it in context.

use crate::syntax::SyntaxKind;

/// One lexed token: its kind and the source slice it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'s> {
    pub kind: SyntaxKind,
    pub text: &'s str,
    /// Byte offset of the token's first byte in the source.
    pub start: usize,
}

impl Token<'_> {
    pub fn end(&self) -> usize {
        self.start + self.text.len()
    }

    pub fn range(&self) -> std::ops::Range<usize> {
        self.start..self.end()
    }
}

/// Lex the whole source. Infallible; unknown sequences come back as
/// [`SyntaxKind::ERROR`] tokens.
pub fn lex(src: &str) -> Vec<Token<'_>> {
    // Tokens average a few bytes; reserving up front avoids the realloc chain
    // on this hot path (the parser lexes the whole source before parsing).
    let mut out = Vec::with_capacity(src.len() / 4);
    let mut pos = 0;
    while pos < src.len() {
        let (kind, len) = next_token(&src[pos..]);
        debug_assert!(len > 0, "lexer must always make progress");
        out.push(Token {
            kind,
            text: &src[pos..pos + len],
            start: pos,
        });
        pos += len;
    }
    out
}

/// Horizontal whitespace: spaces, tabs, `\r`, and form feeds. A `\n` is not
/// included — it lexes as a [`SyntaxKind::BREAK`].
fn is_horizontal_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\x0C')
}

/// Scan one token at the head of `rest`, returning its kind and byte length.
fn next_token(rest: &str) -> (SyntaxKind, usize) {
    use SyntaxKind::*;
    let bytes = rest.as_bytes();
    let b = bytes[0];
    match b {
        _ if is_horizontal_ws(b) => {
            let len = bytes.iter().take_while(|&&b| is_horizontal_ws(b)).count();
            (WHITESPACE, len)
        }
        b'\n' | b'$' => (BREAK, 1),
        b'/' => {
            if bytes.get(1) == Some(&b'/') {
                // Runs to end of line; `\r` (CRLF sources) stays whitespace.
                let len = bytes
                    .iter()
                    .take_while(|&&b| b != b'\n' && b != b'\r')
                    .count();
                (COMMENT, len)
            } else {
                (SLASH, 1)
            }
        }
        b'0'..=b'9' => number(bytes),
        b'\'' => string(bytes),
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            let len = bytes
                .iter()
                .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'_')
                .count();
            (IDENT, len)
        }
        b'(' => (L_PAREN, 1),
        b')' => (R_PAREN, 1),
        b'[' => (L_BRACK, 1),
        b']' => (R_BRACK, 1),
        b'{' => (L_BRACE, 1),
        b'}' => (R_BRACE, 1),
        b'=' => two(bytes, b'=', EQ2, EQ),
        b'!' => two(bytes, b'=', BANG_EQ, BANG),
        b'<' => match bytes.get(1) {
            Some(b'=') => (LT_EQ, 2),
            Some(b'>') => (LT_GT, 2),
            _ => (LT, 1),
        },
        b'>' => two(bytes, b'=', GT_EQ, GT),
        b'&' => two(bytes, b'&', AMP2, ERROR),
        b'|' => two(bytes, b'|', PIPE2, ERROR),
        b'+' => two(bytes, b'=', PLUS_EQ, PLUS),
        b'-' => two(bytes, b'>', ARROW, MINUS),
        b'*' => (STAR, 1),
        b'%' => (PERCENT, 1),
        b'^' => (CARET, 1),
        b'~' => (TILDE, 1),
        b':' => (COLON, 1),
        b',' => (COMMA, 1),
        b';' => (SEMICOLON, 1),
        b'.' => {
            if bytes.get(1) == Some(&b'.') && bytes.get(2) == Some(&b'.') {
                (DOT3, 3)
            } else {
                (DOT, 1)
            }
        }
        // Anything else — one full character, as an error token.
        _ => {
            let ch_len = rest.chars().next().map_or(1, char::len_utf8);
            (ERROR, ch_len)
        }
    }
}

/// A two-byte token `<first><second>`, or the one-byte fallback. A fallback of
/// [`SyntaxKind::ERROR`] marks the single byte as unlexable (`&`, `|`).
fn two(bytes: &[u8], second: u8, long: SyntaxKind, short: SyntaxKind) -> (SyntaxKind, usize) {
    if bytes.get(1) == Some(&second) {
        (long, 2)
    } else {
        (short, 1)
    }
}

/// A numeric literal: `0x` hex, or decimal with optional fraction and
/// exponent. Incomplete suffixes back off — `1.` is `1` then `.`, `1e` is `1`
/// then the identifier `e`.
fn number(bytes: &[u8]) -> (SyntaxKind, usize) {
    if bytes[0] == b'0'
        && matches!(bytes.get(1), Some(b'x' | b'X'))
        && bytes.get(2).is_some_and(u8::is_ascii_hexdigit)
    {
        let len = 2 + bytes[2..]
            .iter()
            .take_while(|b| b.is_ascii_hexdigit())
            .count();
        return (SyntaxKind::NUMBER, len);
    }
    let mut len = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if bytes.get(len) == Some(&b'.') && bytes.get(len + 1).is_some_and(u8::is_ascii_digit) {
        len += 1 + bytes[len + 1..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
    }
    if matches!(bytes.get(len), Some(b'e' | b'E')) {
        let mut exp = len + 1;
        if matches!(bytes.get(exp), Some(b'+' | b'-')) {
            exp += 1;
        }
        if bytes.get(exp).is_some_and(u8::is_ascii_digit) {
            len = exp
                + bytes[exp..]
                    .iter()
                    .take_while(|b| b.is_ascii_digit())
                    .count();
        }
    }
    (SyntaxKind::NUMBER, len)
}

/// A single-quoted string; `\` escapes any character, including line breaks.
/// An unterminated string becomes an [`SyntaxKind::ERROR`] token covering the
/// rest of the input.
fn string(bytes: &[u8]) -> (SyntaxKind, usize) {
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\'' => return (SyntaxKind::STRING, i + 1),
            _ => i += 1,
        }
    }
    (SyntaxKind::ERROR, bytes.len())
}

#[cfg(test)]
mod tests {
    use super::{Token, lex};
    use crate::syntax::SyntaxKind::{self, *};

    /// The non-trivia (kind, text) pairs of `src`.
    fn kinds(src: &str) -> Vec<(SyntaxKind, &str)> {
        lex(src)
            .into_iter()
            .filter(|t| t.kind != WHITESPACE)
            .map(|t| (t.kind, t.text))
            .collect()
    }

    #[test]
    fn single_function_expr() {
        assert_eq!(
            kinds("println('Hello World!')"),
            [
                (IDENT, "println"),
                (L_PAREN, "("),
                (STRING, "'Hello World!'"),
                (R_PAREN, ")"),
            ]
        );
    }

    #[test]
    fn numeric_literals() {
        assert_eq!(
            kinds("0xff 1 1.5 1e-10 67E22 0.7"),
            [
                (NUMBER, "0xff"),
                (NUMBER, "1"),
                (NUMBER, "1.5"),
                (NUMBER, "1e-10"),
                (NUMBER, "67E22"),
                (NUMBER, "0.7"),
            ]
        );
    }

    #[test]
    fn incomplete_number_suffixes_back_off() {
        assert_eq!(kinds("1."), [(NUMBER, "1"), (DOT, ".")]);
        assert_eq!(kinds("1e"), [(NUMBER, "1"), (IDENT, "e")]);
        assert_eq!(kinds("1e+"), [(NUMBER, "1"), (IDENT, "e"), (PLUS, "+")]);
        assert_eq!(kinds("0x"), [(NUMBER, "0"), (IDENT, "x")]);
    }

    #[test]
    fn identifiers_with_digits() {
        assert_eq!(
            kinds("block1 __on_start global_color"),
            [
                (IDENT, "block1"),
                (IDENT, "__on_start"),
                (IDENT, "global_color"),
            ]
        );
    }

    #[test]
    fn operators_and_punctuation() {
        let src = "-> <> ... ^ ! += == != <= >= && ||";
        let got: Vec<SyntaxKind> = kinds(src).into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            got,
            [
                ARROW, LT_GT, DOT3, CARET, BANG, PLUS_EQ, EQ2, BANG_EQ, LT_EQ, GT_EQ, AMP2, PIPE2
            ]
        );
    }

    #[test]
    fn dollar_treated_as_break() {
        assert_eq!(kinds("a$b"), [(IDENT, "a"), (BREAK, "$"), (IDENT, "b")]);
    }

    #[test]
    fn crlf_comment_excludes_carriage_return() {
        // A `\r` before the newline (CRLF source) is whitespace, not part of
        // the comment text, and the `\n` alone is the Break.
        assert_eq!(
            kinds("// hi\r\nx"),
            [(COMMENT, "// hi"), (BREAK, "\n"), (IDENT, "x")]
        );
    }

    #[test]
    fn escaped_quote_stays_inside_string() {
        assert_eq!(kinds(r"'a\'b'"), [(STRING, r"'a\'b'")]);
    }

    #[test]
    fn unterminated_string_is_an_error_token() {
        assert_eq!(kinds("'abc"), [(ERROR, "'abc")]);
    }

    #[test]
    fn lone_ampersand_and_pipe_are_error_tokens() {
        assert_eq!(kinds("&"), [(ERROR, "&")]);
        assert_eq!(kinds("|"), [(ERROR, "|")]);
    }

    #[test]
    fn lexing_is_lossless() {
        let src = "foo(a, b) -> ( // c\n  a + b;\n)\r\n";
        let rebuilt: String = lex(src).iter().map(|t| t.text).collect();
        assert_eq!(rebuilt, src);
        // Offsets are contiguous.
        let mut pos = 0;
        for t in lex(src) {
            assert_eq!(t.start, pos, "token {t:?} starts at the previous end");
            pos = t.end();
        }
        assert_eq!(pos, src.len());
    }

    #[test]
    fn non_ascii_is_a_single_error_token() {
        let toks: Vec<Token> = lex("é");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, ERROR);
        assert_eq!(toks[0].text, "é");
    }
}
