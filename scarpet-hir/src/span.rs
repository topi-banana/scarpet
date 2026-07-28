//! Byte spans into the analysed source.
//!
//! The workspace already has two span representations: `rowan::TextRange` (what
//! every syntax node carries) and `std::ops::Range<usize>` (what
//! [`ParseError`](scarpet_syntax::parser::ParseError) carries and what
//! `scarpet-lsp`'s `byte_range_to_lsp_range` and `scarpet-cli`'s `ariadne`
//! reports consume). [`Span`] is the bridge: `From<TextRange>` on the way in,
//! `From<Span> for Range<usize>` on the way out, so neither downstream crate
//! needs its own `rowan` dependency to talk about a HIR position.
//!
//! `u32` rather than `usize` because that is what `rowan` stores and what a
//! 4 GiB-per-file ceiling comfortably allows — it keeps [`Span`] eight bytes so
//! the parallel span table in [`Hir`](crate::Hir) stays cheap.

use std::fmt;
use std::ops::Range;

use rowan::TextRange;

/// A half-open byte range `start..end` into the source that was analysed.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// A span covering `start..end`. The ends are stored as given; a reversed
    /// pair simply produces an empty span under [`len`](Self::len).
    pub const fn new(start: u32, end: u32) -> Self {
        Span { start, end }
    }

    /// The number of bytes covered, saturating at zero for a reversed span.
    pub const fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Whether `offset` falls inside the span. Half-open, so the byte at `end`
    /// belongs to whatever follows — except for an empty span, which contains
    /// only its own position so that a zero-width node (an
    /// [`Expr::Missing`](crate::Expr::Missing) hole at end of input) is still
    /// reachable from a cursor sitting on it.
    pub const fn contains(self, offset: u32) -> bool {
        if self.is_empty() {
            offset == self.start
        } else {
            self.start <= offset && offset < self.end
        }
    }

    /// The source text this span covers, or `""` when the span does not land on
    /// character boundaries of `src`. Total by design: a `Span` may outlive the
    /// buffer it was measured against (the playground re-analyses on every
    /// keystroke), and slicing must not panic when it does.
    pub fn text(self, src: &str) -> &str {
        src.get(self.start as usize..self.end as usize)
            .unwrap_or("")
    }
}

impl fmt::Debug for Span {
    /// Renders as `12..24`. The derived form (`Span { start: 12, end: 24 }`)
    /// triples the width of every line in an arena dump.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

impl From<TextRange> for Span {
    fn from(range: TextRange) -> Self {
        Span::new(range.start().into(), range.end().into())
    }
}

impl From<Span> for Range<usize> {
    fn from(span: Span) -> Self {
        span.start as usize..span.end as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_total_on_a_mismatched_buffer() {
        assert_eq!(Span::new(1, 3).text("hello"), "el");
        // Past the end, and straddling a multi-byte character: both yield "".
        assert_eq!(Span::new(1, 99).text("hello"), "");
        assert_eq!(Span::new(0, 1).text("あ"), "");
    }

    #[test]
    fn contains_is_half_open_but_empty_spans_are_reachable() {
        let span = Span::new(2, 5);
        assert!(!span.contains(1));
        assert!(span.contains(2));
        assert!(span.contains(4));
        assert!(!span.contains(5));

        let empty = Span::new(5, 5);
        assert!(empty.is_empty());
        assert!(empty.contains(5));
        assert!(!empty.contains(4));
    }

    #[test]
    fn debug_is_compact() {
        assert_eq!(format!("{:?}", Span::new(12, 24)), "12..24");
    }
}
