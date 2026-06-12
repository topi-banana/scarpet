//! The rowan [`Language`](rowan::Language) glue for Scarpet: maps
//! [`SyntaxKind`] to and from rowan's raw `u16` kinds and provides the
//! red-tree type aliases used throughout the crate.

use crate::syntax_kind::SyntaxKind;

/// The Scarpet language definition for rowan trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScarpetLanguage {}

impl rowan::Language for ScarpetLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        SyntaxKind::from_raw(raw.0)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind.to_raw())
    }
}

/// A red-tree node of the Scarpet syntax tree.
pub type SyntaxNode = rowan::SyntaxNode<ScarpetLanguage>;
/// A red-tree token of the Scarpet syntax tree.
pub type SyntaxToken = rowan::SyntaxToken<ScarpetLanguage>;
/// A red-tree node or token.
pub type SyntaxElement = rowan::SyntaxElement<ScarpetLanguage>;

/// Trivia-tolerant structural equality of two syntax trees.
///
/// Walks both trees in parallel: node kinds must match, and the remaining
/// significant tokens must match pairwise by `(kind, text)` in order. Tokens
/// of the following kinds are skipped entirely on both sides:
///
/// - [`WHITESPACE`](SyntaxKind::WHITESPACE), [`NEWLINE`](SyntaxKind::NEWLINE)
///   and [`COMMENT`](SyntaxKind::COMMENT) — trivia; the formatter reflows
///   whitespace and rewraps comments freely.
/// - [`SEMICOLON`](SyntaxKind::SEMICOLON) and [`COMMA`](SyntaxKind::COMMA) —
///   sound to ignore because the chain/argument *structure* they delimit is
///   already encoded in node kinds: `;`-chains are `SEMI_CHAIN` nodes,
///   `,`-chains are `COMMA_CHAIN` nodes, and an omitted argument slot
///   (`f(a,,b)`) is a zero-width `EMPTY_ARG` node. The separator tokens carry
///   no information beyond what those nodes and the order of their children
///   record — and the formatter legally adds or removes exactly these tokens
///   (trailing `;`, trailing commas, collapsing `;;` runs), none of which
///   changes the node structure.
///
/// This is the successor of the old `strip_trivia`-based CST equality: the
/// formatter's corpus round-trip gate uses it to check that re-parsing
/// formatted output yields a structurally equal tree.
pub fn structurally_equal(a: &SyntaxNode, b: &SyntaxNode) -> bool {
    if a.kind() != b.kind() {
        return false;
    }
    let mut left = a.children_with_tokens().filter(is_significant);
    let mut right = b.children_with_tokens().filter(is_significant);
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(rowan::NodeOrToken::Node(x)), Some(rowan::NodeOrToken::Node(y))) => {
                if !structurally_equal(&x, &y) {
                    return false;
                }
            }
            (Some(rowan::NodeOrToken::Token(x)), Some(rowan::NodeOrToken::Token(y))) => {
                if x.kind() != y.kind() || x.text() != y.text() {
                    return false;
                }
            }
            // Node vs token, or one side exhausted early.
            _ => return false,
        }
    }
}

/// Whether an element takes part in [`structurally_equal`]: every node does,
/// tokens do unless their kind is in the ignored set documented there.
fn is_significant(element: &SyntaxElement) -> bool {
    match element {
        rowan::NodeOrToken::Node(_) => true,
        rowan::NodeOrToken::Token(token) => !matches!(
            token.kind(),
            SyntaxKind::WHITESPACE
                | SyntaxKind::NEWLINE
                | SyntaxKind::COMMENT
                | SyntaxKind::SEMICOLON
                | SyntaxKind::COMMA
        ),
    }
}

/// Test-only helpers for hand-building green trees.
#[cfg(test)]
pub(crate) mod testing {
    use rowan::Language;

    use super::{ScarpetLanguage, SyntaxNode};
    use crate::syntax_kind::SyntaxKind;

