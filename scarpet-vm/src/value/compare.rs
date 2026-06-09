use std::cmp::Ordering;

use super::{EPSILON, Value};
use crate::error::VmError;

impl Value {
    /// Scarpet `==` equality (`v1.equals(v2)` in the original `Operators`).
    /// This is NOT the structural equality of the derived `PartialEq` the tests
    /// use: `1 == 1.0` is true, `NaN == NaN` is false, and a number compared
    /// with a string falls back to comparing their `getString()`s. Mirrors
    /// `NumericValue` / `ListValue` / `MapValue` / `NullValue` `equals`, with the
    /// base `Value.equals` (`compareTo == 0`) for the leftover cross-type mixes.
    pub fn scarpet_eq(&self, other: &Value) -> bool {
        match (self, other) {
            // null/undef equal only each other (`NullValue.equals` + `isNull`).
            (Value::Undef | Value::Null, Value::Undef | Value::Null) => true,
            (Value::Undef | Value::Null, _) | (_, Value::Undef | Value::Null) => false,
            // The numeric tower (a bool is a 0/1 number). int vs int is exact;
            // once a double is involved, NaN is unequal to everything and the
            // rest compare within EPSILON (the original `!subtract().getBoolean()`).
            (a, b) if a.is_number_like() && b.is_number_like() => {
                match (a.as_long(), b.as_long()) {
                    (Some(x), Some(y)) => x == y,
                    _ => {
                        let (x, y) = (a.as_double(), b.as_double());
                        !x.is_nan() && !y.is_nan() && (x - y).abs() <= EPSILON
                    }
                }
            }
            // Lists compare structurally, recursing so `[1] == [1.0]` holds.
            // Walking by index means a `range` and the equal list compare equal
            // too, without draining (consuming) either side.
            (Value::List(a), Value::List(b)) => {
                a.len() == b.len()
                    && (0..a.len()).all(|i| {
                        a.get(i)
                            .zip(b.get(i))
                            .is_some_and(|(x, y)| x.scarpet_eq(&y))
                    })
            }
            // Maps compare as an unordered set of key/value pairs.
            (Value::Map(a), Value::Map(b)) => maps_equal(a, b),
            // Any other cross-type mix (number vs string, list vs scalar, …)
            // falls back to the base `Value.equals`: equal `getString()`s.
            (a, b) => a.to_scarpet_string() == b.to_scarpet_string(),
        }
    }

