//! Fixed formatting style.
//!
//! The MVP is not configurable; these constants are isolated here so a future
//! `Config` struct is a localized change.

/// Indentation step, in spaces.
pub const INDENT: isize = 4;

/// Target maximum line width. A `Group` that fits within this stays flat.
pub const MAX_WIDTH: usize = 100;
