use std::rc::Rc;

use scarpet_syntax::ast::{
    Additive, Args, Assign, Code, Compare, Equality, Expr, Get, Land, Lor, Mult, Power, Primary,
    Unary,
};

use crate::{
    Value,
    error::VmError,
    eval::Evalute,
    value::ValueContainer,
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
    pub fn new(params: &Args<'src>, body: Box<Expr<'src>>) -> Option<Self> {
        let Args(codes) = params;
        let params = codes.iter().map(code_ident).collect::<Option<Vec<_>>>()?;
        Some(Self { params, body })
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

// Parameter-name extraction: a plain binder like `x` is a one-expression `Code`
// that threads down every passthrough level of the ladder to a `Primary::Ident`.
// Any operator on the way (so not a bare name) makes this `None`.

fn code_ident<'s>(code: &Code<'s>) -> Option<&'s str> {
    match code.0.as_slice() {
        [expr] => expr_ident(expr),
        _ => None,
    }
}
fn expr_ident<'s>(e: &Expr<'s>) -> Option<&'s str> {
    if let Expr::Assign(a) = e {
        assign_ident(a)
    } else {
        None
    }
}
fn assign_ident<'s>(a: &Assign<'s>) -> Option<&'s str> {
    if let Assign::Lor(l) = a {
        lor_ident(l)
    } else {
        None
    }
}
fn lor_ident<'s>(l: &Lor<'s>) -> Option<&'s str> {
    if let Lor::Land(x) = l {
        land_ident(x)
    } else {
        None
    }
}
fn land_ident<'s>(l: &Land<'s>) -> Option<&'s str> {
    if let Land::Equality(x) = l {
        equality_ident(x)
    } else {
        None
    }
}
fn equality_ident<'s>(e: &Equality<'s>) -> Option<&'s str> {
    if let Equality::Compare(x) = e {
        compare_ident(x)
    } else {
        None
    }
}
fn compare_ident<'s>(c: &Compare<'s>) -> Option<&'s str> {
    if let Compare::Additive(x) = c {
        additive_ident(x)
    } else {
        None
    }
}
fn additive_ident<'s>(a: &Additive<'s>) -> Option<&'s str> {
    if let Additive::Mult(x) = a {
        mult_ident(x)
    } else {
        None
    }
}
fn mult_ident<'s>(m: &Mult<'s>) -> Option<&'s str> {
    if let Mult::Power(x) = m {
        power_ident(x)
    } else {
        None
    }
}
fn power_ident<'s>(p: &Power<'s>) -> Option<&'s str> {
    if let Power::Unary(x) = p {
        unary_ident(x)
    } else {
        None
    }
}
fn unary_ident<'s>(u: &Unary<'s>) -> Option<&'s str> {
    if let Unary::Get(x) = u {
        get_ident(x)
    } else {
        None
    }
}
fn get_ident<'s>(g: &Get<'s>) -> Option<&'s str> {
    if let Get::Primary(x) = g {
        primary_ident(x)
    } else {
        None
    }
}
fn primary_ident<'s>(p: &Primary<'s>) -> Option<&'s str> {
    if let Primary::Ident(name) = p {
        Some(*name)
    } else {
        None
    }
}
