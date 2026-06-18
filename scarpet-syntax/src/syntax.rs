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

#[cfg(test)]
mod tests {
    use rowan::Language;

    use super::ScarpetLanguage;
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
}
