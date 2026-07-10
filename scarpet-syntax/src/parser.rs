//! Hand-written recursive-descent parser producing a lossless `rowan` syntax
//! tree, plus the legacy [`Cst`] view lowered from it (see [`crate::cst`]).
//!
//! The grammar's node shapes are documented in `scarpet.ungram`; the typed
//! accessors over the tree live in [`crate::nodes`].

use crate::lexer::{self, Token};
use crate::syntax::{SyntaxKind, SyntaxNode};
use rowan::{Checkpoint, GreenNode, GreenNodeBuilder};

pub use crate::cst::{BinOp, Cst, CstKind, Trivia, UnaryOp, strip_trivia};

// ====================================================================
// Front-end
// ====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte range the caret points at — the offending token, an unbalanced
    /// delimiter, or `len..len` at end of input.
    pub span: std::ops::Range<usize>,
    /// What the parser expected at `span`, as display-ready labels — concrete
    /// tokens carry back-ticks (`` `,` ``), higher-level patterns read as prose
    /// (`expression`, `end of input`). De-duplicated; empty for delimiter
    /// errors and when nothing specific was on offer.
    pub expected: Vec<String>,
    /// Source text of the offending token, or `None` at end of input.
    pub found: Option<String>,
    /// A fixed headline for structural delimiter errors (`unclosed delimiter`,
    /// `mismatched closing delimiter`, `unmatched closing delimiter`) that
    /// replaces the derived `expected …, found …`. `None` for token mismatches.
    pub headline: Option<String>,
    /// An optional secondary span and label — the opening delimiter a delimiter
    /// error refers back to (e.g. `` unclosed `[` ``).
    pub secondary: Option<(std::ops::Range<usize>, String)>,
    /// A `help:` suggestion, such as a missing comma between list elements.
    pub help: Option<String>,
}

impl ParseError {
    /// The `expected …` clause on its own (no `found`), or `None` when the
    /// parser had nothing specific to expect. Reads as ``expected `,` ``,
    /// ``expected `(` or `[` ``, or ``expected one of `,`, `;`, or `]` ``.
    pub fn expected_phrase(&self) -> Option<String> {
        use std::fmt::Write as _;
        let mut m = String::new();
        match self.expected.as_slice() {
            [] => return None,
            [a] => {
                let _ = write!(m, "expected {a}");
            }
            [a, b] => {
                let _ = write!(m, "expected {a} or {b}");
            }
            [rest @ .., last] => {
                m.push_str("expected one of ");
                for e in rest {
                    let _ = write!(m, "{e}, ");
                }
                let _ = write!(m, "or {last}");
            }
        }
        Some(m)
    }

    /// The label printed at the caret: a delimiter error's fixed `headline`,
    /// otherwise the `expected …` clause (the `found` token already sits there).
    pub fn caret_label(&self) -> String {
        if let Some(h) = &self.headline {
            return h.clone();
        }
        self.expected_phrase()
            .unwrap_or_else(|| "unexpected token".to_string())
    }

    /// A one-line, rustc-style summary for the report title and `FmtError`'s
    /// `Display`: a delimiter `headline` verbatim, else ``expected …, found …``,
    /// else `unexpected token`.
    pub fn message(&self) -> String {
        use std::fmt::Write as _;
        if let Some(h) = &self.headline {
            return h.clone();
        }
        let Some(mut m) = self.expected_phrase() else {
            return "unexpected token".to_string();
        };
        match &self.found {
            Some(found) => {
                let _ = write!(m, ", found `{found}`");
            }
            None => m.push_str(", found end of input"),
        }
        m
    }
}

/// Parse `src` to the legacy [`Cst`] (the shape the formatter and the AST
/// lowering consume). Convenience over [`parse`] + [`crate::cst::from_root`].
pub fn parse_source(src: &str) -> Result<Cst<'_>, Box<ParseError>> {
    let root = parse(src)?;
    Ok(crate::cst::from_root(src, &root))
}

