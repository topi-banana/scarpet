//! Hand-written lossless lexer for Scarpet.
//!
//! [`lex`] produces a flat token stream whose lengths sum to the input
//! length: every byte of the source belongs to exactly one token, so
//! concatenating the token texts reconstructs the source byte-for-byte.
//! Unlexable input becomes [`SyntaxKind::ERROR_TOKEN`] instead of being
//! dropped.
//!
//! The token boundaries intentionally reproduce the `logos` lexer in
//! [`crate::lexer`] exactly — the differential corpus test in this module is
//! the arbiter — with one presentational difference: whitespace, which logos
//! *skips*, is emitted here as [`SyntaxKind::WHITESPACE`] tokens. The old
//! lexer (and the differential test) will be removed once the rowan
//! migration completes.

use crate::syntax_kind::SyntaxKind;

/// A single token produced by [`lex`]: a kind plus the byte length of the
/// matched text.
///
/// Offsets are implicit — each token starts where the previous one ended, so
/// the n-th token's range is the sum of the first n-1 lengths onward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexedToken {
    pub kind: SyntaxKind,
    pub len: u32,
}

/// Lexes `src` into a lossless token stream.
///
/// Invariant: the `len`s of the returned tokens sum to `src.len()`, every
/// token is non-empty, and every token boundary is a `char` boundary.
///
/// # Panics
///
/// Panics if `src` is 4 GiB or larger (token lengths are `u32`; truncating
/// silently would corrupt the stream instead).
pub fn lex(src: &str) -> Vec<LexedToken> {
    assert!(
        u32::try_from(src.len()).is_ok(),
        "lex() supports sources up to u32::MAX bytes"
    );
    let mut lexer = Lexer {
        src: src.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
    };
    while lexer.pos < lexer.src.len() {
        lexer.next_token();
    }
    lexer.tokens
}

/// Lexes `src` and pairs each token with its source text.
///
/// Convenience over [`lex`] for consumers that want the text (or byte
/// ranges, via [`str::len`] on the slices): offsets are reconstructed by
/// accumulating the token lengths, so the yielded slices concatenate back to
/// exactly `src`.
pub fn lex_with_text(src: &str) -> impl Iterator<Item = (SyntaxKind, &str)> {
    let mut offset = 0usize;
    lex(src).into_iter().map(move |tok| {
        let start = offset;
        offset += tok.len as usize;
        (tok.kind, &src[start..offset])
    })
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    tokens: Vec<LexedToken>,
}

/// Byte length of the UTF-8 character whose first byte is `byte`.
///
/// Only called on character starts of valid UTF-8 (the input is a `&str`),
/// where the length equals the number of leading one bits (and one for
/// ASCII).
fn utf8_char_len(byte: u8) -> usize {
    (byte.leading_ones() as usize).max(1)
}

