use scarpet_syntax::ast::{Args, Expr, Params};

use super::Function;
use crate::{Value, error::VmError, eval::Evaluate, value::ValueContainer, vm::ScarpetVm};

/// A user-defined function, lowered from a validated [`Params`] signature: the
/// positional parameter names, the `outer(x)` captures (each a shared slot grabbed
/// from the defining scope at definition time), the optional `...rest` name, and
/// the body AST. `call` runs the body in a fresh scope with the captures injected
/// and the arguments bound.
pub struct DefFunction<'src> {
    fixed: Vec<&'src str>,
    captures: Vec<(&'src str, ValueContainer)>,
    rest: Option<&'src str>,
    body: Box<Expr<'src>>,
}

impl<'src> DefFunction<'src> {
    /// Build from an `Expr::Def`'s [`Params`], its already-resolved `outer(x)`
    /// captures (the caller resolves them from the defining scope, where it has the
    /// vm), and its body. Infallible: lowering has already validated the signature.
    pub fn new(
        params: &Params<'src>,
        captures: Vec<(&'src str, ValueContainer)>,
        body: Box<Expr<'src>>,
    ) -> Self {
        Self {
            fixed: params.fixed.clone(),
            captures,
            rest: params.rest,
            body,
        }
    }
}

impl<'src> Function<'src> for DefFunction<'src> {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        let Args(codes) = args;
        // The fixed parameters must all be supplied; with a `...rest`, extra
        // arguments are allowed (and collected), so the count is a lower bound.
        let ok = match self.rest {
            None => codes.len() == self.fixed.len(),
            Some(_) => codes.len() >= self.fixed.len(),
        };
        if !ok {
            return Err(VmError::WrongArgCount);
        }
        // Arguments are evaluated here, in the caller's scope, before the child
        // exists (`inner` borrows `vm`, so `vm.push` is unavailable once it does).
        let mut values = codes
            .into_iter()
            .map(|arg| Ok(vm.push(arg)?.lock()?.clone()))
            .collect::<Result<Vec<Value>, VmError>>()?;
        // Everything past the fixed parameters is the rest (an empty list when the
        // function has a `...rest` but no extra arguments were passed).
        let rest_values = values.split_off(self.fixed.len());

        // The body runs in its own VM over the same global state. `outer(x)`
        // captures are injected as the *shared* defining-scope slots, so the body
        // sees and can update them; the parameters get fresh slots holding a copy
        // of each argument, so the body cannot reach back into a caller local.
        let mut inner = vm.child();
        for (name, slot) in &self.captures {
            inner.bind(name, slot.clone());
        }
        for (name, value) in self.fixed.iter().zip(values) {
            *inner.get_var(name).lock()? = value;
        }
        if let Some(rest) = self.rest {
            *inner.get_var(rest).lock()? = Value::list(rest_values);
        }
        inner.push((*self.body).clone())
    }
}

#[cfg(test)]
mod tests {
    use crate::error::VmError;
    use crate::test_util::{eval, eval_err};
    use crate::value::Value;

    #[test]
    fn user_function_definition_and_call() {
        assert_eq!(eval("f(x) -> x * 2; f(5)"), Value::Int(10));
        assert_eq!(eval("add(x, y) -> x + y; add(3, 4)"), Value::Int(7));
    }

    /// A function body runs in its own scope, so a caller local is not visible
    /// inside it (it reads as undef).
    #[test]
    fn function_body_has_its_own_scope() {
        assert_eq!(eval("a = 10; f() -> a; f()"), Value::Undef);
    }

    /// `outer(x)` captures a defining-scope variable so the body can see it (the
    /// body otherwise has no access to caller locals).
    #[test]
    fn outer_captures_a_defining_scope_variable() {
        assert_eq!(eval("o = 9; f(a, outer(o)) -> a + o; f(1)"), Value::Int(10));
    }

    /// An `outer(x)` capture does not consume a positional argument, so the fixed
    /// parameters still bind by position.
    #[test]
    fn outer_capture_does_not_consume_a_position() {
        assert_eq!(
            eval("o = 100; g(x, outer(o), y) -> x + y + o; g(1, 2)"),
            Value::Int(103)
        );
    }

    /// `...rest` collects the trailing arguments into a list.
    #[test]
    fn rest_parameter_collects_trailing_arguments() {
        assert_eq!(
            eval("g(a, ...rest) -> rest; g(1, 2, 3)"),
            Value::list(vec![Value::Int(2), Value::Int(3)])
        );
        // The fixed parameter still binds.
        assert_eq!(eval("g(a, ...rest) -> a; g(1, 2, 3)"), Value::Int(1));
    }

    /// `...rest` is an empty list when no arguments are left for it.
    #[test]
    fn rest_parameter_is_empty_without_extras() {
        assert_eq!(eval("g(a, ...rest) -> rest; g(1)"), Value::list(vec![]));
    }

    /// A `...rest` signature accepts any argument count at or above the fixed
    /// parameters; fewer is still an error.
    #[test]
    fn rest_parameter_arity_lower_bound() {
        assert!(matches!(
            eval_err("g(a, ...rest) -> a; g()"),
            VmError::WrongArgCount
        ));
    }

    #[test]
    fn unknown_function_is_an_error() {
        assert!(matches!(eval_err("nope(1)"), VmError::UnknownFunction));
    }

    #[test]
    fn wrong_argument_count_is_an_error() {
        assert!(matches!(
            eval_err("f(x) -> x; f(1, 2)"),
            VmError::WrongArgCount
        ));
    }
}
