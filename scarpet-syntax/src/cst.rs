//! Typed accessors over the rowan syntax tree.
//!
//! Each node kind of the tree gets a struct (plus the `Expr` enum over all
//! expression kinds) implementing [`rowan::ast::AstNode`], with accessors for
//! its children. The definitions are generated from `scarpet.ungram` by
//! `cargo xtask codegen` — edit the grammar and regenerate rather than
//! editing `generated.rs`.

mod generated;

pub use generated::*;

#[cfg(test)]
mod tests {
    use rowan::Language;
    use rowan::ast::AstNode;

    use super::{ArgList, BinExpr, CallExpr, Expr, Literal, SourceFile};
    use crate::syntax::{ScarpetLanguage, SyntaxNode};
    use crate::syntax_kind::SyntaxKind;

    fn raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        ScarpetLanguage::kind_to_raw(kind)
    }

    /// Builds `SOURCE_FILE(BIN_EXPR(LITERAL("1") " " "+" " " LITERAL("2")))`,
    /// i.e. the tree for `1 + 2`.
    fn one_plus_two() -> SyntaxNode {
        let mut builder = rowan::GreenNodeBuilder::new();
        builder.start_node(raw(SyntaxKind::SOURCE_FILE));
        builder.start_node(raw(SyntaxKind::BIN_EXPR));
        builder.start_node(raw(SyntaxKind::LITERAL));
        builder.token(raw(SyntaxKind::NUMBER), "1");
        builder.finish_node();
        builder.token(raw(SyntaxKind::WHITESPACE), " ");
        builder.token(raw(SyntaxKind::PLUS), "+");
        builder.token(raw(SyntaxKind::WHITESPACE), " ");
        builder.start_node(raw(SyntaxKind::LITERAL));
        builder.token(raw(SyntaxKind::NUMBER), "2");
        builder.finish_node();
        builder.finish_node();
        builder.finish_node();
        SyntaxNode::new_root(builder.finish())
    }

    /// Builds `CALL_EXPR(NAME_REF("f") ARG_LIST("(" LITERAL("1") ")"))`,
    /// i.e. the tree for `f(1)`.
    fn call_f_of_one() -> SyntaxNode {
        let mut builder = rowan::GreenNodeBuilder::new();
        builder.start_node(raw(SyntaxKind::CALL_EXPR));
        builder.start_node(raw(SyntaxKind::NAME_REF));
        builder.token(raw(SyntaxKind::IDENT), "f");
        builder.finish_node();
        builder.start_node(raw(SyntaxKind::ARG_LIST));
        builder.token(raw(SyntaxKind::OPEN_PAREN), "(");
        builder.start_node(raw(SyntaxKind::LITERAL));
        builder.token(raw(SyntaxKind::NUMBER), "1");
        builder.finish_node();
        builder.token(raw(SyntaxKind::CLOSE_PAREN), ")");
        builder.finish_node();
        builder.finish_node();
        SyntaxNode::new_root(builder.finish())
    }

    #[test]
    fn typed_accessors_on_a_hand_built_tree() {
        let root = one_plus_two();
        assert_eq!(root.text().to_string(), "1 + 2");

        let source_file = SourceFile::cast(root).expect("SOURCE_FILE casts to SourceFile");
        let expr = source_file.expr().expect("source file has an expression");
        let Expr::BinExpr(bin_expr) = expr else {
            panic!("expected Expr::BinExpr, got {expr:?}");
        };

        let lhs = bin_expr.lhs().expect("bin expr has a lhs");
        let rhs = bin_expr.rhs().expect("bin expr has a rhs");
        assert_eq!(lhs.syntax().text().to_string(), "1");
        assert_eq!(rhs.syntax().text().to_string(), "2");

        let Expr::Literal(lhs_literal) = &lhs else {
            panic!("expected Expr::Literal, got {lhs:?}");
        };
        let number = lhs_literal.number_token().expect("literal wraps a number");
        assert_eq!(number.text(), "1");
        assert_eq!(lhs_literal.string_token(), None);
    }

    #[test]
    fn casting_rejects_other_kinds_and_enum_casts_work() {
        let root = one_plus_two();
        let bin_expr_node = root.first_child().expect("source file has a child");

        // Wrong typed-struct casts return None.
        assert!(SourceFile::cast(bin_expr_node.clone()).is_none());
        assert!(Literal::cast(bin_expr_node.clone()).is_none());
        assert!(Expr::cast(root.clone()).is_none());

        // Right ones succeed, both directly and through the `Expr` enum.
        assert!(BinExpr::cast(bin_expr_node.clone()).is_some());
        let expr = Expr::cast(bin_expr_node.clone()).expect("BIN_EXPR casts to Expr");
        assert!(matches!(expr, Expr::BinExpr(_)));
        assert_eq!(expr.syntax(), &bin_expr_node);

        // can_cast agrees.
        assert!(Expr::can_cast(SyntaxKind::BIN_EXPR));
        assert!(Expr::can_cast(SyntaxKind::EMPTY_ARG));
        assert!(!Expr::can_cast(SyntaxKind::SOURCE_FILE));
        assert!(!Expr::can_cast(SyntaxKind::ARG_LIST));

        // From<variant> for Expr.
        let bin_expr = BinExpr::cast(bin_expr_node).unwrap();
        let as_expr: Expr = bin_expr.clone().into();
        assert_eq!(as_expr, Expr::BinExpr(bin_expr));
    }

    #[test]
    fn call_expr_and_arg_list_accessors() {
        let root = call_f_of_one();
        assert_eq!(root.text().to_string(), "f(1)");

        let call = CallExpr::cast(root).expect("CALL_EXPR casts to CallExpr");
        let name_ref = call.name_ref().expect("call has a callee name");
        assert_eq!(
            name_ref.ident_token().expect("name is an ident").text(),
            "f"
        );

        let arg_list: ArgList = call.arg_list().expect("call has an arg list");
        assert_eq!(arg_list.open_paren_token().expect("has `(`").text(), "(");
        assert_eq!(arg_list.close_paren_token().expect("has `)`").text(), ")");
        let args: Vec<Expr> = arg_list.exprs().collect();
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].syntax().text().to_string(), "1");
    }
}
