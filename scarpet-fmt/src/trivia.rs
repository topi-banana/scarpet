//! Helpers for laying out a node's leading trivia (comments and blank lines).
//!
//! The CST stores trivia only as `leading` on each node. Comments are emitted
//! on their own line by [`own_line_comments`] (the same-line / trailing-comment
//! case is lifted by the chain/collection lowering — see `lower`). Blank lines
//! are not emitted here; they are reconstructed by statement separators via
//! [`has_blank_before`].

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

/// Whether a blank line (two or more consecutive breaks before any comment)
/// precedes the node — i.e. the user left a blank line above it.
pub fn has_blank_before(leading: &[Trivia]) -> bool {
    leading
        .iter()
        .take_while(|t| matches!(t, Trivia::Break))
        .count()
        >= 2
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