/// Parse `src` to the lossless `rowan` syntax tree. The tree's text is the
/// source, byte for byte — nothing (comments, whitespace, `$` breaks) is
/// dropped.
pub fn parse(src: &str) -> Result<SyntaxNode, Box<ParseError>> {
    // Lex once and share the token stream: the delimiter pre-pass scans it, then
    // the parser consumes the same `Vec` (no second lexer pass, no throwaway
    // allocation).
    let tokens = lexer::lex(src);
    // A delimiter pre-pass catches unbalanced brackets with a structural message
    // (and a pointer back to the opener) that's clearer than whatever token
    // mismatch the grammar would otherwise trip over first.
    if let Some(err) = check_delimiters(&tokens) {
        return Err(Box::new(err));
    }
    let parser = Parser {
        src,
        tokens,
        pos: 0,
        builder: GreenNodeBuilder::new(),
    };
    // Boxed errors keep the hot `Ok` arm of every `Result` cheap — the error
    // path is cold and `ParseError` is large (clippy result_large_err).
    Ok(SyntaxNode::new_root(parser.parse_root()?))
}

// --- delimiter pre-pass ---------------------------------------------

/// Scan the token stream for the first unbalanced delimiter — a stray closer, a
/// wrong closer, or an opener that never closes — and describe it structurally.
/// Returns `None` when every bracket balances. Scans the already-lexed token
/// stream, so brackets inside strings and comments (which lex as single tokens)
/// are ignored.
fn check_delimiters(tokens: &[Token]) -> Option<ParseError> {
    let mut stack: Vec<(SyntaxKind, std::ops::Range<usize>)> = Vec::new();
    for tok in tokens {
        match tok.kind {
            k if k.is_opener() => {
                stack.push((tok.kind, tok.range()));
            }
            k if k.is_closer() => {
                match stack.pop() {
                    // A closer with no opener waiting for it.
                    None => {
                        return Some(ParseError {
                            span: tok.range(),
                            expected: Vec::new(),
                            found: Some(tok.text.to_string()),
                            headline: Some("unmatched closing delimiter".to_string()),
                            secondary: None,
                            help: None,
                        });
                    }
                    // The wrong closer for the opener on top of the stack.
                    Some((open, open_span)) if closer_for(open) != tok.kind => {
                        return Some(ParseError {
                            span: tok.range(),
                            expected: vec![delim_label(closer_for(open)).to_string()],
                            found: Some(tok.text.to_string()),
                            headline: Some("mismatched closing delimiter".to_string()),
                            secondary: Some((open_span, format!("unclosed {}", delim_label(open)))),
                            help: None,
                        });
                    }
                    // A matching pair — keep going.
                    Some(_) => {}
                }
            }
            _ => {}
        }
    }
    // Anything left on the stack never closed; report the outermost opener.
    if !stack.is_empty() {
        let (open, open_span) = stack.remove(0);
        return Some(ParseError {
            span: open_span,
            expected: vec![delim_label(closer_for(open)).to_string()],
            found: None,
            headline: Some("unclosed delimiter".to_string()),
            secondary: None,
            help: None,
        });
    }
    None
}

