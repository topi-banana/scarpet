use scarpet_syntax::ast::Args;

use super::Function;
use crate::{Value, error::VmError, eval::Evalute, value::ValueContainer, vm::ScarpetVm};

/// Evaluate the single argument of a one-arity builtin.
fn arg1<'src>(
    vm: &mut ScarpetVm<'_, 'src>,
    Args(mut args): Args<'src>,
) -> Result<ValueContainer, VmError> {
    if args.len() != 1 {
        return Err(VmError::WrongArgCount);
    }
    vm.push(args.pop_front().unwrap())
}

/// `type(x)` — the value's type name (the original `type`).
pub(super) struct Type;
impl<'src> Function<'src> for Type {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        let name = arg1(vm, args)?.lock()?.type_name();
        Ok(ValueContainer::string(name.to_owned()))
    }
}

/// `str(x)` — the value's plain string form (the original `str` with no
/// formatting directives).
pub(super) struct Str;
impl<'src> Function<'src> for Str {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        let s = arg1(vm, args)?.lock()?.to_scarpet_string();
        Ok(ValueContainer::string(s))
    }
}

/// `print(x)` — write the value's string form, newline-terminated, to the VM's
/// configured standard output and return the value. The CLI shows it on its
/// stdout; the playground captures it into a buffer to display.
pub(super) struct Print;
impl<'src> Function<'src> for Print {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        let value = arg1(vm, args)?;
        // Stringify (releasing the value's lock) before writing, so the write
        // borrows only the vm — `print` returns the same value it printed.
        let line = value.lock()?.to_scarpet_string();
        vm.write_line(&line)?;
        Ok(value)
    }
}

pub(super) struct Call;
impl<'src> Function<'src> for Call {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        let Args(mut codes) = args;
        let Some(arg) = codes.pop_front() else {
            return Err(VmError::WrongArgCount);
        };
        // The first argument names the function to call. The original `call`
        // also accepts a first-class function value, but this VM has no
        // function-value type yet, so only a string name resolves to a callable.
        let Value::String(name) = vm.push(arg)?.lock()?.clone() else {
            return Err(VmError::UnknownFunction);
        };
        let Some(function) = vm.function(&name) else {
            return Err(VmError::UnknownFunction);
        };
        function.call(vm, Args(codes))
    }
}

pub(super) struct If;
impl<'src> Function<'src> for If {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        Args(mut args): Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        while let Some(cord) = args.pop_front() {
            let value = vm.push(cord);
            let Some(expr) = args.pop_front() else {
                return value;
            };
            if value?.lock()?.is_true() {
                return vm.push(expr);
            }
        }
        Ok(ValueContainer::null())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::{eval, eval_capturing_stdout};
    use crate::value::Value;

    #[test]
    fn builtin_type_and_str() {
        assert_eq!(eval("type(5)"), Value::String("number".to_owned()));
        assert_eq!(eval("type('hi')"), Value::String("string".to_owned()));
        assert_eq!(eval("str([1, 2])"), Value::String("[1, 2]".to_owned()));
    }

    /// `print` writes to stdout (not checked here) and returns its argument.
    #[test]
    fn builtin_print_returns_its_argument() {
        assert_eq!(eval("print('hi')"), Value::String("hi".to_owned()));
    }

    /// `print` writes each value's string form, newline-terminated, to the
    /// [`GlobalState`](crate::vm::GlobalState)'s configured stdout — here a
    /// shared buffer, exactly as the playground captures it to display a
    /// program's output.
    #[test]
    fn builtin_print_writes_lines_to_configured_stdout() {
        let text = eval_capturing_stdout("print('hello'); print(6 * 7)");
        assert_eq!(text, "hello\n42\n");
    }
}
