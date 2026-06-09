use super::{EPSILON, Value};

impl Value {
    /// Truthiness in a boolean context. Corresponds to the original `getBoolean`.
    pub fn is_true(&self) -> bool {
        match self {
            // The original errors on an uninitialized reference; here it is false.
            Value::Undef | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            // Treat rounding error near zero as zero (as the original does). NaN is false.
            Value::Double(d) => d.abs() > EPSILON,
            Value::String(s) => !s.is_empty(),
            Value::List(items) => !items.is_empty(),
            Value::Map(entries) => !entries.is_empty(),
        }
    }

    /// The type name the original `type()` returns.
    pub fn type_name(&self) -> &'static str {
        match self {
            // In the original, undef also reports the type name "null".
            Value::Undef | Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) | Value::Double(_) => "number",
            Value::String(_) => "string",
            // A realised list reports "list", a lazy one (a `range`) "iterator".
            Value::List(items) => items.type_name(),
            Value::Map(_) => "map",
        }
    }

    /// The plain string representation used when an operator falls back to text
    /// (string concatenation, `replace`, `substring`). Corresponds to the
    /// original `Value.getString`: a list renders as `[a, b]` and a map as
    /// `{k: v}`, with every element rendered by `getString` (so nested strings
    /// carry no quotes).
    pub fn to_scarpet_string(&self) -> String {
        match self {
            Value::Undef | Value::Null => "null".to_owned(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            // Rust's `f64` display already drops the fractional part for
            // integer-valued doubles (`2.0` -> "2"), matching `NumericValue`.
            Value::Double(d) => format!("{d}"),
            Value::String(s) => s.clone(),
            Value::List(items) => {
                let inner = (0..items.len())
                    .filter_map(|i| items.get(i))
                    .map(|item| item.to_scarpet_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{inner}]")
            }
            Value::Map(entries) => {
                let inner = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.to_scarpet_string(), v.to_scarpet_string()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{inner}}}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A list renders with `getString` semantics: `[1, 2]`, no quotes on elements.
    #[test]
    fn to_scarpet_string_renders_list_without_quotes() {
        let list = Value::list(vec![Value::Int(1), Value::String("a".to_owned())]);

        assert_eq!(list.to_scarpet_string(), "[1, a]");
    }
}
