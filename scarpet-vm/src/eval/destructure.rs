use std::cmp::Ordering;

use scarpet_syntax::ast::{AssignOp, Code, LPatterns, LRest, LValue, Place};

use super::Evalute;
use crate::{
    error::VmError,
    value::{Value, ValueContainer},
    vm::ScarpetVm,
};

impl<'state, 'src> ScarpetVm<'state, 'src> {
    /// Spread `value` across a destructuring list pattern (`[a, b] = …`,
    /// `l(a, b) = …`). The pattern is an [`LPatterns`] validated at lowering, so
    /// each element is an [`LValue`] bound by position through
    /// [`assign_lvalue`](Self::assign_lvalue) — a nested `[[a], b]` simply recurses.
    /// `value` must be a list; the optional rest binder collects the leftover middle
    /// into a fresh list. A compound `+=` reads the pattern as an r-value first
    /// (`[a, b] += v` is `[a, b] = [a, b] + v`); `<>` is rejected. Yields `true`.
    pub(super) fn destructure(
        &mut self,
        LPatterns { before, rest }: LPatterns<'src>,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        // Pick the value to bind. `=` binds the RHS directly; `+=` reads the pattern
        // as an r-value list and adds the RHS into it first (`Value`'s `AddAssign`
        // covers both list-pairwise and scalar-broadcast). `<>` would need the RHS
        // as a *pattern of places*, gone once evaluated to a value, so it is
        // rejected.
        let value = match op {
            AssignOp::Assign => value,
            AssignOp::Add => {
                if rest.is_some() {
                    // A rest pattern has no well-defined r-value to add into.
                    return Err(VmError::NotAssignable);
                }
                let mut sum = self.read_lvalues_as_list(&before)?;
                sum += value.lock()?.clone();
                ValueContainer::new(sum)
            }
            AssignOp::Swap => return Err(VmError::NotAssignable),
        };
        let Value::List(list) = value.lock()?.clone() else {
            return Err(VmError::ExpectedList);
        };
        let mut items: Vec<Value> = list.into_iter().collect();
        match rest {
            // No rest binder: the lengths must match exactly.
            None => {
                match items.len().cmp(&before.len()) {
                    Ordering::Greater => return Err(VmError::TooManyValuesToUnpack),
                    Ordering::Less => return Err(VmError::TooFewValuesToUnpack),
                    Ordering::Equal => {}
                }
                for (lv, item) in before.into_iter().zip(items) {
                    self.assign_lvalue(lv, AssignOp::Assign, ValueContainer::new(item))?;
                }
            }
            // `[a, ...rest, b]`: bind `before` from the front and `after` from the
            // back, and collect the middle into the rest binder as a new list.
            Some(LRest { binder, after }) => {
                let fixed = before.len() + after.len();
                if items.len() < fixed {
                    return Err(VmError::TooFewValuesToUnpack);
                }
                let tail = items.split_off(items.len() - after.len());
                let middle = items.split_off(before.len());
                for (lv, item) in before.into_iter().zip(items) {
                    self.assign_lvalue(lv, AssignOp::Assign, ValueContainer::new(item))?;
                }
                self.assign_lvalue(
                    *binder,
                    AssignOp::Assign,
                    ValueContainer::new(Value::list(middle)),
                )?;
                for (lv, item) in after.into_iter().zip(tail) {
                    self.assign_lvalue(lv, AssignOp::Assign, ValueContainer::new(item))?;
                }
            }
        }
        Ok(ValueContainer::bool(true))
    }

    /// Evaluate a dynamic `var(expr)` name [`Code`] to the variable name it selects:
    /// the value's string form is the name (so `var('x' + i)` and `var(key)` both
    /// work). The single-argument shape is guaranteed by lowering.
    pub(super) fn eval_var_name(&mut self, code: Code<'src>) -> Result<String, VmError> {
        let value = self.push(code)?;
        let name = value.lock()?.to_scarpet_string();
        Ok(name)
    }

