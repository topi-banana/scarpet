//! The `rowan` bridge: every token and node kind of the Scarpet syntax tree,
//! and the [`rowan::Language`] implementation tying them to the green tree.
//!
//! Node kinds mirror `scarpet.ungram` (the grammar's source of truth); the
//! sourcegen test in `tests/sourcegen.rs` keeps the typed layer in
//! [`crate::nodes`] in sync with it.

/// Declares [`SyntaxKind`] once and derives the raw-kind round-trip table from
/// the same list, so `kind_from_raw` cannot drift from the enum.
macro_rules! define_syntax_kinds {
    ($($(#[$attr:meta])* $name:ident,)*) => {
        /// Every token and node kind in a Scarpet syntax tree.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        #[allow(non_camel_case_types)]
        pub enum SyntaxKind {
            $($(#[$attr])* $name,)*
        }

        impl SyntaxKind {
            const ALL: &'static [SyntaxKind] = &[$(SyntaxKind::$name,)*];
        }
    };
}

define_syntax_kinds! {
    // ===== Tokens: trivia =====
    /// Horizontal whitespace (spaces, tabs, `\r`, form feeds).
    WHITESPACE,
    /// A newline, or `$` (Scarpet's newline stand-in in one-liners).
    BREAK,
    /// A `//` line comment, up to (excluding) the line end.
    COMMENT,

    // ===== Tokens: literals / atoms =====
    NUMBER,
    STRING,
    IDENT,
    /// Contextual `l` introducing the parenthesized spelling of a list literal.
    L_KW,
    /// Contextual `m` introducing the parenthesized spelling of a map literal.
    M_KW,

    // ===== Tokens: delimiters =====
    L_PAREN,
    R_PAREN,
    L_BRACK,
    R_BRACK,
    L_BRACE,
    R_BRACE,

    // ===== Tokens: operators =====
    /// `==`
    EQ2,
    /// `!=`
    BANG_EQ,
    /// `<`
    LT,
    /// `<=`
    LT_EQ,
    /// `>`
    GT,
    /// `>=`
    GT_EQ,
    /// `&&`
    AMP2,
    /// `||`
    PIPE2,
    /// `!`
    BANG,
    /// `+`
    PLUS,
    /// `-`
    MINUS,
    /// `*`
    STAR,
    /// `/`
    SLASH,
    /// `%`
    PERCENT,
    /// `^`
    CARET,
    /// `=`
    EQ,
    /// `+=`
    PLUS_EQ,
    /// `<>`
    LT_GT,
    /// `~`
    TILDE,
    /// `:`
    COLON,
    /// `.`
    DOT,
    /// `...`
    DOT3,
    /// `,`
    COMMA,
    /// `;`
    SEMICOLON,
    /// `->`
    ARROW,
    /// A character sequence the lexer cannot type (a stray `@`, a lone `&`,
    /// an unterminated string, …).
    ERROR,

    // ===== Nodes =====
    /// A number or string literal.
    LITERAL,
    /// A bare identifier reference (also the callee of a `CALL_EXPR`).
    NAME_REF,
    /// The parenthesized argument list of a call.
    ARG_LIST,
    /// `name(args)`.
    CALL_EXPR,
    /// `[ ... ]`.
    LIST_EXPR,
    /// `{ ... }`.
    MAP_EXPR,
    /// `( ... )`.
    PAREN_EXPR,
    /// A prefix operator application (`-x`, `+x`, `!x`, `...x`).
    PREFIX_EXPR,
    /// A binary operator application; `;` and `,` chains are binary too.
    BIN_EXPR,
    /// The top-level node covering the whole source.
    ROOT,
}

impl SyntaxKind {
    /// True for the token kinds the grammar skips over: whitespace, breaks,
    /// and comments. Breaks and comments still matter to the [`crate::cst`]
    /// lowering, which re-attaches them as leading trivia.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::WHITESPACE | SyntaxKind::BREAK | SyntaxKind::COMMENT
        )
    }

    /// True for the opening delimiters `(`, `[`, `{`.
    pub fn is_opener(self) -> bool {
        matches!(
            self,
            SyntaxKind::L_PAREN | SyntaxKind::L_BRACK | SyntaxKind::L_BRACE
        )
    }

    /// True for the closing delimiters `)`, `]`, `}`.
    pub fn is_closer(self) -> bool {
        matches!(
            self,
            SyntaxKind::R_PAREN | SyntaxKind::R_BRACK | SyntaxKind::R_BRACE
        )
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        rowan::SyntaxKind(kind as u16)
    }
}

/// The Scarpet language marker for `rowan`'s typed API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScarpetLanguage {}

impl rowan::Language for ScarpetLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        SyntaxKind::ALL[raw.0 as usize]
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

pub type SyntaxNode = rowan::SyntaxNode<ScarpetLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<ScarpetLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<ScarpetLanguage>;
pub type SyntaxNodeChildren = rowan::SyntaxNodeChildren<ScarpetLanguage>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_kind_round_trips() {
        use rowan::Language as _;
        for &kind in SyntaxKind::ALL {
            assert_eq!(
                ScarpetLanguage::kind_from_raw(ScarpetLanguage::kind_to_raw(kind)),
                kind
            );
        }
    }
}