    /// Thin wrapper over [`rowan::GreenNodeBuilder`] so tests can build trees
    /// with nested closures instead of paired `start_node`/`finish_node`
    /// calls.
    pub(crate) struct TreeBuilder {
        inner: rowan::GreenNodeBuilder<'static>,
    }

    impl TreeBuilder {
        /// Adds a node of `kind` whose children are produced by `children`.
        pub(crate) fn node(&mut self, kind: SyntaxKind, children: impl FnOnce(&mut TreeBuilder)) {
            self.inner.start_node(ScarpetLanguage::kind_to_raw(kind));
            children(self);
            self.inner.finish_node();
        }

        /// Adds a token of `kind` with the given `text`.
        pub(crate) fn token(&mut self, kind: SyntaxKind, text: &str) {
            self.inner.token(ScarpetLanguage::kind_to_raw(kind), text);
        }
    }

    /// Builds a tree rooted at a node of `kind` whose children are produced
    /// by `children`.
    pub(crate) fn tree(kind: SyntaxKind, children: impl FnOnce(&mut TreeBuilder)) -> SyntaxNode {
        let mut builder = TreeBuilder {
            inner: rowan::GreenNodeBuilder::new(),
        };
        builder.node(kind, children);
        SyntaxNode::new_root(builder.inner.finish())
    }
}

#[cfg(test)]
mod tests {
    use rowan::Language;

    use super::testing::{TreeBuilder, tree};
    use super::{ScarpetLanguage, SyntaxNode, structurally_equal};
    use crate::syntax_kind::SyntaxKind;

    #[test]
    fn raw_roundtrip_over_all_kinds() {
        for raw in 0..SyntaxKind::__LAST as u16 {
            let kind = SyntaxKind::from_raw(raw);
            assert_eq!(kind.to_raw(), raw);
            assert_eq!(SyntaxKind::from(raw), kind);
            assert_eq!(u16::from(kind), raw);
            assert_eq!(ScarpetLanguage::kind_from_raw(rowan::SyntaxKind(raw)), kind);
            assert_eq!(ScarpetLanguage::kind_to_raw(kind), rowan::SyntaxKind(raw));
        }
    }

    #[test]
    #[should_panic(expected = "invalid SyntaxKind raw value")]
    fn from_raw_rejects_out_of_range() {
        let _ = SyntaxKind::from_raw(SyntaxKind::__LAST as u16);
    }

    #[test]
    fn is_trivia_truth_table() {
        // The trivia set, written out independently of the generated impl.
        let trivia = [
            SyntaxKind::WHITESPACE,
            SyntaxKind::NEWLINE,
            SyntaxKind::COMMENT,
        ];
        for raw in 0..SyntaxKind::__LAST as u16 {
            let kind = SyntaxKind::from_raw(raw);
            assert_eq!(
                kind.is_trivia(),
                trivia.contains(&kind),
                "is_trivia disagrees for {kind:?}"
            );
        }
        // Spot-check a few non-trivia kinds explicitly.
        assert!(!SyntaxKind::NUMBER.is_trivia());
        assert!(!SyntaxKind::IDENT.is_trivia());
        assert!(!SyntaxKind::ERROR_TOKEN.is_trivia());
        assert!(!SyntaxKind::SOURCE_FILE.is_trivia());
        assert!(!SyntaxKind::ERROR.is_trivia());
    }

    #[test]
    fn token_kinds_precede_node_kinds() {
        assert!(SyntaxKind::ERROR_TOKEN.to_raw() < SyntaxKind::SOURCE_FILE.to_raw());
        assert!(SyntaxKind::ERROR.to_raw() < SyntaxKind::__LAST as u16);
    }

    #[test]
    fn discriminants_are_frozen() {
        // The variant order (and thus the raw `u16` values) is a contract for
        // the later waves of the rowan migration: pin the section boundaries
        // so an accidental reorder of the codegen tables fails loudly.
        assert_eq!(SyntaxKind::WHITESPACE.to_raw(), 0);
        assert_eq!(SyntaxKind::ERROR_TOKEN.to_raw(), 37);
        assert_eq!(SyntaxKind::SOURCE_FILE.to_raw(), 38);
        assert_eq!(SyntaxKind::ERROR.to_raw(), 51);
        assert_eq!(SyntaxKind::__LAST as u16, 52);
    }

