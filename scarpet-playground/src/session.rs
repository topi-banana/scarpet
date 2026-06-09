//! The persistent notebook kernel: one `scarpet-vm` session reused across cells,
//! so a binding or function defined in one cell is visible to the next.

use std::cell::RefCell;
use std::rc::Rc;

use scarpet_syntax::ast::Code;
use scarpet_vm::{Evalute, GlobalState, ScarpetVm};

use crate::shared::{SharedBuffer, diagnostics_for};

/// The outcome of running one cell, as rendered beneath it.
pub enum CellOutput {
    /// Never run, or cleared by a restart.
    NotRun,
    /// Ran cleanly: `printed` is what the cell sent to `print`; `value` is the
    /// cell's result rendered via [`Value::to_scarpet_string`].
    ///
    /// [`Value::to_scarpet_string`]: scarpet_vm::Value::to_scarpet_string
    Ok { printed: String, value: String },
    /// A parse, lowering, or runtime failure. `title` names the stage and
    /// `lines` are the human-readable messages; `printed` keeps any output the
    /// cell produced before a runtime error (empty for parse/lowering errors).
    Error {
        title: &'static str,
        printed: String,
        lines: Vec<String>,
    },
}

/// A persistent interpreter session shared by every cell, mirroring the CLI
/// REPL: variables live in the one [`ScarpetVm`], function definitions in its
/// [`GlobalState`].
///
/// Holding a `ScarpetVm` (which borrows `&mut GlobalState`) across renders would
/// be self-referential, so the `GlobalState` is leaked to `'static` and the VM
/// borrows that — exactly the REPL's trick, where each submission is leaked too.
/// A restart drops this and builds a fresh one; the leaked state stays
/// allocated, which is fine for a browser dev tool.
pub struct Session {
    vm: ScarpetVm<'static, 'static>,
    /// The `print` sink, shared with the VM's `GlobalState`. Cleared before each
    /// run and read back after, so the captured text is just that cell's output.
    buffer: Rc<RefCell<Vec<u8>>>,
    /// Monotonic execution counter shown as each cell's `[n]` badge; restarts at
    /// zero only when the session is rebuilt.
    pub(crate) exec_counter: u32,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// A fresh session: a leaked `'static` [`GlobalState`] wired to a capture
    /// buffer, plus a VM over it with an empty variable scope.
    pub fn new() -> Self {
        let buffer = Rc::new(RefCell::new(Vec::<u8>::new()));
        // Leak to 'static so the VM can borrow the state for the whole session.
        let global: &'static mut GlobalState<'static> = Box::leak(Box::new(
            GlobalState::with_stdout(Box::new(SharedBuffer(buffer.clone()))),
        ));
        let vm = global.create_new_vm();
        Session {
            vm,
            buffer,
            exec_counter: 0,
        }
    }

    /// Parse, lower, and evaluate `source` in the persistent VM, returning what
    /// to show beneath the cell. Bumps [`exec_counter`](Self::exec_counter)
    /// first, so the caller can read it for the cell's badge. Definitions and
    /// bindings persist to later runs.
    pub fn run(&mut self, source: &str) -> CellOutput {
        self.exec_counter += 1;
        // Leak the cell source so `Code<'static>` (and any function bodies it
        // defines into the kernel) stay valid for the session's lifetime.
        let src: &'static str = Box::leak(source.to_owned().into_boxed_str());
        let cst = match scarpet_syntax::parser::parse_source(src) {
            Ok(cst) => cst,
            Err(err) => {
                return CellOutput::Error {
                    title: "Parse error",
                    printed: String::new(),
                    lines: diagnostics_for(&err),
                };
            }
        };
        // `Code<'static>` borrows `src`, not `cst`, so dropping `cst` after is fine.
        let code = match Code::try_from(&cst) {
            Ok(code) => code,
            Err(err) => {
                return CellOutput::Error {
                    title: "Lowering error",
                    printed: String::new(),
                    lines: vec![err.to_string()],
                };
            }
        };
        // Clear first, so what we read back is only this cell's `print` output.
        self.buffer.borrow_mut().clear();
        let result = self.vm.push(code);
        let printed = String::from_utf8_lossy(&self.buffer.borrow()).into_owned();
        match result {
            Ok(container) => match container.lock() {
                Ok(value) => CellOutput::Ok {
                    printed,
                    value: value.to_scarpet_string(),
                },
                Err(err) => CellOutput::Error {
                    title: "Runtime error",
                    printed,
                    lines: vec![err.to_string()],
                },
            },
            // Keep whatever printed before the error, as the editor's Run does.
            Err(err) => CellOutput::Error {
                title: "Runtime error",
                printed,
                lines: vec![err.to_string()],
            },
        }
    }
}
