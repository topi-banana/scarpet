use std::{
    ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub, SubAssign},
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

    /// Converts a string literal (the source text of `Primary::Str`, with its
    /// surrounding single quotes) into a `String` value. Strips the quotes and
    /// expands escapes the way the original `Tokenizer` does: `\n`/`\t` become
    /// newline/tab, and every other `\x` keeps `x` verbatim (so `\\` and `\'`
    /// are a literal backslash and quote). The original rejects `\r`; lacking an
    /// error channel we let it fall into that pass-through case.
    pub fn from_string_literal(s: &str) -> Value {
        let inner = s
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .unwrap_or(s);
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                // `\\`, `\'`, and any other `\x` keep the second char as-is.
                Some(other) => out.push(other),
                // A trailing backslash; the lexer's string regex never emits one.
                None => out.push('\\'),
            }
        }
        Value::String(out)
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
                let inner = items
                    .iter()
                    .map(Value::to_scarpet_string)
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
            // Any other mix concatenates the two as text (the original
            // `Value.add`): `1 + [1, 2]` -> "1[1, 2]".
            (lhs, rhs) => Value::String(format!(
                "{}{}",
                lhs.to_scarpet_string(),
                rhs.to_scarpet_string()
            )),
        };
        *self = sum;
    }
}

impl SubAssign for Value {
    fn sub_assign(&mut self, rhs: Self) {
        // Numbers subtract numerically; equal-length lists subtract pairwise and
        // a list minus a scalar subtracts it from each element (the original
        // `NumericValue`/`ListValue.subtract`). Any other mix deletes every
        // occurrence of the right side's text from the left side's text
        // (`Value.subtract`): `'hello' - 1` -> "hello", `1 - [1, 2]` -> "1".
        let diff = match (self.to_owned(), rhs) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_sub(b)),
            (Value::Int(a), Value::Double(b)) => Value::Double(a as f64 - b),
            (Value::Double(a), Value::Int(b)) => Value::Double(a - b as f64),
            (Value::Double(a), Value::Double(b)) => Value::Double(a - b),
            (Value::List(a), Value::List(b)) if a.len() == b.len() => {
                let subbed = a
                    .into_iter()
                    .zip(b)
                    .map(|(mut item, subtrahend)| {
                        item -= subtrahend;
                        item
                    })
                    .collect();
                Value::List(subbed)
            }
            (Value::List(items), rhs) => {
                let subbed = items
                    .into_iter()
                    .map(|mut item| {
                        item -= rhs.clone();
                        item
                    })
                    .collect();
                Value::List(subbed)
            }
            (lhs, rhs) => Value::String(
                lhs.to_scarpet_string()
                    .replace(&rhs.to_scarpet_string(), ""),
            ),
        };
        *self = diff;
    }
}

impl MulAssign for Value {
    fn mul_assign(&mut self, rhs: Self) {
        // Numbers multiply numerically; equal-length lists multiply pairwise and
        // a list and a scalar (in either order) scale each element (the original
        // `NumericValue`/`ListValue.multiply`). A string and a number repeat the
        // string (`'hello' * 2` -> "hellohello"), and two strings join with a
        // dot (`Value.multiply`).
        let product = match (self.to_owned(), rhs) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_mul(b)),
            (Value::Int(a), Value::Double(b)) => Value::Double(a as f64 * b),
            (Value::Double(a), Value::Int(b)) => Value::Double(a * b as f64),
            (Value::Double(a), Value::Double(b)) => Value::Double(a * b),
            (Value::List(a), Value::List(b)) if a.len() == b.len() => {
                let multiplied = a
                    .into_iter()
                    .zip(b)
                    .map(|(mut item, factor)| {
                        item *= factor;
                        item
                    })
                    .collect();
                Value::List(multiplied)
            }
            // A list and a scalar, whichever side the list is on.
            (Value::List(items), scalar) | (scalar, Value::List(items)) => {
                let multiplied = items
                    .into_iter()
                    .map(|mut item| {
                        item *= scalar.clone();
                        item
                    })
                    .collect();
                Value::List(multiplied)
            }
            (Value::String(s), Value::Int(n)) | (Value::Int(n), Value::String(s)) => {
                Value::String(s.repeat(n.max(0) as usize))
            }
            (Value::String(s), Value::Double(n)) | (Value::Double(n), Value::String(s)) => {
                Value::String(s.repeat((n as i64).max(0) as usize))
            }
            (Value::String(a), Value::String(b)) => Value::String(format!("{a}.{b}")),
            _ => todo!(),
        };
        *self = product;
    }
}

impl DivAssign for Value {
    fn div_assign(&mut self, rhs: Self) {
        // Numbers always divide as doubles (`4 / 2` -> 2.0); equal-length lists
        // divide pairwise and a list over a scalar divides each element (the
        // original `NumericValue`/`ListValue.divide`). A string over a number
        // keeps its leading `len / n` characters (`'hello' / 2` -> "he"); any
        // other mix joins as `left/right` (`Value.divide`).
        let quotient = match (self.to_owned(), rhs) {
            (Value::Int(a), Value::Int(b)) => Value::Double(a as f64 / b as f64),
            (Value::Int(a), Value::Double(b)) => Value::Double(a as f64 / b),
            (Value::Double(a), Value::Int(b)) => Value::Double(a / b as f64),
            (Value::Double(a), Value::Double(b)) => Value::Double(a / b),
            (Value::List(a), Value::List(b)) if a.len() == b.len() => {
                let divided = a
                    .into_iter()
                    .zip(b)
                    .map(|(mut item, divisor)| {
                        item /= divisor;
                        item
                    })
                    .collect();
                Value::List(divided)
            }
            (Value::List(items), rhs) => {
                let divided = items
                    .into_iter()
                    .map(|mut item| {
                        item /= rhs.clone();
                        item
                    })
                    .collect();
                Value::List(divided)
            }
            (Value::String(s), Value::Int(n)) => Value::String(string_head(&s, n as f64)),
            (Value::String(s), Value::Double(n)) => Value::String(string_head(&s, n)),
            (lhs, rhs) => Value::String(format!(
                "{}/{}",
                lhs.to_scarpet_string(),
                rhs.to_scarpet_string()
            )),
        };
        *self = quotient;
    }
}