/// Whether `src` has at least one delimiter still open — a `(`, `[`, or `{` with
/// no matching closer yet. Unlike [`check_delimiters`], a *surplus* or
/// *mismatched* closer is not reported as open: those are genuine errors for the
/// parser to report, not a reason to wait for more input. Like `check_delimiters`
/// it runs straight on the lexer, so delimiters inside strings and comments
/// (which lex as single tokens) are ignored.
///
/// The REPL uses this to decide whether to hold a multi-line submission open
/// until its brackets balance.
pub fn has_open_delimiter(src: &str) -> bool {
    let mut depth: usize = 0;
    for tok in lexer::lex(src) {
        match tok.kind {
            k if k.is_opener() => {
                depth += 1;
            }
            k if k.is_closer() => {
                // Saturate at zero so a surplus closer never reads as "open".
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    depth > 0
}

/// The closing delimiter kind that matches an opener (identity for non-openers).
fn closer_for(open: SyntaxKind) -> SyntaxKind {
    match open {
        SyntaxKind::L_PAREN => SyntaxKind::R_PAREN,
        SyntaxKind::L_BRACK => SyntaxKind::R_BRACK,
        SyntaxKind::L_BRACE => SyntaxKind::R_BRACE,
        other => other,
    }
}

/// Guess a dropped comma: inside a list / call / map a forgotten `,` surfaces as
/// an expression token sitting where a closer (and only operators besides) was
/// expected — e.g. `[1 2]`. Returns the `help:` text, or `None`.
fn missing_comma_help(expected: &[String], found: Option<SyntaxKind>) -> Option<String> {
    let closer_expected = expected
        .iter()
        .any(|e| e == "`]`" || e == "`)`" || e == "`}`");
    if !closer_expected {
        return None;
    }
    let found_begins_expr = found.is_some_and(begins_expr);
    found_begins_expr.then(|| "missing `,`".to_string())
}

/// Whether a token can begin an expression — the set whose appearance where a
/// closer was due signals a dropped separator.
fn begins_expr(k: SyntaxKind) -> bool {
    matches!(
        k,
        SyntaxKind::NUMBER
            | SyntaxKind::STRING
            | SyntaxKind::IDENT
            | SyntaxKind::L_PAREN
            | SyntaxKind::L_BRACK
            | SyntaxKind::L_BRACE
            | SyntaxKind::MINUS
            | SyntaxKind::PLUS
            | SyntaxKind::BANG
            | SyntaxKind::DOT3
    )
}

/// The literal label for a bracket kind in `expected …` / `unclosed …`
/// messages (`` `(` ``, `` `]` ``). Only ever called with a delimiter — an
/// opener or its matching closer.
fn delim_label(k: SyntaxKind) -> &'static str {
    match k {
        SyntaxKind::L_PAREN => "`(`",
        SyntaxKind::R_PAREN => "`)`",
        SyntaxKind::L_BRACK => "`[`",
        SyntaxKind::R_BRACK => "`]`",
        SyntaxKind::L_BRACE => "`{`",
        SyntaxKind::R_BRACE => "`}`",
        k => unreachable!("delimiter label: {k:?}"),
    }
}

// ====================================================================
// Recursive-descent parser
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
//   primary     = atom | `(` top `)` | list_literal | map_literal | ident `(` arg_list `)`
//   list_literal = `[` arg_list `]` | `l(` arg_list `)`
//   map_literal  = `{` arg_list `}` | `m(` arg_list `)`
//
// Every level lands in a `BIN_EXPR` node (or `PREFIX_EXPR` for the unary
// prefixes); left-associative levels wrap repeatedly at the same checkpoint,
// right-associative ones recurse before wrapping. Trivia tokens are emitted
// into the green tree at the position they occupy in the source — checkpoints
// are always taken *after* flushing pending trivia, so a band before an
// expression stays outside the expression's node, and a band before an
// operator lands inside the operator's `BIN_EXPR`. The `Cst` lowering relies
// on those positions to reproduce the old trivia attachment.

type PResult = Result<(), Box<ParseError>>;

/// An identifier which becomes a literal introducer when immediately followed
/// by `(`. The lexer deliberately leaves these as `IDENT`, preserving their
/// use as ordinary names in every other context.
#[derive(Clone, Copy)]
enum LiteralConstructor {
    List,
    Map,
}

impl LiteralConstructor {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "l" => Some(Self::List),
            "m" => Some(Self::Map),
            _ => None,
        }
    }

    fn token_kind(self) -> SyntaxKind {
        match self {
            Self::List => SyntaxKind::L_KW,
            Self::Map => SyntaxKind::M_KW,
        }
    }

    fn node_kind(self) -> SyntaxKind {
        match self {
            Self::List => SyntaxKind::LIST_EXPR,
            Self::Map => SyntaxKind::MAP_EXPR,
        }
    }
}

struct Parser<'s> {
    src: &'s str,
    tokens: Vec<Token<'s>>,
    /// Index of the next token (trivia included) not yet emitted into the
    /// builder.
    pos: usize,
    builder: GreenNodeBuilder<'static>,
}