    /// Scarpet's `compareTo` total order behind `<`, `<=`, `>`, `>=`. A
    /// *fallible* total order: every pair has an ordering except a map, which
    /// the original `MapValue.compareTo` rejects. null/undef sort below
    /// everything; numbers compare numerically (NaN placed by `total_cmp`, as
    /// the original `Double.compare` does); lists order by length first, then
    /// element-wise; any other mix compares by `getString()` (base
    /// `Value.compareTo`).
    pub fn scarpet_compare(&self, other: &Value) -> Result<Ordering, VmError> {
        Ok(match (self, other) {
            // `MapValue.compareTo` throws — the sole non-total case.
            (Value::Map(_), _) | (_, Value::Map(_)) => return Err(VmError::IncomparableMap),
            // `NullValue.compareTo`: equal to other nulls, else null sorts first.
            (Value::Undef | Value::Null, Value::Undef | Value::Null) => Ordering::Equal,
            (Value::Undef | Value::Null, _) => Ordering::Less,
            (_, Value::Undef | Value::Null) => Ordering::Greater,
            // Numbers: int vs int exact; otherwise `total_cmp`, a total order
            // placing NaN and -0.0 consistently like the original `Double.compare`.
            (a, b) if a.is_number_like() && b.is_number_like() => {
                match (a.as_long(), b.as_long()) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    _ => a.as_double().total_cmp(&b.as_double()),
                }
            }
            // `ListValue.compareTo`: shorter list first, then the first differing
            // element (a nested map surfaces `IncomparableMap` through `?`).
            (Value::List(a), Value::List(b)) => {
                if a.len() != b.len() {
                    a.len().cmp(&b.len())
                } else {
                    let mut ord = Ordering::Equal;
                    for i in 0..a.len() {
                        if let (Some(x), Some(y)) = (a.get(i), b.get(i)) {
                            ord = x.scarpet_compare(&y)?;
                            if ord != Ordering::Equal {
                                break;
                            }
                        }
                    }
                    ord
                }
            }
            // Base `Value.compareTo`: compare the two string forms.
            (a, b) => a.to_scarpet_string().cmp(&b.to_scarpet_string()),
        })
    }

    /// Whether this value joins numeric comparison: a number, or a bool (the
    /// original `BooleanValue` is a `NumericValue` of 0 / 1).
    fn is_number_like(&self) -> bool {
        matches!(self, Value::Bool(_) | Value::Int(_) | Value::Double(_))
    }

    /// The integral value when held at long precision (`Int`, or a `Bool` as
    /// 0 / 1); `None` for a `Double`, which compares as a double. Mirrors the
    /// original's `longValue != null` test.
    fn as_long(&self) -> Option<i64> {
        match self {
            Value::Bool(b) => Some(*b as i64),
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// This value as a double for numeric comparison (`Bool` as 0 / 1). Only
    /// called on a number-like value (guarded by [`Value::is_number_like`]).
    pub(super) fn as_double(&self) -> f64 {
        match self {
            Value::Bool(b) => *b as i64 as f64,
            Value::Int(i) => *i as f64,
            Value::Double(d) => *d,
            _ => f64::NAN,
        }
    }
}

/// Order-independent equality for the association lists behind `Value::Map`:
/// equal length and every left pair has a `scarpet_eq` match on the right (the
/// original `MapValue.equals` compares `HashMap`s, so order is irrelevant).
/// O(n·m) and does not dedupe keys — adequate for the maps the evaluator can
/// build today (map literals are still unimplemented).
fn maps_equal(a: &[(Value, Value)], b: &[(Value, Value)]) -> bool {
    a.len() == b.len()
        && a.iter().all(|(ak, av)| {
            b.iter()
                .any(|(bk, bv)| ak.scarpet_eq(bk) && av.scarpet_eq(bv))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scarpet_eq_compares_ints_exactly() {
        assert!(Value::Int(1).scarpet_eq(&Value::Int(1)));
        assert!(!Value::Int(1).scarpet_eq(&Value::Int(2)));
    }

    /// A `long` and a `double` of the same value are equal (`1 == 1.0`).
    #[test]
    fn scarpet_eq_treats_int_and_double_as_equal() {
        assert!(Value::Int(1).scarpet_eq(&Value::Double(1.0)));
    }

    /// Rounding error within EPSILON counts as equal, like the original.
    #[test]
    fn scarpet_eq_absorbs_rounding_error() {
        assert!(Value::Double(0.1 + 0.2).scarpet_eq(&Value::Double(0.3)));
    }

    /// `NaN == NaN` is false through `equals` (NaN is only ordered by `compareTo`).
    #[test]
    fn scarpet_eq_nan_is_never_equal() {
        assert!(!Value::Double(f64::NAN).scarpet_eq(&Value::Double(f64::NAN)));
    }

    /// A bool is the number 0 / 1: `true == 1`, `false == 0`.
    #[test]
    fn scarpet_eq_bool_is_a_number() {
        assert!(Value::Bool(true).scarpet_eq(&Value::Int(1)));
        assert!(Value::Bool(false).scarpet_eq(&Value::Int(0)));
    }

    /// null and undef equal each other but not `0`.
    #[test]
    fn scarpet_eq_null_only_equals_null() {
        assert!(Value::Null.scarpet_eq(&Value::Undef));
        assert!(!Value::Null.scarpet_eq(&Value::Int(0)));
    }

    /// number vs string falls back to string equality, so `'2' == 2` holds but
    /// `'2.0' != 2` ("2.0" is not the string form of `2`).
    #[test]
    fn scarpet_eq_number_and_string_compare_as_text() {
        assert!(Value::String("2".to_owned()).scarpet_eq(&Value::Int(2)));
        assert!(!Value::String("2.0".to_owned()).scarpet_eq(&Value::Int(2)));
    }

    /// Lists compare element-wise, recursing into numeric equality (`[1] == [1.0]`).
    #[test]
    fn scarpet_eq_lists_recurse() {
        let ints = Value::list(vec![Value::Int(1)]);
        let doubles = Value::list(vec![Value::Double(1.0)]);
        assert!(ints.scarpet_eq(&doubles));

        let two = Value::list(vec![Value::Int(1), Value::Int(2)]);
        let one = Value::list(vec![Value::Int(1)]);
        assert!(!two.scarpet_eq(&one));
    }

    /// Maps are equal regardless of pair order.
    #[test]
    fn scarpet_eq_maps_ignore_order() {
        let a = Value::Map(vec![
            (Value::Int(1), Value::String("a".to_owned())),
            (Value::Int(2), Value::String("b".to_owned())),
        ]);
        let b = Value::Map(vec![
            (Value::Int(2), Value::String("b".to_owned())),
            (Value::Int(1), Value::String("a".to_owned())),
        ]);
        assert!(a.scarpet_eq(&b));
    }

    #[test]
    fn scarpet_compare_orders_numbers() {
        let lt = Value::Int(1).scarpet_compare(&Value::Int(2)).unwrap();
        assert_eq!(lt, Ordering::Less);
        let eq = Value::Int(2).scarpet_compare(&Value::Int(2)).unwrap();
        assert_eq!(eq, Ordering::Equal);
    }

    #[test]
    fn scarpet_compare_orders_strings() {
        let a = Value::String("a".to_owned());
        let b = Value::String("b".to_owned());
        assert_eq!(a.scarpet_compare(&b).unwrap(), Ordering::Less);
    }

    /// Lists order by length first: a shorter list is smaller even when its
    /// element is larger (`[2] < [1, 1, 1]`).
    #[test]
    fn scarpet_compare_lists_by_length_first() {
        let short = Value::list(vec![Value::Int(2)]);
        let long = Value::list(vec![Value::Int(1), Value::Int(1), Value::Int(1)]);
        assert_eq!(short.scarpet_compare(&long).unwrap(), Ordering::Less);
    }

    /// Same-length lists fall back to the first differing element.
    #[test]
    fn scarpet_compare_lists_tie_break_by_element() {
        let a = Value::list(vec![Value::Int(1), Value::Int(2)]);
        let b = Value::list(vec![Value::Int(1), Value::Int(3)]);
        assert_eq!(a.scarpet_compare(&b).unwrap(), Ordering::Less);
    }

    /// null sorts below everything.
    #[test]
    fn scarpet_compare_null_is_smallest() {
        assert_eq!(
            Value::Null.scarpet_compare(&Value::Int(0)).unwrap(),
            Ordering::Less
        );
        assert_eq!(
            Value::Null.scarpet_compare(&Value::Null).unwrap(),
            Ordering::Equal
        );
    }

    /// A number vs a string compares by string form, so `10 < "9"` ("10" < "9")
    /// — the opposite of the numeric `10 > 9`.
    #[test]
    fn scarpet_compare_number_and_string_by_text() {
        let n = Value::Int(10);
        let s = Value::String("9".to_owned());
        assert_eq!(n.scarpet_compare(&s).unwrap(), Ordering::Less);
    }

    /// A map cannot be relationally compared (`MapValue.compareTo` throws).
    #[test]
    fn scarpet_compare_map_is_an_error() {
        let got = Value::Map(vec![]).scarpet_compare(&Value::Int(1));
        assert!(matches!(got, Err(VmError::IncomparableMap)));
    }

    /// The map error propagates out of a list comparison too.
    #[test]
    fn scarpet_compare_nested_map_propagates_error() {
        let a = Value::list(vec![Value::Map(vec![])]);
        let b = Value::list(vec![Value::Map(vec![])]);
        assert!(matches!(
            a.scarpet_compare(&b),
            Err(VmError::IncomparableMap)
        ));
    }

    /// NaN still yields an ordering — it never errors or panics.
    #[test]
    fn scarpet_compare_nan_is_total() {
        let got = Value::Double(f64::NAN).scarpet_compare(&Value::Double(1.0));
        assert!(got.is_ok());
    }
}
