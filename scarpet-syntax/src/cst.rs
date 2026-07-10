//! The legacy CST shape — a compact expression tree that carries comments and
//! breaks as **leading trivia** on each node — plus its lowering from the
//! lossless `rowan` syntax tree.
//!
//! The formatter (`scarpet-fmt`) and the evaluator's AST lowering
//! (`crate::ast`) both consume this shape. The rowan tree is the parse
//! artifact of record (every byte of the source, in order); this module
//! re-derives the trivia *attachment* the old combinator parser produced, so
//! downstream behavior is unchanged:
//!
//! - Trivia before an infix operator belongs to the operator's RHS node
//!   (prepended to its leading), as does trivia after a `;`/`,` link.
//! - Trivia after any *other* operator is consumed at the RHS's leftmost
//!   non-operator node (descending through `Binary` LHS chains).
//! - In argument lists, trivia between items rides on the *next* item;
//!   trivia stranded before the closer is appended onto the *last* item, and
//!   an otherwise-empty list anchors it on a phantom [`CstKind::Empty`].
//!   An omitted argument (`f(a, , b)`) also becomes an `Empty`.
//! - Trivia around a trailing `;`/`,` (before a closer or end of input) is
//!   appended onto the preceding node's leading.
//!
//! The `->` operator never reaches this module as a binary op: the parser
//! already decided, from context alone, whether it is a function definition
//! (`DEFINE_FUNCTION`, anywhere) or a map entry's key/value arrow (folded
//! into the item's `MAP_ENTRY` node). Both lower tag-for-tag — to
//! [`CstKind::DefineFunction`] and [`CstKind::MapEntry`] — and their trivia
//! routes exactly as the old `->` binary did.
//!
//! Deliberately preserved quirks of the old parser (the corpus round-trip
//! depends on byte-identical formatting): trivia between a callee and its
//! `(`, trivia between an expression and a `)` with no separator in between,
//! and trivia between consecutive `;`s are dropped from the *CST view* (the
//! rowan tree still holds them).

use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

// ====================================================================
// CST types (unchanged shape)
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
    /// `signature -> body` — a function definition: any `->` outside the top
    /// level of a map item, as decided by the parser. The signature is
    /// *semantically* a call `name(params)`; the AST lowering rejects any
    /// other shape, so no def-vs-entry judgment happens after the parse.
    DefineFunction {
        signature: Box<Cst<'s>>,
        body: Box<Cst<'s>>,
    },
    /// One item of a map literal (`{…}` / `m(…)`): a bare `key`, or the
    /// item-level pair `key -> value`.
    MapEntry {
        key: Box<Cst<'s>>,
        value: Option<Box<Cst<'s>>>,
    },
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
        CstKind::DefineFunction { signature, body } => CstKind::DefineFunction {
            signature: Box::new(strip_trivia(signature)),
            body: Box::new(strip_trivia(body)),
        },
        CstKind::MapEntry { key, value } => CstKind::MapEntry {
            key: Box::new(strip_trivia(key)),
            value: value.as_ref().map(|v| Box::new(strip_trivia(v))),
        },
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
// Lowering: rowan syntax tree -> Cst
// ====================================================================

/// Lower a parsed `ROOT` node to the legacy CST. The root must come from a
/// successful parse of `src` (the CST borrows its atoms from `src`).
pub fn from_root<'s>(src: &'s str, root: &SyntaxNode) -> Cst<'s> {
    debug_assert_eq!(root.kind(), SyntaxKind::ROOT);
    let conv = Conv { src };
    let elems: Vec<SyntaxElement> = root.children_with_tokens().collect();
    // Trailing separators and trivia anchor onto the root node (`keep_final`),
    // so a comment after the final expression isn't silently dropped.
    conv.lower_chain(&elems, true)
}

struct Conv<'s> {
    src: &'s str,
}

