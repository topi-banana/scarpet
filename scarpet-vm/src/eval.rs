// Prototype evaluator: most match arms are still `todo!()`, so the operands
// they bind (`lhs`, `rhs`, ...) are intentionally unused until those arms get
// implemented. Drop this allow once the evaluator is filled in.
#![allow(unused_variables)]

use std::{cmp::Ordering, rc::Rc};

use scarpet_syntax::ast::{
    Additive, Args, Assign, AssignOp, Code, Compare, Equality, Expr, Get, GetOp, Land, Lor, Mult,
    Power, Primary, Unary,
};

use crate::{
    error::VmError,
    function::DefFunction,
    value::{Value, ValueContainer},
    vm::ScarpetVm,
};

pub trait Evalute<T> {
    fn push(&mut self, st: T) -> Result<ValueContainer, VmError>;
}

impl<'src, 'state> Evalute<Code<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, Code(mut sts): Code<'src>) -> Result<ValueContainer, VmError> {
        let last = sts.pop();
        for st in sts {
            self.push(st)?;
        }
        if let Some(st) = last {
            self.push(st)
        } else {
            Ok(ValueContainer::null())
        }
    }
}

impl<'src, 'state> Evalute<Expr<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Expr<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Expr::Def { name, params, body } => {
                let func = DefFunction::new(&params, body).ok_or(VmError::UnsupportedParameter)?;
                self.define(name, Rc::new(func));
                Ok(ValueContainer::string(name.to_owned()))
            }
            Expr::Assign(ost) => self.push(ost),
            // A bare `->` outside a map (a lambda) is not modelled yet.
            Expr::Arrow { .. } => todo!(),
        }
    }
}