fn is_whitespace(byte: u8) -> bool {
    // Mirrors the logos skip set `[ \t\r\f]+`. `\n` is NEWLINE, not
    // whitespace, so each line break is its own token.
    matches!(byte, b' ' | b'\t' | b'\r' | b'\x0C')
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

impl Lexer<'_> {
    /// The byte `n` positions ahead of the cursor, if any.
    fn at(&self, n: usize) -> Option<u8> {
        self.src.get(self.pos + n).copied()
    }

    /// Extends a run: starting `from` bytes ahead of the cursor, advances
    /// while `pred` holds and returns the first offset past the run.
    fn run_end(&self, mut from: usize, pred: impl Fn(u8) -> bool) -> usize {
        while self.at(from).is_some_and(&pred) {
            from += 1;
        }
        from
    }

    /// Emits a token of `len` bytes starting at the cursor and advances.
    fn push(&mut self, kind: SyntaxKind, len: usize) {
        debug_assert!(len > 0, "tokens must be non-empty to stay lossless");
        self.tokens.push(LexedToken {
            kind,
            len: len as u32,
        });
        self.pos += len;
    }

    fn next_token(&mut self) {
        use SyntaxKind as K;
        let byte = self.src[self.pos];
        match byte {
            b if is_whitespace(b) => self.whitespace(),
            // `$` is Scarpet's newline stand-in in one-liners; each `\n`/`$`
            // is its own NEWLINE token.
            b'\n' | b'$' => self.push(K::NEWLINE, 1),
            b'/' if self.at(1) == Some(b'/') => self.comment(),
            b'\'' => self.string(),
            b'0'..=b'9' => self.number(),
            b if is_ident_start(b) => self.ident(),

            b'(' => self.push(K::OPEN_PAREN, 1),
            b')' => self.push(K::CLOSE_PAREN, 1),
            b'[' => self.push(K::OPEN_BRACK, 1),
            b']' => self.push(K::CLOSE_BRACK, 1),
            b'{' => self.push(K::OPEN_BRACE, 1),
            b'}' => self.push(K::CLOSE_BRACE, 1),

            b'=' => match self.at(1) {
                Some(b'=') => self.push(K::EQ_EQ, 2),
                _ => self.push(K::EQ, 1),
            },
            b'!' => match self.at(1) {
                Some(b'=') => self.push(K::BANG_EQ, 2),
                _ => self.push(K::BANG, 1),
            },
            b'<' => match self.at(1) {
                Some(b'=') => self.push(K::LT_EQ, 2),
                Some(b'>') => self.push(K::SWAP, 2),
                _ => self.push(K::LT, 1),
            },
            b'>' => match self.at(1) {
                Some(b'=') => self.push(K::GT_EQ, 2),
                _ => self.push(K::GT, 1),
            },
            // A lone `&` or `|` is not a Scarpet token.
            b'&' => match self.at(1) {
                Some(b'&') => self.push(K::AND_AND, 2),
                _ => self.push(K::ERROR_TOKEN, 1),
            },
            b'|' => match self.at(1) {
                Some(b'|') => self.push(K::OR_OR, 2),
                _ => self.push(K::ERROR_TOKEN, 1),
            },

            b'+' => match self.at(1) {
                Some(b'=') => self.push(K::PLUS_EQ, 2),
                _ => self.push(K::PLUS, 1),
            },
            b'-' => match self.at(1) {
                Some(b'>') => self.push(K::ARROW, 2),
                _ => self.push(K::MINUS, 1),
            },
            b'*' => self.push(K::STAR, 1),
            b'/' => self.push(K::SLASH, 1),
            b'%' => self.push(K::PERCENT, 1),
            b'^' => self.push(K::CARET, 1),
            b'~' => self.push(K::TILDE, 1),
            b':' => self.push(K::COLON, 1),
            // `...` is ELLIPSIS, but `..` is two DOTs.
            b'.' => {
                if self.at(1) == Some(b'.') && self.at(2) == Some(b'.') {
                    self.push(K::ELLIPSIS, 3);
                } else {
                    self.push(K::DOT, 1);
                }
            }
            b',' => self.push(K::COMMA, 1),
            b';' => self.push(K::SEMICOLON, 1),

            // Any other character (non-ASCII included) is unlexable; emit one
            // ERROR_TOKEN per character, covering the whole UTF-8 character.
            b => self.push(K::ERROR_TOKEN, utf8_char_len(b)),
        }
    }

    /// Maximal run of `[ \t\r\f]`.
    fn whitespace(&mut self) {
        let len = self.run_end(1, is_whitespace);
        self.push(SyntaxKind::WHITESPACE, len);
    }

    /// `//` through end of line, stopping before `\n`/`\r`/EOF (so a CRLF
    /// source keeps the `\r` out of the comment text).
    fn comment(&mut self) {
        let len = self.run_end(2, |b| b != b'\n' && b != b'\r');
        self.push(SyntaxKind::COMMENT, len);
    }

    /// Single-quoted string: `'(\\[\s\S]|[^'\\])*'`. A backslash escapes any
    /// character, including newlines, so the literal may span lines and the
    /// only way for it to fail is running out of input. An unterminated
    /// string therefore becomes a single ERROR_TOKEN covering the opening
    /// quote through end of input — exactly the span the logos lexer reports.
    ///
    /// One wrinkle, also mirrored from logos: when the input ends in the
    /// middle of an escape (a dangling `\` as the last byte), the failed
    /// match is rewound to the last completed repetition, so the error stops
    /// *before* the backslash and the backslash is then re-lexed on its own
    /// (becoming a separate one-byte ERROR_TOKEN via the main loop).
    fn string(&mut self) {
        let mut len = 1;
        loop {
            match self.at(len) {
                None => {
                    self.push(SyntaxKind::ERROR_TOKEN, self.src.len() - self.pos);
                    return;
                }
                Some(b'\'') => {
                    self.push(SyntaxKind::STRING, len + 1);
                    return;
                }
                Some(b'\\') => match self.at(len + 1) {
                    // Dangling backslash at EOF: rewind the incomplete
                    // escape; the backslash is lexed separately.
                    None => {
                        self.push(SyntaxKind::ERROR_TOKEN, len);
                        return;
                    }
                    Some(escaped) => len += 1 + utf8_char_len(escaped),
                },
                Some(b) => len += utf8_char_len(b),
            }
        }
    }

    /// Numbers, mirroring the logos regexes and their backtracking:
    ///
    /// - hex `0[xX][0-9a-fA-F]+` (so `0x` with no hex digit is NUMBER `0`
    ///   then IDENT `x`),
    /// - decimal `\d+(\.\d+)?([eE][+-]?\d+)?` — the `.` only joins when a
    ///   digit follows (`1.` is NUMBER `1` then DOT), and the exponent only
    ///   joins when it terminates with digits (`1e`/`1e+` leave the `e`
    ///   behind as an IDENT).
    fn number(&mut self) {
        let is_digit = |b: u8| b.is_ascii_digit();
        let is_hex = |b: u8| b.is_ascii_hexdigit();

        if self.src[self.pos] == b'0'
            && matches!(self.at(1), Some(b'x' | b'X'))
            && self.at(2).is_some_and(is_hex)
        {
            let len = self.run_end(3, is_hex);
            self.push(SyntaxKind::NUMBER, len);
            return;
        }

        let mut len = self.run_end(1, is_digit);
        if self.at(len) == Some(b'.') && self.at(len + 1).is_some_and(is_digit) {
            len = self.run_end(len + 2, is_digit);
        }
        if matches!(self.at(len), Some(b'e' | b'E')) {
            let mut exp = len + 1;
            if matches!(self.at(exp), Some(b'+' | b'-')) {
                exp += 1;
            }
            if self.at(exp).is_some_and(is_digit) {
                len = self.run_end(exp + 1, is_digit);
            }
        }
        self.push(SyntaxKind::NUMBER, len);
    }

    /// `[a-zA-Z_][a-zA-Z0-9_]*`.
    fn ident(&mut self) {
        let len = self.run_end(1, is_ident_continue);
        self.push(SyntaxKind::IDENT, len);
    }
}

