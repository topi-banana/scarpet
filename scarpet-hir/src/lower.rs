//! Lowering: `scarpet-syntax`'s rowan tree into the [`Hir`] arena.
//!
//! The input is the typed accessor layer (`scarpet_syntax::nodes`), which is
//! generated from `scarpet.ungram` and is the only view of a Scarpet program
//! that carries source positions. The lossless `Cst` and the precedence-laddered
//! `ast` both drop spans, so neither can back a located diagnostic.
//!
//! # Why the walk is iterative
//!
//! `;`, `,` and every arithmetic operator are left-associative, so `a; b; c`
//! parses as `((a; b); c)` and a five-thousand-statement file is five thousand
//! levels of `BinExpr`. A recursive lowering would overflow the stack on real
//! corpus files. Instead this module keeps an explicit worklist and uses
//! **reserve-then-fill**: a child's [`ExprId`] is allocated *before* descending
//! into it, so a parent can be written out complete the moment it is visited and
//! nothing needs patching afterwards.
//!
//! # Traps in the generated layer
//!
//! - `Expr::can_cast(NAME_REF)` is `true`. Hand-rolling
//!   `children().find_map(Expr::cast)` on a `DEFINE_FUNCTION` returns the
//!   function's *name*, not its body — which is exactly why the generated
//!   `DefineFunction::body()` is `nth_child::<Expr>(1)`. Always go through the
//!   generated accessors.
//! - `f(a, b)` does **not** produce a comma `BinExpr`. The parser splits an
//!   argument list into sibling `Expr` children of `ARG_LIST`, so comma chains
//!   only ever appear directly under `ROOT` or inside a `PAREN_EXPR`.

use scarpet_syntax::nodes::{
    ArgList, AstNode, BinExpr, CallExpr, DefineFunction, Expr as SynExpr, ListExpr, Literal,
    MapExpr, NameRef, ParenExpr, PrefixExpr, Root,
};
use scarpet_syntax::syntax::{SyntaxKind, SyntaxNode};

use crate::hir::{
    AssignOp, BinOp, Expr, ExprId, FnDef, FnId, Hir, MapEntry, Name, PrefixOp, SeqSep,
};
use crate::span::Span;

/// Lower a parsed tree. Accepts either the `ROOT` node `parse` returns or a
/// bare expression node.
pub(crate) fn lower(root: &SyntaxNode) -> Hir {
    Lowerer::default().run(root)
}

#[derive(Default)]
struct Lowerer {
    exprs: Vec<Expr>,
    spans: Vec<Span>,
    fns: Vec<FnDef>,
}

impl Lowerer {
    /// Allocate a slot covering `span`, filled with [`Expr::Missing`] until the
    /// node is visited.
    fn reserve(&mut self, span: Span) -> ExprId {
        self.exprs.push(Expr::Missing);
        self.spans.push(span);
        ExprId::from_index(self.exprs.len() - 1)
    }

    fn set(&mut self, slot: ExprId, expr: Expr) {
        self.exprs[slot.index()] = expr;
    }

    fn run(mut self, root: &SyntaxNode) -> Hir {
        // `parse` hands back a `ROOT` whose single child is the program. A
        // caller holding a sub-expression may pass that directly instead.
        let entry = if root.kind() == SyntaxKind::ROOT {
            Root::cast(root.clone()).and_then(|r| r.expr())
        } else {
            SynExpr::cast(root.clone())
        };

        let Some(entry) = entry else {
            // An empty program. Scarpet evaluates it to `null`, which is what an
            // empty sequence yields, so model it as one rather than as a hole.
            let slot = self.reserve(Span::from(root.text_range()));
            self.set(
                slot,
                Expr::Seq {
                    sep: SeqSep::Semi,
                    exprs: Vec::new(),
                },
            );
            return Hir::new(self.exprs, self.spans, self.fns, slot);
        };

        let root_slot = self.reserve(node_span(entry.syntax()));
        let mut work = vec![(entry.syntax().clone(), root_slot)];
        while let Some((node, slot)) = work.pop() {
            self.enter(&node, slot, &mut work);
        }
        Hir::new(self.exprs, self.spans, self.fns, root_slot)
    }

