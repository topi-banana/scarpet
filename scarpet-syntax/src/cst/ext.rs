//! Hand-written extensions over the generated typed nodes.
//!
//! `generated.rs` only knows what the ungrammar can express: which child
//! kinds a node has. This module layers semantic accessors on top — most
//! importantly classifying the operator token of a [`BinExpr`] /
//! [`PrefixExpr`] into [`BinOpKind`] / [`PrefixOpKind`], mirroring the
//! `BinOp` / `UnaryOp` enums of the legacy CST (`crate::parser`).

use rowan::ast::AstChildren;

use crate::cst::{ArgList, BinExpr, CommaChain, Expr, ListExpr, MapExpr, PrefixExpr, SemiChain};
use crate::syntax::{SyntaxNode, SyntaxToken};
use crate::syntax_kind::SyntaxKind;

/// The operator of a [`BinExpr`].
///
/// Covers every infix operator except the chain separators `,` and `;`,
/// which form their own n-ary nodes ([`CommaChain`] / [`SemiChain`]) rather
/// than binary ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOpKind {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `^`
    Pow,
    /// `==`
    Eq,
    /// `!=`
    NotEq,
    /// `<`
    Lt,
    /// `<=`
    LtEq,
    /// `>`
    Gt,
    /// `>=`
    GtEq,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `~`
    Match,
    /// `:`
    Get,
    /// `=`
    Assign,
    /// `+=`
    AddAssign,
    /// `<>`
    Swap,
    /// `->`
    Arrow,
}

impl BinOpKind {
    /// Classifies a token kind as a binary operator, or `None` if the kind
    /// is not one.
    pub const fn from_token(kind: SyntaxKind) -> Option<BinOpKind> {
        Some(match kind {
            SyntaxKind::PLUS => BinOpKind::Add,
            SyntaxKind::MINUS => BinOpKind::Sub,
            SyntaxKind::STAR => BinOpKind::Mul,
            SyntaxKind::SLASH => BinOpKind::Div,
            SyntaxKind::PERCENT => BinOpKind::Rem,
            SyntaxKind::CARET => BinOpKind::Pow,
            SyntaxKind::EQ_EQ => BinOpKind::Eq,
            SyntaxKind::BANG_EQ => BinOpKind::NotEq,
            SyntaxKind::LT => BinOpKind::Lt,
            SyntaxKind::LT_EQ => BinOpKind::LtEq,
            SyntaxKind::GT => BinOpKind::Gt,
            SyntaxKind::GT_EQ => BinOpKind::GtEq,
            SyntaxKind::AND_AND => BinOpKind::And,
            SyntaxKind::OR_OR => BinOpKind::Or,
            SyntaxKind::TILDE => BinOpKind::Match,
            SyntaxKind::COLON => BinOpKind::Get,
            SyntaxKind::EQ => BinOpKind::Assign,
            SyntaxKind::PLUS_EQ => BinOpKind::AddAssign,
            SyntaxKind::SWAP => BinOpKind::Swap,
            SyntaxKind::ARROW => BinOpKind::Arrow,
            _ => return None,
        })
    }
}

/// The operator of a [`PrefixExpr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixOpKind {
    /// `-`
    Neg,
    /// `+`
    Pos,
    /// `!`
    Not,
    /// `...`
    Unpack,
}

impl PrefixOpKind {
    /// Classifies a token kind as a prefix operator, or `None` if the kind
    /// is not one.
    pub const fn from_token(kind: SyntaxKind) -> Option<PrefixOpKind> {
        Some(match kind {
            SyntaxKind::MINUS => PrefixOpKind::Neg,
            SyntaxKind::PLUS => PrefixOpKind::Pos,
            SyntaxKind::BANG => PrefixOpKind::Not,
            SyntaxKind::ELLIPSIS => PrefixOpKind::Unpack,
            _ => return None,
        })
    }
}

