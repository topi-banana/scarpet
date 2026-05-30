//! Configurable formatting style.
//!
//! The CLI parses a TOML config and builds a [`Config`]; this library only
//! consumes it (it stays `wasm`-clean — no file I/O here). Defaults reproduce
//! the original fixed style: a 4-space indent at a 100-column target width,
//! with Unix (`\n`) line endings.

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

/// Formatting style knobs, threaded through lowering and rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Indentation step, in spaces.
    pub indent_width: usize,
    /// Target maximum line width. A `Group` that fits within this stays flat.
    pub max_width: usize,
    /// Line ending emitted for the breaks the formatter inserts.
    pub line_ending: LineEnding,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            indent_width: 4,
            max_width: 100,
            line_ending: LineEnding::Lf,
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
}