    /// Fill `slot` from `node`, reserving a slot for each child and queueing it.
    ///
    /// Children are pushed in reverse so the LIFO worklist visits them left to
    /// right; that keeps the arena in an order where a parent always precedes
    /// its children, which every later pass relies on.
    fn enter(&mut self, node: &SyntaxNode, slot: ExprId, work: &mut Vec<(SyntaxNode, ExprId)>) {
        let expr = match node.kind() {
            SyntaxKind::LITERAL => self.literal(node),
            SyntaxKind::NAME_REF => NameRef::cast(node.clone())
                .and_then(|n| n.ident_token())
                .map_or(Expr::Missing, |tok| Expr::Name(Name::from(tok.text()))),
            SyntaxKind::CALL_EXPR => self.call(node, work),
            SyntaxKind::LIST_EXPR => {
                let args = ListExpr::cast(node.clone()).map(|l| l.args());
                Expr::List(self.queue_all(args.into_iter().flatten(), work))
            }
            SyntaxKind::MAP_EXPR => self.map(node, work),
            SyntaxKind::PAREN_EXPR => return self.paren(node, slot, work),
            SyntaxKind::PREFIX_EXPR => self.prefix(node, work),
            SyntaxKind::DEFINE_FUNCTION => self.define(node, work),
            SyntaxKind::BIN_EXPR => self.binary(node, work),
            // Not an expression node. The parser never produces one here, but
            // lowering stays total rather than asserting.
            _ => Expr::Missing,
        };
        self.set(slot, expr);
    }

    /// Reserve and queue a slot for each expression, returning the ids.
    fn queue_all(
        &mut self,
        nodes: impl IntoIterator<Item = SynExpr>,
        work: &mut Vec<(SyntaxNode, ExprId)>,
    ) -> Vec<ExprId> {
        let nodes: Vec<SyntaxNode> = nodes.into_iter().map(|e| e.syntax().clone()).collect();
        let ids: Vec<ExprId> = nodes
            .iter()
            .map(|n| self.reserve(node_span(n)))
            .collect::<Vec<_>>();
        for (node, id) in nodes.into_iter().zip(ids.iter().copied()).rev() {
            work.push((node, id));
        }
        ids
    }

    /// Reserve and queue one child slot.
    fn queue(&mut self, node: &SyntaxNode, work: &mut Vec<(SyntaxNode, ExprId)>) -> ExprId {
        let id = self.reserve(node_span(node));
        work.push((node.clone(), id));
        id
    }

    fn literal(&mut self, node: &SyntaxNode) -> Expr {
        let Some(token) = Literal::cast(node.clone()).and_then(|l| l.value_token()) else {
            return Expr::Missing;
        };
        match token.kind() {
            SyntaxKind::NUMBER => number_literal(token.text()),
            SyntaxKind::STRING => Expr::Str(string_literal(token.text())),
            _ => Expr::Missing,
        }
    }

    fn call(&mut self, node: &SyntaxNode, work: &mut Vec<(SyntaxNode, ExprId)>) -> Expr {
        let Some(call) = CallExpr::cast(node.clone()) else {
            return Expr::Missing;
        };
        let Some(callee) = call.callee().and_then(|c| c.ident_token()) else {
            return Expr::Missing;
        };
        let args = call.arg_list().map(|list| list.args());
        Expr::Call {
            callee: Name::from(callee.text()),
            callee_span: Span::from(callee.text_range()),
            args: self.queue_all(args.into_iter().flatten(), work),
        }
    }