/// The first child token of `node` that `classify` accepts, paired with its
/// classification.
fn first_op_token<T>(
    node: &SyntaxNode,
    classify: fn(SyntaxKind) -> Option<T>,
) -> Option<(SyntaxToken, T)> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find_map(|token| {
            let kind = classify(token.kind())?;
            Some((token, kind))
        })
}

impl BinExpr {
    /// The operator: the first child *token* that classifies as a
    /// [`BinOpKind`].
    ///
    /// Operand tokens cannot be picked up by mistake — operands are child
    /// *nodes*, so e.g. the `-` of the right operand in `a + -b` sits inside
    /// a nested `PREFIX_EXPR`, not directly under the `BIN_EXPR`.
    pub fn op(&self) -> Option<(SyntaxToken, BinOpKind)> {
        first_op_token(&self.syntax, BinOpKind::from_token)
    }
}

impl PrefixExpr {
    /// The operator: the first child *token* that classifies as a
    /// [`PrefixOpKind`].
    pub fn op(&self) -> Option<(SyntaxToken, PrefixOpKind)> {
        first_op_token(&self.syntax, PrefixOpKind::from_token)
    }
}

impl SemiChain {
    /// The chained item nodes in source order, skipping the `;` separator
    /// tokens, trivia and any non-expression (`ERROR`) nodes.
    pub fn items(&self) -> AstChildren<Expr> {
        self.exprs()
    }
}

impl CommaChain {
    /// The chained item nodes in source order, skipping the `,` separator
    /// tokens, trivia and any non-expression (`ERROR`) nodes.
    pub fn items(&self) -> AstChildren<Expr> {
        self.exprs()
    }
}

impl ArgList {
    /// The argument nodes in source order, including [`EmptyArg`] nodes for
    /// omitted slots (`f(a,,b)`), skipping `,` tokens, parentheses, trivia
    /// and any non-expression (`ERROR`) nodes.
    ///
    /// [`EmptyArg`]: crate::cst::EmptyArg
    pub fn args(&self) -> AstChildren<Expr> {
        self.exprs()
    }
}

impl ListExpr {
    /// The element nodes in source order, including [`EmptyArg`] nodes for
    /// omitted slots, skipping `,` tokens, brackets, trivia and any
    /// non-expression (`ERROR`) nodes.
    ///
    /// [`EmptyArg`]: crate::cst::EmptyArg
    pub fn args(&self) -> AstChildren<Expr> {
        self.exprs()
    }
}

impl MapExpr {
    /// The entry nodes in source order, including [`EmptyArg`] nodes for
    /// omitted slots, skipping `,` tokens, braces, trivia and any
    /// non-expression (`ERROR`) nodes.
    ///
    /// [`EmptyArg`]: crate::cst::EmptyArg
    pub fn args(&self) -> AstChildren<Expr> {
        self.exprs()
    }
}

/// Whether `node` is an atom: a leaf-like expression with no sub-expressions
/// (`LITERAL`, `NAME_REF` or `EMPTY_ARG`).
pub fn is_atom(node: &SyntaxNode) -> bool {
    matches!(
        node.kind(),
        SyntaxKind::LITERAL | SyntaxKind::NAME_REF | SyntaxKind::EMPTY_ARG
    )
}

#[cfg(test)]
mod tests {
    use rowan::ast::AstNode;

    use super::{BinOpKind, PrefixOpKind, is_atom};
    use crate::cst::{
        ArgList, BinExpr, CommaChain, Expr, ListExpr, MapExpr, PrefixExpr, SemiChain,
    };
    use crate::syntax::testing::{TreeBuilder, tree};
    use crate::syntax_kind::SyntaxKind;

    /// `BIN_EXPR(LITERAL("1") WS <op> WS LITERAL("2"))`.
    fn bin_expr(op_kind: SyntaxKind, op_text: &str) -> BinExpr {
        let root = tree(SyntaxKind::BIN_EXPR, |b| {
            b.node(SyntaxKind::LITERAL, |b| b.token(SyntaxKind::NUMBER, "1"));
            b.token(SyntaxKind::WHITESPACE, " ");
            b.token(op_kind, op_text);
            b.token(SyntaxKind::WHITESPACE, " ");
            b.node(SyntaxKind::LITERAL, |b| b.token(SyntaxKind::NUMBER, "2"));
        });
        BinExpr::cast(root).unwrap()
    }