impl<'s> Parser<'s> {
    // --- token access -------------------------------------------------

    /// The next semantic (non-trivia) token, without consuming anything.
    fn peek(&self) -> Option<&Token<'s>> {
        self.peek_nth(0)
    }

    fn peek_kind(&self) -> Option<SyntaxKind> {
        self.peek().map(|t| t.kind)
    }

    /// The `n`th semantic (non-trivia) token ahead (`0` is [`peek`](Self::peek)),
    /// without consuming anything.
    fn peek_nth(&self, n: usize) -> Option<&Token<'s>> {
        self.tokens[self.pos..]
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .nth(n)
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.peek_kind() == Some(kind)
    }

    /// Recognize a contextual literal constructor without reserving its name
    /// in the lexer. Whitespace and comments between the name and `(` are
    /// intentionally allowed, as they are for ordinary calls.
    fn peek_literal_constructor(&self) -> Option<LiteralConstructor> {
        let constructor = LiteralConstructor::from_name(self.peek()?.text)?;
        self.peek_nth(1)
            .is_some_and(|t| t.kind == SyntaxKind::L_PAREN)
            .then_some(constructor)
    }

    fn at_closer_or_eof(&self) -> bool {
        self.peek_kind().is_none_or(SyntaxKind::is_closer)
    }

    /// Emit pending trivia tokens into the builder, up to the next semantic
    /// token (or end of input).
    fn flush_trivia(&mut self) {
        while self
            .tokens
            .get(self.pos)
            .is_some_and(|t| t.kind.is_trivia())
        {
            let t = &self.tokens[self.pos];
            self.builder.token(t.kind.into(), t.text);
            self.pos += 1;
        }
    }

    /// Consume the next semantic token (flushing trivia before it).
    fn bump(&mut self) {
        self.flush_trivia();
        let t = &self.tokens[self.pos];
        debug_assert!(!t.kind.is_trivia());
        self.builder.token(t.kind.into(), t.text);
        self.pos += 1;
    }

    /// Consume the next semantic token while reclassifying it in the syntax
    /// tree. Used for contextual keywords whose lexical kind remains `IDENT`.
    fn bump_as(&mut self, kind: SyntaxKind) {
        self.flush_trivia();
        let t = &self.tokens[self.pos];
        debug_assert_eq!(t.kind, SyntaxKind::IDENT);
        self.builder.token(kind.into(), t.text);
        self.pos += 1;
    }

    /// A checkpoint for retroactive wrapping. Trivia is flushed first, so the
    /// band before the upcoming expression stays *outside* the wrapped node.
    fn checkpoint(&mut self) -> Checkpoint {
        self.flush_trivia();
        self.builder.checkpoint()
    }

    fn start_node(&mut self, kind: SyntaxKind) {
        self.flush_trivia();
        self.builder.start_node(kind.into());
    }

    fn finish_node(&mut self) {
        self.builder.finish_node();
    }

    /// Wrap everything parsed since `cp` into a `kind` node.
    fn wrap(&mut self, cp: Checkpoint, kind: SyntaxKind) {
        self.builder.start_node_at(cp, kind.into());
        self.builder.finish_node();
    }

    // --- errors ---------------------------------------------------------

    /// A [`ParseError`] at the next semantic token (or end of input), with the
    /// given expectation labels and, where it applies, the missing-comma help.
    fn err_here(&self, expected: Vec<String>) -> Box<ParseError> {
        let (span, found, found_kind) = match self.peek() {
            Some(t) => (t.range(), Some(t.text.to_string()), Some(t.kind)),
            None => (self.src.len()..self.src.len(), None, None),
        };
        let help = missing_comma_help(&expected, found_kind);
        Box::new(ParseError {
            span,
            expected,
            found,
            headline: None,
            secondary: None,
            help,
        })
    }

    fn expected_operator_or(&self, closer_label: &str) -> Box<ParseError> {
        self.err_here(vec!["an operator".to_string(), closer_label.to_string()])
    }

    // --- grammar ----------------------------------------------------------

    fn parse_root(mut self) -> Result<GreenNode, Box<ParseError>> {
        self.builder.start_node(SyntaxKind::ROOT.into());
        self.parse_top()?;
        // Anchor trailing trivia (e.g. a comment after the final expression)
        // inside the root so it isn't lost.
        self.flush_trivia();
        if self.peek().is_some() {
            // Anything the ladder could not consume. A stray closer cannot
            // appear here — the delimiter pre-pass rejected it already.
            return Err(self.err_here(vec!["an operator".to_string(), "end of input".to_string()]));
        }
        self.finish_node();
        Ok(self.builder.finish())
    }

    /// `comma_chain`, the grammar's `top`: the root, and paren bodies.
    fn parse_top(&mut self) -> PResult {
        let cp = self.checkpoint();
        self.parse_item()?;
        while self.at(SyntaxKind::COMMA) {
            self.bump();
            if self.at_closer_or_eof() {
                // Trailing `,` — tolerated; the CST lowering anchors the
                // surrounding trivia onto the chain.
                break;
            }
            self.parse_item()?;
            self.wrap(cp, SyntaxKind::BIN_EXPR);
        }
        Ok(())
    }

    /// `seq_chain`: `;`-separated statements — also each argument-list item.
    fn parse_item(&mut self) -> PResult {
        let cp = self.checkpoint();
        self.parse_arrow()?;
        while self.at(SyntaxKind::SEMICOLON) {
            self.bump();
            // Scarpet's preprocessor strips runs of `;`; treat the run as one
            // separator.
            while self.at(SyntaxKind::SEMICOLON) {
                self.bump();
            }
            if self.at_closer_or_eof() || self.at(SyntaxKind::COMMA) {
                // Trailing `;` — tolerated.
                break;
            }
            self.parse_arrow()?;
            self.wrap(cp, SyntaxKind::BIN_EXPR);
        }
        Ok(())
    }

    /// `arrow_chain` (`->`; right-associative).
    fn parse_arrow(&mut self) -> PResult {
        self.parse_right_assoc(&[SyntaxKind::ARROW], Self::parse_assign)
    }

    /// `assign` (`=`, `+=`, `<>`; right-associative).
    fn parse_assign(&mut self) -> PResult {
        self.parse_right_assoc(
            &[SyntaxKind::EQ, SyntaxKind::PLUS_EQ, SyntaxKind::LT_GT],
            Self::parse_lor,
        )
    }

    /// One left-associative binary level: `next (op next)*`.
    fn parse_left_assoc(&mut self, ops: &[SyntaxKind], next: fn(&mut Self) -> PResult) -> PResult {
        let cp = self.checkpoint();
        next(self)?;
        while self.peek_kind().is_some_and(|k| ops.contains(&k)) {
            self.bump();
            next(self)?;
            self.wrap(cp, SyntaxKind::BIN_EXPR);
        }
        Ok(())
    }

    /// One right-associative binary level: `next (op right)?`, where `right`
    /// recurses at this same level (so `a = b = c` nests to the right). The
    /// twin of [`Self::parse_left_assoc`].
    fn parse_right_assoc(&mut self, ops: &[SyntaxKind], next: fn(&mut Self) -> PResult) -> PResult {
        let cp = self.checkpoint();
        next(self)?;
        if self.peek_kind().is_some_and(|k| ops.contains(&k)) {
            self.bump();
            self.parse_right_assoc(ops, next)?;
            self.wrap(cp, SyntaxKind::BIN_EXPR);
        }
        Ok(())
    }

    fn parse_lor(&mut self) -> PResult {
        self.parse_left_assoc(&[SyntaxKind::PIPE2], Self::parse_land)
    }

    fn parse_land(&mut self) -> PResult {
        self.parse_left_assoc(&[SyntaxKind::AMP2], Self::parse_equality)
    }

    fn parse_equality(&mut self) -> PResult {
        self.parse_left_assoc(&[SyntaxKind::EQ2, SyntaxKind::BANG_EQ], Self::parse_compare)
    }

    fn parse_compare(&mut self) -> PResult {
        self.parse_left_assoc(
            &[
                SyntaxKind::LT_EQ,
                SyntaxKind::GT_EQ,
                SyntaxKind::LT,
                SyntaxKind::GT,
            ],
            Self::parse_additive,
        )
    }

    fn parse_additive(&mut self) -> PResult {
        self.parse_left_assoc(&[SyntaxKind::PLUS, SyntaxKind::MINUS], Self::parse_mult)
    }

    fn parse_mult(&mut self) -> PResult {
        self.parse_left_assoc(
            &[SyntaxKind::STAR, SyntaxKind::SLASH, SyntaxKind::PERCENT],
            Self::parse_power,
        )
    }

    /// `power` (`^`; right-associative).
    fn parse_power(&mut self) -> PResult {
        self.parse_right_assoc(&[SyntaxKind::CARET], Self::parse_unary)
    }

    /// `unary`: stackable prefixes (`-`, `+`, `!`, `...`).
    fn parse_unary(&mut self) -> PResult {
        if matches!(
            self.peek_kind(),
            Some(SyntaxKind::MINUS | SyntaxKind::PLUS | SyntaxKind::BANG | SyntaxKind::DOT3)
        ) {
            self.start_node(SyntaxKind::PREFIX_EXPR);
            self.bump();
            self.parse_unary()?;
            self.finish_node();
            Ok(())
        } else {
            self.parse_get()
        }
    }

    /// `get` (`~`, `:`; left-associative; the RHS is always a primary).
    fn parse_get(&mut self) -> PResult {
        self.parse_left_assoc(&[SyntaxKind::TILDE, SyntaxKind::COLON], Self::parse_primary)
    }

    fn parse_primary(&mut self) -> PResult {
        match self.peek_kind() {
            Some(SyntaxKind::NUMBER | SyntaxKind::STRING) => {
                self.start_node(SyntaxKind::LITERAL);
                self.bump();
                self.finish_node();
                Ok(())
            }
            Some(SyntaxKind::IDENT) => {
                if let Some(constructor) = self.peek_literal_constructor() {
                    self.start_node(constructor.node_kind());
                    self.bump_as(constructor.token_kind());
                    self.parse_delimited_tail(SyntaxKind::R_PAREN)?;
                    self.finish_node();
                    return Ok(());
                }
                let cp = self.checkpoint();
                self.start_node(SyntaxKind::NAME_REF);
                self.bump();
                self.finish_node();
                if self.at(SyntaxKind::L_PAREN) {
                    self.builder.start_node_at(cp, SyntaxKind::CALL_EXPR.into());
                    self.parse_delimited(SyntaxKind::ARG_LIST, SyntaxKind::R_PAREN)?;
                    self.finish_node();
                }
                Ok(())
            }
            Some(SyntaxKind::L_PAREN) => {
                self.start_node(SyntaxKind::PAREN_EXPR);
                self.bump();
                self.parse_top()?;
                if !self.at(SyntaxKind::R_PAREN) {
                    return Err(self.expected_operator_or("`)`"));
                }
                self.bump();
                self.finish_node();
                Ok(())
            }
            Some(SyntaxKind::L_BRACK) => {
                self.parse_delimited(SyntaxKind::LIST_EXPR, SyntaxKind::R_BRACK)
            }
            Some(SyntaxKind::L_BRACE) => {
                self.parse_delimited(SyntaxKind::MAP_EXPR, SyntaxKind::R_BRACE)
            }
            _ => Err(self.err_here(vec!["expression".to_string()])),
        }
    }

    /// A bracketed argument list: a call's `( … )`, a list's `[ … ]`, a map's
    /// `{ … }`. The opener is the next token.
    fn parse_delimited(&mut self, node: SyntaxKind, closer: SyntaxKind) -> PResult {
        self.start_node(node);
        self.parse_delimited_tail(closer)?;
        self.finish_node();
        Ok(())
    }

    /// The `( … )` / `[ … ]` / `{ … }` core of [`parse_delimited`]: consumes
    /// from the opener (the next token) through the matching closer into the
    /// current node.
    fn parse_delimited_tail(&mut self, closer: SyntaxKind) -> PResult {
        self.bump();
        self.parse_args(closer)?;
        if !self.at(closer) {
            // Unreachable in practice: the pre-pass balanced every delimiter
            // and `parse_args` stops only at a closer. Kept for robustness.
            return Err(self.err_here(vec![delim_label(closer).to_string()]));
        }
        self.bump();
        Ok(())
    }

    /// The comma-separated items between delimiters, tolerating an empty list,
    /// omitted entries (`f(a, , b)` — the CST lowering synthesizes the phantom
    /// `Empty`), and a trailing comma.
    fn parse_args(&mut self, closer: SyntaxKind) -> PResult {
        loop {
            if self.at_closer_or_eof() {
                return Ok(());
            }
            if !self.at(SyntaxKind::COMMA) {
                // A real item; a `,` here is an omitted entry, represented by
                // nothing at all between the separators.
                self.parse_item()?;
            }
            if self.at(SyntaxKind::COMMA) {
                self.bump();
                continue;
            }
            if self.at_closer_or_eof() {
                return Ok(());
            }
            // Two items with nothing between them — e.g. `[1 2]`.
            return Err(self.expected_operator_or(delim_label(closer)));
        }
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
    fn bin<'s>(op: BinOp, lhs: Cst<'s>, rhs: Cst<'s>) -> Cst<'s> {
        Cst::bare(CstKind::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
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
            parse("l(1, 2, 3)"),
            list(vec![num("1"), num("2"), num("3")])
        );
        assert_eq!(
            parse("m('a' -> 1, 'b' -> 2)"),
            map(vec![
                bin(BinOp::Arrow, str_("'a'"), num("1")),
                bin(BinOp::Arrow, str_("'b'"), num("2")),
            ])
        );
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

    // ----- the rowan tree ------------------------------------------------

    /// The syntax tree is lossless: its text is the source, byte for byte.
    #[test]
    fn syntax_tree_is_lossless() {
        for src in [
            "foo(a, b) -> ( // c\n  a + b;\n)\r\n",
            "// lead\n[1, , 2,]; {'k' -> v}; -x ^ 2 $ + y",
            "f(// note\n)",
            "a; // x\n; b",
        ] {
            let tree = super::parse(src).expect("parse error");
            assert_eq!(tree.text().to_string(), src, "lossless for {src:?}");
        }
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
        // would otherwise be dropped — `parse_root` anchors it on the
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
        // The CST lowering synthesises a single `Empty` to hold the trivia.
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
        // The paren body's leading trivia becomes the leading of the first
        // inner atom.
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

    #[test]
    fn trailing_semicolon_trivia_anchors_on_the_statement() {
        // Trivia around a trailing `;` inside a paren appends onto the last
        // statement's leading rather than being dropped.
        let cst = parse("(a;\n// todo\n)");
        match &cst.kind {
            CstKind::Paren(inner) => {
                assert_eq!(inner.kind, CstKind::Ident("a"));
                assert_eq!(
                    inner.leading,
                    vec![Trivia::Break, Trivia::Comment("// todo"), Trivia::Break]
                );
            }
            other => panic!("expected Paren, got {other:?}"),
        }
    }

    // ----- token-mismatch errors (delimiters balanced) ------------------

    /// Where an operand is due but input ends, the message names the missing
    /// `expression` and reports EOF — not a bare "unexpected token".
    #[test]
    fn error_expected_expression_at_eof() {
        let e = parse_source("1 +").unwrap_err();
        assert_eq!(e.found, None);
        assert_eq!(e.expected, ["expression"]);
        assert_eq!(e.message(), "expected expression, found end of input");
    }

    /// The `found` span covers exactly the offending token, not the leading
    /// whitespace before it.
    #[test]
    fn error_found_span_is_the_token() {
        let src = "[0, 1 2]";
        let e = parse_source(src).unwrap_err();
        assert_eq!(&src[e.span.clone()], "2");
        assert_eq!(e.found.as_deref(), Some("2"));
        assert_eq!(e.message(), "expected an operator or `]`, found `2`");
    }

    /// The ~20 infix/prefix operators collapse to one `an operator` rather than
    /// being spelled out across the whole precedence ladder.
    #[test]
    fn error_operator_ladder_collapses() {
        let e = parse_source("[0 1]").unwrap_err();
        assert!(e.message().contains("an operator"), "{}", e.message());
        assert!(!e.message().contains("`*`"), "{}", e.message());
    }

    /// Exactly two expectations join with `or` (no `one of`).
    #[test]
    fn error_two_expectations_join_with_or() {
        let e = parse_source("1 2").unwrap_err();
        assert_eq!(
            e.message(),
            "expected an operator or end of input, found `2`"
        );
    }

    /// Every parse failure yields a non-empty, specific message.
    #[test]
    fn error_message_is_never_empty() {
        for src in ["", "1 +", "[)", "1 2", "{", "((", "f(,,", ")"] {
            if let Err(e) = parse_source(src) {
                assert!(!e.message().is_empty(), "empty message for {src:?}");
            }
        }
    }

    // ----- delimiter pre-pass errors ------------------------------------

    /// A wrong closer is flagged structurally: the caret sits on the closer and
    /// a secondary label points back at the still-open opener.
    #[test]
    fn error_mismatched_delimiter() {
        let src = "[)";
        let e = parse_source(src).unwrap_err();
        assert_eq!(e.message(), "mismatched closing delimiter");
        assert_eq!(&src[e.span.clone()], ")");
        let (opener, label) = e.secondary.clone().expect("opener label");
        assert_eq!(&src[opener], "[");
        assert_eq!(label, "unclosed `[`");
    }

    /// An opener that never closes points the caret at the opener itself.
    #[test]
    fn error_unclosed_delimiter() {
        let src = "foo(a";
        let e = parse_source(src).unwrap_err();
        assert_eq!(e.message(), "unclosed delimiter");
        assert_eq!(&src[e.span.clone()], "(");
    }

    /// A closer with no opener is its own kind of error.
    #[test]
    fn error_unmatched_closing_delimiter() {
        let e = parse_source("1)").unwrap_err();
        assert_eq!(e.message(), "unmatched closing delimiter");
        assert_eq!(e.found.as_deref(), Some(")"));
    }

    /// A string's contents never count as delimiters.
    #[test]
    fn delimiters_inside_strings_are_ignored() {
        assert!(parse_source("print('(')").is_ok());
    }

    /// `has_open_delimiter` is true only while an opener is still waiting for its
    /// closer — the REPL's signal to keep a multi-line submission open.
    #[test]
    fn has_open_delimiter_tracks_unclosed_openers() {
        // Still open: an opener with no matching closer yet.
        assert!(has_open_delimiter("foo("));
        assert!(has_open_delimiter("[1, 2,"));
        assert!(has_open_delimiter("foo() -> ("));
        assert!(has_open_delimiter("({["));
        // Balanced: nothing is waiting to close.
        assert!(!has_open_delimiter("foo()"));
        assert!(!has_open_delimiter("(a + b) * [c]"));
        assert!(!has_open_delimiter("a = 5"));
        // A surplus or mismatched closer is an error, not "open" — so it is not
        // held for more input; the parser reports it instead.
        assert!(!has_open_delimiter("a)"));
        assert!(!has_open_delimiter("(a]"));
        // Delimiters inside strings and comments never count.
        assert!(!has_open_delimiter("print('(')"));
        assert!(!has_open_delimiter("foo() // (open"));
        // But an opener before the comment still counts as open.
        assert!(has_open_delimiter("foo( // a comment"));
    }

    // ----- help suggestions ---------------------------------------------

    /// Two adjacent elements with no separator suggest the dropped comma.
    #[test]
    fn error_missing_comma_help() {
        let e = parse_source("[1 2]").unwrap_err();
        assert_eq!(e.help.as_deref(), Some("missing `,`"));
    }
}