    fn map(&mut self, node: &SyntaxNode, work: &mut Vec<(SyntaxNode, ExprId)>) -> Expr {
        let Some(map) = MapExpr::cast(node.clone()) else {
            return Expr::Missing;
        };
        // Collect first so the key and value slots of one entry stay adjacent.
        let pairs: Vec<(SyntaxNode, Option<SyntaxNode>)> = map
            .entries()
            .filter_map(|entry| {
                let key = entry.key()?;
                Some((
                    key.syntax().clone(),
                    entry.value().map(|v| v.syntax().clone()),
                ))
            })
            .collect();
        let mut entries = Vec::with_capacity(pairs.len());
        let mut queued = Vec::with_capacity(pairs.len() * 2);
        for (key, value) in pairs {
            let key_id = self.reserve(node_span(&key));
            let value_id = value.as_ref().map(|v| self.reserve(node_span(v)));
            entries.push(MapEntry {
                key: key_id,
                value: value_id,
            });
            queued.push((key, key_id));
            if let (Some(value), Some(id)) = (value, value_id) {
                queued.push((value, id));
            }
        }
        work.extend(queued.into_iter().rev());
        Expr::Map(entries)
    }

    /// Parentheses are transparent: the inner expression takes over `slot` and
    /// keeps the *outer* span, so a click anywhere between the brackets — on the
    /// brackets included — resolves to the expression they group.
    fn paren(&mut self, node: &SyntaxNode, slot: ExprId, work: &mut Vec<(SyntaxNode, ExprId)>) {
        match ParenExpr::cast(node.clone()).and_then(|p| p.expr()) {
            Some(inner) => work.push((inner.syntax().clone(), slot)),
            // `()` — an empty group, which evaluates to `null` like an empty
            // sequence does.
            None => self.set(
                slot,
                Expr::Seq {
                    sep: SeqSep::Comma,
                    exprs: Vec::new(),
                },
            ),
        }
    }

    fn prefix(&mut self, node: &SyntaxNode, work: &mut Vec<(SyntaxNode, ExprId)>) -> Expr {
        let Some(prefix) = PrefixExpr::cast(node.clone()) else {
            return Expr::Missing;
        };
        let op = match prefix.op_token().map(|t| t.kind()) {
            Some(SyntaxKind::MINUS) => PrefixOp::Neg,
            Some(SyntaxKind::PLUS) => PrefixOp::Pos,
            Some(SyntaxKind::BANG) => PrefixOp::Not,
            Some(SyntaxKind::DOT3) => PrefixOp::Spread,
            _ => return Expr::Missing,
        };
        let operand = match prefix.expr() {
            Some(inner) => self.queue(inner.syntax(), work),
            None => self.reserve(end_span(node)),
        };
        Expr::Prefix { op, operand }
    }

    fn define(&mut self, node: &SyntaxNode, work: &mut Vec<(SyntaxNode, ExprId)>) -> Expr {
        let Some(def) = DefineFunction::cast(node.clone()) else {
            return Expr::Missing;
        };
        let Some(name) = def.name().and_then(|n| n.ident_token()) else {
            return Expr::Missing;
        };
        let signature = def
            .args()
            .map_or_else(Signature::default, |args| Signature::from_arg_list(&args));
        let body = match def.body() {
            Some(body) => self.queue(body.syntax(), work),
            None => self.reserve(end_span(node)),
        };
        self.fns.push(FnDef {
            name: Name::from(name.text()),
            name_span: Span::from(name.text_range()),
            params: signature.params,
            captures: signature.captures,
            rest: signature.rest,
            body,
            span: node_span(node),
        });
        Expr::Define(FnId::from_index(self.fns.len() - 1))
    }

