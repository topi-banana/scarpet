use super::ListValue;
use crate::error::VmError;
use crate::value::Value;

/// A lazily generated arithmetic progression — the backing of `range(...)`.
/// Reports its `type()` as "iterator" (the original `LazyListValue` produced by
/// `range`).
///
/// Only `from`, `step`, and the element count are kept; each element is computed
/// as `from + i*step` rather than by accumulation, so a floating-point range
/// neither drifts nor stores a `Vec`. An all-integer range ([`Int`](RangeList::Int))
/// stays at `i64` precision — matching how the original keeps whole `range`
/// values as longs — while any fractional bound promotes the whole range to
/// doubles ([`Float`](RangeList::Float)).
#[derive(Clone, Debug, PartialEq)]
pub enum RangeList {
    Int { from: i64, step: i64, len: usize },
    Float { from: f64, step: f64, len: usize },
}

impl RangeList {
    /// Build a range from its `from` / `to` / `step` bounds, each coerced to a
    /// number the way the original `range` coerces with `NumericValue.asNumber`
    /// (a bool counts as its `0` / `1`; a non-number is [`VmError::ExpectedNumber`]).
    /// The range is integral only when all three bounds are; one fractional bound
    /// makes it a floating range.
    ///
    /// A zero or non-finite `step` yields an empty range. The original loops on
    /// `current < to` and so spins forever on a zero step; an empty range is the
    /// safe, finite reading of "no element is reachable".
    pub fn new(from: &Value, to: &Value, step: &Value) -> Result<RangeList, VmError> {
        match (as_integral(from), as_integral(to), as_integral(step)) {
            (Some(from), Some(to), Some(step)) => Ok(RangeList::Int {
                from,
                step,
                len: int_len(from, to, step),
            }),
            // Any non-integral bound promotes the whole range to doubles; a
            // non-numeric bound surfaces as `ExpectedNumber` through `as_double`.
            _ => {
                let (from, to, step) = (as_double(from)?, as_double(to)?, as_double(step)?);
                Ok(RangeList::Float {
                    from,
                    step,
                    len: float_len(from, to, step),
                })
            }
        }
    }
}

impl ListValue for RangeList {
    fn type_name(&self) -> &'static str {
        "iterator"
    }
    fn len(&self) -> usize {
        match *self {
            RangeList::Int { len, .. } | RangeList::Float { len, .. } => len,
        }
    }
    fn get(&self, index: usize) -> Option<Value> {
        match *self {
            RangeList::Int { from, step, len } if index < len => {
                Some(Value::Int(from.wrapping_add(index as i64 * step)))
            }
            RangeList::Float { from, step, len } if index < len => {
                Some(Value::Double(from + index as f64 * step))
            }
            _ => None,
        }
    }
    fn get_mut(&mut self, _index: usize) -> Option<&mut Value> {
        // A lazy arithmetic progression has no stored elements to borrow.
        None
    }
    fn pop_first(&mut self) -> Option<Value> {
        match self {
            RangeList::Int { from, step, len } => {
                if *len == 0 {
                    return None;
                }
                let value = *from;
                *from = from.wrapping_add(*step);
                *len -= 1;
                Some(Value::Int(value))
            }
            RangeList::Float { from, step, len } => {
                if *len == 0 {
                    return None;
                }
                let value = *from;
                *from += *step;
                *len -= 1;
                Some(Value::Double(value))
            }
        }
    }
    fn pop_last(&mut self) -> Option<Value> {
        match self {
            // After decrementing, `len` is the index of the element we drop, so
            // `from + len*step` is its value — no `from` adjustment needed.
            RangeList::Int { from, step, len } => {
                if *len == 0 {
                    return None;
                }
                *len -= 1;
                Some(Value::Int(from.wrapping_add(*len as i64 * *step)))
            }
            RangeList::Float { from, step, len } => {
                if *len == 0 {
                    return None;
                }
                *len -= 1;
                Some(Value::Double(*from + *len as f64 * *step))
            }
        }
    }
    fn clone_box(&self) -> Box<dyn ListValue> {
        Box::new(self.clone())
    }
}

/// The integral value of a number-like value (an `Int`, or a `Bool` as its
/// `0` / `1`), or `None` for a `Double` or a non-number — the test for "can this
/// bound stay a long".
fn as_integral(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        Value::Bool(b) => Some(*b as i64),
        _ => None,
    }
}

/// A bound as a double, coercing a number-like value (the original
/// `NumericValue.asNumber().getDouble()`); a non-number is rejected.
fn as_double(v: &Value) -> Result<f64, VmError> {
    match v {
        Value::Int(i) => Ok(*i as f64),
        Value::Bool(b) => Ok(*b as i64 as f64),
        Value::Double(d) => Ok(*d),
        _ => Err(VmError::ExpectedNumber),
    }
}