impl<'s> Conv<'s> {
    /// The source slice a token covers (the CST borrows from the source, not
    /// from the green tree).
    fn text(&self, tok: &SyntaxToken) -> &'s str {
        let range = tok.text_range();
        &self.src[u32::from(range.start()) as usize..u32::from(range.end()) as usize]
    }

    /// The CST-visible trivia in `elems` (breaks and comments; whitespace has
    /// no CST representation).
    fn band(&self, elems: &[SyntaxElement]) -> Vec<Trivia<'s>> {
        let mut out = Vec::new();
        for el in elems {
            if let Some(tok) = el.as_token() {
                self.push_trivia(&mut out, tok);
            }
        }
        out
    }

    fn push_trivia(&self, band: &mut Vec<Trivia<'s>>, tok: &SyntaxToken) {
        match tok.kind() {
            SyntaxKind::BREAK => band.push(Trivia::Break),
            SyntaxKind::COMMENT => band.push(Trivia::Comment(self.text(tok))),
            _ => {}
        }
    }

    fn expr(&self, node: &SyntaxNode) -> Cst<'s> {
        match node.kind() {
            SyntaxKind::LITERAL => {
                // A `LITERAL`/`NAME_REF` node is built by flushing trivia
                // *before* the node opens (see `Parser::start_node`), so it
                // wraps exactly its one semantic token — `first_token` is it.
                let tok = node.first_token().expect("a literal wraps its token");
                let text = self.text(&tok);
                Cst::bare(match tok.kind() {
                    SyntaxKind::NUMBER => CstKind::Number(text),
                    SyntaxKind::STRING => CstKind::Str(text),
                    k => unreachable!("literal token: {k:?}"),
                })
            }
            SyntaxKind::NAME_REF => {
                let tok = node.first_token().expect("a name wraps its token");
                Cst::bare(CstKind::Ident(self.text(&tok)))
            }
            SyntaxKind::CALL_EXPR => {
                let callee = node
                    .children()
                    .find(|n| n.kind() == SyntaxKind::NAME_REF)
                    .expect("a call has a callee");
                let arg_list = node
                    .children()
                    .find(|n| n.kind() == SyntaxKind::ARG_LIST)
                    .expect("a call has an argument list");
                // Trivia between the callee and its `(` is dropped from the
                // CST view, as the old parser did.
                Cst::bare(CstKind::Call {
                    callee: Box::new(self.expr(&callee)),
                    args: self.args(&arg_list),
                })
            }
            SyntaxKind::LIST_EXPR => Cst::bare(CstKind::List(self.args(node))),
            SyntaxKind::MAP_EXPR => Cst::bare(CstKind::Map(self.args(node))),
            SyntaxKind::PAREN_EXPR => self.paren(node),
            SyntaxKind::PREFIX_EXPR => self.prefix(node),
            SyntaxKind::DEFINE_FUNCTION => self.define_function(node),
            SyntaxKind::MAP_ENTRY => self.map_entry(node),
            SyntaxKind::BIN_EXPR => self.bin(node),
            k => unreachable!("expression node: {k:?}"),
        }
    }

    /// Lower a chain body: the band before its first expression rides onto that
    /// expression's leftmost leaf, then trailing separators/trivia anchor via
    /// [`Self::chain_tail`]. Shared by the root and paren bodies, which differ
    /// only in whether a final band with no separator before it is kept
    /// (`keep_final`) — the root keeps it, a paren drops it.
    fn lower_chain(&self, interior: &[SyntaxElement], keep_final: bool) -> Cst<'s> {
        let expr_at = interior
            .iter()
            .position(|e| e.as_node().is_some())
            .expect("a chain body has an expression");
        let band = self.band(&interior[..expr_at]);
        let mut cst = self.expr(interior[expr_at].as_node().unwrap());
        prepend_deep(&mut cst, band);
        self.chain_tail(&mut cst, &interior[expr_at + 1..], keep_final);
        cst
    }

    /// `( body )` — trivia after `(` belongs to the body's leftmost node;
    /// trailing separators anchor onto the body. Trivia directly before `)`
    /// (no separator in between) is dropped, as the old parser did.
    fn paren(&self, node: &SyntaxNode) -> Cst<'s> {
        let elems: Vec<SyntaxElement> = node.children_with_tokens().collect();
        let closer_at = elems
            .iter()
            .rposition(|e| {
                e.as_token()
                    .is_some_and(|t| t.kind() == SyntaxKind::R_PAREN)
            })
            .expect("a paren is closed");
        // The body sits between `(` (element 0) and `)` (`closer_at`).
        let inner = self.lower_chain(&elems[1..closer_at], false);
        Cst::bare(CstKind::Paren(Box::new(inner)))
    }

    /// A prefix application: the operator's own leading trivia is the parent's
    /// business; trivia between the operator and its operand sinks to the
    /// operand's leftmost node.
    fn prefix(&self, node: &SyntaxNode) -> Cst<'s> {
        let elems: Vec<SyntaxElement> = node.children_with_tokens().collect();
        let op_tok = elems
            .iter()
            .find_map(|e| e.as_token().filter(|t| !t.kind().is_trivia()))
            .expect("a prefix has its operator");
        let op = match op_tok.kind() {
            SyntaxKind::MINUS => UnaryOp::Neg,
            SyntaxKind::PLUS => UnaryOp::Pos,
            SyntaxKind::BANG => UnaryOp::Not,
            SyntaxKind::DOT3 => UnaryOp::Unpack,
            k => unreachable!("prefix operator: {k:?}"),
        };
        let operand_at = elems
            .iter()
            .position(|e| e.as_node().is_some())
            .expect("a prefix has an operand");
        let band = self.band(&elems[..operand_at]);
        let mut operand = self.expr(elems[operand_at].as_node().unwrap());
        prepend_deep(&mut operand, band);
        Cst::bare(CstKind::Unary {
            op,
            operand: Box::new(operand),
        })
    }

    /// A binary application. Trivia routes via [`Self::infix_operands`].
    fn bin(&self, node: &SyntaxNode) -> Cst<'s> {
        let (lhs, op, rhs) = self
            .infix_operands(node)
            .expect("a binary has its operator");
        Cst::bare(CstKind::Binary {
            op: binop_of(op),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        })
    }

    /// A function definition (a dedicated `DEFINE_FUNCTION`, always a single
    /// operator — right-associativity puts any further `->` inside the body
    /// node). Trivia routes via [`Self::infix_operands`], exactly as the old
    /// `->` binary did.
    fn define_function(&self, node: &SyntaxNode) -> Cst<'s> {
        let (signature, _, body) = self
            .infix_operands(node)
            .expect("a definition has its `->`");
        Cst::bare(CstKind::DefineFunction {
            signature: Box::new(signature),
            body: Box::new(body),
        })
    }

    /// One item of a map literal (a `MAP_ENTRY`). A bare entry wraps exactly
    /// its key node — no interior bands. A pair routes its `->` trivia via
    /// [`Self::infix_operands`].
    fn map_entry(&self, node: &SyntaxNode) -> Cst<'s> {
        match self.infix_operands(node) {
            Some((key, _, value)) => Cst::bare(CstKind::MapEntry {
                key: Box::new(key),
                value: Some(Box::new(value)),
            }),
            None => {
                let key = node.children().next().expect("a map entry has a key");
                Cst::bare(CstKind::MapEntry {
                    key: Box::new(self.expr(&key)),
                    value: None,
                })
            }
        }
    }

    /// The `lhs op rhs` decomposition of an infix node (a `BIN_EXPR`, a
    /// `DEFINE_FUNCTION`, or a pair `MAP_ENTRY`), or `None` for a node with a
    /// single operand (a bare `MAP_ENTRY`). Trivia before the operator
    /// prepends onto the RHS node itself; trivia after it sinks to the RHS's
    /// leftmost node — except for `;`/`,` links, where both bands prepend
    /// onto the RHS node (the old chain parsers collected them at the chain
    /// level). A `;` link may carry several `;` tokens (Scarpet treats runs
    /// of `;` as one separator); trivia between them is dropped, as the old
    /// parser did.
    fn infix_operands(&self, node: &SyntaxNode) -> Option<(Cst<'s>, SyntaxKind, Cst<'s>)> {
        let elems: Vec<SyntaxElement> = node.children_with_tokens().collect();
        let lhs_at = elems
            .iter()
            .position(|e| e.as_node().is_some())
            .expect("an infix node has an operand");
        let rhs_at = elems
            .iter()
            .rposition(|e| e.as_node().is_some())
            .expect("an infix node has an operand");
        if lhs_at == rhs_at {
            return None;
        }
        let op_at = elems[lhs_at + 1..rhs_at]
            .iter()
            .position(|e| e.as_token().is_some_and(|t| !t.kind().is_trivia()))
            .map(|i| i + lhs_at + 1)
            .expect("an infix node has its operator");
        let last_op_at = elems[..rhs_at]
            .iter()
            .rposition(|e| e.as_token().is_some_and(|t| !t.kind().is_trivia()))
            .expect("an infix node has its operator");
        let op = elems[op_at].as_token().unwrap().kind();

        let band_before = self.band(&elems[lhs_at + 1..op_at]);
        let band_after = self.band(&elems[last_op_at + 1..rhs_at]);

        let lhs = self.expr(elems[lhs_at].as_node().unwrap());
        let mut rhs = self.expr(elems[rhs_at].as_node().unwrap());
        if matches!(op, SyntaxKind::SEMICOLON | SyntaxKind::COMMA) {
            let mut band = band_before;
            band.extend(band_after);
            rhs = rhs.with_leading(band);
        } else {
            prepend_deep(&mut rhs, band_after);
            rhs = rhs.with_leading(band_before);
        }
        Some((lhs, op, rhs))
    }

    /// The comma-separated items between a node's delimiters (a call's
    /// `ARG_LIST`, a `LIST_EXPR`, a `MAP_EXPR`), with the old parser's trivia
    /// routing and phantom [`CstKind::Empty`] slots.
    fn args(&self, node: &SyntaxNode) -> Vec<Cst<'s>> {
        let elems: Vec<SyntaxElement> = node.children_with_tokens().collect();
        let open_at = elems
            .iter()
            .position(|e| e.as_token().is_some_and(|t| t.kind().is_opener()))
            .expect("an argument list opens");
        let close_at = elems
            .iter()
            .rposition(|e| e.as_token().is_some_and(|t| t.kind().is_closer()))
            .unwrap_or(elems.len());

        let mut items: Vec<Cst<'s>> = Vec::new();
        let mut pending: Vec<Trivia<'s>> = Vec::new();
        // Is the walk at an item position (list head, or just past a `,`)?
        let mut expect_item = true;
        // Was the previous semantic element a trailing `;`?
        let mut prev_semi = false;
        for el in &elems[open_at + 1..close_at] {
            match el {
                SyntaxElement::Token(tok) if tok.kind().is_trivia() => {
                    self.push_trivia(&mut pending, tok);
                }
                SyntaxElement::Token(tok) if tok.kind() == SyntaxKind::COMMA => {
                    if expect_item {
                        // Omitted entry: a phantom Empty carries the pending
                        // trivia (which would otherwise be lost).
                        items.push(Cst {
                            leading: std::mem::take(&mut pending),
                            kind: CstKind::Empty,
                        });
                    } else if prev_semi {
                        // Trivia between a trailing `;` and the `,` stays on
                        // the finished item.
                        flush_onto_last(&mut items, &mut pending);
                    }
                    // Otherwise the band between an item and its `,` flows
                    // into the next item's leading — leave it pending.
                    expect_item = true;
                    prev_semi = false;
                }
                SyntaxElement::Token(tok) if tok.kind() == SyntaxKind::SEMICOLON => {
                    // A trailing `;` after an item. The band before the first
                    // `;` of a run stays on the item; bands between `;`s are
                    // dropped (old behavior).
                    if prev_semi {
                        pending.clear();
                    } else {
                        flush_onto_last(&mut items, &mut pending);
                    }
                    prev_semi = true;
                }
                SyntaxElement::Token(tok) => {
                    unreachable!("argument separator: {:?}", tok.kind())
                }
                SyntaxElement::Node(n) => {
                    let cst = self.expr(n).with_leading(std::mem::take(&mut pending));
                    items.push(cst);
                    expect_item = false;
                    prev_semi = false;
                }
            }
        }
        if !pending.is_empty() {
            if items.is_empty() {
                // Trivia inside an otherwise-empty list needs an anchor: a
                // phantom Empty, so it is not lost.
                items.push(Cst {
                    leading: pending,
                    kind: CstKind::Empty,
                });
            } else {
                // Trailing-comma bands and trivia stranded before the closer
                // re-attach onto the last item.
                flush_onto_last(&mut items, &mut pending);
            }
        }
        items
    }

    /// Trailing separators (`;` runs, a `,`) and trivia after a chain's last
    /// expression — at the root or inside a paren. Bands adjacent to a
    /// separator append onto `acc`'s leading; bands between consecutive `;`s
    /// are dropped (old behavior). A final band with no separator before it
    /// is kept only when `keep_final_without_sep` (the root keeps it; a paren
    /// dropped it).
    fn chain_tail(&self, acc: &mut Cst<'s>, elems: &[SyntaxElement], keep_final_without_sep: bool) {
        let mut pending: Vec<Trivia<'s>> = Vec::new();
        let mut prev_semi = false;
        let mut seen_sep = false;
        for el in elems {
            match el {
                SyntaxElement::Token(tok) if tok.kind().is_trivia() => {
                    self.push_trivia(&mut pending, tok);
                }
                SyntaxElement::Token(tok) if tok.kind() == SyntaxKind::SEMICOLON => {
                    if prev_semi {
                        pending.clear();
                    } else {
                        acc.leading.append(&mut pending);
                    }
                    prev_semi = true;
                    seen_sep = true;
                }
                SyntaxElement::Token(tok) if tok.kind() == SyntaxKind::COMMA => {
                    acc.leading.append(&mut pending);
                    prev_semi = false;
                    seen_sep = true;
                }
                el => unreachable!("chain tail element: {el:?}"),
            }
        }
        if !pending.is_empty() && (seen_sep || keep_final_without_sep) {
            acc.leading.append(&mut pending);
        }
    }
}

