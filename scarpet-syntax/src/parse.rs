//! Hand-written recursive-descent parser producing a rowan green tree.
//!
//! This is the successor of the chumsky parser in [`crate::parser`]; both
//! coexist until the rowan migration completes (the old parser remains the
//! behavioral spec, and the differential tests in this module hold the two
//! to the same parse-success verdict on every corpus file and to
//! field-identical [`ParseError`]s on broken input). Like the old parser it
//! is fail-fast: the first violation of the grammar aborts the parse with a
//! [`ParseError`]; there is no error recovery.
//!
//! # Tree shape
//!
//! The tree is **lossless**: every token of [`crate::lex::lex`] — trivia
//! included — appears in the tree in source order, so
//! `parse.syntax().text() == src` always holds. The structure mirrors the
//! old CST with these mappings:
//!
//! - Binary operators build [`BIN_EXPR`](SyntaxKind::BIN_EXPR) nodes; prefix
//!   operators build [`PREFIX_EXPR`](SyntaxKind::PREFIX_EXPR) nodes.
//! - The chain separators `,` and `;` build *n-ary*
//!   [`COMMA_CHAIN`](SyntaxKind::COMMA_CHAIN) /
//!   [`SEMI_CHAIN`](SyntaxKind::SEMI_CHAIN) nodes (where the old CST nested
//!   `Binary{Comma/Semi}` pairs), with the separator tokens inside the chain
//!   node. A single expression without separators gets no chain wrapper, but
//!   a *trailing* separator does wrap: `a;` is a `SEMI_CHAIN` of one item
//!   plus its `;` token. (The old parser dropped a trailing `;` and returned
//!   the bare item; in a lossless tree the token must live somewhere, and
//!   keeping it inside the chain makes "this statement is `;`-terminated"
//!   structural.) Lenient `;;` runs stay inside a single `SEMI_CHAIN`.
//! - [`EMPTY_ARG`](SyntaxKind::EMPTY_ARG) is a *zero-width* node marking a
//!   genuinely omitted slot between separators (`f(a,,b)`, `f(,a)`). An
//!   empty argument list has none (`f()` — even with a comment inside, which
//!   is just a trivia token in the `ARG_LIST`), and a trailing comma adds
//!   none (`f(a,)` is one item plus a trailing `,` token). The old parser's
//!   trivia-anchoring phantom `Empty`s are obsolete: trivia lives in the
//!   tree now.
//!
//! # Trivia policy
//!
//! **No node starts with a trivia token.** Pending trivia is flushed into
//! the *currently open* node right before a child node starts or a
//! significant token is bumped. Consequences, reproducing the old "leading
//! trivia" semantics in source order:
//!
//! - trivia between an operand and the operator after it sits inside the
//!   `BIN_EXPR`, before the operator token (old rule: "trivia before an
//!   operator belongs to its RHS");
//! - trivia after a separator sits inside the chain node, before the next
//!   item; trivia after a *trailing* separator also stays inside the chain
//!   node (old rule: anchored onto the chain accumulator);
//! - trivia still pending at end of input lands directly in `SOURCE_FILE`
//!   (old rule: "anchored on root").

use rowan::ast::AstNode as _;
use rowan::{Checkpoint, GreenNode, GreenNodeBuilder, Language as _};

use crate::lex::{LexedToken, lex};
use crate::syntax::{ScarpetLanguage, SyntaxNode};
use crate::syntax_kind::SyntaxKind;

type PResult<T> = Result<T, Box<ParseError>>;

/// The result of a successful [`parse_source`]: an owned green tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Parse {
    green: GreenNode,
}

impl Parse {
    /// The red-tree root; always of kind [`SyntaxKind::SOURCE_FILE`].
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// The typed view of the root.
    pub fn source_file(&self) -> crate::cst::SourceFile {
        crate::cst::SourceFile::cast(self.syntax()).expect("the parse root is always SOURCE_FILE")
    }
}

/// A parse failure, mirroring `crate::parser::ParseError` field for field
/// (both types coexist until the migration completes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte range the caret points at — the offending token, an unbalanced
    /// delimiter, or `len..len` at end of input.
    pub span: std::ops::Range<usize>,
    /// What the parser expected at `span`, as display-ready labels — concrete
    /// tokens carry back-ticks (`` `,` ``), higher-level patterns read as prose
    /// (`expression`, `end of input`). De-duplicated, in the order they were
    /// recorded; empty for delimiter errors and when nothing specific was on
    /// offer.
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

// ====================================================================
// Entry points
// ====================================================================

// Precedence ladder (low → high), mirroring `crate::parser::top_parser`:
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
// `power`, `assign` and `arrow_chain` are right-associative (natural via
// recursion); the other binary levels are left-associative (loop +
// `start_node_at`). A trailing `;` is allowed (and `;;` runs are tolerated);
// `,` and `;` build n-ary chain nodes rather than nested binaries.

/// Parses `src` into a lossless rowan tree rooted at
/// [`SOURCE_FILE`](SyntaxKind::SOURCE_FILE).
pub fn parse_source(src: &str) -> Result<Parse, Box<ParseError>> {
    let tokens = tokenize(src);
    // A delimiter pre-pass catches unbalanced brackets with a structural
    // message (and a pointer back to the opener) that's clearer than whatever
    // token mismatch the grammar would otherwise trip over first.
    if let Some(err) = check_delimiters(src, &tokens) {
        return Err(Box::new(err));
    }
    let mut p = Parser {
        src,
        tokens,
        pos: 0,
        builder: GreenNodeBuilder::new(),
        expected: Vec::new(),
        expected_at: 0,
    };
    // The root starts before any token, so it must not go through
    // `Parser::start_node` (which would try to flush leading trivia into a
    // not-yet-open node).
    p.builder
        .start_node(ScarpetLanguage::kind_to_raw(SyntaxKind::SOURCE_FILE));
    comma_chain(&mut p)?;
    if p.peek().is_some() {
        return Err(p.fail("end of input"));
    }
    // Trailing trivia (e.g. a comment after the final expression) lands
    // directly in SOURCE_FILE so it isn't silently dropped.
    p.flush_trivia();
    p.builder.finish_node();
    Ok(Parse {
        green: p.builder.finish(),
    })
}

