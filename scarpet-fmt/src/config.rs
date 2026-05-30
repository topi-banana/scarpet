//! Configurable formatting style.
//!
//! The CLI parses a TOML config and builds a [`Config`]; this library only
//! consumes it (it stays `wasm`-clean — no file I/O here). Defaults reproduce
//! the original fixed style: a 4-space indent at a 100-column target width.

/// Formatting style knobs, threaded through lowering and rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Indentation step, in spaces.
    pub indent_width: usize,
    /// Target maximum line width. A `Group` that fits within this stays flat.
    pub max_width: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            indent_width: 4,
            max_width: 100,
        }
    }
}