impl<'src, 'state> Evalute<Assign<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Assign<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Assign::Set { target, op, value } => {
                // The right-hand side is evaluated once, in the current scope,
                // before binding; `assign` then routes it to the target — a single
                // place for `op` to update, or a destructure to spread across.
                let value = self.push(*value)?;
                self.assign(target, op, value)
            }
            Assign::Lor(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Lor<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Lor<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Lor::Or { lhs, rhs } => Ok(ValueContainer::bool(
                self.push(*lhs)?.lock()?.is_true() || self.push(rhs)?.lock()?.is_true(),
            )),
            Lor::Land(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Land<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Land<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Land::And { lhs, rhs } => Ok(ValueContainer::bool(
                self.push(*lhs)?.lock()?.is_true() && self.push(rhs)?.lock()?.is_true(),
            )),
            Land::Equality(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Equality<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Equality<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Equality::Eq { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(lhs.scarpet_eq(&rhs)?))
            }
            Equality::Ne { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(!lhs.scarpet_eq(&rhs)?))
            }
            Equality::Compare(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Compare<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Compare<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Compare::Lt { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(
                    lhs.scarpet_compare(&rhs)? == Ordering::Less,
                ))
            }
            Compare::Le { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(
                    lhs.scarpet_compare(&rhs)? != Ordering::Greater,
                ))
            }
            Compare::Gt { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(
                    lhs.scarpet_compare(&rhs)? == Ordering::Greater,
                ))
            }
            Compare::Ge { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(
                    lhs.scarpet_compare(&rhs)? != Ordering::Less,
                ))
            }
            Compare::Additive(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Additive<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Additive<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Additive::Add { lhs, rhs } => self.push(*lhs)? + self.push(rhs)?,
            Additive::Sub { lhs, rhs } => self.push(*lhs)? - self.push(rhs)?,
            Additive::Mult(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Mult<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Mult<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Mult::Mul { lhs, rhs } => self.push(*lhs)? * self.push(rhs)?,
            Mult::Div { lhs, rhs } => self.push(*lhs)? / self.push(rhs)?,
            Mult::Rem { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                lhs.scarpet_rem(&rhs)
            }
            Mult::Power(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Power<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Power<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Power::Pow { base, exp } => {
                let (base, exp) = (self.push(base)?, self.push(*exp)?);
                base.scarpet_pow(&exp)
            }
            Power::Unary(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Unary<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Unary<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Unary::Neg(v) => self.push(*v)?.scarpet_neg(),
            Unary::Pos(v) => self.push(*v)?.scarpet_pos(),
            Unary::Not(v) => self.push(*v)?.scarpet_not(),
            Unary::Unpack(v) => Ok(ValueContainer::Expand(match self.push(*v)? {
                ValueContainer::Single(v) => v,
                ValueContainer::Expand(v) => v,
            })),
            Unary::Get(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Get<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Get<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Get::Index { base, op, key } => {
                let base = self.push(*base)?;
                let key = self.push(key)?;
                match op {
                    GetOp::Get => base.scarpet_get(&key),
                    GetOp::Match => base.scarpet_match(&key),
                }
            }
            Get::Primary(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Primary<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Primary<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Primary::Number(v) => Ok(ValueContainer::new(Value::from_number_literal(v))),
            Primary::Str(v) => Ok(ValueContainer::new(Value::from_string_literal(v))),
            // A bare variable reference yields its binding, materialising an
            // unset name as `undef` (the original `strict`-config UndefValue).
            Primary::Ident(name) => Ok(self.get_var(name)),
            // `name(args)`: look the function up in the global table (builtin or
            // user-defined) and hand it the still-unevaluated arguments — each
            // callable evaluates its own args, so a special form can choose not to.
            Primary::Call { name, args } => self
                .function(name)
                .ok_or(VmError::UnknownFunction)?
                .call(self, args),
            // `[a, b, …]`: evaluate each comma-separated element to a value.
            Primary::List(Args(codes)) => {
                let mut items = Vec::with_capacity(codes.len());
                for code in codes {
                    items.push(self.push(code)?.lock()?.clone());
                }
                Ok(ValueContainer::new(Value::list(items)))
            }
            // `{k -> v, …}`: each entry is evaluated in map context, where a
            // top-level `->` is a key/value pair (the original desugars `{…}` to
            // `m(…)`). Duplicate keys are last-wins.
            Primary::Map(Args(codes)) => {
                let mut entries: Vec<(Value, Value)> = Vec::new();
                for code in codes {
                    let (key, value) = self.eval_map_entry(code)?;
                    match entries.iter_mut().find(|(k, _)| k.scarpet_eq(&key)) {
                        Some(slot) => slot.1 = value,
                        None => entries.push((key, value)),
                    }
                }
                Ok(ValueContainer::new(Value::Map(entries)))
            }
            // `( … )`: evaluate the body and yield its last value.
            Primary::Paren(Args(codes)) => {
                let mut result = ValueContainer::null();
                for code in codes {
                    result = self.push(code)?;
                }
                Ok(result)
            }
        }
    }
}

impl<'state, 'src> ScarpetVm<'state, 'src> {
    /// Evaluate one entry of a map literal (`{…}` / `m(…)`) into a key/value
    /// pair. In map context a top-level `->` is not a lambda but a pair (the
    /// original evaluates these args in `MAPDEF` context). Otherwise the entry
    /// is a value handled like `MapValue.put`: a 2-element list is a pair, any
    /// other list is an error, and a non-list becomes a key with a null value.
    fn eval_map_entry(&mut self, Code(mut exprs): Code<'src>) -> Result<(Value, Value), VmError> {
        if exprs.len() == 1 && matches!(exprs.first(), Some(Expr::Arrow { .. })) {
            let Some(Expr::Arrow { lhs, body }) = exprs.pop() else {
                unreachable!()
            };
            let key = self.push(lhs)?.lock()?.clone();
            let value = self.push(*body)?.lock()?.clone();
            return Ok((key, value));
        }
        let value = self.push(Code(exprs))?.lock()?.clone();
        match value {
            Value::List(items) if items.len() == 2 => {
                let mut it = items.into_iter();
                Ok((it.next().unwrap(), it.next().unwrap()))
            }
            Value::List(_) => Err(VmError::MapEntryNotPair),
            other => Ok((other, Value::Null)),
        }
    }

    /// Carry out `target <op> value` — the body of an [`Assign::Set`]. The target
    /// is a general expression now, so peel the precedence-ladder passthrough down
    /// to the `get` level (`:`/`~` and the primaries): anything with an operator
    /// above that level is not an l-value. [`assign_get`](Self::assign_get) then
    /// dispatches on the shape.
    fn assign(
        &mut self,
        target: Lor<'src>,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        let get = peel_get(target).ok_or(VmError::NotAssignable)?;
        self.assign_get(get, op, value)
    }

    /// Route a target peeled to the `get` level to its handler: a *destructuring*
    /// list pattern (`[a, b]`, `l(a, b)`) spreads `value` across several
    /// sub-targets and is `=`-only; a `base:key` index writes into a container
    /// element in place; everything else resolves to a single mutable place that
    /// `op` updates (the slot itself being the assignment's value, so `b = a = 1`
    /// reads `a`'s slot for `b`).
    fn assign_get(
        &mut self,
        get: Get<'src>,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        match get {
            // `base:key = …` writes into a container element in place, rather than
            // rebinding a variable slot.
            Get::Index {
                base,
                op: get_op,
                key,
            } => self.assign_index(*base, get_op, key, op, value),
            // `[a, b] = …` and `l(a, b) = …` are the same list-constructor l-value.
            Get::Primary(Primary::List(args) | Primary::Call { name: "l", args }) => {
                self.destructure(args, op, value)
            }
            // Everything else resolves to a single mutable place, which `op` writes
            // through; the slot itself is the assignment's value.
            Get::Primary(primary) => {
                let place = self.resolve_place(primary)?;
                match op {
                    AssignOp::Assign => *place.lock()? = value.lock()?.clone(),
                    AssignOp::Add => *place.lock()? += value.lock()?.clone(),
                    AssignOp::Swap => std::mem::swap(&mut *place.lock()?, &mut *value.lock()?),
                }
                Ok(place)
            }
        }
    }

    /// Resolve a single-place primary target to its shared [`ValueContainer`]
    /// slot: the local slot for a bare variable, or the dynamically named slot for
    /// `var(expr)` (whose argument's value is the variable name). Any other primary
    /// — a literal, a map, an l-value-returning call (`if(…)`) — is not a place.
    /// (A list / `l(…)` is multi-value, handled in [`assign_get`](Self::assign_get)
    /// before it reaches here.)
    fn resolve_place(&mut self, primary: Primary<'src>) -> Result<ValueContainer, VmError> {
        match primary {
            Primary::Ident(name) => Ok(self.get_var(name)),
            // `var(expr)` names a variable dynamically — `var('x' + i) = …`.
            Primary::Call { name: "var", args } => {
                let name = self.eval_var_name(args)?;
                Ok(self.get_var(&name))
            }
            _ => Err(VmError::NotAssignable),
        }
    }

    /// Carry out `base:key <op> value` — assignment into a (possibly nested)
    /// container element (the original `LContainerValue` → `container.put`). The
    /// `root:k0:…:kn` chain is flattened to its root l-value (a bare variable /
    /// `var(...)` primary) plus the key path; the root's place is locked once and
    /// the path walked *by reference* through [`Value::element_mut`], so even a
    /// deep write (`x:0:1 = …`) lands in the original. The final element is
    /// replaced through [`Value::scarpet_put`] (a compound `+=` reads it first).
    /// Only `:` (`GetOp::Get`) addresses a writable element; `~` is a match
    /// operator, not a target. Yields the stored value, as the original does.
    fn assign_index(
        &mut self,
        base: Get<'src>,
        get_op: GetOp,
        key: Primary<'src>,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        // `~` (Match) is a search operator, not a writable container address.
        if get_op != GetOp::Get {
            return Err(VmError::NotAssignable);
        }
        // Flatten `root:k0:…:kn` into the root primary and the key path
        // `[k0, …, kn]`. Keys are gathered innermost-first, then reversed.
        let mut keys = vec![self.push(key)?.lock()?.clone()];
        let mut current = base;
        let root = loop {
            match current {
                Get::Index {
                    base,
                    op: GetOp::Get,
                    key,
                } => {
                    keys.push(self.push(key)?.lock()?.clone());
                    current = *base;
                }
                // A `~` anywhere along the path is not a writable address either.
                Get::Index { .. } => return Err(VmError::NotAssignable),
                Get::Primary(primary) => break primary,
            }
        };
        keys.reverse();
        let place = self.resolve_place(root)?;
        let new = value.lock()?.clone();
        let mut guard = place.lock()?;
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

    /// Spread `value` across a destructuring list pattern (`[a, b] = …`,
    /// `l(a, b) = …`). The pattern's elements are plain expressions now: each must
    /// peel to a `get`-level l-value, and a leading `...` (an unpack unary) marks
    /// the one rest binder. `value` must be a list; its elements bind by position,
    /// the rest binder collecting the leftover middle into a fresh list. Sub-
    /// patterns recurse through [`assign_get`](Self::assign_get), so a nested
    /// `[[a], b]` works. A compound `+=` reads the pattern as an r-value first
    /// (`[a, b] += v` is `[a, b] = [a, b] + v`); `<>` is rejected. Yields `true`.
    fn destructure(
        &mut self,
        Args(elems): Args<'src>,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        // Classify each element into the fixed front (`before`), the single
        // optional rest binder, and the fixed tail (`after`). A second `...` at
        // this level is not a representable l-value.
        let mut before: Vec<Get<'src>> = Vec::new();
        let mut rest: Option<Get<'src>> = None;
        let mut after: Vec<Get<'src>> = Vec::new();
        for elem in elems {
            let (is_rest, get) = peel_elem(elem)?;
            if is_rest {
                if rest.is_some() {
                    return Err(VmError::NotAssignable);
                }
                rest = Some(get);
            } else if rest.is_none() {
                before.push(get);
            } else {
                after.push(get);
            }
        }
        // Pick the value to bind. `=` binds the RHS directly; `+=` reads the
        // pattern as an r-value list and adds the RHS into it first (the original
        // delegates `+=` to `+`, then destructures — `Value`'s `AddAssign` covers
        // both list-pairwise and scalar-broadcast). `<>` would need the RHS as a
        // *pattern of places*, which is gone once it has been evaluated to an
        // ordinary value, so it is rejected.
        let value = match op {
            AssignOp::Assign => value,
            AssignOp::Add => {
                if rest.is_some() {
                    // A rest pattern has no well-defined r-value to add into.
                    return Err(VmError::NotAssignable);
                }
                let mut sum = self.eval_gets_as_list(&before)?;
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
                for (get, item) in before.into_iter().zip(items) {
                    self.assign_get(get, AssignOp::Assign, ValueContainer::new(item))?;
                }
            }
            // `[a, ...rest, b]`: bind `before` from the front and `after` from the
            // back, and collect the middle into the rest binder as a new list.
            Some(binder) => {
                let fixed = before.len() + after.len();
                if items.len() < fixed {
                    return Err(VmError::TooFewValuesToUnpack);
                }
                let tail = items.split_off(items.len() - after.len());
                let middle = items.split_off(before.len());
                for (get, item) in before.into_iter().zip(items) {
                    self.assign_get(get, AssignOp::Assign, ValueContainer::new(item))?;
                }
                self.assign_get(
                    binder,
                    AssignOp::Assign,
                    ValueContainer::new(Value::list(middle)),
                )?;
                for (get, item) in after.into_iter().zip(tail) {
                    self.assign_get(get, AssignOp::Assign, ValueContainer::new(item))?;
                }
            }
        }
        Ok(ValueContainer::bool(true))
    }

    /// Evaluate the single argument of a dynamic `var(expr)` target to the name
    /// of the variable it selects: `var` takes one argument, and its value's
    /// string form is the name (so `var('x' + i)` and `var(key)` both work).
    fn eval_var_name(&mut self, Args(mut elems): Args<'src>) -> Result<String, VmError> {
        if elems.len() != 1 {
            return Err(VmError::WrongArgCount);
        }
        let arg = elems.pop().expect("checked len == 1");
        let value = self.push(arg)?;
        let name = value.lock()?.to_scarpet_string();
        Ok(name)
    }

    /// Evaluate a slice of `get`-level pattern elements into a realised
    /// [`Value::list`] — the r-value of a `[a, b, …]` / `l(a, b, …)` pattern, used
    /// by the compound `[a, b] += …` path. Each element evaluates as the ordinary
    /// expression it is.
    fn eval_gets_as_list(&mut self, gets: &[Get<'src>]) -> Result<Value, VmError> {
        let mut items = Vec::with_capacity(gets.len());
        for get in gets {
            items.push(self.push(get.clone())?.lock()?.clone());
        }
        Ok(Value::list(items))
    }
}

/// Peel the precedence-ladder passthrough of an assignment-target expression down
/// to the `unary` level. Returns `None` if any binary operator is applied above
/// it, since such an expression is not a bare l-value.
fn peel_unary<'src>(lor: Lor<'src>) -> Option<Unary<'src>> {
    let Lor::Land(Land::Equality(Equality::Compare(Compare::Additive(Additive::Mult(
        Mult::Power(Power::Unary(unary)),
    ))))) = lor
    else {
        return None;
    };
    Some(unary)
}

/// Peel further to the `get` level (`:`/`~` and the primaries). Returns `None` if
/// a prefix unary (`-`, `+`, `!`, `...`) is applied — a bare target carries none.
fn peel_get<'src>(lor: Lor<'src>) -> Option<Get<'src>> {
    match peel_unary(lor)? {
        Unary::Get(get) => Some(get),
        _ => None,
    }
}