    fn literal(b: &mut TreeBuilder, text: &str) {
        b.node(SyntaxKind::LITERAL, |b| b.token(SyntaxKind::NUMBER, text));
    }

    #[test]
    fn bin_expr_op_over_every_operator() {
        let cases: &[(SyntaxKind, &str, BinOpKind)] = &[
            (SyntaxKind::PLUS, "+", BinOpKind::Add),
            (SyntaxKind::MINUS, "-", BinOpKind::Sub),
            (SyntaxKind::STAR, "*", BinOpKind::Mul),
            (SyntaxKind::SLASH, "/", BinOpKind::Div),
            (SyntaxKind::PERCENT, "%", BinOpKind::Rem),
            (SyntaxKind::CARET, "^", BinOpKind::Pow),
            (SyntaxKind::EQ_EQ, "==", BinOpKind::Eq),
            (SyntaxKind::BANG_EQ, "!=", BinOpKind::NotEq),
            (SyntaxKind::LT, "<", BinOpKind::Lt),
            (SyntaxKind::LT_EQ, "<=", BinOpKind::LtEq),
            (SyntaxKind::GT, ">", BinOpKind::Gt),
            (SyntaxKind::GT_EQ, ">=", BinOpKind::GtEq),
            (SyntaxKind::AND_AND, "&&", BinOpKind::And),
            (SyntaxKind::OR_OR, "||", BinOpKind::Or),
            (SyntaxKind::TILDE, "~", BinOpKind::Match),
            (SyntaxKind::COLON, ":", BinOpKind::Get),
            (SyntaxKind::EQ, "=", BinOpKind::Assign),
            (SyntaxKind::PLUS_EQ, "+=", BinOpKind::AddAssign),
            (SyntaxKind::SWAP, "<>", BinOpKind::Swap),
            (SyntaxKind::ARROW, "->", BinOpKind::Arrow),
        ];
        for &(token_kind, token_text, expected) in cases {
            let (token, op) = bin_expr(token_kind, token_text)
                .op()
                .unwrap_or_else(|| panic!("no op found for {token_kind:?}"));
            assert_eq!(op, expected, "wrong BinOpKind for {token_kind:?}");
            assert_eq!(token.kind(), token_kind);
            assert_eq!(token.text(), token_text);
            assert_eq!(BinOpKind::from_token(token_kind), Some(expected));
        }
    }

    #[test]
    fn bin_op_from_token_rejects_non_operators() {
        for kind in [
            SyntaxKind::COMMA,
            SyntaxKind::SEMICOLON,
            SyntaxKind::BANG,
            SyntaxKind::ELLIPSIS,
            SyntaxKind::DOT,
            SyntaxKind::NUMBER,
            SyntaxKind::WHITESPACE,
            SyntaxKind::BIN_EXPR,
        ] {
            assert_eq!(BinOpKind::from_token(kind), None, "{kind:?}");
        }
    }

    #[test]
    fn bin_expr_op_skips_operators_nested_in_operands() {
        // `1 + -2`: the rhs `-` lives inside a PREFIX_EXPR node, so `op()`
        // must report the `+`.
        let root = tree(SyntaxKind::BIN_EXPR, |b| {
            literal(b, "1");
            b.token(SyntaxKind::WHITESPACE, " ");
            b.token(SyntaxKind::PLUS, "+");
            b.token(SyntaxKind::WHITESPACE, " ");
            b.node(SyntaxKind::PREFIX_EXPR, |b| {
                b.token(SyntaxKind::MINUS, "-");
                literal(b, "2");
            });
        });
        let (token, op) = BinExpr::cast(root).unwrap().op().unwrap();
        assert_eq!(op, BinOpKind::Add);
        assert_eq!(token.text(), "+");
    }

