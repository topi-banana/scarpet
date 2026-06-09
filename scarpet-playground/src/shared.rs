//! Bits shared by the editor and notebook views: the `print`-capture sink, the
//! parse-error formatter, and the Tailwind class strings reused across buttons
//! and panes.

use std::cell::RefCell;
use std::rc::Rc;

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

/// Render a parse error as a `start..end  message` headline plus an optional
/// `help:` line — the form both views show in their diagnostics strip.
pub fn diagnostics_for(err: &ParseError) -> Vec<String> {
    let mut out = vec![format!(
        "{}..{}  {}",
        err.span.start,
        err.span.end,
        err.message()
    )];
    if let Some(help) = &err.help {
        out.push(format!("help: {help}"));
    }
    out
}