#[cfg(test)]
mod tests {
    use super::{LexedToken, lex, lex_with_text};
    use crate::syntax_kind::SyntaxKind as K;

    /// Full lossless stream as `(kind, text)` pairs.
    fn texts(src: &str) -> Vec<(K, &str)> {
        lex_with_text(src).collect()
    }

    /// Stream with WHITESPACE filtered out, for tests that mirror the old
    /// logos tests (logos skips whitespace).
    fn non_ws(src: &str) -> Vec<(K, &str)> {
        lex_with_text(src)
            .filter(|(k, _)| *k != K::WHITESPACE)
            .collect()
    }

    // ===== Ports of the logos lexer tests (`crate::lexer::tests`) =====

    #[test]
    fn single_function_expr() {
        assert_eq!(
            texts("println('Hello World!')"),
            [
                (K::IDENT, "println"),
                (K::OPEN_PAREN, "("),
                (K::STRING, "'Hello World!'"),
                (K::CLOSE_PAREN, ")"),
            ]
        );
    }

    #[test]
    fn numeric_literals() {
        assert_eq!(
            non_ws("0xff 1 1.5 1e-10 67E22 0.7"),
            [
                (K::NUMBER, "0xff"),
                (K::NUMBER, "1"),
                (K::NUMBER, "1.5"),
                (K::NUMBER, "1e-10"),
                (K::NUMBER, "67E22"),
                (K::NUMBER, "0.7"),
            ]
        );
    }

    #[test]
    fn identifiers_with_digits() {
        assert_eq!(
            non_ws("block1 __on_start global_color"),
            [
                (K::IDENT, "block1"),
                (K::IDENT, "__on_start"),
                (K::IDENT, "global_color"),
            ]
        );
    }

    #[test]
    fn operators_and_punctuation() {
        assert_eq!(
            non_ws("-> <> ... ^ ! += == != <= >= && ||"),
            [
                (K::ARROW, "->"),
                (K::SWAP, "<>"),
                (K::ELLIPSIS, "..."),
                (K::CARET, "^"),
                (K::BANG, "!"),
                (K::PLUS_EQ, "+="),
                (K::EQ_EQ, "=="),
                (K::BANG_EQ, "!="),
                (K::LT_EQ, "<="),
                (K::GT_EQ, ">="),
                (K::AND_AND, "&&"),
                (K::OR_OR, "||"),
            ]
        );
    }

