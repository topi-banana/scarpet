use std::collections::BTreeMap;

use crate::value::ValueContainer;

pub struct GlobalState {}

impl GlobalState {
    pub fn create_new_vm<'me>(&'me mut self) -> ScarpetVm<'me> {
        ScarpetVm::new(self)
    }
}

pub struct ScarpetVm<'state> {
    // Prototype: kept for the evaluator's future use, not read yet.
    #[allow(dead_code)]
    global: &'state mut GlobalState,
    pub(crate) var: BTreeMap<String, ValueContainer>,
}

impl<'state> ScarpetVm<'state> {
    pub fn new(global: &'state mut GlobalState) -> Self {
        Self {
            global,
            var: BTreeMap::new(),
        }
    }
}
