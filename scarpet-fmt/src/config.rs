//! Configurable formatting style.
//!
//! The CLI parses a TOML config and builds a [`Config`]; this library only
//! consumes it (it stays `wasm`-clean — no file I/O here). Defaults reproduce
//! the original fixed style: a 4-space indent at a 100-column target width,
//! unwrapped comments, Unix (`\n`) line endings, and same-line opening
//! delimiters.

/// The line ending the formatter emits for the breaks it inserts.
///
/// This affects only breaks the formatter introduces between tokens; bytes
/// inside string and comment text are copied through verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// Unix-style line feed, `\n` (the default).
    #[default]
    Lf,
    /// Windows-style carriage return + line feed, `\r\n`.
    Crlf,
    /// Match the line ending already used by the source: the style of its first
    /// line break wins. Falls back to [`Native`](LineEnding::Native) when the
    /// source has no break — or when a CST is formatted without its source text,
    /// since then there is nothing to detect.
    Auto,
    /// The host platform's native line ending, fixed at compile time: `\r\n` on
    /// Windows, `\n` everywhere else (including `wasm32`).
    Native,
}

impl LineEnding {
    /// The platform-native break string, fixed at compile time: `\r\n` on
    /// Windows, `\n` everywhere else.
    const NATIVE: &'static str = if cfg!(windows) { "\r\n" } else { "\n" };

    /// The concrete break string for this style, resolved without any source
    /// text. [`Lf`](Self::Lf) / [`Crlf`](Self::Crlf) map to their literal bytes
    /// and [`Native`](Self::Native) to the host platform; [`Auto`](Self::Auto)
    /// has no source to inspect here, so it too resolves to the native ending.
    /// Resolve `Auto` against real source with [`resolve`](Self::resolve) first
    /// (as [`format_source`](crate::format_source) does).
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
            LineEnding::Auto | LineEnding::Native => Self::NATIVE,
        }
    }

    /// Collapse [`Auto`](Self::Auto) to the concrete style implied by `source`:
    /// the first line break decides ([`Crlf`](Self::Crlf) if it is `\r\n`,
    /// else [`Lf`](Self::Lf)), and a source with no break yields
    /// [`Native`](Self::Native). Every other variant is returned unchanged, so
    /// this is a no-op for an already-concrete setting.
    pub fn resolve(self, source: &str) -> LineEnding {
        match self {
            LineEnding::Auto => match source.find('\n') {
                Some(i) if source.as_bytes()[..i].last() == Some(&b'\r') => LineEnding::Crlf,
                Some(_) => LineEnding::Lf,
                None => LineEnding::Native,
            },
            other => other,
        }
    }
}

/// Where the opening delimiter of a `(...)`, `[...]`, or `{...}` block sits when
/// the block is broken across multiple lines.
///
/// This only affects blocks the formatter breaks; one that fits on a single line
/// keeps its delimiters inline regardless. It is the rough analogue of rustfmt's
/// `brace_style`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BraceStyle {
    /// Keep the opening delimiter on the head's line — `foo(` … `)` (the
    /// default, reproducing the original fixed style).
    #[default]
    SameLine,
    /// Put the opening delimiter on its own line — `foo` ⏎ `(` … `)`.
    NextLine,
}

/// Whether a trailing comma is emitted after the last item of a `(...)`,
/// `[...]`, or `{...}`.
///
/// The rough analogue of rustfmt's `trailing_comma`. An empty collection never
/// gets one regardless of this setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrailingComma {
    /// A trailing comma only when the collection is broken across lines — `[\n
    /// 1,\n 2,\n]` but a flat `[1, 2]` without (the default, reproducing the
    /// original fixed style).
    #[default]
    Vertical,
    /// A trailing comma in both layouts, including a one-line `[1, 2,]`.
    Always,
    /// Never a trailing comma, even when the collection is broken.
    Never,
}

/// Where a binary operator sits when its expression wraps across lines.
///
/// The rough analogue of rustfmt's `binop_separator`. It applies only to the
/// spaced operators (arithmetic, comparison, logical, and `~`); the
/// assignment-like operators (`=`, `+=`, `<>`) always stay [`Back`](Self::Back)
/// so their right-hand side keeps hugging the operator line. A flat expression
/// that fits on one line is unaffected.
///
/// Note that rustfmt defaults this to `Front`; this crate defaults to
/// [`Back`](Self::Back) to leave the original fixed style unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BinopSeparator {
    /// Keep the operator at the tail of the line before the break — `a +` ⏎
    /// `    b` (the default, reproducing the original fixed style).
    #[default]
    Back,
    /// Move the operator to the head of the wrapped line — `a` ⏎ `    + b`.
    Front,
}