/// Classify a destructuring-list element. A list element is a `Code` (a statement
/// sequence); a valid l-value element is a single expression peeled to the `get`
/// level. A leading `...` (an unpack unary) marks the rest binder — the returned
/// flag is `true` for it, and its operand is peeled to the binder's `get`.
fn peel_elem<'src>(Code(mut exprs): Code<'src>) -> Result<(bool, Get<'src>), VmError> {
    if exprs.len() != 1 {
        return Err(VmError::NotAssignable);
    }
    let Expr::Assign(Assign::Lor(lor)) = exprs.pop().expect("checked len == 1") else {
        return Err(VmError::NotAssignable);
    };
    match peel_unary(lor).ok_or(VmError::NotAssignable)? {
        // `...x` — the rest binder; peel its operand to the bound `get`.
        Unary::Unpack(inner) => match *inner {
            Unary::Get(get) => Ok((true, get)),
            _ => Err(VmError::NotAssignable),
        },
        Unary::Get(get) => Ok((false, get)),
        _ => Err(VmError::NotAssignable),
    }
}

#[cfg(test)]
mod tests {
    use scarpet_syntax::parser::parse_source;

    use super::*;
    use crate::vm::GlobalState;

    /// Parse, lower, and evaluate `src` in a fresh VM, returning its value.
    fn eval(src: &str) -> Value {
        let cst = parse_source(src).expect("parse");
        let code = Code::try_from(&cst).expect("lower");
        let mut global = GlobalState::new();
        let mut vm = global.create_new_vm();
        vm.push(code).expect("eval").lock().expect("lock").clone()
    }

