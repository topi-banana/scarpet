use scarpet_syntax::ast::{AssignOp, LValue, Place, Primary};

use super::Evaluate;
use crate::{
    error::VmError,
    value::{Value, ValueContainer},
    vm::ScarpetVm,
};

impl<'state, 'src> ScarpetVm<'state, 'src> {
    /// Carry out `target <op> value` — the body of an [`Assign::Set`]. The target
    /// is a structurally validated [`LValue`], so this dispatches on its shape with
    /// no re-validation: a single [`Place`] that `op` writes through, a
    /// [`Destructure`](LValue::Destructure) that spreads `value` across several
    /// sub-targets, or a [`Computed`](LValue::Computed) call whose value is the
    /// place (Scarpet's runtime l-value).
    ///
    /// [`Assign::Set`]: scarpet_syntax::ast::Assign::Set
    pub(super) fn assign_lvalue(
        &mut self,
        target: LValue<'src>,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        match target {
            LValue::Place(place) => self.assign_place(place, op, value),
            LValue::Destructure(pats) => self.destructure(pats, op, value),
            // `if(c, a, b) = …`: the call evaluates to one of its bound arguments'
            // shared slots, which `op` then writes through.
            LValue::Computed(call) => {
                let slot = self.push(*call)?;
                self.assign_to_slot(slot, op, value)
            }
        }
    }

    /// Carry out `<place> <op> value` for a single [`Place`]: a `base:key` index
    /// writes into a container element in place ([`assign_index`](Self::assign_index)),
    /// while a variable / `var(…)` resolves to a mutable slot that `op` writes
    /// through (the slot itself being the assignment's value, so `b = a = 1` reads
    /// `a`'s slot for `b`).
    fn assign_place(
        &mut self,
        place: Place<'src>,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        match place {
            Place::Index { base, key } => self.assign_index(*base, key, op, value),
            other => {
                let slot = self.resolve_root(other)?;
                self.assign_to_slot(slot, op, value)
            }
        }
    }

    /// Apply `op` to a resolved variable slot: `=` overwrites it, `+=` adds in
    /// place, `<>` swaps it with the other slot. Yields the slot, which is the
    /// assignment's value.
    fn assign_to_slot(
        &mut self,
        slot: ValueContainer,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        match op {
            AssignOp::Assign => *slot.lock()? = value.lock()?.clone(),
            AssignOp::Add => *slot.lock()? += value.lock()?.clone(),
            AssignOp::Swap => std::mem::swap(&mut *slot.lock()?, &mut *value.lock()?),
        }
        Ok(slot)
    }

    /// Resolve the root of a place chain to its shared [`ValueContainer`] slot: the
    /// local slot for a bare variable, or the dynamically named slot for `var(expr)`
    /// (whose argument's value is the variable name). An [`Index`](Place::Index)
    /// never reaches here — [`assign_index`](Self::assign_index) flattens the chain
    /// to its non-index root first.
    fn resolve_root(&mut self, root: Place<'src>) -> Result<ValueContainer, VmError> {
        match root {
            Place::Var(name) => Ok(self.get_var(name)),
            Place::DynVar(code) => {
                let name = self.eval_var_name(*code)?;
                Ok(self.get_var(&name))
            }
            Place::Index { .. } => {
                unreachable!("an index chain flattens to a non-index root")
            }
        }
    }

    /// Carry out `base:key <op> value` — assignment into a (possibly nested)
    /// container element (the original `LContainerValue` → `container.put`). The
    /// `root:k0:…:kn` chain is flattened to its root place (a variable / `var(...)`)
    /// plus the key path; the root's slot is locked once and the path walked *by
    /// reference* through [`Value::element_mut`], so even a deep write (`x:0:1 = …`)
    /// lands in the original. The final element is replaced through
    /// [`Value::scarpet_put`] (a compound `+=` reads it first). Yields the stored
    /// value, as the original does. Lowering guarantees only `:` reaches here.
    fn assign_index(
        &mut self,
        base: Place<'src>,
        key: Primary<'src>,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        // Flatten `root:k0:…:kn` into the root place and the key path `[k0, …, kn]`.
        // Keys are gathered innermost-first, then reversed.
        let mut keys = vec![self.push(key)?.lock()?.clone()];
        let mut current = base;
        let root = loop {
            match current {
                Place::Index { base, key } => {
                    keys.push(self.push(key)?.lock()?.clone());
                    current = *base;
                }
                other => break other,
            }
        };
        keys.reverse();
        let slot = self.resolve_root(root)?;
        let new = value.lock()?.clone();
        let mut guard = slot.lock()?;
        // Walk to the innermost container by reference, then write the last key.
        let mut container: &mut Value = &mut guard;
        let (last, mids) = keys
            .split_last()
            .expect("keys holds at least the final key");
        for k in mids {
            container = container.element_mut(k)?;
        }
        let stored = match op {
            AssignOp::Assign => new,
            // `c:k += v` is `c:k = c:k + v`: read the current element, then add.
            AssignOp::Add => {
                let mut element = container.scarpet_get(last)?;
                element += new;
                element
            }
            // `c:k <> v` swaps the element with the l-value `v`: the element takes
            // `v`'s value (through the common `scarpet_put` below) while `v` takes
            // the element's old value in place, through its shared slot.
            AssignOp::Swap => {
                let old = container.scarpet_get(last)?;
                std::mem::replace(&mut *value.lock()?, old)
            }
        };
        container.scarpet_put(last, stored.clone())?;
        Ok(ValueContainer::new(stored))
    }
}

