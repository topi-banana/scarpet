use std::rc::Rc;

use scarpet_syntax::ast::{
    Additive, Args, Assign, Code, Compare, Equality, Expr, Get, Land, Lor, Mult, Power, Primary,
    Unary,
};

use crate::{
    error::VmError,
    eval::Evalute,
    value::ValueContainer,
    vm::{GlobalState, ScarpetVm},
};

/// A callable: a builtin (a unit struct such as [`Type`]) or a user-defined
/// [`DefFunction`]. `call` receives the already-evaluated arguments plus the vm,
/// which a builtin may ignore and a user function uses to evaluate its body.
pub trait Function<'src> {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Vec<ValueContainer>,
    ) -> Result<ValueContainer, VmError>;
}

/// Register the builtin functions into a fresh [`GlobalState`].
pub(crate) fn register_builtins(state: &mut GlobalState<'_>) {
    state.register("type", Rc::new(Type));
    state.register("str", Rc::new(Str));
    state.register("print", Rc::new(Print));
}

/// Unwrap the single argument of a one-arity builtin.
fn arg1(mut args: Vec<ValueContainer>) -> Result<ValueContainer, VmError> {
    if args.len() != 1 {
        return Err(VmError::WrongArgCount);
    }
    Ok(args.pop().unwrap())
}

/// `type(x)` — the value's type name (the original `type`).
struct Type;
impl<'src> Function<'src> for Type {
    fn call(
        &self,
        _vm: &mut ScarpetVm<'_, 'src>,
        args: Vec<ValueContainer>,
    ) -> Result<ValueContainer, VmError> {
        let name = arg1(args)?.lock()?.type_name();
        Ok(ValueContainer::string(name.to_owned()))
    }
}

/// `str(x)` — the value's plain string form (the original `str` with no
/// formatting directives).
struct Str;
impl<'src> Function<'src> for Str {
    fn call(
        &self,
        _vm: &mut ScarpetVm<'_, 'src>,
        args: Vec<ValueContainer>,
    ) -> Result<ValueContainer, VmError> {
        let s = arg1(args)?.lock()?.to_scarpet_string();
        Ok(ValueContainer::string(s))
    }
}

/// `print(x)` — write the value's string form to stdout and return it.
struct Print;
impl<'src> Function<'src> for Print {
    fn call(
        &self,
        _vm: &mut ScarpetVm<'_, 'src>,
        args: Vec<ValueContainer>,
    ) -> Result<ValueContainer, VmError> {
        let value = arg1(args)?;
        println!("{}", value.lock()?.to_scarpet_string());
        Ok(value)
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
        args: Vec<ValueContainer>,
    ) -> Result<ValueContainer, VmError> {
        if args.len() != self.params.len() {
            return Err(VmError::WrongArgCount);
        }
        // A function body runs in its own scope: swap the caller's variables out
        // and restore them after. Scarpet functions do not see caller locals
        // without `outer` / `global`, which are not modelled yet.
        let saved = std::mem::take(&mut vm.var);
        for (name, arg) in self.params.iter().zip(args) {
            vm.var.insert((*name).to_owned(), arg);
        }
        let result = vm.push((*self.body).clone());
        vm.var = saved;
        result
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
