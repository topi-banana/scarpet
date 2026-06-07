use std::rc::Rc;

use scarpet_syntax::ast::{Args, Assignable, Expr, Patterns};

use crate::{
    Value,
    error::VmError,
    eval::Evalute,
    value::{RangeList, ValueContainer},
    vm::{GlobalState, ScarpetVm},
};

/// A callable: a builtin (a unit struct such as [`Type`]) or a user-defined
/// [`DefFunction`]. `call` receives the still-unevaluated argument [`Args`] plus
/// the vm, and evaluates through the vm whichever arguments it needs: an ordinary
/// function evaluates them all up front, while a special form like [`Call`] can
/// evaluate them selectively.
pub trait Function<'src> {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Args<'src>,
    ) -> Result<ValueContainer, VmError>;
}

/// Register the builtin functions into a fresh [`GlobalState`].
pub(crate) fn register_builtins(state: &mut GlobalState<'_>) {
    state.register("type", Rc::new(Type));
    state.register("str", Rc::new(Str));
    state.register("print", Rc::new(Print));
    state.register("call", Rc::new(Call));
    state.register("if", Rc::new(If));
    state.register("range", Rc::new(Range));
}

/// Evaluate the single argument of a one-arity builtin.
fn arg1<'src>(
    vm: &mut ScarpetVm<'_, 'src>,
    Args(mut args): Args<'src>,
) -> Result<ValueContainer, VmError> {
    if args.len() != 1 {
        return Err(VmError::WrongArgCount);
    }
    vm.push(args.pop().unwrap())
}

/// `type(x)` — the value's type name (the original `type`).
struct Type;
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
struct Str;
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

/// `print(x)` — write the value's string form to stdout and return it.
struct Print;
impl<'src> Function<'src> for Print {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        let value = arg1(vm, args)?;
        println!("{}", value.lock()?.to_scarpet_string());
        Ok(value)
    }
}

struct Call;
impl<'src> Function<'src> for Call {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        let Args(mut codes) = args;
        if codes.is_empty() {
            return Err(VmError::WrongArgCount);
        }
        // The first argument names the function to call. The original `call`
        // also accepts a first-class function value, but this VM has no
        // function-value type yet, so only a string name resolves to a callable.
        let Value::String(name) = vm.push(codes.remove(0))?.lock()?.clone() else {
            return Err(VmError::UnknownFunction);
        };
        let Some(function) = vm.function(&name) else {
            return Err(VmError::UnknownFunction);
        };
        function.call(vm, Args(codes))
    }
}

struct If;
impl<'src> Function<'src> for If {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        Args(mut args): Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        args.reverse();
        let Some(res) = args.pop() else {
            return Err(VmError::WrongArgCount);
        };
        let if_true = args.pop();
        let if_false = args.pop();
        if vm.push(res)?.lock()?.is_true() {
            if let Some(expr) = if_true {
                return vm.push(expr);
            }
        } else if let Some(expr) = if_false {
            return vm.push(expr);
        }
        Ok(ValueContainer::null())
    }
}

/// `range(to)` / `range(from, to)` / `range(from, to, step)` — a lazy arithmetic
/// progression (the original `range`, whose `type()` is "iterator"). `from`
/// defaults to `0` and `step` to `1`, and `to` is exclusive. Each bound coerces
/// to a number; the range stays integral unless a bound is fractional. A
/// negative step counts down; a zero or wrong-way step is an empty range. The
/// list is generated lazily — `range(1000000)` is a handful of numbers until
/// something walks it.
struct Range;
impl<'src> Function<'src> for Range {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        Args(codes): Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        if codes.is_empty() || codes.len() > 3 {
            return Err(VmError::WrongArgCount);
        }
        let nums = codes
            .into_iter()
            .map(|code| Ok(vm.push(code)?.lock()?.clone()))
            .collect::<Result<Vec<Value>, VmError>>()?;
        let (zero, one) = (Value::Int(0), Value::Int(1));
        let range = match nums.as_slice() {
            [to] => RangeList::new(&zero, to, &one),
            [from, to] => RangeList::new(from, to, &one),
            [from, to, step] => RangeList::new(from, to, step),
            // The argument count is bounded to 1..=3 above.
            _ => unreachable!(),
        }?;
        Ok(ValueContainer::new(Value::List(Box::new(range))))
    }
}

/// A user-defined function: its parameter names and its body AST. `call` runs
/// the body in a fresh variable scope with the parameters bound to the args.
pub struct DefFunction<'src> {
    params: Vec<&'src str>,
    body: Box<Expr<'src>>,
}

impl<'src> DefFunction<'src> {
    /// Build from an `Expr::Def`'s parameter list and body. Each parameter must
    /// be a plain variable name; anything richer (literal patterns, `...rest`,
    /// `outer(x)`) is not modelled yet and yields `None`.
    pub fn new(params: &Patterns<'src>, body: Box<Expr<'src>>) -> Option<Self> {
        // A rest binder is unsupported for now; only a fixed list of plain
        // binders is bound. Anything else (a destructure, an `outer(x)`
        // capture, a literal pattern) also yields `None`.
        if params.rest.is_some() {
            return None;
        }
        let names = params
            .before
            .iter()
            .map(|p| match p {
                Assignable::Var(name) => Some(*name),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            params: names,
            body,
        })
    }
}

impl<'src> Function<'src> for DefFunction<'src> {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        let Args(codes) = args;
        if codes.len() != self.params.len() {
            return Err(VmError::WrongArgCount);
        }
        // A function body runs in its own VM over the same global state, with
        // only the parameters bound. Scarpet functions do not see caller locals
        // without `outer` / `global`, which are not modelled yet. Each parameter
        // gets its own fresh slot holding a copy of the argument's value, so the
        // body cannot reach back and mutate a caller variable passed by name.
        //
        // The arguments are evaluated here, in the caller's scope, before the
        // child exists: `inner` borrows `vm`, so `vm.push` is unavailable once
        // it does.
        let values = codes
            .into_iter()
            .map(|arg| Ok(vm.push(arg)?.lock()?.clone()))
            .collect::<Result<Vec<Value>, VmError>>()?;
        let mut inner = vm.child();
        for (name, value) in self.params.iter().zip(values) {
            *inner.get_var(name).lock()? = value;
        }
        inner.push((*self.body).clone())
    }
}
