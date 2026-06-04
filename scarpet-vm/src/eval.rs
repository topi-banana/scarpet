// Prototype evaluator: most match arms are still `todo!()`, so the operands
// they bind (`lhs`, `rhs`, ...) are intentionally unused until those arms get
// implemented. Drop this allow once the evaluator is filled in.
#![allow(unused_variables)]

use std::cmp::Ordering;

use scarpet_syntax::ast::{
    Additive, Args, Assign, Code, Compare, Equality, Expr, Get, Land, Lor, Mult, Power, Primary,
    Unary,
};

use crate::{
    error::VmError,
    value::{Value, ValueContainer},
    vm::ScarpetVm,
};

pub trait Evalute<T> {
    fn push(&mut self, st: T) -> Result<ValueContainer, VmError>;
}

impl<'src, 'state> Evalute<Code<'src>> for ScarpetVm<'state> {
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

impl<'src, 'state> Evalute<Expr<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Expr<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Expr::Assign(ost) => self.push(ost),
            _ => todo!(),
        }
    }
}

impl<'src, 'state> Evalute<Assign<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Assign<'src>) -> Result<ValueContainer, VmError> {
        use scarpet_syntax::ast::{AssignOp, Assignable};
        match st {
            Assign::Set { target, op, value } => {
                let var = match target {
                    Assignable::Var(name) => self.var.get(name).cloned().unwrap_or_else(|| {
                        let v = ValueContainer::null();
                        self.var.insert(name.to_owned(), v.clone());
                        v
                    }),
                    _ => todo!(),
                };
                let val = self.push(*value)?;
                match op {
                    AssignOp::Assign => *var.lock()? = val.lock()?.clone(),
                    AssignOp::Add => *var.lock()? += val.lock()?.clone(),
                    AssignOp::Swap => std::mem::swap(&mut *var.lock()?, &mut *val.lock()?),
                }
                Ok(var.clone())
            }
            Assign::Lor(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Lor<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Lor<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Lor::Or { lhs, rhs } => Ok(ValueContainer::bool(
                self.push(*lhs)?.lock()?.is_true() || self.push(rhs)?.lock()?.is_true(),
            )),
            Lor::Land(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Land<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Land<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Land::And { lhs, rhs } => Ok(ValueContainer::bool(
                self.push(*lhs)?.lock()?.is_true() && self.push(rhs)?.lock()?.is_true(),
            )),
            Land::Equality(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Equality<'src>> for ScarpetVm<'state> {
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

impl<'src, 'state> Evalute<Compare<'src>> for ScarpetVm<'state> {
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

impl<'src, 'state> Evalute<Additive<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Additive<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Additive::Add { lhs, rhs } => self.push(*lhs)? + self.push(rhs)?,
            Additive::Sub { lhs, rhs } => self.push(*lhs)? - self.push(rhs)?,
            Additive::Mult(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Mult<'src>> for ScarpetVm<'state> {
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

impl<'src, 'state> Evalute<Power<'src>> for ScarpetVm<'state> {
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

impl<'src, 'state> Evalute<Unary<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Unary<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Unary::Neg(v) => self.push(*v)?.scarpet_neg(),
            Unary::Pos(v) => self.push(*v)?.scarpet_pos(),
            Unary::Not(v) => self.push(*v)?.scarpet_not(),
            Unary::Unpack(v) => todo!(),
            Unary::Get(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Get<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Get<'src>) -> Result<ValueContainer, VmError> {
        use scarpet_syntax::ast::GetOp;
        match st {
            Get::Index { base, op, key } => {
                let base = self.push(*base)?;
                let key = self.push(key)?;
                match op {
                    GetOp::Get => base.scarpet_get(&key),
                    // `~` (match) needs regex for the string case; not yet done.
                    GetOp::Match => todo!(),
                }
            }
            Get::Primary(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Primary<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Primary<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Primary::Number(v) => Ok(ValueContainer::new(Value::from_number_literal(v))),
            Primary::Str(v) => Ok(ValueContainer::new(Value::from_string_literal(v))),
            // A bare variable reference reads the current binding; an unset name
            // evaluates to `undef` (the original `strict`-config UndefValue).
            Primary::Ident(name) => Ok(self
                .var
                .get(name)
                .cloned()
                .unwrap_or_else(ValueContainer::undef)),
            Primary::Call { name, args } => todo!(),
            // `[a, b, …]`: evaluate each comma-separated element to a value.
            Primary::List(Args(codes)) => {
                let mut items = Vec::with_capacity(codes.len());
                for code in codes {
                    items.push(self.push(code)?.lock()?.clone());
                }
                Ok(ValueContainer::new(Value::List(items)))
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

impl<'state> ScarpetVm<'state> {
    /// Evaluate one entry of a map literal (`{…}` / `m(…)`) into a key/value
    /// pair. In map context a top-level `->` is not a lambda but a pair (the
    /// original evaluates these args in `MAPDEF` context). Otherwise the entry
    /// is a value handled like `MapValue.put`: a 2-element list is a pair, any
    /// other list is an error, and a non-list becomes a key with a null value.
    fn eval_map_entry<'src>(
        &mut self,
        Code(mut exprs): Code<'src>,
    ) -> Result<(Value, Value), VmError> {
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
        let mut global = GlobalState {};
        let mut vm = global.create_new_vm();
        vm.push(code).expect("eval").lock().expect("lock").clone()
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
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
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
        let mut global = GlobalState {};
        let mut vm = global.create_new_vm();
        assert!(matches!(vm.push(code), Err(VmError::MapEntryNotPair)));
    }
}