    fn binary(&mut self, node: &SyntaxNode, work: &mut Vec<(SyntaxNode, ExprId)>) -> Expr {
        let Some(bin) = BinExpr::cast(node.clone()) else {
            return Expr::Missing;
        };
        let Some(op_kind) = bin.op_token().map(|t| t.kind()) else {
            return Expr::Missing;
        };

        if let Some(sep) = seq_sep(op_kind) {
            let operands = flatten_seq(&bin, op_kind);
            let ids: Vec<ExprId> = operands
                .iter()
                .map(|n| self.reserve(node_span(n)))
                .collect();
            for (node, id) in operands.into_iter().zip(ids.iter().copied()).rev() {
                work.push((node, id));
            }
            return Expr::Seq { sep, exprs: ids };
        }

        let lhs = match bin.lhs() {
            Some(lhs) => self.queue(lhs.syntax(), work),
            None => self.reserve(start_span(node)),
        };
        let rhs = match bin.rhs() {
            Some(rhs) => self.queue(rhs.syntax(), work),
            None => self.reserve(end_span(node)),
        };

        match assign_op(op_kind) {
            Some(op) => Expr::Assign {
                op,
                target: lhs,
                value: rhs,
            },
            None => match bin_op(op_kind) {
                Some(op) => Expr::Binary { op, lhs, rhs },
                None => Expr::Missing,
            },
        }
    }
}

/// The binders a `DefineFunction`'s argument list declares.
///
/// Mirrors `scarpet_syntax::ast`'s `lower_params`: a plain name is positional,
/// `outer(x)` is a capture that does not consume a positional slot, and a single
/// `...rest` collects the surplus. The parser already restricts a definition's
/// left side to a call signature, so anything else here is a malformed program
/// that has been diagnosed upstream; lowering ignores it rather than failing.
#[derive(Default)]
struct Signature {
    params: Vec<Name>,
    captures: Vec<Name>,
    rest: Option<Name>,
}

impl Signature {
    fn from_arg_list(args: &ArgList) -> Signature {
        let mut signature = Signature::default();
        for arg in args.args() {
            match &arg {
                SynExpr::NameRef(name) => {
                    if let Some(token) = name.ident_token() {
                        signature.params.push(Name::from(token.text()));
                    }
                }
                SynExpr::PrefixExpr(prefix)
                    if prefix.op_token().map(|t| t.kind()) == Some(SyntaxKind::DOT3) =>
                {
                    // Only the first `...x` binds; `ast.rs` rejects a second one
                    // outright, and there is nothing sensible to do with it here.
                    if signature.rest.is_none()
                        && let Some(SynExpr::NameRef(inner)) = prefix.expr()
                        && let Some(token) = inner.ident_token()
                    {
                        signature.rest = Some(Name::from(token.text()));
                    }
                }
                SynExpr::CallExpr(call) => {
                    let word = call.callee().and_then(|c| c.ident_token());
                    if word.is_some_and(|w| w.text() == "outer")
                        && let Some(list) = call.arg_list()
                    {
                        let mut inner = list.args();
                        if let (Some(SynExpr::NameRef(bound)), None) = (inner.next(), inner.next())
                            && let Some(token) = bound.ident_token()
                        {
                            signature.captures.push(Name::from(token.text()));
                        }
                    }
                }
                _ => {}
            }
        }
        signature
    }
}

/// Walk a left-nested chain of one separator into its operands, left to right.
///
/// Only a chain of the *same* separator is merged, so `a; b, c; d` stays
/// `Seq(",", [Seq(";", [a, b]), Seq(";", [c, d])])` — which is what the
/// precedence ladder says it means, `,` binding looser than `;`.
///
/// A missing operand (the tail of a trailing `a;`) is dropped rather than
/// becoming a hole: a trailing separator is punctuation, not an empty statement.
fn flatten_seq(bin: &BinExpr, sep: SyntaxKind) -> Vec<SyntaxNode> {
    let mut operands = Vec::new();
    let mut current = bin.clone();
    loop {
        if let Some(rhs) = current.rhs() {
            operands.push(rhs.syntax().clone());
        }
        match current.lhs() {
            Some(SynExpr::BinExpr(inner)) if inner.op_token().map(|t| t.kind()) == Some(sep) => {
                current = inner;
            }
            Some(lhs) => {
                operands.push(lhs.syntax().clone());
                break;
            }
            None => break,
        }
    }
    operands.reverse();
    operands
}