    /// Like [`eval`], but expects evaluation to fail and returns the `VmError`.
    fn eval_err(src: &str) -> VmError {
        let cst = parse_source(src).expect("parse");
        let code = Code::try_from(&cst).expect("lower");
        let mut global = GlobalState::new();
        let mut vm = global.create_new_vm();
        vm.push(code).expect_err("expected an evaluation error")
    }

    #[test]
    fn string_literal_strips_quotes() {
        assert_eq!(eval("'hello'"), Value::String("hello".to_owned()));
    }

    #[test]
    fn string_literal_expands_escapes() {
        assert_eq!(eval(r"'a\nb'"), Value::String("a\nb".to_owned()));
        assert_eq!(eval(r"'it\'s'"), Value::String("it's".to_owned()));
    }

    #[test]
    fn ident_reads_an_assigned_variable() {
        assert_eq!(eval("x = 42; x"), Value::Int(42));
    }

    #[test]
    fn ident_unset_is_undef() {
        assert_eq!(eval("missing"), Value::Undef);
    }

    #[test]
    fn list_literal_collects_its_elements() {
        assert_eq!(
            eval("[1, 2, 3]"),
            Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
    }

    #[test]
    fn paren_yields_its_inner_value() {
        assert_eq!(eval("(1 + 2) * 3"), Value::Int(9));
    }

    #[test]
    fn equality_compares_values() {
        assert_eq!(eval("1 == 1"), Value::Bool(true));
        assert_eq!(eval("1 != 2"), Value::Bool(true));
    }

    /// `1 == 1.0` is true end-to-end (the literal `1.0` lowers to a `Double`).
    #[test]
    fn equality_treats_int_and_double_as_equal() {
        assert_eq!(eval("1 == 1.0"), Value::Bool(true));
    }

    #[test]
    fn relational_operators_compare_numbers() {
        assert_eq!(eval("1 < 2"), Value::Bool(true));
        assert_eq!(eval("2 <= 2"), Value::Bool(true));
        assert_eq!(eval("3 > 2"), Value::Bool(true));
        assert_eq!(eval("2 >= 3"), Value::Bool(false));
    }

    #[test]
    fn relational_operators_compare_strings() {
        assert_eq!(eval("'a' < 'b'"), Value::Bool(true));
        assert_eq!(eval("'b' < 'a'"), Value::Bool(false));
    }

    #[test]
    fn equality_and_order_on_lists() {
        assert_eq!(eval("[1, 2] == [1, 2]"), Value::Bool(true));
        assert_eq!(eval("[1] == [1.0]"), Value::Bool(true));
        // Length-first ordering: the shorter list is smaller.
        assert_eq!(eval("[2] < [1, 1]"), Value::Bool(true));
    }

    /// Additive binds tighter than equality, so this lowers as `(1 + 1) == 2`.
    #[test]
    fn additive_binds_tighter_than_equality() {
        assert_eq!(eval("1 + 1 == 2"), Value::Bool(true));
    }

    /// Equality is left-associative: `1 == 2 == 0` is `(1 == 2) == 0`, i.e.
    /// `false == 0`, and a `false` bool equals the number `0`, so it is true.
    #[test]
    fn equality_is_left_associative() {
        assert_eq!(eval("1 == 2 == 0"), Value::Bool(true));
    }

    /// `null` currently lowers to an unset variable (`undef`), but undef and
    /// null share comparison semantics, so these still hold.
    #[test]
    fn comparisons_on_null() {
        assert_eq!(eval("null == null"), Value::Bool(true));
        assert_eq!(eval("null < 1"), Value::Bool(true));
    }

    #[test]
    fn modulo_floors_with_divisor_sign() {
        assert_eq!(eval("5 % 3"), Value::Int(2));
        // `-5` is unary-negated first (binds tighter than `%`), then floorMod's
        // sign follows the divisor `3`, so the result is +1.
        assert_eq!(eval("-5 % 3"), Value::Int(1));
    }

    #[test]
    fn power_is_right_associative_and_double() {
        assert_eq!(eval("2 ^ 10"), Value::Double(1024.0));
        // Right-associative: `2 ^ (3 ^ 2)` = `2 ^ 9` = 512.
        assert_eq!(eval("2 ^ 3 ^ 2"), Value::Double(512.0));
    }

    #[test]
    fn unary_minus_negates() {
        assert_eq!(eval("-5"), Value::Int(-5));
        assert_eq!(eval("-3.5"), Value::Double(-3.5));
    }

    #[test]
    fn unary_plus_coerces_to_number() {
        assert_eq!(eval("+5"), Value::Int(5));
    }

    #[test]
    fn unary_not_negates_truthiness() {
        assert_eq!(eval("!0"), Value::Bool(true));
        assert_eq!(eval("!1"), Value::Bool(false));
        assert_eq!(eval("!(1 == 2)"), Value::Bool(true));
    }

    #[test]
    fn element_access_indexes_a_list() {
        assert_eq!(eval("[10, 20, 30]:0"), Value::Int(10));
        assert_eq!(eval("[10, 20, 30]:2"), Value::Int(30));
        // Out-of-range wraps (3 mod 3 = 0).
        assert_eq!(eval("[10, 20, 30]:3"), Value::Int(10));
        // A parenthesized negative index reaches from the end.
        assert_eq!(eval("[10, 20, 30]:(-1)"), Value::Int(30));
    }

    #[test]
    fn element_access_on_empty_list_is_null() {
        assert_eq!(eval("[]:0"), Value::Null);
    }

    #[test]
    fn map_literal_builds_pairs() {
        assert_eq!(
            eval("{'a' -> 1, 'b' -> 2}"),
            Value::Map(vec![
                (Value::String("a".to_owned()), Value::Int(1)),
                (Value::String("b".to_owned()), Value::Int(2)),
            ])
        );
    }

    #[test]
    fn map_literal_empty_is_an_empty_map() {
        assert_eq!(eval("{}"), Value::Map(vec![]));
    }

    /// Duplicate keys are last-wins.
    #[test]
    fn map_literal_duplicate_keys_last_wins() {
        assert_eq!(
            eval("{'a' -> 1, 'a' -> 2}"),
            Value::Map(vec![(Value::String("a".to_owned()), Value::Int(2))])
        );
    }

    /// A non-arrow entry becomes a key with a null value (a set-like map).
    #[test]
    fn map_literal_bare_values_have_null_values() {
        assert_eq!(
            eval("{1, 2}"),
            Value::Map(vec![
                (Value::Int(1), Value::Null),
                (Value::Int(2), Value::Null),
            ])
        );
    }

    /// A 2-element list entry is taken as a key/value pair.
    #[test]
    fn map_literal_two_element_list_is_a_pair() {
        assert_eq!(
            eval("{[1, 2]}"),
            Value::Map(vec![(Value::Int(1), Value::Int(2))])
        );
    }

    /// A map literal composes with `:` element access.
    #[test]
    fn map_literal_then_element_access() {
        assert_eq!(eval("{'a' -> 1, 'b' -> 2}:'b'"), Value::Int(2));
    }

    /// A list entry whose length is not 2 cannot be a pair.
    #[test]
    fn map_entry_wrong_length_list_is_an_error() {
        let cst = parse_source("{[1, 2, 3]}").expect("parse");
        let code = Code::try_from(&cst).expect("lower");
        let mut global = GlobalState::new();
        let mut vm = global.create_new_vm();
        assert!(matches!(vm.push(code), Err(VmError::MapEntryNotPair)));
    }

    #[test]
    fn match_finds_a_list_index() {
        assert_eq!(eval("[10, 20, 30] ~ 20"), Value::Int(1));
        // A missing element yields null.
        assert_eq!(eval("[10, 20, 30] ~ 99"), Value::Null);
    }

    #[test]
    fn match_tests_map_key_presence() {
        assert_eq!(
            eval("{'a' -> 1, 'b' -> 2} ~ 'a'"),
            Value::String("a".to_owned())
        );
        assert_eq!(eval("{'a' -> 1} ~ 'z'"), Value::Null);
    }

    #[test]
    fn match_runs_a_regex_on_a_string() {
        // No capture groups: the whole match.
        assert_eq!(eval("'hello' ~ 'l+'"), Value::String("ll".to_owned()));
        // No match yields null.
        assert_eq!(eval("'hello' ~ 'z'"), Value::Null);
    }

    #[test]
    fn match_regex_capture_groups() {
        // One group yields that group.
        assert_eq!(
            eval("'abc123' ~ '([a-z]+)'"),
            Value::String("abc".to_owned())
        );
        // Several groups yield a list.
        assert_eq!(
            eval("'a1b2' ~ '([a-z])([0-9])'"),
            Value::list(vec![
                Value::String("a".to_owned()),
                Value::String("1".to_owned()),
            ])
        );
    }

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
    /// [`GlobalState`]'s configured stdout — here a shared buffer, exactly as the
    /// playground captures it to display a program's output.
    #[test]
    fn builtin_print_writes_lines_to_configured_stdout() {
        use std::sync::{Arc, Mutex};

        /// A `Write` sink over a shared buffer, mirroring the playground's
        /// capture writer.
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let cst = parse_source("print('hello'); print(6 * 7)").expect("parse");
        let code = Code::try_from(&cst).expect("lower");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let mut global = GlobalState::with_stdout(Box::new(Buf(captured.clone())));
        let mut vm = global.create_new_vm();
        vm.push(code).expect("eval");

        let text = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert_eq!(text, "hello\n42\n");
    }

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

    #[test]
    fn unknown_function_is_an_error() {
        let cst = parse_source("nope(1)").expect("parse");
        let code = Code::try_from(&cst).expect("lower");
        let mut global = GlobalState::new();
        let mut vm = global.create_new_vm();
        assert!(matches!(vm.push(code), Err(VmError::UnknownFunction)));
    }

    #[test]
    fn wrong_argument_count_is_an_error() {
        let cst = parse_source("f(x) -> x; f(1, 2)").expect("parse");
        let code = Code::try_from(&cst).expect("lower");
        let mut global = GlobalState::new();
        let mut vm = global.create_new_vm();
        assert!(matches!(vm.push(code), Err(VmError::WrongArgCount)));
    }

    /// `range` reports its `type()` as "iterator", not "list" — a lazy list.
    #[test]
    fn range_is_an_iterator() {
        assert_eq!(eval("type(range(5))"), Value::String("iterator".to_owned()));
    }

    /// `range(to)` counts from 0; the three forms match Python's `range`. It is
    /// an iterator, but compares element-wise equal to the realised list.
    #[test]
    fn range_forms_yield_the_expected_elements() {
        assert_eq!(
            eval("range(5)"),
            Value::list(vec![
                Value::Int(0),
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
            ])
        );
        assert_eq!(
            eval("range(2, 6)"),
            Value::list(vec![
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
                Value::Int(5),
            ])
        );
        assert_eq!(
            eval("range(0, 10, 2)"),
            Value::list(vec![
                Value::Int(0),
                Value::Int(2),
                Value::Int(4),
                Value::Int(6),
                Value::Int(8),
            ])
        );
    }

    /// A negative step counts down; a wrong-way or empty span yields no elements.
    #[test]
    fn range_negative_step_and_empty() {
        assert_eq!(
            eval("range(5, 0, -1)"),
            Value::list(vec![
                Value::Int(5),
                Value::Int(4),
                Value::Int(3),
                Value::Int(2),
                Value::Int(1),
            ])
        );
        assert_eq!(eval("range(0)"), Value::list(vec![]));
        assert_eq!(eval("range(5, 0)"), Value::list(vec![]));
    }

    /// A fractional bound promotes the whole range to doubles.
    #[test]
    fn range_with_fractional_step_is_double() {
        assert_eq!(
            eval("range(0, 1, 0.5)"),
            Value::list(vec![Value::Double(0.0), Value::Double(0.5)])
        );
    }

    /// A range behaves as a list everywhere it flows: `str`, `:` indexing
    /// (wrapping like any list), `==`, and element-wise arithmetic.
    #[test]
    fn range_behaves_like_a_list() {
        assert_eq!(eval("str(range(3))"), Value::String("[0, 1, 2]".to_owned()));
        assert_eq!(eval("range(3):1"), Value::Int(1));
        assert_eq!(eval("range(5):(-1)"), Value::Int(4));
        assert_eq!(eval("range(3) == [0, 1, 2]"), Value::Bool(true));
        assert_eq!(
            eval("range(3) + 1"),
            Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
    }

    /// The list is lazy: a billion-element range is created and randomly indexed
    /// (including from the end) without ever realising its elements.
    #[test]
    fn range_is_lazy() {
        assert_eq!(eval("range(1000000000):5"), Value::Int(5));
        assert_eq!(eval("range(1000000000):(-1)"), Value::Int(999999999));
    }

    /// `range` takes one to three arguments, each numeric.
    #[test]
    fn range_argument_errors() {
        assert!(matches!(eval_err("range()"), VmError::WrongArgCount));
        assert!(matches!(
            eval_err("range(1, 2, 3, 4)"),
            VmError::WrongArgCount
        ));
        assert!(matches!(eval_err("range('a')"), VmError::ExpectedNumber));
    }

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

    /// A literal in a destructuring pattern is not an assignable target.
    #[test]
    fn destructure_literal_target_is_an_error() {
        assert!(matches!(
            eval_err("[a, 1] = [1, 2]"),
            VmError::NotAssignable
        ));
    }

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

    /// `~` is a match operator, not a writable address, so it cannot be assigned.
    #[test]
    fn match_operator_is_not_an_assignment_target() {
        assert!(matches!(
            eval_err("x = [1, 2]; x ~ 1 = 9"),
            VmError::NotAssignable
        ));
    }

    /// An l-value-returning function call (`if(...) = …`) is not a supported target.
    #[test]
    fn lvalue_returning_call_is_not_a_target() {
        assert!(matches!(
            eval_err("if(1, a, b) = 5"),
            VmError::NotAssignable
        ));
    }

    /// A non-assignable left side — a literal, an operator expression, or a
    /// parenthesised expression — lowers fine now (the target is a general
    /// expression) but is rejected at evaluation, where l-value checking moved.
    #[test]
    fn non_assignable_targets_are_rejected_at_eval() {
        assert!(matches!(eval_err("1 = 2"), VmError::NotAssignable));
        assert!(matches!(eval_err("a + b = c"), VmError::NotAssignable));
        assert!(matches!(eval_err("(a) = b"), VmError::NotAssignable));
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

    /// A `var(...)` argument may itself be a container read (`var(m:0)`), which
    /// the r-value evaluator now handles.
    #[test]
    fn var_name_from_a_container_read() {
        assert_eq!(eval("m = ['x', 'y']; var(m:0) = 7; x"), Value::Int(7));
    }
}