/// Whether `src` has at least one delimiter still open — a `(`, `[`, or `{`
/// with no matching closer yet. Unlike [`check_delimiters`], a *surplus* or
/// *mismatched* closer is not reported as open: those are genuine errors for
/// the parser to report, not a reason to wait for more input. It runs
/// straight on the lexer, so delimiters inside strings and comments (which
/// lex as single tokens) are ignored.
///
/// The REPL uses this to decide whether to hold a multi-line submission open
/// until its brackets balance.
pub fn has_open_delimiter(src: &str) -> bool {
    let mut depth: usize = 0;
    for tok in lex(src) {
        match tok.kind {
            SyntaxKind::OPEN_PAREN | SyntaxKind::OPEN_BRACK | SyntaxKind::OPEN_BRACE => {
                depth += 1;
            }
            SyntaxKind::CLOSE_PAREN | SyntaxKind::CLOSE_BRACK | SyntaxKind::CLOSE_BRACE => {
                // Saturate at zero so a surplus closer never reads as "open".
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    depth > 0
}

// ====================================================================
// Delimiter pre-pass
// ====================================================================

/// Scan the token stream for the first unbalanced delimiter — a stray closer,
/// a wrong closer, or an opener that never closes — and describe it
/// structurally. Returns `None` when every bracket balances. Delimiters
/// inside strings and comments are naturally ignored (they lex as single
/// tokens), as is unlexable input ([`ERROR_TOKEN`](SyntaxKind::ERROR_TOKEN)
/// — the grammar reports that instead).
fn check_delimiters(src: &str, tokens: &[Tok]) -> Option<ParseError> {
    let mut stack: Vec<(SyntaxKind, std::ops::Range<usize>)> = Vec::new();
    for tok in tokens {
        let span = tok.start..tok.end;
        match tok.kind {
            SyntaxKind::OPEN_PAREN | SyntaxKind::OPEN_BRACK | SyntaxKind::OPEN_BRACE => {
                stack.push((tok.kind, span));
            }
            SyntaxKind::CLOSE_PAREN | SyntaxKind::CLOSE_BRACK | SyntaxKind::CLOSE_BRACE => {
                match stack.pop() {
                    // A closer with no opener waiting for it.
                    None => {
                        return Some(ParseError {
                            span: span.clone(),
                            expected: Vec::new(),
                            found: Some(src[span].to_string()),
                            headline: Some("unmatched closing delimiter".to_string()),
                            secondary: None,
                            help: None,
                        });
                    }
                    // The wrong closer for the opener on top of the stack.
                    Some((open, open_span)) if closer_for(open) != tok.kind => {
                        return Some(ParseError {
                            span: span.clone(),
                            expected: vec![kind_label(closer_for(open)).to_string()],
                            found: Some(src[span].to_string()),
                            headline: Some("mismatched closing delimiter".to_string()),
                            secondary: Some((open_span, format!("unclosed {}", kind_label(open)))),
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
            expected: vec![kind_label(closer_for(open)).to_string()],
            found: None,
            headline: Some("unclosed delimiter".to_string()),
            secondary: None,
            help: None,
        });
    }
    None
}

/// The closing delimiter kind that matches an opener (identity for
/// non-openers).
fn closer_for(open: SyntaxKind) -> SyntaxKind {
    match open {
        SyntaxKind::OPEN_PAREN => SyntaxKind::CLOSE_PAREN,
        SyntaxKind::OPEN_BRACK => SyntaxKind::CLOSE_BRACK,
        SyntaxKind::OPEN_BRACE => SyntaxKind::CLOSE_BRACE,
        other => other,
    }
}

/// The display label for a token kind in `expected …` messages. Delimiters
/// and separators are shown literally (`` `,` ``, `` `]` ``); the whole
/// operator ladder collapses to a single `an operator`, so a position that
/// accepts any of ~20 operators doesn't spell them all out; literals read as
/// prose. Today's grammar only asks for delimiter labels (the `an operator`
/// and `expression` labels at error sites are literals at the recording
/// points); the full table is ported verbatim from the old parser's
/// `kind_label` so the two pipelines' messages stay at byte-parity until the
/// old one is deleted. (`WHITESPACE` and `ERROR_TOKEN` have no old
/// counterpart — the old lexer skipped whitespace and dropped lex-error
/// placeholders from expected sets — and are never recorded as expected.)
fn kind_label(k: SyntaxKind) -> &'static str {
    match k {
        // Delimiters & separators — shown literally.
        SyntaxKind::OPEN_PAREN => "`(`",
        SyntaxKind::CLOSE_PAREN => "`)`",
        SyntaxKind::OPEN_BRACK => "`[`",
        SyntaxKind::CLOSE_BRACK => "`]`",
        SyntaxKind::OPEN_BRACE => "`{`",
        SyntaxKind::CLOSE_BRACE => "`}`",
        SyntaxKind::COMMA => "`,`",
        SyntaxKind::SEMICOLON => "`;`",
        SyntaxKind::DOT => "`.`",
        // Literals / words.
        SyntaxKind::NUMBER => "number",
        SyntaxKind::STRING => "string",
        SyntaxKind::IDENT => "identifier",
        SyntaxKind::NEWLINE => "line break",
        SyntaxKind::COMMENT => "comment",
        SyntaxKind::WHITESPACE => "whitespace",
        SyntaxKind::ERROR_TOKEN => "unrecognized token",
        // The entire precedence ladder collapses to one label.
        SyntaxKind::ARROW
        | SyntaxKind::EQ
        | SyntaxKind::PLUS_EQ
        | SyntaxKind::SWAP
        | SyntaxKind::PLUS
        | SyntaxKind::MINUS
        | SyntaxKind::STAR
        | SyntaxKind::SLASH
        | SyntaxKind::PERCENT
        | SyntaxKind::CARET
        | SyntaxKind::EQ_EQ
        | SyntaxKind::BANG_EQ
        | SyntaxKind::LT
        | SyntaxKind::LT_EQ
        | SyntaxKind::GT
        | SyntaxKind::GT_EQ
        | SyntaxKind::AND_AND
        | SyntaxKind::OR_OR
        | SyntaxKind::BANG
        | SyntaxKind::TILDE
        | SyntaxKind::COLON
        | SyntaxKind::ELLIPSIS => "an operator",
        // Node kinds never reach here: labels are only requested for tokens.
        _ => unreachable!("kind_label called on a node kind: {k:?}"),
    }
}

/// Whether a token can begin an expression — the set whose appearance where a
/// closer was due signals a dropped separator.
fn begins_expr(k: SyntaxKind) -> bool {
    matches!(
        k,
        SyntaxKind::NUMBER
            | SyntaxKind::STRING
            | SyntaxKind::IDENT
            | SyntaxKind::OPEN_PAREN
            | SyntaxKind::OPEN_BRACK
            | SyntaxKind::OPEN_BRACE
            | SyntaxKind::MINUS
            | SyntaxKind::PLUS
            | SyntaxKind::BANG
            | SyntaxKind::ELLIPSIS
    )
}

/// Guess a dropped comma: inside a list / call / map a forgotten `,` surfaces
/// as an expression token sitting where a closer (and only operators besides)
/// was expected — e.g. `[1 2]`. Returns the `help:` text, or `None`.
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

// ====================================================================
// Token cursor + tree builder
// ====================================================================

/// A lexed token with its byte range resolved.
#[derive(Debug, Clone, Copy)]
struct Tok {
    kind: SyntaxKind,
    start: usize,
    end: usize,
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut offset = 0usize;
    lex(src)
        .into_iter()
        .map(|LexedToken { kind, len }| {
            let start = offset;
            offset += len as usize;
            Tok {
                kind,
                start,
                end: offset,
            }
        })
        .collect()
}

struct Parser<'s> {
    src: &'s str,
    tokens: Vec<Tok>,
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    /// Farthest-failure recorder: the labels expected at byte offset
    /// `expected_at` (the start of the farthest significant token peeked at
    /// so far). [`Parser::expected`] appends to it (de-duplicated, in
    /// recording order — which matches the order chumsky merged the same
    /// labels in, i.e. grammar/choice order); [`Parser::error`] turns it
    /// into a [`ParseError`]. The `error_parity_with_old_parser` test holds
    /// the resulting labels to byte-parity with the old pipeline.
    expected: Vec<&'static str>,
    expected_at: usize,
}

impl Parser<'_> {
    /// Index of the first significant (non-trivia) token at or after the
    /// cursor.
    fn cur_index(&self) -> Option<usize> {
        (self.pos..self.tokens.len()).find(|&i| !self.tokens[i].kind.is_trivia())
    }

    /// The kind of the next significant token, without consuming anything.
    fn peek(&self) -> Option<SyntaxKind> {
        self.cur_index().map(|i| self.tokens[i].kind)
    }

    /// The kind of the n-th significant token from the cursor (`nth(0)` ==
    /// `peek()`), without consuming anything.
    fn nth(&self, n: usize) -> Option<SyntaxKind> {
        (self.pos..self.tokens.len())
            .filter(|&i| !self.tokens[i].kind.is_trivia())
            .nth(n)
            .map(|i| self.tokens[i].kind)
    }

    /// Byte offset of the next significant token, or `src.len()` at EOF.
    fn cur_offset(&self) -> usize {
        self.cur_index()
            .map_or(self.src.len(), |i| self.tokens[i].start)
    }

    fn push_token(&mut self, tok: Tok) {
        self.builder.token(
            ScarpetLanguage::kind_to_raw(tok.kind),
            &self.src[tok.start..tok.end],
        );
    }

    /// Emits all pending trivia tokens into the currently open node.
    fn flush_trivia(&mut self) {
        while let Some(&tok) = self.tokens.get(self.pos) {
            if !tok.kind.is_trivia() {
                break;
            }
            self.push_token(tok);
            self.pos += 1;
        }
    }

    /// Consumes the next significant token into the currently open node,
    /// flushing pending trivia first. Must only be called after a successful
    /// `peek`.
    fn bump(&mut self) {
        self.flush_trivia();
        let tok = self.tokens[self.pos];
        debug_assert!(!tok.kind.is_trivia(), "bump after flush hit trivia");
        self.push_token(tok);
        self.pos += 1;
    }

    /// A checkpoint for a retroactive [`Self::start_node_at`] wrap. Pending
    /// trivia is flushed into the open parent first, so a node started here
    /// never begins with trivia.
    fn checkpoint(&mut self) -> Checkpoint {
        self.flush_trivia();
        self.builder.checkpoint()
    }

    /// Opens a node. Pending trivia is flushed into the *parent* first, so
    /// the new node never begins with trivia.
    fn start_node(&mut self, kind: SyntaxKind) {
        self.flush_trivia();
        self.builder.start_node(ScarpetLanguage::kind_to_raw(kind));
    }

    /// Retroactively wraps everything since `checkpoint` in a new node.
    fn start_node_at(&mut self, checkpoint: Checkpoint, kind: SyntaxKind) {
        self.builder
            .start_node_at(checkpoint, ScarpetLanguage::kind_to_raw(kind));
    }

    fn finish_node(&mut self) {
        self.builder.finish_node();
    }

    /// Records `label` as expected at the current position. The recorder
    /// keeps the labels of the *farthest* position only — recording at a
    /// position past the previous farthest one resets the set.
    fn expected(&mut self, label: &'static str) {
        let at = self.cur_offset();
        // Fail-fast parsing never rewinds, so `at` is monotone: it is either
        // past the previous farthest point (reset the set) or exactly at it.
        debug_assert!(at >= self.expected_at, "expected() rewound");
        if at > self.expected_at {
            self.expected_at = at;
            self.expected.clear();
        }
        if !self.expected.contains(&label) {
            self.expected.push(label);
        }
    }

    /// Records `label` as expected and fails at the current position — the
    /// one idiom for giving up on the parse.
    fn fail(&mut self, label: &'static str) -> Box<ParseError> {
        self.expected(label);
        self.error()
    }

    /// Builds the error for a hard failure at the current position from the
    /// farthest-failure recorder.
    fn error(&self) -> Box<ParseError> {
        // The span is the offending token's own range (`len..len` at EOF),
        // matching the old pipeline's observable spans — with one intentional
        // sharpening: on a mid-expression lex error the old pipeline kept
        // chumsky's raw error span, which folds the whitespace before the
        // unlexable token into `found` (`"a & b"` → span 1..3, found `" &"`).
        // Pointing exactly at the ERROR_TOKEN renders strictly better; see
        // `error_lex_error_keeps_precise_span`.
        let (span, found_kind) = match self.cur_index() {
            Some(i) => {
                let tok = self.tokens[i];
                (tok.start..tok.end, Some(tok.kind))
            }
            None => (self.src.len()..self.src.len(), None),
        };
        let found = found_kind.map(|_| self.src[span.clone()].to_string());
        let mut expected: Vec<String> = if self.expected_at == span.start {
            self.expected.iter().map(|s| (*s).to_string()).collect()
        } else {
            Vec::new()
        };
        // A position that accepts an `expression` also accepts the prefix
        // operators that begin one (`-`, `!`, …), so listing `an operator`
        // next to it is just noise — drop it.
        if expected.iter().any(|s| s == "expression") {
            expected.retain(|s| s != "an operator");
        }
        let help = missing_comma_help(&expected, found_kind);
        // Boxed: `ParseError` is large and the error path is cold, so we
        // keep the hot `Ok` arm of the `Result` cheap (clippy result_large_err).
        Box::new(ParseError {
            span,
            expected,
            found,
            headline: None,
            secondary: None,
            help,
        })
    }
}

// ====================================================================
// Grammar
// ====================================================================

/// Runs `f` with room to recurse, growing the native stack when the red zone
/// is hit. Every recursion cycle of the grammar passes through one of these
/// guards, so deeply nested input parses instead of aborting the process.
/// This is the same mechanism (the same crate, even) that backs the old
/// chumsky parser via its default `spill-stack` feature, keeping the two
/// parsers' depth tolerance at parity; on targets without stack manipulation
/// (`wasm32`) it degrades to a plain call, also like the old parser.
///
/// The red zone must comfortably exceed one trip around the deepest cycle —
/// a bracket level is ~26 frames (the full ladder plus `delimited`), which
/// in unoptimized builds runs to the order of 100 KiB.
fn with_stack<T>(f: impl FnOnce() -> T) -> T {
    stacker::maybe_grow(256 * 1024, 4 * 1024 * 1024, f)
}

/// Whether the next token ends a comma chain: a closer or end of input.
fn at_comma_chain_end(p: &Parser<'_>) -> bool {
    matches!(
        p.peek(),
        None | Some(SyntaxKind::CLOSE_PAREN | SyntaxKind::CLOSE_BRACK | SyntaxKind::CLOSE_BRACE)
    )
}

/// Whether the next token ends a semi chain: a closer, end of input, or the
/// `,` of an enclosing comma chain / argument list.
fn at_seq_chain_end(p: &Parser<'_>) -> bool {
    at_comma_chain_end(p) || p.peek() == Some(SyntaxKind::COMMA)
}

/// `top = comma_chain = seq_chain (`,` seq_chain)*` — n-ary, lenient about a
/// trailing `,`.
fn comma_chain(p: &mut Parser<'_>) -> PResult<()> {
    let cp = p.checkpoint();
    seq_chain(p)?;
    if p.peek() != Some(SyntaxKind::COMMA) {
        return Ok(());
    }
    p.start_node_at(cp, SyntaxKind::COMMA_CHAIN);
    while p.peek() == Some(SyntaxKind::COMMA) {
        p.bump();
        if at_comma_chain_end(p) {
            // Trailing `,`: trivia after it stays inside the chain node.
            p.flush_trivia();
            break;
        }
        seq_chain(p)?;
    }
    p.finish_node();
    Ok(())
}

/// `seq_chain = arrow_chain (`;` arrow_chain)*` — n-ary, lenient about a
/// trailing `;` and about `;;` runs (Scarpet's preprocessor strips them).
fn seq_chain(p: &mut Parser<'_>) -> PResult<()> {
    let cp = p.checkpoint();
    arrow_chain(p)?;
    if p.peek() != Some(SyntaxKind::SEMICOLON) {
        return Ok(());
    }
    p.start_node_at(cp, SyntaxKind::SEMI_CHAIN);
    while p.peek() == Some(SyntaxKind::SEMICOLON) {
        while p.peek() == Some(SyntaxKind::SEMICOLON) {
            p.bump();
        }
        if at_seq_chain_end(p) {
            // Trailing `;`: trivia after it stays inside the chain node.
            p.flush_trivia();
            break;
        }
        arrow_chain(p)?;
    }
    p.finish_node();
    Ok(())
}

/// One left-associative binary level: `operand (op operand)*`, folding each
/// link into a `BIN_EXPR` wrapped around everything since the level's start.
fn left_assoc(
    p: &mut Parser<'_>,
    ops: &[SyntaxKind],
    operand: fn(&mut Parser<'_>) -> PResult<()>,
) -> PResult<()> {
    let cp = p.checkpoint();
    operand(p)?;
    loop {
        match p.peek() {
            Some(k) if ops.contains(&k) => {
                p.start_node_at(cp, SyntaxKind::BIN_EXPR);
                p.bump();
                operand(p)?;
                p.finish_node();
            }
            _ => {
                p.expected("an operator");
                break;
            }
        }
    }
    Ok(())
}

/// One right-associative binary level: `operand (op rhs)?` where `rhs`
/// re-enters the level — right associativity is the self-recursion.
fn right_assoc(
    p: &mut Parser<'_>,
    ops: &[SyntaxKind],
    operand: fn(&mut Parser<'_>) -> PResult<()>,
) -> PResult<()> {
    let cp = p.checkpoint();
    operand(p)?;
    match p.peek() {
        Some(k) if ops.contains(&k) => {
            p.start_node_at(cp, SyntaxKind::BIN_EXPR);
            p.bump();
            with_stack(|| right_assoc(p, ops, operand))?;
            p.finish_node();
        }
        _ => p.expected("an operator"),
    }
    Ok(())
}

fn arrow_chain(p: &mut Parser<'_>) -> PResult<()> {
    right_assoc(p, &[SyntaxKind::ARROW], assign)
}

fn assign(p: &mut Parser<'_>) -> PResult<()> {
    right_assoc(
        p,
        &[SyntaxKind::EQ, SyntaxKind::PLUS_EQ, SyntaxKind::SWAP],
        lor,
    )
}

fn lor(p: &mut Parser<'_>) -> PResult<()> {
    left_assoc(p, &[SyntaxKind::OR_OR], land)
}

fn land(p: &mut Parser<'_>) -> PResult<()> {
    left_assoc(p, &[SyntaxKind::AND_AND], equality)
}

fn equality(p: &mut Parser<'_>) -> PResult<()> {
    left_assoc(p, &[SyntaxKind::EQ_EQ, SyntaxKind::BANG_EQ], compare)
}

fn compare(p: &mut Parser<'_>) -> PResult<()> {
    left_assoc(
        p,
        &[
            SyntaxKind::LT,
            SyntaxKind::LT_EQ,
            SyntaxKind::GT,
            SyntaxKind::GT_EQ,
        ],
        additive,
    )
}

fn additive(p: &mut Parser<'_>) -> PResult<()> {
    left_assoc(p, &[SyntaxKind::PLUS, SyntaxKind::MINUS], multiplicative)
}

fn multiplicative(p: &mut Parser<'_>) -> PResult<()> {
    left_assoc(
        p,
        &[SyntaxKind::STAR, SyntaxKind::SLASH, SyntaxKind::PERCENT],
        power,
    )
}

fn power(p: &mut Parser<'_>) -> PResult<()> {
    right_assoc(p, &[SyntaxKind::CARET], unary)
}

/// `unary = (`+` | `-` | `!` | `...`)* get`. The innermost prefix wraps the
/// operand; each outer prefix wraps the result.
fn unary(p: &mut Parser<'_>) -> PResult<()> {
    match p.peek() {
        Some(SyntaxKind::PLUS | SyntaxKind::MINUS | SyntaxKind::BANG | SyntaxKind::ELLIPSIS) => {
            p.start_node(SyntaxKind::PREFIX_EXPR);
            p.bump();
            with_stack(|| unary(p))?;
            p.finish_node();
        }
        _ => get(p)?,
    }
    Ok(())
}

fn get(p: &mut Parser<'_>) -> PResult<()> {
    left_assoc(p, &[SyntaxKind::TILDE, SyntaxKind::COLON], primary)
}

fn primary(p: &mut Parser<'_>) -> PResult<()> {
    match p.peek() {
        Some(SyntaxKind::NUMBER | SyntaxKind::STRING) => {
            p.start_node(SyntaxKind::LITERAL);
            p.bump();
            p.finish_node();
        }
        Some(SyntaxKind::IDENT) => {
            // An identifier directly followed by `(` is a call. "Directly"
            // is trivia-transparent, matching the old parser: `f (x)` and
            // `f\n(x)` are calls too.
            if p.nth(1) == Some(SyntaxKind::OPEN_PAREN) {
                p.start_node(SyntaxKind::CALL_EXPR);
                p.start_node(SyntaxKind::NAME_REF);
                p.bump();
                p.finish_node();
                delimited(
                    p,
                    SyntaxKind::ARG_LIST,
                    SyntaxKind::OPEN_PAREN,
                    SyntaxKind::CLOSE_PAREN,
                )?;
                p.finish_node();
            } else {
                p.start_node(SyntaxKind::NAME_REF);
                p.bump();
                p.finish_node();
                // The old parser tried an arg list here before settling for a
                // bare name, so `(` joins the expected set of whatever error
                // comes right after an identifier.
                p.expected(kind_label(SyntaxKind::OPEN_PAREN));
            }
        }
        Some(SyntaxKind::OPEN_PAREN) => {
            p.start_node(SyntaxKind::PAREN_EXPR);
            p.bump();
            with_stack(|| comma_chain(p))?;
            expect(p, SyntaxKind::CLOSE_PAREN)?;
            p.finish_node();
        }
        Some(SyntaxKind::OPEN_BRACK) => delimited(
            p,
            SyntaxKind::LIST_EXPR,
            SyntaxKind::OPEN_BRACK,
            SyntaxKind::CLOSE_BRACK,
        )?,
        Some(SyntaxKind::OPEN_BRACE) => delimited(
            p,
            SyntaxKind::MAP_EXPR,
            SyntaxKind::OPEN_BRACE,
            SyntaxKind::CLOSE_BRACE,
        )?,
        _ => {
            return Err(p.fail("expression"));
        }
    }
    Ok(())
}

/// Consumes a token of `kind` or fails, recording the kind's label.
fn expect(p: &mut Parser<'_>, kind: SyntaxKind) -> PResult<()> {
    if p.peek() == Some(kind) {
        p.bump();
        Ok(())
    } else {
        Err(p.fail(kind_label(kind)))
    }
}

/// A `node` of comma-separated `seq_chain` items between `open` and `close`
/// (the body of a call's `ARG_LIST`, a `LIST_EXPR` or a `MAP_EXPR`),
/// tolerating:
///
/// - an empty list right before the closer (no `EMPTY_ARG` — an interior
///   comment is just a trivia token inside the node);
/// - omitted entries: `f(a, , b)` puts a zero-width `EMPTY_ARG` in the slot;
/// - a trailing comma: `f(a,)` does NOT add a trailing `EMPTY_ARG`.
fn delimited(
    p: &mut Parser<'_>,
    node: SyntaxKind,
    open: SyntaxKind,
    close: SyntaxKind,
) -> PResult<()> {
    debug_assert_eq!(p.peek(), Some(open), "delimited called off an opener");
    p.start_node(node);
    p.bump();
    loop {
        if p.peek() == Some(close) {
            break;
        }
        if p.peek().is_none() {
            // Unreachable after `check_delimiters`, but fail cleanly rather
            // than loop forever if the pre-pass ever changes.
            return Err(p.fail(kind_label(close)));
        }
        if p.peek() == Some(SyntaxKind::COMMA) {
            // Omitted entry: mark the slot with a zero-width EMPTY_ARG.
            // (`start_node` flushes the slot's trivia into `node` first.)
            p.start_node(SyntaxKind::EMPTY_ARG);
            p.finish_node();
        } else {
            with_stack(|| seq_chain(p))?;
        }
        match p.peek() {
            Some(SyntaxKind::COMMA) => p.bump(),
            Some(k) if k == close => break,
            // Mirrors the old parser's expected set: the `,` was peeked
            // structurally there too and never made it into the labels.
            _ => {
                return Err(p.fail(kind_label(close)));
            }
        }
    }
    p.bump();
    p.finish_node();
    Ok(())
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses `src`, asserting success and losslessness, and returns the
    /// rowan debug dump of the tree.
    fn dump(src: &str) -> String {
        let parse =
            parse_source(src).unwrap_or_else(|e| panic!("{src:?}: parse error: {}", e.message()));
        let syntax = parse.syntax();
        assert_eq!(syntax.kind(), SyntaxKind::SOURCE_FILE);
        assert_eq!(syntax.text().to_string(), src, "lossless violated");
        assert_eq!(
            parse.source_file().syntax(),
            &syntax,
            "typed root accessor disagrees"
        );
        format!("{:#?}", syntax)
    }

    #[track_caller]
    fn check(src: &str, expected: &str) {
        let actual = dump(src);
        assert_eq!(
            actual.trim_end(),
            expected.trim(),
            "tree mismatch for {src:?}\nactual:\n{actual}"
        );
    }

    fn parse_ok(src: &str) -> SyntaxNode {
        let parse =
            parse_source(src).unwrap_or_else(|e| panic!("{src:?}: parse error: {}", e.message()));
        let syntax = parse.syntax();
        assert_eq!(syntax.text().to_string(), src, "lossless violated");
        syntax
    }

    /// The kinds of `node`'s direct children (nodes and tokens), in order.
    fn child_kinds(node: &SyntaxNode) -> Vec<SyntaxKind> {
        node.children_with_tokens().map(|e| e.kind()).collect()
    }

    /// The first descendant node of `kind`, in preorder.
    fn first_node(root: &SyntaxNode, kind: SyntaxKind) -> SyntaxNode {
        root.descendants()
            .find(|n| n.kind() == kind)
            .unwrap_or_else(|| panic!("no {kind:?} node in {root:#?}"))
    }

    // ----- structure tests (ported from `crate::parser::tests`) ---------

    #[test]
    fn hello_world() {
        check(
            "print('Hello World!')",
            r#"
SOURCE_FILE@0..21
  CALL_EXPR@0..21
    NAME_REF@0..5
      IDENT@0..5 "print"
    ARG_LIST@5..21
      OPEN_PAREN@5..6 "("
      LITERAL@6..20
        STRING@6..20 "'Hello World!'"
      CLOSE_PAREN@20..21 ")"
"#,
        );
    }

    #[test]
    fn arithmetic_precedence() {
        check(
            "2 + 3 * 4",
            r#"
SOURCE_FILE@0..9
  BIN_EXPR@0..9
    LITERAL@0..1
      NUMBER@0..1 "2"
    WHITESPACE@1..2 " "
    PLUS@2..3 "+"
    WHITESPACE@3..4 " "
    BIN_EXPR@4..9
      LITERAL@4..5
        NUMBER@4..5 "3"
      WHITESPACE@5..6 " "
      STAR@6..7 "*"
      WHITESPACE@7..8 " "
      LITERAL@8..9
        NUMBER@8..9 "4"
"#,
        );
    }

    #[test]
    fn additive_left_assoc() {
        check(
            "2 + 3 - 1",
            r#"
SOURCE_FILE@0..9
  BIN_EXPR@0..9
    BIN_EXPR@0..5
      LITERAL@0..1
        NUMBER@0..1 "2"
      WHITESPACE@1..2 " "
      PLUS@2..3 "+"
      WHITESPACE@3..4 " "
      LITERAL@4..5
        NUMBER@4..5 "3"
    WHITESPACE@5..6 " "
    MINUS@6..7 "-"
    WHITESPACE@7..8 " "
    LITERAL@8..9
      NUMBER@8..9 "1"
"#,
        );
    }

    #[test]
    fn power_right_assoc() {
        check(
            "2 ^ 3 ^ 2",
            r#"
SOURCE_FILE@0..9
  BIN_EXPR@0..9
    LITERAL@0..1
      NUMBER@0..1 "2"
    WHITESPACE@1..2 " "
    CARET@2..3 "^"
    WHITESPACE@3..4 " "
    BIN_EXPR@4..9
      LITERAL@4..5
        NUMBER@4..5 "3"
      WHITESPACE@5..6 " "
      CARET@6..7 "^"
      WHITESPACE@7..8 " "
      LITERAL@8..9
        NUMBER@8..9 "2"
"#,
        );
    }

    #[test]
    fn unary_minus_then_get() {
        check(
            "-foo:0",
            r#"
SOURCE_FILE@0..6
  PREFIX_EXPR@0..6
    MINUS@0..1 "-"
    BIN_EXPR@1..6
      NAME_REF@1..4
        IDENT@1..4 "foo"
      COLON@4..5 ":"
      LITERAL@5..6
        NUMBER@5..6 "0"
"#,
        );
    }

    #[test]
    fn match_and_get_chain() {
        check(
            "a:b:c",
            r#"
SOURCE_FILE@0..5
  BIN_EXPR@0..5
    BIN_EXPR@0..3
      NAME_REF@0..1
        IDENT@0..1 "a"
      COLON@1..2 ":"
      NAME_REF@2..3
        IDENT@2..3 "b"
    COLON@3..4 ":"
    NAME_REF@4..5
      IDENT@4..5 "c"
"#,
        );
        check(
            "a~b",
            r#"
SOURCE_FILE@0..3
  BIN_EXPR@0..3
    NAME_REF@0..1
      IDENT@0..1 "a"
    TILDE@1..2 "~"
    NAME_REF@2..3
      IDENT@2..3 "b"
"#,
        );
    }

    #[test]
    fn function_definition() {
        check(
            "foo(a, b) -> a + b",
            r#"
SOURCE_FILE@0..18
  BIN_EXPR@0..18
    CALL_EXPR@0..9
      NAME_REF@0..3
        IDENT@0..3 "foo"
      ARG_LIST@3..9
        OPEN_PAREN@3..4 "("
        NAME_REF@4..5
          IDENT@4..5 "a"
        COMMA@5..6 ","
        WHITESPACE@6..7 " "
        NAME_REF@7..8
          IDENT@7..8 "b"
        CLOSE_PAREN@8..9 ")"
    WHITESPACE@9..10 " "
    ARROW@10..12 "->"
    WHITESPACE@12..13 " "
    BIN_EXPR@13..18
      NAME_REF@13..14
        IDENT@13..14 "a"
      WHITESPACE@14..15 " "
      PLUS@15..16 "+"
      WHITESPACE@16..17 " "
      NAME_REF@17..18
        IDENT@17..18 "b"
"#,
        );
    }

    #[test]
    fn list_and_map_literals() {
        check(
            "[1, 2, 3]",
            r#"
SOURCE_FILE@0..9
  LIST_EXPR@0..9
    OPEN_BRACK@0..1 "["
    LITERAL@1..2
      NUMBER@1..2 "1"
    COMMA@2..3 ","
    WHITESPACE@3..4 " "
    LITERAL@4..5
      NUMBER@4..5 "2"
    COMMA@5..6 ","
    WHITESPACE@6..7 " "
    LITERAL@7..8
      NUMBER@7..8 "3"
    CLOSE_BRACK@8..9 "]"
"#,
        );
        check(
            "{'a' -> 1, 'b' -> 2}",
            r#"
SOURCE_FILE@0..20
  MAP_EXPR@0..20
    OPEN_BRACE@0..1 "{"
    BIN_EXPR@1..9
      LITERAL@1..4
        STRING@1..4 "'a'"
      WHITESPACE@4..5 " "
      ARROW@5..7 "->"
      WHITESPACE@7..8 " "
      LITERAL@8..9
        NUMBER@8..9 "1"
    COMMA@9..10 ","
    WHITESPACE@10..11 " "
    BIN_EXPR@11..19
      LITERAL@11..14
        STRING@11..14 "'b'"
      WHITESPACE@14..15 " "
      ARROW@15..17 "->"
      WHITESPACE@17..18 " "
      LITERAL@18..19
        NUMBER@18..19 "2"
    CLOSE_BRACE@19..20 "}"
"#,
        );
    }

    #[test]
    fn semi_binds_looser_than_arrow_in_map() {
        // `;` (seq_chain) sits outside `->` (arrow_chain), so a map entry
        // `{1+2 ; 'a'->3*4}` groups as `{(1+2) ; ('a'->(3*4))}`. This mirrors
        // Scarpet, where `->` (precedence 2) binds tighter than `;` (1).
        check(
            "{1+2;'a'->3*4}",
            r#"
SOURCE_FILE@0..14
  MAP_EXPR@0..14
    OPEN_BRACE@0..1 "{"
    SEMI_CHAIN@1..13
      BIN_EXPR@1..4
        LITERAL@1..2
          NUMBER@1..2 "1"
        PLUS@2..3 "+"
        LITERAL@3..4
          NUMBER@3..4 "2"
      SEMICOLON@4..5 ";"
      BIN_EXPR@5..13
        LITERAL@5..8
          STRING@5..8 "'a'"
        ARROW@8..10 "->"
        BIN_EXPR@10..13
          LITERAL@10..11
            NUMBER@10..11 "3"
          STAR@11..12 "*"
          LITERAL@12..13
            NUMBER@12..13 "4"
    CLOSE_BRACE@13..14 "}"
"#,
        );
    }

    #[test]
    fn arrow_right_assoc() {
        // `->` is right-associative, so `{f()->g()->h()}` groups as
        // `{f() -> (g() -> h())}`.
        check(
            "{f()->g()->h()}",
            r#"
SOURCE_FILE@0..15
  MAP_EXPR@0..15
    OPEN_BRACE@0..1 "{"
    BIN_EXPR@1..14
      CALL_EXPR@1..4
        NAME_REF@1..2
          IDENT@1..2 "f"
        ARG_LIST@2..4
          OPEN_PAREN@2..3 "("
          CLOSE_PAREN@3..4 ")"
      ARROW@4..6 "->"
      BIN_EXPR@6..14
        CALL_EXPR@6..9
          NAME_REF@6..7
            IDENT@6..7 "g"
          ARG_LIST@7..9
            OPEN_PAREN@7..8 "("
            CLOSE_PAREN@8..9 ")"
        ARROW@9..11 "->"
        CALL_EXPR@11..14
          NAME_REF@11..12
            IDENT@11..12 "h"
          ARG_LIST@12..14
            OPEN_PAREN@12..13 "("
            CLOSE_PAREN@13..14 ")"
    CLOSE_BRACE@14..15 "}"
"#,
        );
    }

    #[test]
    fn assignment_right_assoc() {
        check(
            "a = b = 5",
            r#"
SOURCE_FILE@0..9
  BIN_EXPR@0..9
    NAME_REF@0..1
      IDENT@0..1 "a"
    WHITESPACE@1..2 " "
    EQ@2..3 "="
    WHITESPACE@3..4 " "
    BIN_EXPR@4..9
      NAME_REF@4..5
        IDENT@4..5 "b"
      WHITESPACE@5..6 " "
      EQ@6..7 "="
      WHITESPACE@7..8 " "
      LITERAL@8..9
        NUMBER@8..9 "5"
"#,
        );
    }

    #[test]
    fn semi_and_comma_sequence() {
        // The old CST nested `Semi(Semi(a, b), c)`; the chain node is n-ary.
        check(
            "a; b; c",
            r#"
SOURCE_FILE@0..7
  SEMI_CHAIN@0..7
    NAME_REF@0..1
      IDENT@0..1 "a"
    SEMICOLON@1..2 ";"
    WHITESPACE@2..3 " "
    NAME_REF@3..4
      IDENT@3..4 "b"
    SEMICOLON@4..5 ";"
    WHITESPACE@5..6 " "
    NAME_REF@6..7
      IDENT@6..7 "c"
"#,
        );
        // `,` binds looser than `;`: the semi chain is an item of the comma
        // chain.
        check(
            "a; b, c",
            r#"
SOURCE_FILE@0..7
  COMMA_CHAIN@0..7
    SEMI_CHAIN@0..4
      NAME_REF@0..1
        IDENT@0..1 "a"
      SEMICOLON@1..2 ";"
      WHITESPACE@2..3 " "
      NAME_REF@3..4
        IDENT@3..4 "b"
    COMMA@4..5 ","
    WHITESPACE@5..6 " "
    NAME_REF@6..7
      IDENT@6..7 "c"
"#,
        );
    }

    #[test]
    fn unpacking_in_call() {
        check(
            "f(...xs)",
            r#"
SOURCE_FILE@0..8
  CALL_EXPR@0..8
    NAME_REF@0..1
      IDENT@0..1 "f"
    ARG_LIST@1..8
      OPEN_PAREN@1..2 "("
      PREFIX_EXPR@2..7
        ELLIPSIS@2..5 "..."
        NAME_REF@5..7
          IDENT@5..7 "xs"
      CLOSE_PAREN@7..8 ")"
"#,
        );
    }

    #[test]
    fn nested_function_call() {
        check(
            "print(format('f » ', 'g hi'))",
            r#"
SOURCE_FILE@0..30
  CALL_EXPR@0..30
    NAME_REF@0..5
      IDENT@0..5 "print"
    ARG_LIST@5..30
      OPEN_PAREN@5..6 "("
      CALL_EXPR@6..29
        NAME_REF@6..12
          IDENT@6..12 "format"
        ARG_LIST@12..29
          OPEN_PAREN@12..13 "("
          LITERAL@13..20
            STRING@13..20 "'f » '"
          COMMA@20..21 ","
          WHITESPACE@21..22 " "
          LITERAL@22..28
            STRING@22..28 "'g hi'"
          CLOSE_PAREN@28..29 ")"
      CLOSE_PAREN@29..30 ")"
"#,
        );
    }

    #[test]
    fn lenient_trailing_semicolon() {
        // The old parser dropped the trailing `;` and returned the bare item;
        // the lossless tree keeps the token inside a single-item SEMI_CHAIN.
        check(
            "a;",
            r#"
SOURCE_FILE@0..2
  SEMI_CHAIN@0..2
    NAME_REF@0..1
      IDENT@0..1 "a"
    SEMICOLON@1..2 ";"
"#,
        );
        // Without the separator there is no chain wrapper at all.
        check(
            "a",
            r#"
SOURCE_FILE@0..1
  NAME_REF@0..1
    IDENT@0..1 "a"
"#,
        );
    }

    #[test]
    fn anonymous_function_in_call() {
        check(
            "map([1,2,3], _(x) -> x * x)",
            r#"
SOURCE_FILE@0..27
  CALL_EXPR@0..27
    NAME_REF@0..3
      IDENT@0..3 "map"
    ARG_LIST@3..27
      OPEN_PAREN@3..4 "("
      LIST_EXPR@4..11
        OPEN_BRACK@4..5 "["
        LITERAL@5..6
          NUMBER@5..6 "1"
        COMMA@6..7 ","
        LITERAL@7..8
          NUMBER@7..8 "2"
        COMMA@8..9 ","
        LITERAL@9..10
          NUMBER@9..10 "3"
        CLOSE_BRACK@10..11 "]"
      COMMA@11..12 ","
      WHITESPACE@12..13 " "
      BIN_EXPR@13..26
        CALL_EXPR@13..17
          NAME_REF@13..14
            IDENT@13..14 "_"
          ARG_LIST@14..17
            OPEN_PAREN@14..15 "("
            NAME_REF@15..16
              IDENT@15..16 "x"
            CLOSE_PAREN@16..17 ")"
        WHITESPACE@17..18 " "
        ARROW@18..20 "->"
        WHITESPACE@20..21 " "
        BIN_EXPR@21..26
          NAME_REF@21..22
            IDENT@21..22 "x"
          WHITESPACE@22..23 " "
          STAR@23..24 "*"
          WHITESPACE@24..25 " "
          NAME_REF@25..26
            IDENT@25..26 "x"
      CLOSE_PAREN@26..27 ")"
"#,
        );
    }

    #[test]
    fn full_source_from_compdisplay() {
        check(
            "toggle() -> (\n    print(player(), 'hi');\n);",
            r#"
SOURCE_FILE@0..43
  SEMI_CHAIN@0..43
    BIN_EXPR@0..42
      CALL_EXPR@0..8
        NAME_REF@0..6
          IDENT@0..6 "toggle"
        ARG_LIST@6..8
          OPEN_PAREN@6..7 "("
          CLOSE_PAREN@7..8 ")"
      WHITESPACE@8..9 " "
      ARROW@9..11 "->"
      WHITESPACE@11..12 " "
      PAREN_EXPR@12..42
        OPEN_PAREN@12..13 "("
        NEWLINE@13..14 "\n"
        WHITESPACE@14..18 "    "
        SEMI_CHAIN@18..41
          CALL_EXPR@18..39
            NAME_REF@18..23
              IDENT@18..23 "print"
            ARG_LIST@23..39
              OPEN_PAREN@23..24 "("
              CALL_EXPR@24..32
                NAME_REF@24..30
                  IDENT@24..30 "player"
                ARG_LIST@30..32
                  OPEN_PAREN@30..31 "("
                  CLOSE_PAREN@31..32 ")"
              COMMA@32..33 ","
              WHITESPACE@33..34 " "
              LITERAL@34..38
                STRING@34..38 "'hi'"
              CLOSE_PAREN@38..39 ")"
          SEMICOLON@39..40 ";"
          NEWLINE@40..41 "\n"
        CLOSE_PAREN@41..42 ")"
    SEMICOLON@42..43 ";"
"#,
        );
    }

    #[test]
    fn chain_and_empty_arg_mapping() {
        // Trailing `,` at top level: single-item COMMA_CHAIN keeps the token.
        check(
            "a,",
            r#"
SOURCE_FILE@0..2
  COMMA_CHAIN@0..2
    NAME_REF@0..1
      IDENT@0..1 "a"
    COMMA@1..2 ","
"#,
        );
        // A lenient `;;` run stays inside one SEMI_CHAIN.
        check(
            "a;;b",
            r#"
SOURCE_FILE@0..4
  SEMI_CHAIN@0..4
    NAME_REF@0..1
      IDENT@0..1 "a"
    SEMICOLON@1..2 ";"
    SEMICOLON@2..3 ";"
    NAME_REF@3..4
      IDENT@3..4 "b"
"#,
        );
        // Omitted slots get zero-width EMPTY_ARG nodes...
        check(
            "f(a,,b)",
            r#"
SOURCE_FILE@0..7
  CALL_EXPR@0..7
    NAME_REF@0..1
      IDENT@0..1 "f"
    ARG_LIST@1..7
      OPEN_PAREN@1..2 "("
      NAME_REF@2..3
        IDENT@2..3 "a"
      COMMA@3..4 ","
      EMPTY_ARG@4..4
      COMMA@4..5 ","
      NAME_REF@5..6
        IDENT@5..6 "b"
      CLOSE_PAREN@6..7 ")"
"#,
        );
        check(
            "f(,a)",
            r#"
SOURCE_FILE@0..5
  CALL_EXPR@0..5
    NAME_REF@0..1
      IDENT@0..1 "f"
    ARG_LIST@1..5
      OPEN_PAREN@1..2 "("
      EMPTY_ARG@2..2
      COMMA@2..3 ","
      NAME_REF@3..4
        IDENT@3..4 "a"
      CLOSE_PAREN@4..5 ")"
"#,
        );
        // ...but a trailing comma and an empty list do not.
        check(
            "f(a,)",
            r#"
SOURCE_FILE@0..5
  CALL_EXPR@0..5
    NAME_REF@0..1
      IDENT@0..1 "f"
    ARG_LIST@1..5
      OPEN_PAREN@1..2 "("
      NAME_REF@2..3
        IDENT@2..3 "a"
      COMMA@3..4 ","
      CLOSE_PAREN@4..5 ")"
"#,
        );
        check(
            "f()",
            r#"
SOURCE_FILE@0..3
  CALL_EXPR@0..3
    NAME_REF@0..1
      IDENT@0..1 "f"
    ARG_LIST@1..3
      OPEN_PAREN@1..2 "("
      CLOSE_PAREN@2..3 ")"
"#,
        );
    }

    #[test]
    fn call_tolerates_trivia_between_name_and_args() {
        // The old parser consumed trivia between tokens uniformly, so an
        // identifier reaches its `(` across spaces and line breaks.
        for src in ["f (x)", "f\n(x)", "f // call\n (x)"] {
            let root = parse_ok(src);
            let call = first_node(&root, SyntaxKind::CALL_EXPR);
            assert_eq!(
                call.text().to_string(),
                src,
                "whole input should be one call"
            );
        }
    }

    // ----- trivia-preservation tests (ported from `crate::parser::tests`) --
    //
    // The old assertions about `leading` vectors are re-expressed as: the
    // tree is lossless (checked by `parse_ok`) and the trivia tokens sit
    // inside the right node, at the right position among its children.

    #[test]
    fn comments_attach_as_leading_trivia() {
        // Leading trivia precedes the expression inside SOURCE_FILE (no node
        // starts with trivia); the trailing break stays on the root.
        let root = parse_ok("// hello\n  a + b\n");
        assert_eq!(
            child_kinds(&root),
            [
                SyntaxKind::COMMENT,
                SyntaxKind::NEWLINE,
                SyntaxKind::WHITESPACE,
                SyntaxKind::BIN_EXPR,
                SyntaxKind::NEWLINE,
            ]
        );
        let bin = first_node(&root, SyntaxKind::BIN_EXPR);
        assert_eq!(
            child_kinds(&bin),
            [
                SyntaxKind::NAME_REF,
                SyntaxKind::WHITESPACE,
                SyntaxKind::PLUS,
                SyntaxKind::WHITESPACE,
                SyntaxKind::NAME_REF,
            ]
        );
    }

    #[test]
    fn break_inside_call_args_attaches_to_next_arg() {
        let root = parse_ok("f(a,\n b)");
        let args = first_node(&root, SyntaxKind::ARG_LIST);
        assert_eq!(
            child_kinds(&args),
            [
                SyntaxKind::OPEN_PAREN,
                SyntaxKind::NAME_REF,
                SyntaxKind::COMMA,
                SyntaxKind::NEWLINE,
                SyntaxKind::WHITESPACE,
                SyntaxKind::NAME_REF,
                SyntaxKind::CLOSE_PAREN,
            ]
        );
    }

    #[test]
    fn comment_between_operator_and_rhs_attaches_to_rhs() {
        // The comment sits inside the BIN_EXPR, between the operator token
        // and the rhs operand node.
        let root = parse_ok("a + // mid\n b");
        let bin = first_node(&root, SyntaxKind::BIN_EXPR);
        assert_eq!(
            child_kinds(&bin),
            [
                SyntaxKind::NAME_REF,
                SyntaxKind::WHITESPACE,
                SyntaxKind::PLUS,
                SyntaxKind::WHITESPACE,
                SyntaxKind::COMMENT,
                SyntaxKind::NEWLINE,
                SyntaxKind::WHITESPACE,
                SyntaxKind::NAME_REF,
            ]
        );
    }

    #[test]
    fn semicolon_trivia_flows_to_next_statement() {
        // Inter-statement trivia sits inside the SEMI_CHAIN, between the `;`
        // and the next item.
        let root = parse_ok("a;\n// note\n b");
        let chain = first_node(&root, SyntaxKind::SEMI_CHAIN);
        assert_eq!(
            child_kinds(&chain),
            [
                SyntaxKind::NAME_REF,
                SyntaxKind::SEMICOLON,
                SyntaxKind::NEWLINE,
                SyntaxKind::COMMENT,
                SyntaxKind::NEWLINE,
                SyntaxKind::WHITESPACE,
                SyntaxKind::NAME_REF,
            ]
        );
    }

    #[test]
    fn trailing_comment_anchored_on_root() {
        // No operator/separator follows `a`, so the pending trivia flows up
        // and lands directly in SOURCE_FILE.
        let root = parse_ok("a\n// trailing");
        assert_eq!(
            child_kinds(&root),
            [
                SyntaxKind::NAME_REF,
                SyntaxKind::NEWLINE,
                SyntaxKind::COMMENT,
            ]
        );
    }

    #[test]
    fn comment_inside_empty_parens_becomes_phantom_empty() {
        // The old parser synthesised a phantom Empty to anchor the comment;
        // in the lossless tree the comment is just a trivia token inside the
        // (otherwise empty) ARG_LIST — no EMPTY_ARG.
        let root = parse_ok("f(// note\n)");
        let args = first_node(&root, SyntaxKind::ARG_LIST);
        assert_eq!(
            child_kinds(&args),
            [
                SyntaxKind::OPEN_PAREN,
                SyntaxKind::COMMENT,
                SyntaxKind::NEWLINE,
                SyntaxKind::CLOSE_PAREN,
            ]
        );
    }

    #[test]
    fn comment_attaches_to_omitted_empty_arg() {
        // `f(a, , b)` synthesises an EMPTY_ARG between the commas; the
        // comment in that slot sits next to it inside the ARG_LIST.
        let root = parse_ok("f(a, // gap\n , b)");
        let args = first_node(&root, SyntaxKind::ARG_LIST);
        assert_eq!(
            child_kinds(&args),
            [
                SyntaxKind::OPEN_PAREN,
                SyntaxKind::NAME_REF,
                SyntaxKind::COMMA,
                SyntaxKind::WHITESPACE,
                SyntaxKind::COMMENT,
                SyntaxKind::NEWLINE,
                SyntaxKind::WHITESPACE,
                SyntaxKind::EMPTY_ARG,
                SyntaxKind::COMMA,
                SyntaxKind::WHITESPACE,
                SyntaxKind::NAME_REF,
                SyntaxKind::CLOSE_PAREN,
            ]
        );
        // The EMPTY_ARG itself is zero-width.
        let empty = first_node(&root, SyntaxKind::EMPTY_ARG);
        assert_eq!(empty.text().to_string(), "");
    }

    #[test]
    fn comment_after_last_arg_in_call_attaches_back() {
        // No comma follows `a`; the trailing trivia stays inside the
        // ARG_LIST, before the closer.
        let root = parse_ok("f(a // tail\n)");
        let args = first_node(&root, SyntaxKind::ARG_LIST);
        assert_eq!(
            child_kinds(&args),
            [
                SyntaxKind::OPEN_PAREN,
                SyntaxKind::NAME_REF,
                SyntaxKind::WHITESPACE,
                SyntaxKind::COMMENT,
                SyntaxKind::NEWLINE,
                SyntaxKind::CLOSE_PAREN,
            ]
        );
    }

    #[test]
    fn comment_around_trailing_comma_in_list_flushes_onto_last() {
        // The trailing-comma branch keeps both trivia bands (pre- and
        // post-comma) inside the LIST_EXPR rather than dropping them.
        let root = parse_ok("[1, // tail\n]");
        let list = first_node(&root, SyntaxKind::LIST_EXPR);
        assert_eq!(
            child_kinds(&list),
            [
                SyntaxKind::OPEN_BRACK,
                SyntaxKind::LITERAL,
                SyntaxKind::COMMA,
                SyntaxKind::WHITESPACE,
                SyntaxKind::COMMENT,
                SyntaxKind::NEWLINE,
                SyntaxKind::CLOSE_BRACK,
            ]
        );
    }

    #[test]
    fn comment_inside_paren_attaches_to_inner_first_token() {
        // Trivia immediately after `(` is flushed into the PAREN_EXPR before
        // the inner expression starts (which therefore owns no leading
        // trivia).
        let root = parse_ok("(// note\n a + b)");
        let paren = first_node(&root, SyntaxKind::PAREN_EXPR);
        assert_eq!(
            child_kinds(&paren),
            [
                SyntaxKind::OPEN_PAREN,
                SyntaxKind::COMMENT,
                SyntaxKind::NEWLINE,
                SyntaxKind::WHITESPACE,
                SyntaxKind::BIN_EXPR,
                SyntaxKind::CLOSE_PAREN,
            ]
        );
        let bin = first_node(&root, SyntaxKind::BIN_EXPR);
        assert_eq!(
            child_kinds(&bin),
            [
                SyntaxKind::NAME_REF,
                SyntaxKind::WHITESPACE,
                SyntaxKind::PLUS,
                SyntaxKind::WHITESPACE,
                SyntaxKind::NAME_REF,
            ]
        );
    }

    #[test]
    fn comment_after_trailing_separator_stays_in_chain() {
        // Trivia after a trailing `;` stays inside the chain node (the old
        // parser anchored it onto the chain accumulator), not in the
        // enclosing PAREN_EXPR.
        let root = parse_ok("(a; // tail\n)");
        let chain = first_node(&root, SyntaxKind::SEMI_CHAIN);
        assert_eq!(
            child_kinds(&chain),
            [
                SyntaxKind::NAME_REF,
                SyntaxKind::SEMICOLON,
                SyntaxKind::WHITESPACE,
                SyntaxKind::COMMENT,
                SyntaxKind::NEWLINE,
            ]
        );
        // Same for a trailing `,` at the end of input.
        let root = parse_ok("a, // tail");
        let chain = first_node(&root, SyntaxKind::COMMA_CHAIN);
        assert_eq!(
            child_kinds(&chain),
            [
                SyntaxKind::NAME_REF,
                SyntaxKind::COMMA,
                SyntaxKind::WHITESPACE,
                SyntaxKind::COMMENT,
            ]
        );
    }

    #[test]
    fn comment_between_unary_prefix_and_operand() {
        // Trivia between the prefix operator and its operand sits inside the
        // PREFIX_EXPR, before the operand node.
        let root = parse_ok("! // note\n x");
        let prefix = first_node(&root, SyntaxKind::PREFIX_EXPR);
        assert_eq!(
            child_kinds(&prefix),
            [
                SyntaxKind::BANG,
                SyntaxKind::WHITESPACE,
                SyntaxKind::COMMENT,
                SyntaxKind::NEWLINE,
                SyntaxKind::WHITESPACE,
                SyntaxKind::NAME_REF,
            ]
        );
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

    /// An EOF error carets the empty range `len..len` with no `found` token,
    /// like the old pipeline — including on empty input, at offset zero.
    #[test]
    fn error_eof_span_is_len_to_len() {
        let src = "1 +";
        let e = parse_source(src).unwrap_err();
        assert_eq!(e.span, src.len()..src.len());
        assert_eq!(e.found, None);
        assert_eq!(e.expected, ["expression"]);
        let e = parse_source("").unwrap_err();
        assert_eq!(e.span, 0..0);
        assert_eq!(e.found, None);
        assert_eq!(e.expected, ["expression"]);
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

    /// A bare identifier could have been a call, so `(` joins the expected
    /// set of an error right after it — same as the old parser, whose
    /// arg-list attempt recorded the label.
    #[test]
    fn error_after_identifier_offers_the_call_opener() {
        let e = parse_source("a.b").unwrap_err();
        assert_eq!(
            e.message(),
            "expected one of `(`, an operator, or end of input, found `.`"
        );
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

    /// Unlexable input — an [`ERROR_TOKEN`](SyntaxKind::ERROR_TOKEN) in the
    /// stream — fails the parse like the old pipeline's lex-error path, with
    /// one intentional delta: the span. The old pipeline kept chumsky's raw
    /// error span, which folds the whitespace before the unlexable token into
    /// `found` (`"a & b"` → span 1..3, found `" &"`, rendering as
    /// ``found ` &` ``); the new parser carets exactly the offending token
    /// (span 2..3, found `"&"`), which reads strictly better under ariadne.
    /// The expected labels match the old pipeline exactly.
    #[test]
    fn error_lex_error_keeps_precise_span() {
        let src = "a & b";
        let e = parse_source(src).unwrap_err();
        assert_eq!(e.span, 2..3); // old pipeline: 1..3
        assert_eq!(e.found.as_deref(), Some("&")); // old pipeline: " &"
        assert_eq!(
            e.message(),
            "expected one of `(`, an operator, or end of input, found `&`"
        );
        let old = crate::parser::parse_source(src).unwrap_err();
        assert_eq!(old.span, 1..3, "old span sharpened upstream — drop delta?");
        assert_eq!(old.found.as_deref(), Some(" &"));
        assert_eq!(e.expected, old.expected);
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

    /// `has_open_delimiter` is true only while an opener is still waiting for
    /// its closer — the REPL's signal to keep a multi-line submission open.
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

    /// Deeply nested input parses instead of overflowing the native stack:
    /// every recursion cycle of the grammar passes a `with_stack` guard that
    /// grows the stack on demand, like the old chumsky parser's `spill-stack`
    /// feature. Exercises all four cycles — brackets (via `primary` /
    /// `delimited`), the assign family, `^`, and prefix operators.
    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        // The interspersed trivia keeps every node above rowan's small-node
        // interning threshold (nodes with <= 3 children are hashed
        // recursively by its node cache, which would make this test
        // quadratic). Unguarded, a debug build aborts at ~10k bracket depth.
        let depth = 50_000;
        for src in [
            format!("{}a{}", "( ".repeat(depth), " )".repeat(depth)),
            format!("{}a{}", "[ ".repeat(depth), " ]".repeat(depth)),
            format!("{}1", "a = \n".repeat(depth)),
            format!("{}1", "2 ^ \n".repeat(depth)),
            format!("{}x", "- \n".repeat(depth)),
        ] {
            let parse = parse_source(&src)
                .unwrap_or_else(|e| panic!("deep input failed to parse: {}", e.message()));
            assert_eq!(parse.syntax().text().to_string(), src, "lossless violated");
            // *Dropping* a green tree this deep overflows the stack too —
            // rowan's recursive drop is the tree's own depth limit,
            // independent of how it was built. Leak it so this test pins
            // only the parser's depth tolerance.
            std::mem::forget(parse);
        }
    }

    // ----- old-pipeline error differential (deleted in wave 6) -----------

    /// Asserts the old and the new pipeline reject `src` with field-identical
    /// errors, including the rendered `message()` / `caret_label()` surfaces
    /// the CLI, LSP, and playground print.
    #[track_caller]
    fn assert_error_parity(src: &str) {
        let old = crate::parser::parse_source(src).expect_err("old parser accepted input");
        let new = parse_source(src).expect_err("new parser accepted input");
        assert_eq!(new.span, old.span, "span for {src:?}");
        assert_eq!(new.expected, old.expected, "expected labels for {src:?}");
        assert_eq!(new.found, old.found, "found for {src:?}");
        assert_eq!(new.headline, old.headline, "headline for {src:?}");
        assert_eq!(new.secondary, old.secondary, "secondary for {src:?}");
        assert_eq!(new.help, old.help, "help for {src:?}");
        assert_eq!(new.message(), old.message(), "message for {src:?}");
        assert_eq!(
            new.caret_label(),
            old.caret_label(),
            "caret label for {src:?}"
        );
    }

    /// Until the old parser is deleted, broken input must error identically
    /// through both pipelines: same caret span, same expected labels in the
    /// same order, same `found` text, headline, secondary span, and help.
    /// The one intentional exception — a sharper span on mid-expression lex
    /// errors — is pinned by `error_lex_error_keeps_precise_span` instead.
    #[test]
    fn error_parity_with_old_parser() {
        // Delimiter pre-pass errors, all three shapes.
        assert_error_parity("[)"); // mismatched closing delimiter
        assert_error_parity("foo(a"); // unclosed delimiter
        assert_error_parity("1)"); // unmatched closing delimiter
        // An operand is due but input ends.
        assert_error_parity("");
        assert_error_parity("1 +");
        assert_error_parity("x = ;");
        // A dropped comma between elements (the `help:` case).
        assert_error_parity("[1 2]");
        assert_error_parity("f(a b)");
        assert_error_parity("{1 2}");
        // Token mismatches mid-ladder.
        assert_error_parity("1 2");
        assert_error_parity("a.b");
        assert_error_parity("a ~ ~ b");
        assert_error_parity("f(;)");
        // Unlexable input where an expression was due (no preceding
        // whitespace folded in, so even the spans agree).
        assert_error_parity("&");
    }

    // ----- corpus gates ---------------------------------------------------

    /// Files the old parser cannot parse (kept in sync with the other copies
    /// of this list, in `scarpet-fmt/src/lib.rs` and in the `ast` round-trip
    /// test of this crate); the differential test below still covers them.
    const KNOWN_BAD: &[&str] = &[
        "gnembon/scarpet/programs/survival/portalorient.sc",
        "gnembon/scarpet/programs/survival/rifts/rifts.sc",
        "Ghoulboy78/Scarpet-edit/se.sc",
    ];

    fn corpus_files() -> Vec<(String, String)> {
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

        files
            .into_iter()
            .map(|file| {
                let rel = file
                    .strip_prefix(&root)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .replace('\\', "/");
                let src = std::fs::read_to_string(&file)
                    .unwrap_or_else(|e| panic!("{rel}: failed to read: {e}"));
                (rel, src)
            })
            .collect()
    }

    /// Every parseable corpus file round-trips byte-for-byte through the
    /// tree.
    #[test]
    fn corpus_lossless() {
        for (rel, src) in corpus_files() {
            if KNOWN_BAD.contains(&rel.as_str()) {
                continue;
            }
            let parse = parse_source(&src)
                .unwrap_or_else(|e| panic!("{rel}: unexpected parse failure: {}", e.message()));
            assert_eq!(
                parse.syntax().text().to_string(),
                src,
                "{rel}: lossless violated"
            );
        }
    }

    /// The load-bearing gate of this migration wave: on EVERY corpus file
    /// (known-bad included) the old chumsky parser and this parser agree on
    /// whether the source parses — and, where both reject, on the error
    /// itself, field for field. Deleted in wave 6 together with
    /// `crate::parser`.
    #[test]
    fn corpus_parse_success_differential() {
        for (rel, src) in corpus_files() {
            let old = crate::parser::parse_source(&src);
            let new = parse_source(&src);
            assert_eq!(
                old.is_ok(),
                new.is_ok(),
                "{rel}: parsers disagree (old: {:?}, new: {:?})",
                old.as_ref().map(|_| ()).map_err(|e| e.message()),
                new.as_ref().map(|_| ()).map_err(|e| e.message()),
            );
            if let (Err(old), Err(new)) = (&old, &new) {
                // Error parity on real-world broken files. (None of today's
                // known-bad files trips the documented lex-error span delta;
                // if a future corpus update does, exempt it here and lean on
                // `error_lex_error_keeps_precise_span`.)
                assert_eq!(new.span, old.span, "{rel}: error span");
                assert_eq!(new.expected, old.expected, "{rel}: expected labels");
                assert_eq!(new.found, old.found, "{rel}: found");
                assert_eq!(new.headline, old.headline, "{rel}: headline");
                assert_eq!(new.secondary, old.secondary, "{rel}: secondary");
                assert_eq!(new.help, old.help, "{rel}: help");
            }
        }
    }
}
