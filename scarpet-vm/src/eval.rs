// Prototype evaluator: most match arms are still `todo!()`, so the operands
// they bind (`lhs`, `rhs`, ...) are intentionally unused until those arms get
// implemented. Drop this allow once the evaluator is filled in.
#![allow(unused_variables)]

use std::{cmp::Ordering, rc::Rc};

use scarpet_syntax::ast::{
    Additive, Args, Assign, AssignOp, Assignable, Code, Compare, Equality, Expr, Get, Land, Lor,
    Mult, Patterns, Power, Primary, RestPat, Unary,
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
        use scarpet_syntax::ast::GetOp;
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

    /// Carry out `target <op> value` — the body of an [`Assign::Set`]. The two
    /// target shapes need different handling: a *destructuring* list pattern
    /// (`[a, b]`, `l(a, b)`) spreads `value` across several sub-targets and is
    /// `=`-only, while a *single place* (`Var`, `var(expr)`, and eventually
    /// `base:key`) resolves to one shared slot that `op` updates in place.
    fn assign(
        &mut self,
        target: Assignable<'src>,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        match target {
            // `[a, b] = …` and `l(a, b) = …` are the same list-constructor l-value.
            Assignable::List(patterns) => self.destructure(patterns, op, value),
            Assignable::Call { name: "l", args } => self.destructure(args, op, value),
            // Everything else resolves to a single mutable place, which `op` writes
            // through; the slot itself is the assignment's value (so `b = a = 1`
            // reads `a`'s slot for `b`).
            target => {
                let place = self.resolve_place(target)?;
                match op {
                    AssignOp::Assign => *place.lock()? = value.lock()?.clone(),
                    AssignOp::Add => *place.lock()? += value.lock()?.clone(),
                    AssignOp::Swap => std::mem::swap(&mut *place.lock()?, &mut *value.lock()?),
                }
                Ok(place)
            }
        }
    }

    /// Resolve a single-place assignment target to its shared [`ValueContainer`]
    /// slot: the local slot for a bare `Var`, or the dynamically named slot for
    /// `var(expr)` (whose argument evaluates to the variable name). Writing
    /// through the returned slot updates the binding in place.
    fn resolve_place(&mut self, target: Assignable<'src>) -> Result<ValueContainer, VmError> {
        match target {
            Assignable::Var(name) => Ok(self.get_var(name)),
            // `var(expr)` names a variable dynamically — `var('x' + i) = …`.
            Assignable::Call { name: "var", args } => {
                let name = self.eval_var_name(args)?;
                Ok(self.get_var(&name))
            }
            // TODO(assign:index) container element assignment (`x:0 = 5`,
            // `m:'k' = v`) still needs an in-place write path: resolve `base` to
            // its place, then mutate the `Value` inside via a new
            // `Value::scarpet_put` (the read path `scarpet_get` only clones). The
            // `ListValue` trait is read-only today, so it needs a `set` primitive.
            Assignable::Index { .. } => todo!("indexed container assignment"),
            // Any other call would be an l-value-returning function (`if(c, a, b)`),
            // not modelled yet; `l(…)` is handled as a destructure before here.
            Assignable::Call { .. } => todo!("call-shaped assignment target"),
            // A list pattern is multi-value (handled in `assign`), and a computed
            // `Expr` element — the `1` in `[a, 1] = …` — is not a valid l-value.
            Assignable::List(_) | Assignable::Expr(_) => Err(VmError::NotAssignable),
        }
    }

    /// Spread `value` across a destructuring list pattern (`[a, b] = …`,
    /// `l(a, b) = …`). The value must be a list; its elements bind to the pattern
    /// elements by position, with a `...rest` binder collecting the leftover
    /// middle into a fresh list. Sub-patterns recurse through [`assign`](Self::assign),
    /// so a nested `[[a], b]` works. Only `=` destructures; the result is `true`,
    /// as in the original.
    fn destructure(
        &mut self,
        Patterns { before, rest }: Patterns<'src>,
        op: AssignOp,
        value: ValueContainer,
    ) -> Result<ValueContainer, VmError> {
        if op != AssignOp::Assign {
            // `+=` / `<>` over a list pattern follow the original's operator
            // delegation, not modelled yet.
            todo!("destructuring assignment supports only `=`");
        }
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
                for (pat, item) in before.into_iter().zip(items) {
                    self.assign(pat, AssignOp::Assign, ValueContainer::new(item))?;
                }
            }
            // `[a, ...rest, b]`: bind `before` from the front and `after` from the
            // back, and collect the middle into the rest binder as a new list.
            Some(RestPat { binder, after }) => {
                let fixed = before.len() + after.len();
                if items.len() < fixed {
                    return Err(VmError::TooFewValuesToUnpack);
                }
                let tail = items.split_off(items.len() - after.len());
                let middle = items.split_off(before.len());
                for (pat, item) in before.into_iter().zip(items) {
                    self.assign(pat, AssignOp::Assign, ValueContainer::new(item))?;
                }
                self.assign(
                    *binder,
                    AssignOp::Assign,
                    ValueContainer::new(Value::list(middle)),
                )?;
                for (pat, item) in after.into_iter().zip(tail) {
                    self.assign(pat, AssignOp::Assign, ValueContainer::new(item))?;
                }
            }
        }
        Ok(ValueContainer::bool(true))
    }

    /// Evaluate the single argument of a dynamic `var(expr)` target to the name
    /// of the variable it selects: `var` takes one argument, and its value's
    /// string form is the name (so `var('x' + i)` and `var(key)` both work).
    fn eval_var_name(&mut self, args: Patterns<'src>) -> Result<String, VmError> {
        if args.rest.is_some() || args.before.len() != 1 {
            return Err(VmError::WrongArgCount);
        }
        let arg = args.before.into_iter().next().unwrap();
        let value = self.eval_pattern(arg)?;
        let name = value.lock()?.to_scarpet_string();
        Ok(name)
    }

    /// Evaluate a pattern element as an ordinary value rather than a place — used
    /// for a target argument that names something (the `expr` in `var(expr)`). A
    /// computed `Expr` evaluates its expression; a bare `Var` reads its binding.
    fn eval_pattern(&mut self, pat: Assignable<'src>) -> Result<ValueContainer, VmError> {
        match pat {
            Assignable::Expr(assign) => self.push(*assign),
            Assignable::Var(name) => Ok(self.get_var(name)),
            // Other shapes as a `var` argument are unusual; not modelled yet.
            _ => todo!("non-trivial var() argument"),
        }
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
}
