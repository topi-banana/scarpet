//! Shared helpers for the evaluator and builtin tests: parse, lower, and
//! evaluate a source string, with a variant that captures `print` output.

use scarpet_syntax::ast::Code;
use scarpet_syntax::parser::parse_source;

use crate::error::VmError;
use crate::eval::Evaluate;
use crate::value::Value;
use crate::vm::GlobalState;

/// Parse, lower, and evaluate `src` in a fresh VM, returning its value.
pub(crate) fn eval(src: &str) -> Value {
    let cst = parse_source(src).expect("parse");
    let code = Code::try_from(&cst).expect("lower");
    let mut global = GlobalState::new();
    let mut vm = global.create_new_vm();
    vm.push(code).expect("eval").lock().expect("lock").clone()
}

/// Like [`eval`], but expects evaluation to fail and returns the `VmError`.
pub(crate) fn eval_err(src: &str) -> VmError {
    let cst = parse_source(src).expect("parse");
    let code = Code::try_from(&cst).expect("lower");
    let mut global = GlobalState::new();
    let mut vm = global.create_new_vm();
    vm.push(code).expect_err("expected an evaluation error")
}

/// Parse, lower, and evaluate `src`, returning everything it wrote to `print`'s
/// configured stdout — the same in-memory capture the playground uses to show a
/// program's output.
pub(crate) fn eval_capturing_stdout(src: &str) -> String {
    use std::sync::{Arc, Mutex};

    /// A `Write` sink over a shared buffer, mirroring the playground's capture
    /// writer.
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let cst = parse_source(src).expect("parse");
    let code = Code::try_from(&cst).expect("lower");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let mut global = GlobalState::with_stdout(Box::new(Buf(captured.clone())));
    let mut vm = global.create_new_vm();
    vm.push(code).expect("eval");
    String::from_utf8(captured.lock().unwrap().clone()).unwrap()
}
