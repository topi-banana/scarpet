//! A plain-text rendering of an analysis, for tests and terminal front ends.
//!
//! One line per expression: the node's label, indented by depth, and its
//! inferred type in a right-hand column.
//!
//! ```text
//! Seq `;` [2]                               string
//!   Define `fib`/1                          string
//!     Call `if`                             int
//!       Bin `<`                             bool
//! ```
//!
//! The walk uses an explicit stack. Lowering flattens `;` and `,`, so most
//! programs are shallow — but arithmetic still left-nests without bound, and a
//! dump must never be the thing that overflows.

use crate::hir::{ExprId, Hir};
use crate::ty::Ty;

/// Spaces per nesting level.
const INDENT: usize = 2;
/// Column the type is aligned to. A longer label pushes past it and gets a
/// single separating space instead.
const TYPE_COLUMN: usize = 44;

pub(crate) fn dump(hir: &Hir, tys: &[Ty]) -> String {
    let mut out = String::new();
    let mut stack = vec![(hir.root(), 0usize)];
    // Children are pushed in reverse so the LIFO stack emits them top to bottom.
    let mut children = Vec::new();
    while let Some((id, depth)) = stack.pop() {
        let indent = depth * INDENT;
        let label = hir.label(id);
        let ty = tys
            .get(id.index())
            .map_or_else(|| Ty::Unknown.to_string(), Ty::to_string);

        let width = indent + label.chars().count();
        let padding = TYPE_COLUMN.saturating_sub(width).max(1);
        out.push_str(&" ".repeat(indent));
        out.push_str(&label);
        out.push_str(&" ".repeat(padding));
        out.push_str(&ty);
        out.push('\n');

        children.clear();
        children.extend(hir.children(id));
        for child in children.iter().rev() {
            stack.push((*child, depth + 1));
        }
    }
    out
}

/// Every expression in `hir`, in the order an analysis dump prints them, with
/// its depth.
///
/// Exposed so a graphical front end can lay out the same tree without
/// re-deriving the traversal — and without recursing, which is the whole reason
/// this is written with a stack. `ExprId` is all a caller needs to reach the
/// label, span, type, and children.
pub fn walk(hir: &Hir) -> Vec<(ExprId, usize)> {
    let mut rows = Vec::with_capacity(hir.len());
    let mut stack = vec![(hir.root(), 0usize)];
    let mut children = Vec::new();
    while let Some((id, depth)) = stack.pop() {
        rows.push((id, depth));
        children.clear();
        children.extend(hir.children(id));
        for child in children.iter().rev() {
            stack.push((*child, depth + 1));
        }
    }
    rows
}