    #[test]
    fn bin_expr_without_operator_token_has_no_op() {
        let root = tree(SyntaxKind::BIN_EXPR, |b| {
            literal(b, "1");
            b.token(SyntaxKind::WHITESPACE, " ");
            literal(b, "2");
        });
        assert_eq!(BinExpr::cast(root).unwrap().op(), None);
    }

    #[test]
    fn prefix_expr_op_over_every_operator() {
        let cases: &[(SyntaxKind, &str, PrefixOpKind)] = &[
            (SyntaxKind::MINUS, "-", PrefixOpKind::Neg),
            (SyntaxKind::PLUS, "+", PrefixOpKind::Pos),
            (SyntaxKind::BANG, "!", PrefixOpKind::Not),
            (SyntaxKind::ELLIPSIS, "...", PrefixOpKind::Unpack),
        ];
        for &(token_kind, token_text, expected) in cases {
            let root = tree(SyntaxKind::PREFIX_EXPR, |b| {
                b.token(token_kind, token_text);
                literal(b, "1");
            });
            let (token, op) = PrefixExpr::cast(root)
                .unwrap()
                .op()
                .unwrap_or_else(|| panic!("no op found for {token_kind:?}"));
            assert_eq!(op, expected, "wrong PrefixOpKind for {token_kind:?}");
            assert_eq!(token.kind(), token_kind);
            assert_eq!(token.text(), token_text);
            assert_eq!(PrefixOpKind::from_token(token_kind), Some(expected));
        }
    }

    #[test]
    fn prefix_op_from_token_rejects_non_operators() {
        for kind in [
            SyntaxKind::STAR,
            SyntaxKind::TILDE,
            SyntaxKind::NUMBER,
            SyntaxKind::PREFIX_EXPR,
        ] {
            assert_eq!(PrefixOpKind::from_token(kind), None, "{kind:?}");
        }
    }

    #[test]
    fn prefix_expr_without_operator_token_has_no_op() {
        let root = tree(SyntaxKind::PREFIX_EXPR, |b| literal(b, "1"));
        assert_eq!(PrefixExpr::cast(root).unwrap().op(), None);
    }

    #[test]
    fn semi_chain_items_skip_separators_and_trivia() {
        // `1; 2; /*…*/ 3;` (with a trailing `;`).
        let root = tree(SyntaxKind::SEMI_CHAIN, |b| {
            literal(b, "1");
            b.token(SyntaxKind::SEMICOLON, ";");
            b.token(SyntaxKind::WHITESPACE, " ");
            literal(b, "2");
            b.token(SyntaxKind::SEMICOLON, ";");
            b.token(SyntaxKind::NEWLINE, "\n");
            b.token(SyntaxKind::COMMENT, "// note");
            b.token(SyntaxKind::NEWLINE, "\n");
            literal(b, "3");
            b.token(SyntaxKind::SEMICOLON, ";");
        });
        let items: Vec<String> = SemiChain::cast(root)
            .unwrap()
            .items()
            .map(|item| item.syntax().text().to_string())
            .collect();
        assert_eq!(items, ["1", "2", "3"]);
    }

    #[test]
    fn comma_chain_items_skip_separators_and_trivia() {
        let root = tree(SyntaxKind::COMMA_CHAIN, |b| {
            literal(b, "1");
            b.token(SyntaxKind::COMMA, ",");
            b.token(SyntaxKind::WHITESPACE, " ");
            literal(b, "2");
            b.token(SyntaxKind::COMMA, ",");
            b.token(SyntaxKind::WHITESPACE, " ");
            literal(b, "3");
        });
        let items: Vec<String> = CommaChain::cast(root)
            .unwrap()
            .items()
            .map(|item| item.syntax().text().to_string())
            .collect();
        assert_eq!(items, ["1", "2", "3"]);
    }

