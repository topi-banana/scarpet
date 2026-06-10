//! The two-pane editor: type Scarpet on the left, then run the formatter, dump
//! the CST, lower to the AST, or evaluate it once on the right.
//!
//! The view is split into two presentational components — [`EditorActions`] (the
//! header buttons) and [`EditorView`] (the panes), in [`view`] — while the tool
//! logic stays on [`App`], which the components reach only through callbacks.

mod view;

use scarpet_syntax::ast::Code;
use scarpet_syntax::parser::ParseError;
use scarpet_vm::{Evaluate, GlobalState};
use std::cell::RefCell;
use std::rc::Rc;

use crate::app::App;
use crate::shared::{SharedBuffer, diagnostics_for};

pub use view::{EditorActions, EditorView};

/// Which tool produced the current editor output.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// `scarpet-fmt` pretty-printer output.
    Format,
    /// `scarpet-syntax` lossless CST dump.
    Syntax,
    /// `scarpet-syntax` typed AST dump — the CST lowered via `Code::try_from`.
    Ast,
    /// `scarpet-vm` evaluation — lower to the AST, run it once, and show what the
    /// program printed (captured from the VM's stdout).
    Run,
}

impl Mode {
    /// Label shown above the output pane.
    fn output_title(self) -> &'static str {
        match self {
            Mode::Format => "Formatted",
            Mode::Syntax => "Syntax tree",
            Mode::Ast => "AST",
            Mode::Run => "Output",
        }
    }
}

/// A small sample so the playground does something on first load.
pub const SAMPLE: &str = "// Scarpet sample — hit Run, Format, Syntax tree, or AST.
fib(n) -> if(n < 2, n, fib(n-1)+fib(n-2));
print('fib(10) = '+fib(10));
print('fib(20) = '+fib(20));

fizzbuzz(n) -> for(range(1, n + 1), if(_ % 15 == 0, print('FizzBuzz'), _ % 3 == 0, print('Fizz'), _ % 5 == 0, print('Buzz'), print(_)));
fizzbuzz(20)
";

impl App {
    /// Run `mode` over the current editor input, refreshing `output` and
    /// `diagnostics`.
    pub(crate) fn run(&mut self, mode: Mode) {
        match mode {
            Mode::Format => match scarpet_fmt::format_source(&self.input, &self.config) {
                Ok(formatted) => {
                    self.output = formatted;
                    self.diagnostics = Vec::new();
                }
                Err(scarpet_fmt::FmtError::Parse(err)) => self.report_parse(&err),
            },
            Mode::Syntax => match scarpet_syntax::parser::parse_source(&self.input) {
                Ok(cst) => {
                    self.output = format!("{cst:#?}");
                    self.diagnostics = Vec::new();
                }
                Err(err) => self.report_parse(&err),
            },
            // The AST is the CST lowered via `Code::try_from`, so it has two
            // failure modes: a parse error (shared with the other tools) or a
            // lowering error where a well-formed parse has no valid AST (e.g.
            // `1 = 2`, no assignable target). The lowering error carries no span.
            Mode::Ast => match scarpet_syntax::parser::parse_source(&self.input) {
                Ok(cst) => match Code::try_from(&cst) {
                    Ok(code) => {
                        self.output = format!("{code:#?}");
                        self.diagnostics = Vec::new();
                    }
                    Err(err) => {
                        self.output = String::new();
                        self.diagnostics = vec![err.to_string()];
                        self.diagnostics_title = "Lowering error";
                    }
                },
                Err(err) => self.report_parse(&err),
            },
            Mode::Run => self.run_vm(),
        }
        self.mode = Some(mode);
    }

    /// Evaluate the editor input with `scarpet-vm` and show what it printed. This
    /// is a fresh, one-shot VM each time (unlike the notebook's persistent
    /// kernel): parse and lower like [`Mode::Ast`], then run the AST in a VM
    /// whose `print` output is captured into a buffer.
    fn run_vm(&mut self) {
        let cst = match scarpet_syntax::parser::parse_source(&self.input) {
            Ok(cst) => cst,
            Err(err) => return self.report_parse(&err),
        };
        let code = match Code::try_from(&cst) {
            Ok(code) => code,
            Err(err) => {
                self.output = String::new();
                self.diagnostics = vec![err.to_string()];
                self.diagnostics_title = "Lowering error";
                return;
            }
        };
        let buffer = Rc::new(RefCell::new(Vec::<u8>::new()));
        let mut global = GlobalState::with_stdout(Box::new(SharedBuffer(buffer.clone())));
        let mut vm = global.create_new_vm();
        let result = vm.push(code);
        let printed = String::from_utf8_lossy(&buffer.borrow()).into_owned();
        match result {
            Ok(_) => {
                self.output = printed;
                self.diagnostics = Vec::new();
            }
            Err(err) => {
                self.output = printed;
                self.diagnostics = vec![err.to_string()];
                self.diagnostics_title = "Runtime error";
            }
        }
    }

    /// Record a parse error as the current diagnostics, clearing any stale output.
    fn report_parse(&mut self, err: &ParseError) {
        self.output = String::new();
        self.diagnostics = diagnostics_for(err);
        self.diagnostics_title = "Parse error";
    }

    /// Re-run the formatter in place after a config change, but only while the
    /// Format view is showing. The syntax tree and AST are config-independent.
    pub(crate) fn reformat_if_showing(&mut self) {
        if self.mode == Some(Mode::Format) {
            self.run(Mode::Format);
        }
    }
}