    #[test]
    fn dollar_treated_as_break() {
        assert_eq!(
            texts("a$b"),
            [(K::IDENT, "a"), (K::NEWLINE, "$"), (K::IDENT, "b")]
        );
    }

    #[test]
    fn crlf_comment_excludes_carriage_return() {
        // The `\r` of a CRLF line ending is whitespace, not part of the
        // comment text, and the `\n` alone is the NEWLINE.
        assert_eq!(
            texts("// hi\r\nx"),
            [
                (K::COMMENT, "// hi"),
                (K::WHITESPACE, "\r"),
                (K::NEWLINE, "\n"),
                (K::IDENT, "x"),
            ]
        );
    }

    // ===== Boundary cases =====

    #[test]
    fn number_boundaries() {
        // `.` only joins a number when a digit follows.
        assert_eq!(texts("1."), [(K::NUMBER, "1"), (K::DOT, ".")]);
        // The exponent only joins when it terminates with digits.
        assert_eq!(texts("1e"), [(K::NUMBER, "1"), (K::IDENT, "e")]);
        assert_eq!(
            texts("1e+"),
            [(K::NUMBER, "1"), (K::IDENT, "e"), (K::PLUS, "+")]
        );
        assert_eq!(texts("1.5e"), [(K::NUMBER, "1.5"), (K::IDENT, "e")]);
        assert_eq!(texts("1.5e-10"), [(K::NUMBER, "1.5e-10")]);
        // Hex needs at least one hex digit; `0x` alone backtracks to `0`.
        assert_eq!(texts("0x"), [(K::NUMBER, "0"), (K::IDENT, "x")]);
        assert_eq!(texts("0xFF"), [(K::NUMBER, "0xFF")]);
        assert_eq!(texts("0xZ"), [(K::NUMBER, "0"), (K::IDENT, "xZ")]);
        assert_eq!(
            texts("1..2"),
            [
                (K::NUMBER, "1"),
                (K::DOT, "."),
                (K::DOT, "."),
                (K::NUMBER, "2")
            ]
        );
    }

    #[test]
    fn dots_and_ellipsis() {
        // `..` is two DOTs; `...` is one ELLIPSIS; `....` is ELLIPSIS + DOT.
        assert_eq!(texts(".."), [(K::DOT, "."), (K::DOT, ".")]);
        assert_eq!(texts("..."), [(K::ELLIPSIS, "...")]);
        assert_eq!(texts("...."), [(K::ELLIPSIS, "..."), (K::DOT, ".")]);
    }

    #[test]
    fn lone_ampersand_and_pipe_are_errors() {
        assert_eq!(texts("&"), [(K::ERROR_TOKEN, "&")]);
        assert_eq!(texts("|"), [(K::ERROR_TOKEN, "|")]);
        assert_eq!(texts("&x"), [(K::ERROR_TOKEN, "&"), (K::IDENT, "x")]);
        assert_eq!(texts("&&"), [(K::AND_AND, "&&")]);
        assert_eq!(texts("||"), [(K::OR_OR, "||")]);
    }

    #[test]
    fn strings() {
        assert_eq!(texts("''"), [(K::STRING, "''")]);
        // Escaped quote.
        assert_eq!(texts(r"'a\'b'"), [(K::STRING, r"'a\'b'")]);
        // A backslash escapes anything, including a newline.
        assert_eq!(texts("'a\\\nb'"), [(K::STRING, "'a\\\nb'")]);
    }

    #[test]
    fn unterminated_string_is_one_error_to_eof() {
        // Like logos, the failed string match swallows everything from the
        // opening quote to end of input as a single error.
        assert_eq!(texts("'abc"), [(K::ERROR_TOKEN, "'abc")]);
        assert_eq!(
            texts("x = 'abc\ny = 1"),
            [
                (K::IDENT, "x"),
                (K::WHITESPACE, " "),
                (K::EQ, "="),
                (K::WHITESPACE, " "),
                (K::ERROR_TOKEN, "'abc\ny = 1"),
            ]
        );
        assert_eq!(texts("'''"), [(K::STRING, "''"), (K::ERROR_TOKEN, "'")]);
    }