/// The leading `floor(len / divisor)` characters of `s` — the original
/// `Value.divide` behaviour for a string over a number (`'hello' / 2` -> "he").
fn string_head(s: &str, divisor: f64) -> String {
    let len = s.chars().count();
    // Truncate toward zero like the original `(int)` cast. The saturating `f64`
    // cast maps a non-positive or non-finite count to 0 / `usize::MAX`, and
    // `take` stops at the end of the string if the count overshoots, so this
    // never panics the way Java's `substring` would.
    let take = (len as f64 / divisor) as usize;
    s.chars().take(take).collect()
}

impl Add for ValueContainer {
    type Output = Result<ValueContainer, VmError>;
    fn add(self, rhs: Self) -> Self::Output {
        let mut sum = self.lock()?.clone();
        sum += rhs.lock()?.clone();
        Ok(ValueContainer::new(sum))
    }
}

impl Sub for ValueContainer {
    type Output = Result<ValueContainer, VmError>;
    fn sub(self, rhs: Self) -> Self::Output {
        let mut diff = self.lock()?.clone();
        diff -= rhs.lock()?.clone();
        Ok(ValueContainer::new(diff))
    }
}

impl Mul for ValueContainer {
    type Output = Result<ValueContainer, VmError>;
    fn mul(self, rhs: Self) -> Self::Output {
        let mut product = self.lock()?.clone();
        product *= rhs.lock()?.clone();
        Ok(ValueContainer::new(product))
    }
}

impl Div for ValueContainer {
    type Output = Result<ValueContainer, VmError>;
    fn div(self, rhs: Self) -> Self::Output {
        let mut quotient = self.lock()?.clone();
        quotient /= rhs.lock()?.clone();
        Ok(ValueContainer::new(quotient))
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

    /// `1 + [1, 2]` has no numeric/list rule, so it concatenates as text: "1[1, 2]".
    #[test]
    fn add_assign_falls_back_to_string_concatenation() {
        let mut n = Value::Int(1);
        n += Value::List(vec![Value::Int(1), Value::Int(2)]);

        assert_eq!(n, Value::String("1[1, 2]".to_owned()));
    }

    /// `5 - 3` subtracts numerically.
    #[test]
    fn sub_assign_subtracts_numbers() {
        let mut n = Value::Int(5);
        n -= Value::Int(3);

        assert_eq!(n, Value::Int(2));
    }

    /// `'hello' - 1` deletes the right side's text; "1" is absent, so "hello" stays.
    #[test]
    fn sub_assign_removes_substring_from_string() {
        let mut s = Value::String("hello".to_owned());
        s -= Value::Int(1);

        assert_eq!(s, Value::String("hello".to_owned()));
    }

    /// `1 - [1, 2]` deletes "[1, 2]" from "1"; it is absent, so "1" stays.
    #[test]
    fn sub_assign_number_minus_list_keeps_number_text() {
        let mut n = Value::Int(1);
        n -= Value::List(vec![Value::Int(1), Value::Int(2)]);

        assert_eq!(n, Value::String("1".to_owned()));
    }

    /// `[1, 2] * [1, 2]` multiplies pairwise: `[1, 4]`.
    #[test]
    fn mul_assign_multiplies_equal_length_lists_pairwise() {
        let mut list = Value::List(vec![Value::Int(1), Value::Int(2)]);
        list *= Value::List(vec![Value::Int(1), Value::Int(2)]);

        assert_eq!(list, Value::List(vec![Value::Int(1), Value::Int(4)]));
    }

    /// `'hello' * 2` repeats the string, regardless of operand order.
    #[test]
    fn mul_assign_repeats_string_by_number() {
        let mut s = Value::String("hello".to_owned());
        s *= Value::Int(2);
        assert_eq!(s, Value::String("hellohello".to_owned()));

        let mut n = Value::Int(2);
        n *= Value::String("hello".to_owned());
        assert_eq!(n, Value::String("hellohello".to_owned()));
    }

    /// `'hello' / 2` keeps the leading `floor(5 / 2)` = 2 characters: "he".
    #[test]
    fn div_assign_takes_leading_chars_of_string() {
        let mut s = Value::String("hello".to_owned());
        s /= Value::Int(2);

        assert_eq!(s, Value::String("he".to_owned()));
    }

    /// `4 / 2` always yields a double (`2.0`), as the original divides via `getDouble`.
    #[test]
    fn div_assign_divides_numbers_as_double() {
        let mut n = Value::Int(4);
        n /= Value::Int(2);

        assert_eq!(n, Value::Double(2.0));
    }

    /// A list renders with `getString` semantics: `[1, 2]`, no quotes on elements.
    #[test]
    fn to_scarpet_string_renders_list_without_quotes() {
        let list = Value::List(vec![Value::Int(1), Value::String("a".to_owned())]);

        assert_eq!(list.to_scarpet_string(), "[1, a]");
    }
}
