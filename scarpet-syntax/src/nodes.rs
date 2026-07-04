//! Typed accessors over the rowan syntax tree.
//!
//! The node structs and their accessors live in [`generated`], produced from
//! `scarpet.ungram` by the sourcegen test (`tests/sourcegen.rs`) — edit the
//! grammar, run `cargo test -p scarpet-syntax`, and commit the regenerated
//! file. This module supplies the [`AstNode`] trait and the lookup helpers
//! the generated code leans on.

mod generated;

pub use generated::*;

use std::marker::PhantomData;

use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxNodeChildren, SyntaxToken};

/// A typed view of a [`SyntaxNode`]. Casting is free — a typed node is the
/// same green data behind a kind check.
pub trait AstNode: Sized {
    fn can_cast(kind: SyntaxKind) -> bool;
    fn cast(syntax: SyntaxNode) -> Option<Self>;
    fn syntax(&self) -> &SyntaxNode;
}

/// An iterator over the children of a node that cast to `N`.
#[derive(Debug, Clone)]
pub struct AstChildren<N> {
    inner: SyntaxNodeChildren,
    _phantom: PhantomData<N>,
}

impl<N: AstNode> Iterator for AstChildren<N> {
    type Item = N;

    fn next(&mut self) -> Option<N> {
        self.inner.find_map(N::cast)
    }
}

/// Lookup helpers for the generated accessors.
mod support {
    use super::{AstChildren, AstNode, PhantomData, SyntaxKind, SyntaxNode, SyntaxToken};

    pub(super) fn child<N: AstNode>(parent: &SyntaxNode) -> Option<N> {
        parent.children().find_map(N::cast)
    }

    pub(super) fn nth_child<N: AstNode>(parent: &SyntaxNode, n: usize) -> Option<N> {
        parent.children().filter_map(N::cast).nth(n)
    }

    pub(super) fn children<N: AstNode>(parent: &SyntaxNode) -> AstChildren<N> {
        AstChildren {
            inner: parent.children(),
            _phantom: PhantomData,
        }
    }

    pub(super) fn token(parent: &SyntaxNode, kinds: &[SyntaxKind]) -> Option<SyntaxToken> {
        parent
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .find(|tok| kinds.contains(&tok.kind()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn typed_view_walks_a_call() {
        let tree = parse("foo(1, 'two')").expect("parse error");
        let root = Root::cast(tree).expect("root casts");
        let Some(Expr::CallExpr(call)) = root.expr() else {
            panic!("expected a call");
        };
        let callee = call.callee().expect("callee");
        assert_eq!(callee.ident_token().expect("ident").text(), "foo");
        let args: Vec<Expr> = call.arg_list().expect("arg list").args().collect();
        assert_eq!(args.len(), 2);
        assert!(matches!(&args[0], Expr::Literal(l)
            if l.value_token().is_some_and(|t| t.text() == "1")));
        assert!(matches!(&args[1], Expr::Literal(l)
            if l.value_token().is_some_and(|t| t.text() == "'two'")));
    }

    #[test]
    fn typed_view_walks_a_binary() {
        let tree = parse("a + b * 2").expect("parse error");
        let root = Root::cast(tree).expect("root casts");
        let Some(Expr::BinExpr(add)) = root.expr() else {
            panic!("expected a binary");
        };
        assert_eq!(add.op_token().expect("op").text(), "+");
        assert!(matches!(add.lhs(), Some(Expr::NameRef(_))));
        let Some(Expr::BinExpr(mul)) = add.rhs() else {
            panic!("expected a nested binary");
        };
        assert_eq!(mul.op_token().expect("op").text(), "*");
    }
}