    #[test]
    fn arg_list_args_include_empty_arg() {
        // `(1,,2)` — the omitted middle slot is a zero-width EMPTY_ARG node.
        let root = tree(SyntaxKind::ARG_LIST, |b| {
            b.token(SyntaxKind::OPEN_PAREN, "(");
            literal(b, "1");
            b.token(SyntaxKind::COMMA, ",");
            b.node(SyntaxKind::EMPTY_ARG, |_| {});
            b.token(SyntaxKind::COMMA, ",");
            literal(b, "2");
            b.token(SyntaxKind::CLOSE_PAREN, ")");
        });
        let args: Vec<Expr> = ArgList::cast(root).unwrap().args().collect();
        assert_eq!(args.len(), 3);
        assert!(matches!(args[0], Expr::Literal(_)));
        assert!(matches!(args[1], Expr::EmptyArg(_)));
        assert!(matches!(args[2], Expr::Literal(_)));
    }

    #[test]
    fn list_expr_args_include_empty_arg() {
        // `[1,,2]` — the omitted middle slot is a zero-width EMPTY_ARG node.
        let root = tree(SyntaxKind::LIST_EXPR, |b| {
            b.token(SyntaxKind::OPEN_BRACK, "[");
            literal(b, "1");
            b.token(SyntaxKind::COMMA, ",");
            b.node(SyntaxKind::EMPTY_ARG, |_| {});
            b.token(SyntaxKind::COMMA, ",");
            b.token(SyntaxKind::WHITESPACE, " ");
            literal(b, "2");
            b.token(SyntaxKind::CLOSE_BRACK, "]");
        });
        let args: Vec<Expr> = ListExpr::cast(root).unwrap().args().collect();
        assert_eq!(args.len(), 3);
        assert!(matches!(args[0], Expr::Literal(_)));
        assert!(matches!(args[1], Expr::EmptyArg(_)));
        assert!(matches!(args[2], Expr::Literal(_)));
    }

    #[test]
    fn map_expr_args_include_empty_arg() {
        // `{'a' -> 1,,}` — a BIN_EXPR entry followed by an omitted slot.
        let root = tree(SyntaxKind::MAP_EXPR, |b| {
            b.token(SyntaxKind::OPEN_BRACE, "{");
            b.node(SyntaxKind::BIN_EXPR, |b| {
                b.node(SyntaxKind::LITERAL, |b| b.token(SyntaxKind::STRING, "'a'"));
                b.token(SyntaxKind::WHITESPACE, " ");
                b.token(SyntaxKind::ARROW, "->");
                b.token(SyntaxKind::WHITESPACE, " ");
                literal(b, "1");
            });
            b.token(SyntaxKind::COMMA, ",");
            b.node(SyntaxKind::EMPTY_ARG, |_| {});
            b.token(SyntaxKind::COMMA, ",");
            b.token(SyntaxKind::CLOSE_BRACE, "}");
        });
        let args: Vec<Expr> = MapExpr::cast(root).unwrap().args().collect();
        assert_eq!(args.len(), 2);
        let Expr::BinExpr(entry) = &args[0] else {
            panic!("expected Expr::BinExpr, got {:?}", args[0]);
        };
        assert_eq!(entry.op().unwrap().1, BinOpKind::Arrow);
        assert!(matches!(args[1], Expr::EmptyArg(_)));
    }

    #[test]
    fn is_atom_truth_table() {
        let atom_roots = [
            tree(SyntaxKind::LITERAL, |b| b.token(SyntaxKind::NUMBER, "1")),
            tree(SyntaxKind::NAME_REF, |b| b.token(SyntaxKind::IDENT, "x")),
            tree(SyntaxKind::EMPTY_ARG, |_| {}),
        ];
        for root in &atom_roots {
            assert!(is_atom(root), "{:?} should be an atom", root.kind());
        }

        let non_atom_roots = [
            tree(SyntaxKind::SOURCE_FILE, |_| {}),
            tree(SyntaxKind::BIN_EXPR, |_| {}),
            tree(SyntaxKind::CALL_EXPR, |_| {}),
            tree(SyntaxKind::PAREN_EXPR, |_| {}),
            tree(SyntaxKind::ERROR, |_| {}),
        ];
        for root in &non_atom_roots {
            assert!(!is_atom(root), "{:?} should not be an atom", root.kind());
        }
    }
}
