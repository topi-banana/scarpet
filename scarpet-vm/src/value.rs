use std::{
    ops::{Add, AddAssign},
    sync::{Arc, Mutex},
};

use crate::error::VmError;

/// Tolerance for treating a number as zero. Matches the value the original
/// `NumericValue` derives from `abs(32 * ((7 * 0.1) * 10 - 7))` (about 3.4e-14),
/// used to absorb floating-point rounding error.
const EPSILON: f64 = 3.410_605_131_648_481e-14;

/// A Scarpet value.
///
/// Mirrors the `carpet.script.value.Value` hierarchy from the original
/// fabric-carpet. This only models the language-core types that do not depend on
/// Minecraft; for the name `type()` returns, see [`Value::type_name`].
///
/// Types that exist in the original but are not carried here yet, because the VM
/// lacks the machinery for them:
/// - `iterator`: a lazy list such as `range` (`LazyListValue`)
/// - `function`: a first-class function (`FunctionValue`)
/// - `task`: a concurrent task (`ThreadValue`)
///
/// Minecraft-specific value types (`block` / `entity` / `nbt` / `screen` /
/// `text`) are not primitives either, so they are excluded here.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A variable referenced before initialization under the `strict` app config
    /// (`UndefValue`). Treated the same as `null` by `type()`.
    Undef,
    /// `null` (`NullValue`).
    Null,
    /// A boolean (`BooleanValue`). In Scarpet, a subtype of the numbers `0` / `1`.
    Bool(bool),
    /// An integer. Scarpet keeps `long` precision, so integers get a dedicated
    /// representation (matching the long form of `NumericValue`).
    Int(i64),
    /// A floating-point number. Scarpet's base numeric representation is `double`
    /// (`NumericValue`).
    Double(f64),
    /// A string (`StringValue`).
    String(String),
    /// A list `[...]` / `l(...)` (`ListValue`).
    List(Vec<Value>),
    /// A map `{...}` / `m(...)` (`MapValue`). The original is an unordered hash
    /// map, but since keys may be arbitrary values we keep it here simply as a
    /// sequence of key/value pairs.
    Map(Vec<(Value, Value)>),
}

impl Value {
    /// Converts a numeric literal (the source text of `Primary::Number`) into an
    /// `Int` or a `Double`: `Int` when it parses as an integer, `Double` for a
    /// fractional or exponent form.
    pub fn from_number_literal(s: &str) -> Value {
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))
            && let Ok(i) = i64::from_str_radix(hex, 16)
        {
            return Value::Int(i);
        }
        if let Ok(i) = s.parse::<i64>() {
            return Value::Int(i);
        }
        if let Ok(d) = s.parse::<f64>() {
            return Value::Double(d);
        }
        Value::Double(f64::NAN)
    }

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
            Value::List(_) => "list",
            Value::Map(_) => "map",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ValueContainer(Arc<Mutex<Value>>);

impl ValueContainer {
    pub fn new(value: Value) -> Self {
        Self(Arc::new(Mutex::new(value)))
    }
    pub fn undef() -> Self {
        Self::new(Value::Undef)
    }
    pub fn null() -> Self {
        Self::new(Value::Null)
    }
    pub fn bool(value: bool) -> Self {
        Self::new(Value::Bool(value))
    }
    pub fn int(value: i64) -> Self {
        Self::new(Value::Int(value))
    }
    pub fn double(value: f64) -> Self {
        Self::new(Value::Double(value))
    }
    pub fn string(value: String) -> Self {
        Self::new(Value::String(value))
    }
    pub fn lock(&self) -> Result<std::sync::MutexGuard<'_, Value>, VmError> {
        match self.0.lock() {
            Ok(val) => Ok(val),
            Err(_) => todo!(),
        }
    }
}

impl AddAssign for Value {
    fn add_assign(&mut self, rhs: Self) {
        // Integers stay at long precision; if either side is floating-point,
        // promote to double arithmetic (the original `NumericValue.add`). Lists
        // add element-wise. The remaining combinations are not implemented yet.
        let sum = match (self.to_owned(), rhs) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_add(b)),
            (Value::Int(a), Value::Double(b)) => Value::Double(a as f64 + b),
            (Value::Int(a), Value::String(b)) => Value::String(format!("{a}{b}")),
            (Value::Double(a), Value::Int(b)) => Value::Double(a + b as f64),
            (Value::Double(a), Value::Double(b)) => Value::Double(a + b),
            (Value::Double(a), Value::String(b)) => Value::String(format!("{a}{b}")),
            (Value::String(a), Value::Int(b)) => Value::String(format!("{a}{b}")),
            (Value::String(a), Value::Double(b)) => Value::String(format!("{a}{b}")),
            (Value::String(a), Value::String(b)) => Value::String(format!("{a}{b}")),
            // Two lists of equal length add pairwise (the original `ListValue.add`).
            (Value::List(a), Value::List(b)) if a.len() == b.len() => {
                let summed = a
                    .into_iter()
                    .zip(b)
                    .map(|(mut item, addend)| {
                        item += addend;
                        item
                    })
                    .collect();
                Value::List(summed)
            }
            // A list plus a scalar adds the scalar to each element
            // (`[1, 2, 3] + 1` -> `[2, 3, 4]`).
            (Value::List(items), rhs) => {
                let summed = items
                    .into_iter()
                    .map(|mut item| {
                        item += rhs.clone();
                        item
                    })
                    .collect();
                Value::List(summed)
            }
            _ => todo!(),
        };
        *self = sum;
    }
}

impl Add for ValueContainer {
    type Output = Result<ValueContainer, VmError>;
    fn add(self, rhs: Self) -> Self::Output {
        let mut sum = self.lock()?.clone();
        sum += rhs.lock()?.clone();
        Ok(ValueContainer::new(sum))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[1, 2, 3] + 1` adds the scalar to each element, yielding `[2, 3, 4]`.
    #[test]
    fn add_assign_adds_scalar_to_each_list_element() {
        let mut list = Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        list += Value::Int(1);

        assert_eq!(
            list,
            Value::List(vec![Value::Int(2), Value::Int(3), Value::Int(4)])
        );
    }

    /// Two lists of equal length add pairwise: `[1, 2, 3] + [10, 20, 30]` -> `[11, 22, 33]`.
    #[test]
    fn add_assign_adds_equal_length_lists_pairwise() {
        let mut list = Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        list += Value::List(vec![Value::Int(10), Value::Int(20), Value::Int(30)]);

        assert_eq!(
            list,
            Value::List(vec![Value::Int(11), Value::Int(22), Value::Int(33)])
        );
    }
}
