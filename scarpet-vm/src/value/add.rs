use std::ops::{AddAssign, SubAssign};

use super::Value;

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
            // Each owned operand drains through `into_iter`, so a `range` works.
            (Value::List(a), Value::List(b)) if a.len() == b.len() => {
                let summed = a
                    .into_iter()
                    .zip(b)
                    .map(|(mut item, addend)| {
                        item += addend;
                        item
                    })
                    .collect();
                Value::list(summed)
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
                Value::list(summed)
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
                Value::list(subbed)
            }
            (Value::List(items), rhs) => {
                let subbed = items
                    .into_iter()
                    .map(|mut item| {
                        item -= rhs.clone();
                        item
                    })
                    .collect();
                Value::list(subbed)
            }
            (lhs, rhs) => Value::String(
                lhs.to_scarpet_string()
                    .replace(&rhs.to_scarpet_string(), ""),
            ),
        };
        *self = diff;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[1, 2, 3] + 1` adds the scalar to each element, yielding `[2, 3, 4]`.
    #[test]
    fn add_assign_adds_scalar_to_each_list_element() {
        let mut list = Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        list += Value::Int(1);

        assert_eq!(
            list,
            Value::list(vec![Value::Int(2), Value::Int(3), Value::Int(4)])
        );
    }

    /// Two lists of equal length add pairwise: `[1, 2, 3] + [10, 20, 30]` -> `[11, 22, 33]`.
    #[test]
    fn add_assign_adds_equal_length_lists_pairwise() {
        let mut list = Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        list += Value::list(vec![Value::Int(10), Value::Int(20), Value::Int(30)]);

        assert_eq!(
            list,
            Value::list(vec![Value::Int(11), Value::Int(22), Value::Int(33)])
        );
    }

    /// `1 + [1, 2]` has no numeric/list rule, so it concatenates as text: "1[1, 2]".
    #[test]
    fn add_assign_falls_back_to_string_concatenation() {
        let mut n = Value::Int(1);
        n += Value::list(vec![Value::Int(1), Value::Int(2)]);

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
        n -= Value::list(vec![Value::Int(1), Value::Int(2)]);

        assert_eq!(n, Value::String("1".to_owned()));
    }
}