#[cfg(test)]
mod tests {
    use crate::error::VmError;
    use crate::test_util::{eval, eval_err};
    use crate::value::Value;

    /// `var('test') = 7` binds the variable named by the string. It is a single
    /// place, so it yields the assigned value, and the plain name reads it back.
    #[test]
    fn var_assigns_a_dynamically_named_variable() {
        assert_eq!(eval("var('test') = 7"), Value::Int(7));
        assert_eq!(eval("var('test') = 7; test"), Value::Int(7));
    }

    /// `var(name)` takes the variable name from the *value* of its argument, so a
    /// bare variable argument names the variable its value spells.
    #[test]
    fn var_name_comes_from_the_argument_value() {
        assert_eq!(eval("k = 'foo'; var(k) = 9; foo"), Value::Int(9));
    }

    /// The `var` argument is an arbitrary expression: `var('x' + i)` selects the
    /// variable whose name is the computed string.
    #[test]
    fn var_name_can_be_computed() {
        assert_eq!(eval("i = 1; var('x' + i) = 5; x1"), Value::Int(5));
    }

    /// `x:0 = 9` replaces a list element in place, visible through the variable.
    #[test]
    fn element_assign_replaces_a_list_element() {
        assert_eq!(
            eval("x = [1, 2, 3]; x:0 = 9; x"),
            Value::list(vec![Value::Int(9), Value::Int(2), Value::Int(3)])
        );
    }

    /// An element assignment yields the stored value (not `true`, unlike a
    /// destructure).
    #[test]
    fn element_assign_yields_the_stored_value() {
        assert_eq!(eval("x = [1, 2, 3]; x:0 = 9"), Value::Int(9));
    }

