use std::ops::{DivAssign, MulAssign};

use super::Value;

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
                Value::list(multiplied)
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
                Value::list(multiplied)
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
                Value::list(divided)
            }
            (Value::List(items), rhs) => {
                let divided = items
                    .into_iter()
                    .map(|mut item| {
                        item /= rhs.clone();
                        item
                    })
                    .collect();
                Value::list(divided)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `[1, 2] * [1, 2]` multiplies pairwise: `[1, 4]`.
    #[test]
    fn mul_assign_multiplies_equal_length_lists_pairwise() {
        let mut list = Value::list(vec![Value::Int(1), Value::Int(2)]);
        list *= Value::list(vec![Value::Int(1), Value::Int(2)]);

        assert_eq!(list, Value::list(vec![Value::Int(1), Value::Int(4)]));
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
}
