//! Helpers for laying out a node's leading trivia (comments and blank lines).
//!
//! The CST stores trivia only as `leading` on each node. Comments are emitted
//! on their own line by [`own_line_comments`] (the same-line / trailing-comment
//! case is lifted by the chain/collection lowering — see `lower`). Blank lines
//! are not emitted here; they are reconstructed by statement separators via
//! [`blank_lines_before`].

use scarpet_syntax::parser::Trivia;

use crate::doc::{Doc, comment, concat, hardline};

/// Lower the comments in `leading`, each on its own line (`comment` + hardline).
/// Breaks are ignored — blank-line preservation is the separator's job.
pub fn own_line_comments(leading: &[Trivia]) -> Doc {
    concat(leading.iter().filter_map(|t| match t {
        Trivia::Comment(c) => Some(concat([comment(c.to_string()), hardline()])),
        Trivia::Break => None,
    }))
}

/// The number of blank lines directly above the node: consecutive leading
/// breaks, less the one that merely ends the previous line. Zero when the node
/// abuts the previous one or sits on the very next line. Only breaks *before*
/// any comment are counted, so a leading own-line comment terminates the run.
pub fn blank_lines_before(leading: &[Trivia]) -> usize {
    leading
        .iter()
        .take_while(|t| matches!(t, Trivia::Break))
        .count()
        .saturating_sub(1)
}

/// Whether `leading` contains any comment (same-line or own-line). Used to keep
/// a binary operator in tail position when its RHS carries a comment, since the
/// operator's leading trivia rebinds to the RHS on re-parse.
pub fn has_comment(leading: &[Trivia]) -> bool {
    leading.iter().any(|t| matches!(t, Trivia::Comment(_)))
}

/// The same-line trailing comment at the very head of `leading`, if any: a
/// comment with no preceding break was written on the previous token's line.
pub fn same_line_comment<'s>(leading: &[Trivia<'s>]) -> Option<&'s str> {
    match leading.first() {
        Some(Trivia::Comment(c)) => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::blank_lines_before;
    use scarpet_syntax::parser::Trivia;

    #[test]
    fn counts_leading_breaks_minus_one() {
        // N consecutive breaks span N-1 blank lines (one break ends the line).
        assert_eq!(blank_lines_before(&[]), 0);
        assert_eq!(blank_lines_before(&[Trivia::Break]), 0);
        assert_eq!(blank_lines_before(&[Trivia::Break, Trivia::Break]), 1);
        assert_eq!(
            blank_lines_before(&[Trivia::Break, Trivia::Break, Trivia::Break]),
            2
        );
    }

    #[test]
    fn stops_at_the_first_comment() {
        // A same-line comment (comment first) reports zero blanks above; an
        // own-line comment terminates the leading break run after counting it.
        assert_eq!(
            blank_lines_before(&[Trivia::Comment("// c"), Trivia::Break]),
            0
        );
        assert_eq!(
            blank_lines_before(&[Trivia::Break, Trivia::Comment("// c"), Trivia::Break]),
            0
        );
        assert_eq!(
            blank_lines_before(&[
                Trivia::Break,
                Trivia::Break,
                Trivia::Comment("// c"),
                Trivia::Break,
            ]),
            1
        );
    }
}