    /// The index wraps modulo the length, like `:` reading does.
    #[test]
    fn element_assign_wraps_the_index() {
        // `3 mod 3 == 0` writes the first element.
        assert_eq!(
            eval("x = [1, 2, 3]; x:3 = 9; x"),
            Value::list(vec![Value::Int(9), Value::Int(2), Value::Int(3)])
        );
        // A parenthesised negative index reaches from the end.
        assert_eq!(
            eval("x = [1, 2, 3]; x:(-1) = 9; x"),
            Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(9)])
        );
    }

    /// On a map, an existing key is updated and a new key is inserted.
    #[test]
    fn element_assign_updates_and_inserts_map_keys() {
        assert_eq!(eval("m = {'a' -> 1}; m:'a' = 9; m:'a'"), Value::Int(9));
        assert_eq!(eval("m = {'a' -> 1}; m:'b' = 2; m:'b'"), Value::Int(2));
    }

    /// A compound `+=` reads the current element, then writes the sum back.
    #[test]
    fn element_compound_add_updates_in_place() {
        assert_eq!(eval("x = [1, 2, 3]; x:0 += 10; x:0"), Value::Int(11));
    }

    /// The base may itself be a dynamic `var(...)` target.
    #[test]
    fn element_assign_through_a_dynamic_var_base() {
        assert_eq!(
            eval("m = {'a' -> 1}; var('m'):'a' = 9; m:'a'"),
            Value::Int(9)
        );
    }

    /// Writing into a non-container (a number) is an error.
    #[test]
    fn element_assign_into_non_container_is_an_error() {
        assert!(matches!(eval_err("x = 5; x:0 = 1"), VmError::NotAContainer));
    }

    /// Writing into a lazy, immutable list (a `range`) is an error.
    #[test]
    fn element_assign_into_immutable_list_is_an_error() {
        assert!(matches!(
            eval_err("r = range(3); r:0 = 9"),
            VmError::ImmutableList
        ));
    }

    /// An empty list has no slot to write — the index cannot wrap.
    #[test]
    fn element_assign_into_empty_list_is_an_error() {
        assert!(matches!(
            eval_err("x = []; x:0 = 1"),
            VmError::IndexOutOfRange
        ));
    }

    /// A nested `x:0:1 = …` walks into the inner list by reference, so the write
    /// lands in the original.
    #[test]
    fn nested_element_assign_writes_through() {
        assert_eq!(
            eval("x = [[1, 2], [3, 4]]; x:0:1 = 9; x"),
            Value::list(vec![
                Value::list(vec![Value::Int(1), Value::Int(9)]),
                Value::list(vec![Value::Int(3), Value::Int(4)]),
            ])
        );
        assert_eq!(
            eval("x = [[1, 2], [3, 4]]; x:0:1 = 9; x:0:1"),
            Value::Int(9)
        );
    }

    /// The path may mix list indices and map keys (`x:0:'a'`).
    #[test]
    fn nested_element_assign_into_map_in_list() {
        assert_eq!(
            eval("x = [{'a' -> 1}]; x:0:'a' = 9; x:0:'a'"),
            Value::Int(9)
        );
    }

    /// Nesting walks to any depth (`x:0:0:0`).
    #[test]
    fn nested_element_assign_is_arbitrarily_deep() {
        assert_eq!(eval("x = [[[1]]]; x:0:0:0 = 9; x:0:0:0"), Value::Int(9));
    }

    /// A compound `+=` works at depth too.
    #[test]
    fn nested_element_compound_add() {
        assert_eq!(eval("x = [[1, 2]]; x:0:0 += 10; x:0:0"), Value::Int(11));
    }

    /// A missing map key cannot be walked through mid-path.
    #[test]
    fn nested_element_assign_through_missing_key_is_an_error() {
        assert!(matches!(
            eval_err("m = {}; m:'a':'b' = 1"),
            VmError::IndexOutOfRange
        ));
    }

    /// Reaching an immutable list at the end of the path still errors.
    #[test]
    fn nested_element_assign_into_immutable_list_is_an_error() {
        assert!(matches!(
            eval_err("x = [range(3)]; x:0:0 = 9"),
            VmError::ImmutableList
        ));
    }

    /// `a <> b` exchanges two variables' values through their shared slots.
    #[test]
    fn swap_exchanges_two_variables() {
        assert_eq!(
            eval("a = 1; b = 2; a <> b; [a, b]"),
            Value::list(vec![Value::Int(2), Value::Int(1)])
        );
    }

    /// `x:0 <> y` swaps a container element with a variable: the element takes the
    /// variable's value and the variable takes the element's old value.
    #[test]
    fn swap_exchanges_a_container_element_with_a_variable() {
        assert_eq!(
            eval("x = [1, 2, 3]; y = 9; x:0 <> y; [x, y]"),
            Value::list(vec![
                Value::list(vec![Value::Int(9), Value::Int(2), Value::Int(3)]),
                Value::Int(1),
            ])
        );
    }

    /// `if(cond, a, b) = v` assigns through whichever bound argument the call
    /// selects — Scarpet's dynamic l-value, lowered to a `Computed` target. A true
    /// condition binds `a` (leaving `b` untouched), a false one binds `b`.
    #[test]
    fn computed_call_target_assigns_through_if() {
        assert_eq!(
            eval("if(1, a, b) = 5; [a, b]"),
            Value::list(vec![Value::Int(5), Value::Undef])
        );
        assert_eq!(
            eval("if(0, a, b) = 5; [a, b]"),
            Value::list(vec![Value::Undef, Value::Int(5)])
        );
    }

    /// A computed call target also drives a compound `+=`: `if(c, a, b) += v`
    /// updates whichever variable the condition selects — the idiom from
    /// `world_map.sc` (`if(…, loot_rooms, shulker_rooms) += …`).
    #[test]
    fn computed_call_target_compound_add() {
        assert_eq!(
            eval("a = 10; b = 20; if(1, a, b) += 5; [a, b]"),
            Value::list(vec![Value::Int(15), Value::Int(20)])
        );
    }

    /// A `var(...)` argument may itself be a container read (`var(m:0)`), which
    /// the r-value evaluator now handles.
    #[test]
    fn var_name_from_a_container_read() {
        assert_eq!(eval("m = ['x', 'y']; var(m:0) = 7; x"), Value::Int(7));
    }
}
