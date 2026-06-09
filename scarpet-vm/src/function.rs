use std::rc::Rc;

use scarpet_syntax::ast::Args;

use crate::{
    error::VmError,
    value::ValueContainer,
    vm::{GlobalState, ScarpetVm},
};

mod builtin;
mod def;
mod iter;

pub use def::DefFunction;

/// A callable: a builtin (a unit struct such as [`Type`](builtin::Type)) or a
/// user-defined [`DefFunction`]. `call` receives the still-unevaluated argument
/// [`Args`] plus the vm, and evaluates through the vm whichever arguments it
/// needs: an ordinary function evaluates them all up front, while a special form
/// like [`Call`](builtin::Call) can evaluate them selectively.
pub trait Function<'src> {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        args: Args<'src>,
    ) -> Result<ValueContainer, VmError>;
}

/// Register the builtin functions into a fresh [`GlobalState`].
pub(crate) fn register_builtins(state: &mut GlobalState<'_>) {
    state.register("type", Rc::new(builtin::Type));
    state.register("str", Rc::new(builtin::Str));
    state.register("print", Rc::new(builtin::Print));
    state.register("call", Rc::new(builtin::Call));
    state.register("if", Rc::new(builtin::If));
    state.register("range", Rc::new(iter::Range));
    state.register("for", Rc::new(iter::For));
}