fn seq_sep(kind: SyntaxKind) -> Option<SeqSep> {
    match kind {
        SyntaxKind::SEMICOLON => Some(SeqSep::Semi),
        SyntaxKind::COMMA => Some(SeqSep::Comma),
        _ => None,
    }
}

fn assign_op(kind: SyntaxKind) -> Option<AssignOp> {
    match kind {
        SyntaxKind::EQ => Some(AssignOp::Assign),
        SyntaxKind::PLUS_EQ => Some(AssignOp::Add),
        SyntaxKind::LT_GT => Some(AssignOp::Swap),
        _ => None,
    }
}

fn bin_op(kind: SyntaxKind) -> Option<BinOp> {
    let op = match kind {
        SyntaxKind::PLUS => BinOp::Add,
        SyntaxKind::MINUS => BinOp::Sub,
        SyntaxKind::STAR => BinOp::Mul,
        SyntaxKind::SLASH => BinOp::Div,
        SyntaxKind::PERCENT => BinOp::Rem,
        SyntaxKind::CARET => BinOp::Pow,
        SyntaxKind::EQ2 => BinOp::Eq,
        SyntaxKind::BANG_EQ => BinOp::Ne,
        SyntaxKind::LT => BinOp::Lt,
        SyntaxKind::LT_EQ => BinOp::Le,
        SyntaxKind::GT => BinOp::Gt,
        SyntaxKind::GT_EQ => BinOp::Ge,
        SyntaxKind::AMP2 => BinOp::And,
        SyntaxKind::PIPE2 => BinOp::Or,
        SyntaxKind::COLON => BinOp::Get,
        SyntaxKind::TILDE => BinOp::Match,
        _ => return None,
    };
    Some(op)
}

fn node_span(node: &SyntaxNode) -> Span {
    Span::from(node.text_range())
}

/// A zero-width span at a node's start, for a missing left operand.
fn start_span(node: &SyntaxNode) -> Span {
    let start = u32::from(node.text_range().start());
    Span::new(start, start)
}

/// A zero-width span at a node's end, for a missing right operand or body.
fn end_span(node: &SyntaxNode) -> Span {
    let end = u32::from(node.text_range().end());
    Span::new(end, end)
}

/// Parse a numeric literal, matching `scarpet_vm::Value::from_number_literal`:
/// hexadecimal and plain integers stay integers, everything else that parses
/// becomes a double, and an unparsable literal becomes `NaN` rather than an
/// error (the lexer only ever hands us shapes that do parse).
fn number_literal(text: &str) -> Expr {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))
        && let Ok(value) = i64::from_str_radix(hex, 16)
    {
        return Expr::Int(value);
    }
    if let Ok(value) = text.parse::<i64>() {
        return Expr::Int(value);
    }
    match text.parse::<f64>() {
        Ok(value) => Expr::Double(value),
        Err(_) => Expr::Double(f64::NAN),
    }
}