    /// Realise a slice of destructure elements as a [`Value::list`] — the r-value of
    /// a `[a, b, …]` / `l(a, b, …)` pattern, used by the compound `[a, b] += …`
    /// path. Each element is read as the expression it denotes.
    fn read_lvalues_as_list(&mut self, lvalues: &[LValue<'src>]) -> Result<Value, VmError> {
        let mut items = Vec::with_capacity(lvalues.len());
        for lv in lvalues {
            items.push(self.read_lvalue(lv)?);
        }
        Ok(Value::list(items))
    }

    /// Read an [`LValue`] as an r-value (its current value), for the `+=` destructure
    /// path: a [`Place`] reads its slot / element, a [`Computed`](LValue::Computed)
    /// evaluates its call, and a nested [`Destructure`](LValue::Destructure) reads
    /// its elements into a list.
    fn read_lvalue(&mut self, lv: &LValue<'src>) -> Result<Value, VmError> {
        match lv {
            LValue::Place(place) => self.read_place(place),
            LValue::Computed(call) => Ok(self.push((**call).clone())?.lock()?.clone()),
            LValue::Destructure(pats) => self.read_lvalues_as_list(&pats.before),
        }
    }

    /// Read a [`Place`] as an r-value: a variable / `var(…)` reads its slot, an
    /// index reads the addressed element.
    fn read_place(&mut self, place: &Place<'src>) -> Result<Value, VmError> {
        match place {
            Place::Var(name) => Ok(self.get_var(name).lock()?.clone()),
            Place::DynVar(code) => {
                let name = self.eval_var_name((**code).clone())?;
                Ok(self.get_var(&name).lock()?.clone())
            }
            Place::Index { base, key } => {
                let base = self.read_place(base)?;
                let key = self.push(key.clone())?.lock()?.clone();
                base.scarpet_get(&key)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::error::VmError;
    use crate::test_util::{eval, eval_err};
    use crate::value::Value;

    /// `[a, b] = [1, 2]` binds each element of the pattern to the matching value.
    #[test]
    fn destructure_list_binds_each_element() {
        assert_eq!(eval("[a, b] = [10, 20]; a + b"), Value::Int(30));
    }

    /// A destructuring assignment yields `true` (the original's result), not the
    /// list — distinct from a single-place assignment, which yields the value.
    #[test]
    fn destructure_yields_true() {
        assert_eq!(eval("[a] = [1]"), Value::Bool(true));
    }

    /// `l(a, b)` is the same list-constructor l-value as `[a, b]`.
    #[test]
    fn destructure_l_constructor() {
        assert_eq!(eval("l(a, b) = [1, 2]; a + b"), Value::Int(3));
    }

    /// A nested list pattern destructures element-wise (`[[a], b]`).
    #[test]
    fn destructure_nested_list() {
        assert_eq!(eval("[[a], b] = [[1], 2]; a + b"), Value::Int(3));
    }

    /// `...rest` collects the leftover elements after the fixed ones into a list.
    #[test]
    fn destructure_rest_collects_the_tail() {
        assert_eq!(
            eval("[a, ...rest] = [1, 2, 3, 4]; rest"),
            Value::list(vec![Value::Int(2), Value::Int(3), Value::Int(4)])
        );
    }

    /// A rest binder between fixed elements binds `before` from the front and
    /// `after` from the back, leaving the middle for the rest.
    #[test]
    fn destructure_rest_between_fixed_elements() {
        assert_eq!(
            eval("[first, ...mid, last] = [1, 2, 3, 4, 5]; mid"),
            Value::list(vec![Value::Int(2), Value::Int(3), Value::Int(4)])
        );
    }

    /// A nested destructure with its own rest binds recursively: in `[a, [...b, c]]`
    /// the inner list splits into a rest `b` and a trailing `c`.
    #[test]
    fn destructure_nested_with_inner_rest() {
        assert_eq!(
            eval("[a, [...b, c]] = [1, [2, 5, 3]]; [a, b, c]"),
            Value::list(vec![
                Value::Int(1),
                Value::list(vec![Value::Int(2), Value::Int(5)]),
                Value::Int(3),
            ])
        );
    }

    /// The rest binder may itself be a nested destructure: `[a, ...[b, c]]` binds
    /// `a` from the front, then unpacks the middle list into `[b, c]`.
    #[test]
    fn destructure_rest_binder_into_nested() {
        assert_eq!(
            eval("[a, ...[b, c]] = [1, 2, 3]; [a, b, c]"),
            Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
    }

    /// Without a rest binder the lengths must match exactly.
    #[test]
    fn destructure_arity_mismatch_is_an_error() {
        assert!(matches!(
            eval_err("[a] = [1, 2]"),
            VmError::TooManyValuesToUnpack
        ));
        assert!(matches!(
            eval_err("[a, b] = [1]"),
            VmError::TooFewValuesToUnpack
        ));
    }

    /// Destructuring a non-list value cannot be unpacked.
    #[test]
    fn destructure_non_list_is_an_error() {
        assert!(matches!(eval_err("[a] = 5"), VmError::ExpectedList));
    }

    /// `[a, b] += [1, 2]` reads the pattern as an r-value and adds element-wise.
    #[test]
    fn compound_destructure_adds_pairwise() {
        assert_eq!(
            eval("a = 10; b = 20; [a, b] += [1, 2]; [a, b]"),
            Value::list(vec![Value::Int(11), Value::Int(22)])
        );
    }

    /// A scalar right-hand side broadcasts to every element (`[a, b] += 5`).
    #[test]
    fn compound_destructure_adds_scalar_to_each() {
        assert_eq!(
            eval("a = 10; b = 20; [a, b] += 5; [a, b]"),
            Value::list(vec![Value::Int(15), Value::Int(25)])
        );
    }

    /// `l(a, b) += …` behaves like the `[a, b]` form.
    #[test]
    fn compound_destructure_through_l_constructor() {
        assert_eq!(
            eval("a = 1; b = 2; l(a, b) += [10, 20]; [a, b]"),
            Value::list(vec![Value::Int(11), Value::Int(22)])
        );
    }

    /// `<>` over a list pattern needs the right side as places, which are lost
    /// once it is an ordinary value, so it is rejected.
    #[test]
    fn swap_destructure_is_rejected() {
        assert!(matches!(
            eval_err("a = 1; b = 2; c = 3; d = 4; [a, b] <> [c, d]"),
            VmError::NotAssignable
        ));
    }

    /// A compound assignment into a rest pattern has no r-value form.
    #[test]
    fn compound_destructure_with_rest_is_rejected() {
        assert!(matches!(
            eval_err("[a, ...rest] += [1, 2]"),
            VmError::NotAssignable
        ));
    }
}