/// Formatting style knobs, threaded through lowering and rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Indentation step, in spaces.
    pub indent_width: usize,
    /// Target maximum line width. A `Group` that fits within this stays flat.
    pub max_width: usize,
    /// Target maximum width for `//` comments. `None` leaves comments
    /// unwrapped, equivalent to `comment_width = -1` in the TOML config.
    pub comment_width: Option<usize>,
    /// Line ending emitted for the breaks the formatter inserts.
    pub line_ending: LineEnding,
    /// Placement of the opening delimiter of a broken `()`/`[]`/`{}` block.
    pub brace_style: BraceStyle,
    /// Whether the last item of a `()`/`[]`/`{}` carries a trailing comma.
    pub trailing_comma: TrailingComma,
    /// Whether a call's last argument — when it is a `(…)` block or a bare
    /// `;`-chain — "hugs" the closing `)`, keeping the leading arguments on the
    /// opening line instead of exploding every argument one-per-line. The rough
    /// analogue of rustfmt's `overflow_delimited_expr`; off by default.
    pub overflow_delimited_expr: bool,
    /// Where a binary operator sits when its expression wraps across lines.
    pub binop_separator: BinopSeparator,
    /// The maximum number of consecutive blank lines kept between statements;
    /// longer runs are truncated to it. The analogue of rustfmt's
    /// `blank_lines_upper_bound`. Defaults to `1`, reproducing the original
    /// fixed style (any run of blank lines collapses to a single one).
    pub blank_lines_upper_bound: usize,
    /// The minimum number of blank lines forced between statements; adjacent
    /// statements get blanks inserted up to it. The analogue of rustfmt's
    /// `blank_lines_lower_bound`. Defaults to `0` (no minimum). Assumed not to
    /// exceed `blank_lines_upper_bound` (the CLI rejects that).
    pub blank_lines_lower_bound: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            indent_width: 4,
            max_width: 100,
            comment_width: None,
            line_ending: LineEnding::Lf,
            brace_style: BraceStyle::SameLine,
            trailing_comma: TrailingComma::Vertical,
            overflow_delimited_expr: false,
            binop_separator: BinopSeparator::Back,
            blank_lines_upper_bound: 1,
            blank_lines_lower_bound: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_ending_as_str() {
        assert_eq!(LineEnding::Lf.as_str(), "\n");
        assert_eq!(LineEnding::Crlf.as_str(), "\r\n");
    }

    #[test]
    fn native_and_auto_as_str_match_the_platform() {
        let native = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(LineEnding::Native.as_str(), native);
        // Without source text `Auto` has nothing to detect, so it too falls
        // back to the native ending.
        assert_eq!(LineEnding::Auto.as_str(), native);
    }

    #[test]
    fn auto_resolves_from_the_first_break() {
        assert_eq!(LineEnding::Auto.resolve("a\nb\r\n"), LineEnding::Lf);
        assert_eq!(LineEnding::Auto.resolve("a\r\nb\n"), LineEnding::Crlf);
        assert_eq!(LineEnding::Auto.resolve("a\nb"), LineEnding::Lf);
        assert_eq!(LineEnding::Auto.resolve("a\r\nb"), LineEnding::Crlf);
    }

    #[test]
    fn auto_without_a_break_falls_back_to_native() {
        assert_eq!(LineEnding::Auto.resolve("no breaks"), LineEnding::Native);
        assert_eq!(LineEnding::Auto.resolve(""), LineEnding::Native);
    }

    #[test]
    fn resolve_is_a_noop_for_concrete_styles() {
        assert_eq!(LineEnding::Lf.resolve("a\r\nb"), LineEnding::Lf);
        assert_eq!(LineEnding::Crlf.resolve("a\nb"), LineEnding::Crlf);
        assert_eq!(LineEnding::Native.resolve("a\nb"), LineEnding::Native);
    }

    #[test]
    fn default_config_uses_lf() {
        assert_eq!(Config::default().line_ending, LineEnding::Lf);
    }

    #[test]
    fn default_config_uses_same_line_braces() {
        assert_eq!(Config::default().brace_style, BraceStyle::SameLine);
    }

    #[test]
    fn default_config_leaves_comments_unwrapped() {
        assert_eq!(Config::default().comment_width, None);
    }

    #[test]
    fn default_config_uses_vertical_trailing_comma() {
        assert_eq!(Config::default().trailing_comma, TrailingComma::Vertical);
    }

    #[test]
    fn default_config_disables_overflow_delimited_expr() {
        assert!(!Config::default().overflow_delimited_expr);
    }

    #[test]
    fn default_config_uses_back_binop_separator() {
        assert_eq!(Config::default().binop_separator, BinopSeparator::Back);
    }

    #[test]
    fn default_config_keeps_one_blank_line_upper_bound() {
        assert_eq!(Config::default().blank_lines_upper_bound, 1);
    }

    #[test]
    fn default_config_uses_zero_blank_line_lower_bound() {
        assert_eq!(Config::default().blank_lines_lower_bound, 0);
    }
}
