//! Bits shared by the editor and notebook views: the `print`-capture sink, the
//! diagnostic formatter, and the Tailwind class strings reused across buttons
//! and panes.

use std::cell::RefCell;
use std::rc::Rc;

use scarpet_hir::{Diagnostic, Severity};
use scarpet_syntax::parser::ParseError;

/// Shared base classes for a toolbar button.
pub const BTN_BASE: &str = "inline-flex h-9 cursor-pointer items-center rounded-md px-3 text-sm font-medium transition-colors";
/// A smaller button, for the per-cell controls.
pub const BTN_SM: &str = "inline-flex h-8 cursor-pointer items-center rounded-md px-2.5 text-xs font-medium transition-colors";
/// Neutral, outlined button variant.
pub const BTN_BORDERED: &str = "border border-hairline bg-canvas text-ink hover:bg-canvas-soft";
/// Filled (ink) button variant — the primary non-Run action.
pub const BTN_INK: &str = "bg-ink text-canvas hover:opacity-90";
/// Accent (link-blue) button variant — Run / Run all.
pub const BTN_LINK: &str = "bg-link text-canvas hover:opacity-90";

/// The small uppercase heading above a pane.
pub const LABEL: &str = "border-b border-hairline bg-canvas px-4 py-2 font-mono text-xs font-medium uppercase tracking-wider text-mute";
/// The monospace editor / output surface that fills a flex column.
pub const EDITOR: &str = "min-h-0 flex-1 overflow-auto whitespace-pre bg-canvas p-4 font-mono text-[13px] leading-5 text-ink outline-none";

/// An [`std::io::Write`] sink that appends to a shared in-memory buffer, so a
/// view can capture the VM's `print` output and display it. The VM holds one
/// clone of the handle inside its [`GlobalState`](scarpet_vm::GlobalState); the
/// caller keeps another to read the bytes back. Single-threaded (`Rc`/`RefCell`),
/// matching the browser's `wasm` runtime.
#[derive(Clone)]
pub struct SharedBuffer(pub Rc<RefCell<Vec<u8>>>);

impl std::io::Write for SharedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// One line of the diagnostics strip, coloured by how much it matters.
#[derive(Clone, PartialEq, Eq)]
pub struct Diag {
    pub severity: Severity,
    pub text: String,
}

impl Diag {
    /// A finding with no location — a lowering or run-time error, neither of
    /// which carries a span.
    pub fn error(text: impl Into<String>) -> Diag {
        Diag {
            severity: Severity::Error,
            text: text.into(),
        }
    }

    /// The Tailwind text colour for this line's severity.
    pub fn class(&self) -> &'static str {
        match self.severity {
            Severity::Error => "text-error",
            Severity::Warning => "text-warning",
            Severity::Info => "text-mute",
        }
    }
}

/// Render a `scarpet-hir` diagnostic as a `start..end  message` headline plus
/// an optional `help:` line and any related locations.
pub fn render(diagnostic: &Diagnostic) -> Vec<Diag> {
    let mut out = vec![Diag {
        severity: diagnostic.severity,
        text: format!(
            "{}..{}  {}",
            diagnostic.span.start, diagnostic.span.end, diagnostic.message
        ),
    }];
    if let Some(help) = &diagnostic.help {
        out.push(Diag {
            severity: Severity::Info,
            text: format!("help: {help}"),
        });
    }
    for related in &diagnostic.related {
        out.push(Diag {
            severity: Severity::Info,
            text: format!(
                "{}..{}  {}",
                related.span.start, related.span.end, related.message
            ),
        });
    }
    out
}

/// Render a parse error, by way of the same [`Diagnostic`] every other finding
/// goes through — so the editor has one rendering path rather than two.
pub fn diagnostics_for(err: &ParseError) -> Vec<Diag> {
    render(&Diagnostic::from_parse_error(err))
}