    // ----- structurally_equal -----

    fn literal(b: &mut TreeBuilder, text: &str) {
        b.node(SyntaxKind::LITERAL, |b| b.token(SyntaxKind::NUMBER, text));
    }

    fn name_ref(b: &mut TreeBuilder, text: &str) {
        b.node(SyntaxKind::NAME_REF, |b| b.token(SyntaxKind::IDENT, text));
    }

    /// `BIN_EXPR(LITERAL("1") <op> LITERAL("2"))`, no trivia.
    fn one_op_two(op_kind: SyntaxKind, op_text: &str) -> SyntaxNode {
        tree(SyntaxKind::BIN_EXPR, |b| {
            literal(b, "1");
            b.token(op_kind, op_text);
            literal(b, "2");
        })
    }

    /// `CALL_EXPR(NAME_REF("f") ARG_LIST(...))` with the given argument names
    /// (`None` builds an EMPTY_ARG slot), comma-separated.
    fn call(args: &[Option<&str>]) -> SyntaxNode {
        tree(SyntaxKind::CALL_EXPR, |b| {
            name_ref(b, "f");
            b.node(SyntaxKind::ARG_LIST, |b| {
                b.token(SyntaxKind::OPEN_PAREN, "(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        b.token(SyntaxKind::COMMA, ",");
                    }
                    match arg {
                        Some(name) => name_ref(b, name),
                        None => b.node(SyntaxKind::EMPTY_ARG, |_| {}),
                    }
                }
                b.token(SyntaxKind::CLOSE_PAREN, ")");
            });
        })
    }

    #[test]
    fn structurally_equal_on_identical_trees() {
        let a = one_op_two(SyntaxKind::PLUS, "+");
        let b = one_op_two(SyntaxKind::PLUS, "+");
        assert!(structurally_equal(&a, &b));
        assert!(structurally_equal(&a, &a));
    }

    #[test]
    fn structurally_equal_ignores_whitespace_newlines_and_comments() {
        let bare = one_op_two(SyntaxKind::PLUS, "+");
        let spaced = tree(SyntaxKind::BIN_EXPR, |b| {
            b.token(SyntaxKind::COMMENT, "// leading note");
            b.token(SyntaxKind::NEWLINE, "\n");
            literal(b, "1");
            b.token(SyntaxKind::WHITESPACE, " ");
            b.token(SyntaxKind::PLUS, "+");
            b.token(SyntaxKind::NEWLINE, "\n");
            b.token(SyntaxKind::WHITESPACE, "    ");
            literal(b, "2");
        });
        assert!(structurally_equal(&bare, &spaced));
        assert!(structurally_equal(&spaced, &bare));
    }

    #[test]
    fn structurally_equal_ignores_trailing_semicolon() {
        let semi_chain = |trailing: bool| {
            tree(SyntaxKind::SEMI_CHAIN, |b| {
                literal(b, "1");
                b.token(SyntaxKind::SEMICOLON, ";");
                literal(b, "2");
                if trailing {
                    b.token(SyntaxKind::SEMICOLON, ";");
                }
            })
        };
        assert!(structurally_equal(&semi_chain(true), &semi_chain(false)));
    }

    #[test]
    fn structurally_equal_ignores_trailing_comma() {
        let comma_chain = |trailing: bool| {
            tree(SyntaxKind::COMMA_CHAIN, |b| {
                literal(b, "1");
                b.token(SyntaxKind::COMMA, ",");
                literal(b, "2");
                if trailing {
                    b.token(SyntaxKind::COMMA, ",");
                }
            })
        };
        assert!(structurally_equal(&comma_chain(true), &comma_chain(false)));
    }

    #[test]
    fn structurally_equal_ignores_semicolon_runs() {
        let semi_chain = |separator_count: usize| {
            tree(SyntaxKind::SEMI_CHAIN, |b| {
                literal(b, "1");
                for _ in 0..separator_count {
                    b.token(SyntaxKind::SEMICOLON, ";");
                }
                literal(b, "2");
            })
        };
        assert!(structurally_equal(&semi_chain(2), &semi_chain(1)));
    }

    #[test]
    fn structurally_unequal_on_different_operator() {
        let add = one_op_two(SyntaxKind::PLUS, "+");
        let mul = one_op_two(SyntaxKind::STAR, "*");
        assert!(!structurally_equal(&add, &mul));
    }

    #[test]
    fn structurally_unequal_on_empty_arg_vs_no_empty_arg() {
        // `f(a,,b)` vs `f(a,b)`: the EMPTY_ARG node must not be ignored even
        // though the `,` tokens around it are.
        let with_hole = call(&[Some("a"), None, Some("b")]);
        let without_hole = call(&[Some("a"), Some("b")]);
        assert!(!structurally_equal(&with_hole, &without_hole));
        assert!(!structurally_equal(&without_hole, &with_hole));
    }

    #[test]
    fn structurally_unequal_on_chain_kind() {
        let chain = |kind: SyntaxKind, sep_kind: SyntaxKind, sep_text: &str| {
            tree(kind, |b| {
                literal(b, "1");
                b.token(sep_kind, sep_text);
                literal(b, "2");
            })
        };
        let commas = chain(SyntaxKind::COMMA_CHAIN, SyntaxKind::COMMA, ",");
        let semis = chain(SyntaxKind::SEMI_CHAIN, SyntaxKind::SEMICOLON, ";");
        assert!(!structurally_equal(&commas, &semis));
    }

    #[test]
    fn structurally_unequal_on_literal_text() {
        let one = tree(SyntaxKind::LITERAL, |b| b.token(SyntaxKind::NUMBER, "1"));
        let two = tree(SyntaxKind::LITERAL, |b| b.token(SyntaxKind::NUMBER, "2"));
        assert!(!structurally_equal(&one, &two));
    }

    #[test]
    fn structurally_unequal_on_token_kind_with_same_text() {
        // Same text, different token kind: `<` as LT vs as ERROR_TOKEN.
        let lt = one_op_two(SyntaxKind::LT, "<");
        let err = one_op_two(SyntaxKind::ERROR_TOKEN, "<");
        assert!(!structurally_equal(&lt, &err));
    }

    #[test]
    fn structurally_unequal_on_different_nesting() {
        // `(1+2)+3` vs `1+(2+3)` (without parens): the flat token sequence
        // matches, the nesting does not.
        let left_leaning = tree(SyntaxKind::BIN_EXPR, |b| {
            b.node(SyntaxKind::BIN_EXPR, |b| {
                literal(b, "1");
                b.token(SyntaxKind::PLUS, "+");
                literal(b, "2");
            });
            b.token(SyntaxKind::PLUS, "+");
            literal(b, "3");
        });
        let right_leaning = tree(SyntaxKind::BIN_EXPR, |b| {
            literal(b, "1");
            b.token(SyntaxKind::PLUS, "+");
            b.node(SyntaxKind::BIN_EXPR, |b| {
                literal(b, "2");
                b.token(SyntaxKind::PLUS, "+");
                literal(b, "3");
            });
        });
        assert!(!structurally_equal(&left_leaning, &right_leaning));
    }

    #[test]
    fn structurally_unequal_on_extra_item() {
        let two_items = tree(SyntaxKind::SEMI_CHAIN, |b| {
            literal(b, "1");
            b.token(SyntaxKind::SEMICOLON, ";");
            literal(b, "2");
        });
        let three_items = tree(SyntaxKind::SEMI_CHAIN, |b| {
            literal(b, "1");
            b.token(SyntaxKind::SEMICOLON, ";");
            literal(b, "2");
            b.token(SyntaxKind::SEMICOLON, ";");
            literal(b, "3");
        });
        assert!(!structurally_equal(&two_items, &three_items));
        assert!(!structurally_equal(&three_items, &two_items));
    }
}
