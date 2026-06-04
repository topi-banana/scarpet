use std::{collections::BTreeMap, rc::Rc};

use crate::{
    function::{Function, register_builtins},
    value::ValueContainer,
};

/// Shared state that outlives any single [`ScarpetVm`]: the function table.
///
/// Functions are keyed by name and stored behind an [`Rc`] rather than a [`Box`]
/// so a call site can clone the handle out of the table (releasing the borrow on
/// `GlobalState`) before invoking it with the `&mut ScarpetVm` it needs — a
/// `Box<dyn Function>` would keep the table borrowed for the whole call.
///
/// The `'src` lifetime is the source the bodies of user-defined functions
/// ([`DefFunction`](crate::function::DefFunction)) borrow from; builtins are
/// `'static` and fit any `'src`.
pub struct GlobalState<'src> {
    functions: BTreeMap<String, Rc<dyn Function<'src> + 'src>>,
}

impl<'src> Default for GlobalState<'src> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'src> GlobalState<'src> {
    /// A fresh state with the builtin functions registered.
    pub fn new() -> Self {
        let mut state = Self {
            functions: BTreeMap::new(),
        };
        register_builtins(&mut state);
        state
    }

    /// Register (or replace) a function under `name`.
    pub fn register(&mut self, name: &str, function: Rc<dyn Function<'src> + 'src>) {
        self.functions.insert(name.to_owned(), function);
    }

    /// Look up a function, cloning the `Rc` so the table is no longer borrowed.
    pub(crate) fn function(&self, name: &str) -> Option<Rc<dyn Function<'src> + 'src>> {
        self.functions.get(name).cloned()
    }

    pub fn create_new_vm<'me>(&'me mut self) -> ScarpetVm<'me, 'src> {
        ScarpetVm::new(self)
    }
}

pub struct ScarpetVm<'state, 'src> {
    global: &'state mut GlobalState<'src>,
    var: BTreeMap<String, ValueContainer>,
}

impl<'state, 'src> ScarpetVm<'state, 'src> {
    pub fn new(global: &'state mut GlobalState<'src>) -> Self {
        Self {
            global,
            var: BTreeMap::new(),
        }
    }

    /// A fresh VM over the same [`GlobalState`] but with its own empty variable
    /// scope. A user function body runs in one of these: it shares the function
    /// table (so it can call anything the caller can, and its own `Def`s
    /// persist) but does not inherit the caller's locals — Scarpet functions
    /// only reach outer scopes through `outer` / `global`, not modelled yet.
    pub(crate) fn child(&mut self) -> ScarpetVm<'_, 'src> {
        self.global.create_new_vm()
    }

    /// The function registered under `name`, if any (builtin or user-defined).
    pub(crate) fn function(&self, name: &str) -> Option<Rc<dyn Function<'src> + 'src>> {
        self.global.function(name)
    }

    /// Define (or redefine) a user function in the shared state.
    pub(crate) fn define(&mut self, name: &str, function: Rc<dyn Function<'src> + 'src>) {
        self.global.register(name, function);
    }

    /// The container bound to `name` in this VM's local scope, inserting a fresh
    /// `undef` binding first if the name is not bound yet.
    ///
    /// The returned [`ValueContainer`] shares the stored slot, so reading it
    /// sees the current value and writing through its `lock()` updates the
    /// variable in place. Returning the slot itself (rather than an `Option`)
    /// is what lets a bare name serve as an assignment target: an unset name
    /// materialises as `undef` — the original `strict`-config UndefValue — and
    /// is writable in the same step.
    pub fn get_var(&mut self, name: &str) -> ValueContainer {
        self.var
            .entry(name.to_owned())
            .or_insert_with(ValueContainer::undef)
            .clone()
    }
}
