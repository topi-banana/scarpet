use scarpet_syntax::ast::{Args, Code, Expr, Primary};

use super::Evaluate;
use crate::{
    error::VmError,
    value::{Value, ValueContainer},
    vm::ScarpetVm,
};

impl<'src, 'state> Evaluate<Primary<'src>> for ScarpetVm<'state, 'src> {
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
}

#[cfg(test)]
mod tests {
    use crate::error::VmError;
    use crate::test_util::{eval, eval_err};
    use crate::value::Value;

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
        assert_eq!(
            eval("l(1, 2, 3)"),
            Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
    }

    #[test]
    fn paren_yields_its_inner_value() {
        assert_eq!(eval("(1 + 2) * 3"), Value::Int(9));
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
        assert!(matches!(eval_err("{[1, 2, 3]}"), VmError::MapEntryNotPair));
    }
}