    #[test]
    fn dangling_backslash_at_eof_rewinds_the_escape() {
        // logos rewinds an escape left incomplete by EOF: the error stops
        // before the backslash and the backslash errors on its own.
        assert_eq!(
            texts(r"'a\"),
            [(K::ERROR_TOKEN, "'a"), (K::ERROR_TOKEN, r"\")]
        );
        assert_eq!(
            texts(r"'\"),
            [(K::ERROR_TOKEN, "'"), (K::ERROR_TOKEN, r"\")]
        );
        // A *completed* escape before the dangling one stays in the error.
        assert_eq!(
            texts(r"'ab\\cd\"),
            [(K::ERROR_TOKEN, r"'ab\\cd"), (K::ERROR_TOKEN, r"\")]
        );
        // ...and an escape completed by a non-EOF byte keeps error-to-EOF.
        assert_eq!(texts("'a\\é"), [(K::ERROR_TOKEN, "'a\\é")]);
    }

    #[test]
    fn non_ascii_is_one_error_per_char() {
        assert_eq!(
            texts("héllo"),
            [(K::IDENT, "h"), (K::ERROR_TOKEN, "é"), (K::IDENT, "llo")]
        );
        assert_eq!(
            texts("日本"),
            [(K::ERROR_TOKEN, "日"), (K::ERROR_TOKEN, "本")]
        );
    }

    #[test]
    fn whitespace_and_newlines() {
        assert_eq!(texts(""), [] as [(K, &str); 0]);
        // Maximal whitespace runs; every `\n` is its own NEWLINE.
        assert_eq!(
            texts(" \t\r\x0C\n\n a"),
            [
                (K::WHITESPACE, " \t\r\x0C"),
                (K::NEWLINE, "\n"),
                (K::NEWLINE, "\n"),
                (K::WHITESPACE, " "),
                (K::IDENT, "a"),
            ]
        );
        // Comments run to end of line, exclusive; `//` at EOF is fine.
        assert_eq!(texts("//"), [(K::COMMENT, "//")]);
        assert_eq!(
            texts("// a $ b\nc"),
            [
                (K::COMMENT, "// a $ b"),
                (K::NEWLINE, "\n"),
                (K::IDENT, "c")
            ]
        );
    }

    // ===== Lossless property =====

    #[test]
    fn lossless_on_tricky_sources() {
        let sources = [
            "",
            "println('Hello World!')",
            "1. 1e 1e+ 0x 0xFF 1.5e-10 .. ... & | 'abc",
            "'a\\'b' 'a\\\nb' // comment\r\nx $ y\n",
            "héllo 日本 \u{0C}\t\r",
            "x = 'unterminated\nrest of file",
            r"'trailing backslash\",
        ];
        for src in sources {
            assert_lossless(src);
        }
    }

    fn assert_lossless(src: &str) {
        let tokens = lex(src);
        let total: u32 = tokens.iter().map(|t| t.len).sum();
        assert_eq!(total, src.len() as u32, "lens must sum to the input length");
        assert!(tokens.iter().all(|t| t.len > 0), "tokens must be non-empty");
        let rebuilt: String = lex_with_text(src).map(|(_, text)| text).collect();
        assert_eq!(rebuilt, src, "token texts must reconstruct the source");
    }

    // ===== Differential corpus test against the logos lexer =====
    //
    // This is the load-bearing gate of the migration wave: for every corpus
    // file, the old logos lexer and `lex` must agree token-for-token,
    // span-for-span (after dropping WHITESPACE, which logos skips). It gets
    // deleted in wave 6 together with `crate::lexer`.

    /// Maps an old logos token (or error) to the new [`K`]. 1:1 by design;
    /// logos's failure spans were verified (empirically, including
    /// differential fuzzing) to match ours exactly: one error per unlexable
    /// char; a single error from an unterminated string's opening quote to
    /// EOF (the string pattern can only fail at EOF); and, when the input
    /// ends in a dangling `\`, the incomplete escape rewound so that the
    /// backslash errors separately.
    fn map_old_token(tok: Result<crate::lexer::Token<'_>, ()>) -> K {
        use crate::lexer::Token as T;
        match tok {
            Err(()) => K::ERROR_TOKEN,
            Ok(T::Break) => K::NEWLINE,
            Ok(T::Comment(_)) => K::COMMENT,
            Ok(T::Number(_)) => K::NUMBER,
            Ok(T::String(_)) => K::STRING,
            Ok(T::Ident(_)) => K::IDENT,
            Ok(T::OpenParen) => K::OPEN_PAREN,
            Ok(T::CloseParen) => K::CLOSE_PAREN,
            Ok(T::OpenBrack) => K::OPEN_BRACK,
            Ok(T::CloseBrack) => K::CLOSE_BRACK,
            Ok(T::OpenBrace) => K::OPEN_BRACE,
            Ok(T::CloseBrace) => K::CLOSE_BRACE,
            Ok(T::EqEq) => K::EQ_EQ,
            Ok(T::BangEq) => K::BANG_EQ,
            Ok(T::Lt) => K::LT,
            Ok(T::LtEq) => K::LT_EQ,
            Ok(T::Gt) => K::GT,
            Ok(T::GtEq) => K::GT_EQ,
            Ok(T::And) => K::AND_AND,
            Ok(T::Or) => K::OR_OR,
            Ok(T::Bang) => K::BANG,
            Ok(T::Add) => K::PLUS,
            Ok(T::Sub) => K::MINUS,
            Ok(T::Mul) => K::STAR,
            Ok(T::Div) => K::SLASH,
            Ok(T::Rem) => K::PERCENT,
            Ok(T::Pow) => K::CARET,
            Ok(T::Assign) => K::EQ,
            Ok(T::AddAssign) => K::PLUS_EQ,
            Ok(T::Swap) => K::SWAP,
            Ok(T::Tilde) => K::TILDE,
            Ok(T::Colon) => K::COLON,
            Ok(T::Dot) => K::DOT,
            Ok(T::Ellipsis) => K::ELLIPSIS,
            Ok(T::Comma) => K::COMMA,
            Ok(T::SemiColon) => K::SEMICOLON,
            Ok(T::Arrow) => K::ARROW,
        }
    }

    /// Asserts that both lexers produce the identical `(kind, span)` stream
    /// for `src`, and that the new stream is lossless.
    fn assert_lexers_agree(src: &str, file: &str) {
        use logos::Logos;

        let mut new_stream = Vec::new();
        let mut offset = 0usize;
        for LexedToken { kind, len } in lex(src) {
            let start = offset;
            offset += len as usize;
            assert!(len > 0, "{file}: empty token at {start}");
            if kind != K::WHITESPACE {
                new_stream.push((kind, start..offset));
            }
        }
        assert_eq!(offset, src.len(), "{file}: lossless invariant violated");

        let old_stream: Vec<_> = crate::lexer::Token::lexer(src)
            .spanned()
            .map(|(tok, span)| (map_old_token(tok), span))
            .collect();

        if new_stream != old_stream {
            let idx = new_stream
                .iter()
                .zip(&old_stream)
                .position(|(new, old)| new != old)
                .unwrap_or_else(|| new_stream.len().min(old_stream.len()));
            // `get` (not indexing): a diverging span could fall on a
            // non-char boundary, and a slice panic here would mask the
            // actual diagnostic.
            let text_at = |stream: &[(K, std::ops::Range<usize>)]| {
                stream.get(idx).and_then(|(_, s)| src.get(s.clone()))
            };
            panic!(
                "{file}: lexers diverge at token {idx}:\n  new: {:?} {:?}\n  old: {:?} {:?}\n  \
                 (new stream has {} tokens, old has {})",
                new_stream.get(idx),
                text_at(&new_stream),
                old_stream.get(idx),
                text_at(&old_stream),
                new_stream.len(),
                old_stream.len(),
            );
        }
    }

    #[test]
    fn differential_corpus() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("example");
        assert!(
            root.is_dir(),
            "corpus missing at {} — run `git submodule update --init --recursive`",
            root.display()
        );

        fn walk_sc(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk_sc(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("sc") {
                    out.push(path);
                }
            }
        }

        let mut files = Vec::new();
        walk_sc(&root, &mut files);
        files.sort();
        assert!(
            files.len() >= 100,
            "corpus looks incomplete ({} .sc files found) — run `git submodule update --init \
             --recursive`",
            files.len()
        );

        for file in &files {
            let rel = file
                .strip_prefix(&root)
                .unwrap_or(file)
                .display()
                .to_string();
            // All corpus files are UTF-8 today; if one ever isn't, neither
            // lexer can take it (`&str` input), so failing loudly is right.
            let src = std::fs::read_to_string(file)
                .unwrap_or_else(|e| panic!("{rel}: failed to read: {e}"));
            assert_lexers_agree(&src, &rel);
        }
    }
}