/// The element count of an integer range: `ceil((to - from) / step)`, or 0 when
/// `step` is 0 or points away from `to`. Computed with `i64` so a wide range
/// counts exactly rather than through a lossy `f64`.
fn int_len(from: i64, to: i64, step: i64) -> usize {
    let span = to - from;
    // No element is reachable when the step is zero or heads away from `to`.
    if step == 0 || (step > 0) != (span > 0) {
        return 0;
    }
    // `span` and `step` now share a sign; normalise both positive so the ceiling
    // division `(num + den - 1) / den` is the count of steps that stay short of
    // `to`.
    let (num, den) = if step > 0 {
        (span, step)
    } else {
        (-span, -step)
    };
    ((num + den - 1) / den) as usize
}

/// The element count of a floating range: `ceil((to - from) / step)`, or 0 when
/// the ratio is non-positive or non-finite (a zero step, or bounds that produce
/// NaN / infinity).
fn float_len(from: f64, to: f64, step: f64) -> usize {
    let ratio = (to - from) / step;
    if ratio.is_finite() && ratio > 0.0 {
        ratio.ceil() as usize
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(from: i64, to: i64, step: i64) -> RangeList {
        RangeList::new(&Value::Int(from), &Value::Int(to), &Value::Int(step)).unwrap()
    }

    /// Collect a list's elements by draining a clone through `into_iter`
    /// (`pop_first`), leaving the original intact.
    fn elements(list: &impl ListValue) -> Vec<Value> {
        list.clone_box().into_iter().collect()
    }

    #[test]
    fn int_range_counts_and_yields_ascending() {
        let r = range(0, 5, 1);
        assert_eq!(r.len(), 5);
        assert_eq!(
            elements(&r),
            vec![
                Value::Int(0),
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
            ]
        );
    }

    #[test]
    fn int_range_with_step_skips() {
        assert_eq!(
            elements(&range(0, 10, 2)),
            vec![
                Value::Int(0),
                Value::Int(2),
                Value::Int(4),
                Value::Int(6),
                Value::Int(8),
            ]
        );
    }

    #[test]
    fn negative_step_descends() {
        assert_eq!(
            elements(&range(5, 0, -1)),
            vec![
                Value::Int(5),
                Value::Int(4),
                Value::Int(3),
                Value::Int(2),
                Value::Int(1),
            ]
        );
    }

    /// A step pointing away from `to` (or a zero step) reaches nothing.
    #[test]
    fn empty_ranges() {
        assert_eq!(range(5, 0, 1).len(), 0);
        assert_eq!(range(0, 5, -1).len(), 0);
        assert_eq!(range(0, 5, 0).len(), 0);
        assert_eq!(range(0, 0, 1).len(), 0);
    }

    /// `get` answers in range and returns `None` past the end (no wrapping — the
    /// `Value::scarpet_get` caller normalises first).
    #[test]
    fn get_indexes_without_wrapping() {
        let r = range(0, 5, 1);
        assert_eq!(r.get(0), Some(Value::Int(0)));
        assert_eq!(r.get(4), Some(Value::Int(4)));
        assert_eq!(r.get(5), None);
    }

    /// A fractional bound promotes the whole range to doubles, computed as
    /// `from + i*step` so it does not drift.
    #[test]
    fn fractional_bound_makes_a_double_range() {
        let r = RangeList::new(&Value::Int(0), &Value::Int(1), &Value::Double(0.25)).unwrap();
        assert_eq!(r.type_name(), "iterator");
        assert_eq!(
            elements(&r),
            vec![
                Value::Double(0.0),
                Value::Double(0.25),
                Value::Double(0.5),
                Value::Double(0.75),
            ]
        );
    }

    #[test]
    fn non_numeric_bound_is_rejected() {
        let got = RangeList::new(
            &Value::Int(0),
            &Value::String("x".to_owned()),
            &Value::Int(1),
        );
        assert!(matches!(got, Err(VmError::ExpectedNumber)));
    }

    /// `pop_first` / `pop_last` drain a range from both ends, shrinking it until
    /// both report `None`.
    #[test]
    fn range_pops_from_both_ends() {
        let mut r = range(0, 5, 1);
        assert_eq!(r.pop_first(), Some(Value::Int(0)));
        assert_eq!(r.pop_last(), Some(Value::Int(4)));
        assert_eq!(r.len(), 3);
        assert_eq!(r.pop_first(), Some(Value::Int(1)));
        assert_eq!(r.pop_last(), Some(Value::Int(3)));
        assert_eq!(r.pop_first(), Some(Value::Int(2)));
        assert_eq!(r.pop_first(), None);
        assert_eq!(r.pop_last(), None);
    }
}
