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
}

impl LineEnding {
    /// The string emitted for a single break in this style.
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            indent_width: 4,
            max_width: 100,
            comment_width: None,
            line_ending: LineEnding::Lf,
            brace_style: BraceStyle::SameLine,
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
}