/// Strip a string literal's quotes and expand its escapes, matching
/// `scarpet_vm::Value::from_string_literal`: `\n` and `\t` are newline and tab,
/// and every other `\x` keeps `x` verbatim (so `\\` and `\'` are a backslash and
/// a quote).
fn string_literal(text: &str) -> String {
    let inner = text
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .unwrap_or(text);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            // A trailing backslash; the lexer's string rule never emits one.
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarpet_syntax::parser::parse;

    fn hir(src: &str) -> Hir {
        let tree = parse(src).expect("parse error");
        lower(&tree)
    }

    /// The root expression of `src`, for a shape assertion.
    fn root_of(src: &str) -> (Hir, Expr) {
        let hir = hir(src);
        let root = hir.expr(hir.root()).clone();
        (hir, root)
    }

    #[test]
    fn literals_match_the_vm() {
        assert_eq!(root_of("42").1, Expr::Int(42));
        assert_eq!(root_of("0xff").1, Expr::Int(255));
        assert_eq!(root_of("1.5").1, Expr::Double(1.5));
        assert_eq!(root_of("'hi'").1, Expr::Str("hi".to_owned()));
        assert_eq!(root_of(r"'a\nb'").1, Expr::Str("a\nb".to_owned()));
        // Every other `\x` keeps `x`, so `\\` and `\'` are literal.
        assert_eq!(root_of(r"'a\\b\'c'").1, Expr::Str(r"a\b'c".to_owned()));
    }

    #[test]
    fn semicolon_chains_flatten() {
        let (_, root) = root_of("a; b; c; d");
        let Expr::Seq { sep, exprs } = root else {
            panic!("expected a sequence, got {root:?}");
        };
        assert_eq!(sep, SeqSep::Semi);
        assert_eq!(exprs.len(), 4);
    }

    /// `,` binds looser than `;`, so a mixed chain nests rather than merging.
    #[test]
    fn mixed_separators_nest_by_precedence() {
        let (hir, root) = root_of("a; b, c; d");
        let Expr::Seq { sep, exprs } = &root else {
            panic!("expected a sequence, got {root:?}");
        };
        assert_eq!(*sep, SeqSep::Comma);
        assert_eq!(exprs.len(), 2);
        for id in exprs {
            let Expr::Seq { sep, exprs } = hir.expr(*id) else {
                panic!("expected a nested `;` sequence");
            };
            assert_eq!(*sep, SeqSep::Semi);
            assert_eq!(exprs.len(), 2);
        }
    }

    /// A trailing separator is punctuation, not an empty statement.
    #[test]
    fn a_trailing_separator_adds_no_operand() {
        let (_, root) = root_of("a; b;");
        let Expr::Seq { exprs, .. } = root else {
            panic!("expected a sequence");
        };
        assert_eq!(exprs.len(), 2);
    }

    /// The left-nested spine must not be walked recursively — this is roughly
    /// the statement count of the largest corpus file, an order of magnitude
    /// over.
    #[test]
    fn long_chains_do_not_overflow_the_stack() {
        for chain in [";1", "+1"] {
            let src = format!("1{}", chain.repeat(5_000));
            let hir = hir(&src);
            assert!(hir.len() >= 5_000);
        }
    }

    #[test]
    fn parentheses_are_transparent_but_keep_their_span() {
        let (hir, root) = root_of("(a + b)");
        assert!(matches!(root, Expr::Binary { op: BinOp::Add, .. }));
        // The span covers the brackets, so a cursor on `(` still resolves.
        assert_eq!(hir.span(hir.root()), Span::new(0, 7));
        assert_eq!(root_of("((a))").1, Expr::Name(Name::from("a")));
    }

    #[test]
    fn assignment_is_not_a_binary_operator() {
        assert!(matches!(
            root_of("a = 1").1,
            Expr::Assign {
                op: AssignOp::Assign,
                ..
            }
        ));
        assert!(matches!(
            root_of("a += 1").1,
            Expr::Assign {
                op: AssignOp::Add,
                ..
            }
        ));
        assert!(matches!(
            root_of("a <> b").1,
            Expr::Assign {
                op: AssignOp::Swap,
                ..
            }
        ));
    }

    /// `l(..)` and `[..]`, `m(..)` and `{..}` are the same construct spelled two
    /// ways; the HIR keeps only the construct.
    #[test]
    fn both_container_spellings_lower_alike() {
        let bracket = root_of("[1, 2]").1;
        let call = root_of("l(1, 2)").1;
        assert!(matches!(&bracket, Expr::List(items) if items.len() == 2));
        assert_eq!(
            std::mem::discriminant(&bracket),
            std::mem::discriminant(&call)
        );

        let brace = root_of("{1 -> 2}").1;
        let m_call = root_of("m(1 -> 2)").1;
        assert!(matches!(&brace, Expr::Map(entries) if entries.len() == 1));
        assert_eq!(
            std::mem::discriminant(&brace),
            std::mem::discriminant(&m_call)
        );
    }

    #[test]
    fn a_bare_map_entry_has_no_value() {
        let (_, root) = root_of("{pair}");
        let Expr::Map(entries) = root else {
            panic!("expected a map");
        };
        assert_eq!(entries.len(), 1);
        assert!(entries[0].value.is_none());
    }

    /// Arguments are siblings under `ARG_LIST`, so a call never contains a comma
    /// sequence — the flattener must not expect one.
    #[test]
    fn call_arguments_are_not_a_comma_sequence() {
        let (_, root) = root_of("foo(1, 2, 3)");
        let Expr::Call { callee, args, .. } = root else {
            panic!("expected a call");
        };
        assert_eq!(&*callee, "foo");
        assert_eq!(args.len(), 3);
    }

    #[test]
    fn a_signature_splits_into_params_captures_and_rest() {
        let hir = hir("foo(a, outer(o), b, ...rest) -> a");
        let def = &hir.fns()[0];
        assert_eq!(&*def.name, "foo");
        assert_eq!(
            def.params.iter().map(|p| &**p).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(def.captures.iter().map(|c| &**c).collect::<Vec<_>>(), ["o"]);
        assert_eq!(def.rest.as_deref(), Some("rest"));
        // Captures do not consume a positional slot.
        assert!(def.accepts(2));
        assert!(def.accepts(5));
        assert!(!def.accepts(1));
    }

    /// A definition nested in another body is still global, so it lands in the
    /// same flat list.
    #[test]
    fn nested_definitions_are_collected_flatly() {
        let hir = hir("outer_fn() -> (inner_fn() -> 1); outer_fn()");
        let names: Vec<&str> = hir.fns().iter().map(|f| &*f.name).collect();
        assert_eq!(names, ["outer_fn", "inner_fn"]);
    }

    #[test]
    fn every_node_has_a_span_inside_the_source() {
        let src = "fib(n) -> if(n < 2, n, fib(n-1) + fib(n-2)); print(fib(10))";
        let hir = hir(src);
        for id in hir.ids() {
            let span = hir.span(id);
            assert!(span.start <= span.end);
            assert!(
                span.end as usize <= src.len(),
                "{span:?} escapes the source"
            );
        }
    }

    #[test]
    fn a_parent_always_precedes_its_children() {
        let hir = hir("a = [1, {2 -> 3}]; foo(a, -a, a: 0)");
        for parent in hir.ids() {
            for child in hir.children(parent) {
                assert!(child > parent, "{child:?} is not after {parent:?}");
            }
        }
    }

    #[test]
    fn expr_at_finds_the_innermost_node() {
        let src = "foo(1 + 2)";
        let hir = hir(src);
        // The `1` token starts at offset 4.
        let id = hir.expr_at(4).expect("a node covers the literal");
        assert_eq!(hir.expr(id), &Expr::Int(1));
        // The callee's own span belongs to the call node.
        let id = hir.expr_at(0).expect("a node covers the callee");
        assert!(matches!(hir.expr(id), Expr::Call { .. }));
    }

    /// The parser rejects an empty program outright, so the "no expression"
    /// arm is reachable only through `analyze_syntax` on a node that holds no
    /// expression. It must still produce a program rather than fail — an empty
    /// sequence, which evaluates to `null` exactly as an empty group does.
    #[test]
    fn a_node_without_an_expression_lowers_to_an_empty_sequence() {
        let tree = parse("f()").expect("parse error");
        let arg_list = tree
            .descendants()
            .find(|node| node.kind() == SyntaxKind::ARG_LIST)
            .expect("the call has an argument list");
        let hir = lower(&arg_list);
        assert_eq!(
            hir.expr(hir.root()),
            &Expr::Seq {
                sep: SeqSep::Semi,
                exprs: Vec::new()
            }
        );
    }
}