/// Prepend `band` onto the leading of `cst`'s leftmost non-`Binary`
/// descendant — where the old parser's leaf/prefix/primary sub-parsers
/// consumed trivia sitting at the start of an expression.
fn prepend_deep<'s>(cst: &mut Cst<'s>, band: Vec<Trivia<'s>>) {
    if band.is_empty() {
        return;
    }
    if let CstKind::Binary { lhs, .. } = &mut cst.kind {
        return prepend_deep(lhs, band);
    }
    let mut leading = band;
    leading.append(&mut cst.leading);
    cst.leading = leading;
}

/// Append the pending band onto the last item's leading.
fn flush_onto_last<'s>(items: &mut [Cst<'s>], pending: &mut Vec<Trivia<'s>>) {
    if let Some(last) = items.last_mut() {
        last.leading.append(pending);
    } else {
        debug_assert!(pending.is_empty(), "a separator implies a preceding item");
    }
}

fn binop_of(kind: SyntaxKind) -> BinOp {
    match kind {
        SyntaxKind::PLUS => BinOp::Add,
        SyntaxKind::MINUS => BinOp::Sub,
        SyntaxKind::STAR => BinOp::Mul,
        SyntaxKind::SLASH => BinOp::Div,
        SyntaxKind::PERCENT => BinOp::Rem,
        SyntaxKind::CARET => BinOp::Pow,
        SyntaxKind::EQ2 => BinOp::Eq,
        SyntaxKind::BANG_EQ => BinOp::NotEq,
        SyntaxKind::LT => BinOp::Lt,
        SyntaxKind::LT_EQ => BinOp::LtEq,
        SyntaxKind::GT => BinOp::Gt,
        SyntaxKind::GT_EQ => BinOp::GtEq,
        SyntaxKind::AMP2 => BinOp::And,
        SyntaxKind::PIPE2 => BinOp::Or,
        SyntaxKind::TILDE => BinOp::Match,
        SyntaxKind::COLON => BinOp::Get,
        SyntaxKind::EQ => BinOp::Assign,
        SyntaxKind::PLUS_EQ => BinOp::AddAssign,
        SyntaxKind::LT_GT => BinOp::Swap,
        SyntaxKind::SEMICOLON => BinOp::Semi,
        SyntaxKind::COMMA => BinOp::Comma,
        k => unreachable!("binary operator: {k:?}"),
    }
}
