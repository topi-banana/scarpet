use std::fmt::Debug;

use crate::value::Value;

mod array;
mod range;

pub use array::ArrayList;
pub use range::RangeList;

/// The backing of a [`Value::List`](super::Value::List): either a fully realised
/// list ([`ArrayList`]) or a lazily generated one such as a `range`
/// ([`RangeList`]).
///
/// Mirrors fabric-carpet's `AbstractListValue` hierarchy, whose `ListValue` and
/// `LazyListValue` both *present* as a list — they index, iterate, and compare
/// the same way — yet report different `type()` names ("list" vs "iterator").
/// Keeping the variant a trait object lets a million-element `range` stay a few
/// numbers in memory until something actually walks it, while ordinary list
/// literals keep their eager `Vec` storage.
///
/// A trait object cannot derive `Clone` / `PartialEq`, so [`Value`] leans on the
/// hand-written `impl`s for `Box<dyn ListValue>` at the bottom of this module to
/// keep its own `#[derive(Clone, PartialEq)]` working; `Debug` comes for free
/// from the supertrait bound.
///
/// `Send + Sync` are required so a `Value` stays `Send + Sync` — every value
/// lives behind a [`ValueContainer`](super::ValueContainer)'s `Arc<Mutex<…>>`,
/// the shared, thread-safe slot the original uses to pass values to `task`
/// threads. Both backings here ([`ArrayList`], [`RangeList`]) already are.
pub trait ListValue: Debug + Send + Sync {
    /// The name `type()` reports: "list" for a realised list, "iterator" for a
    /// lazy one (the original `getTypeString`).
    fn type_name(&self) -> &'static str;

    /// The number of elements.
    fn len(&self) -> usize;

    /// Whether the list has no elements. Drives list truthiness and the empty
    /// short-circuit in `:` indexing.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The element at a 0-based, already in-range index, or `None` past the end.
    /// Callers normalise (wrap) the index against [`len`](ListValue::len) first,
    /// so a [`RangeList`] can answer in O(1) without realising its neighbours.
    fn get(&self, index: usize) -> Option<Value>;

    /// A mutable borrow of the element at an already in-range `index`, or `None`
    /// when there is no stored slot to lend — a lazy [`RangeList`] computes its
    /// elements on demand, so it always returns `None`. The basis for in-place
    /// element writes and for walking by reference into a nested container
    /// assignment target. Callers normalise the index against
    /// [`len`](ListValue::len) first, exactly as for [`get`](ListValue::get).
    fn get_mut(&mut self, index: usize) -> Option<&mut Value>;

    /// Remove and return the first element, or `None` when empty. The lazy,
    /// consuming primitive in place of an `iter()`: walking a list means draining
    /// it from one end, exactly as the original `LazyListValue` is pulled through
    /// its `Iterator`. [`Drain`] / [`Box<dyn ListValue>::into_iter`] layer a
    /// standard iterator on top.
    fn pop_first(&mut self) -> Option<Value>;

    /// Remove and return the last element, or `None` when empty — `pop_first`
    /// from the back, feeding [`Drain`]'s `DoubleEndedIterator`.
    fn pop_last(&mut self) -> Option<Value>;

    /// Clone into a fresh box. `Box<dyn ListValue>` cannot derive `Clone`, so the
    /// blanket `Clone` impl below routes through here.
    fn clone_box(&self) -> Box<dyn ListValue>;
}

// `Box<dyn ListValue>` is reachable through `Value::List`, so it needs the same
// `Clone` / `PartialEq` the rest of `Value` derives. `Box` is `#[fundamental]`,
// so these impls fall under the local trait `ListValue` for coherence.

impl Clone for Box<dyn ListValue> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl PartialEq for Box<dyn ListValue> {
    /// Structural element-wise equality (the equality the derived `PartialEq` on
    /// `Value` and the test suite expect — *not* Scarpet's `==`, which lives in
    /// [`Value::scarpet_eq`](super::Value::scarpet_eq)). A list and an
    /// equal-length range with the same elements compare equal regardless of
    /// which backing holds them. Walks by index rather than draining, so neither
    /// side is consumed.
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && (0..self.len()).all(|i| self.get(i) == other.get(i))
    }
}

/// A consuming, double-ended iterator over a list, draining it through
/// [`pop_first`](ListValue::pop_first) (front) and [`pop_last`](ListValue::pop_last)
/// (back). Build one with `list.into_iter()`; clone the backing first if the list
/// must survive the walk.
pub struct Drain(Box<dyn ListValue>);

impl Iterator for Drain {
    type Item = Value;
    fn next(&mut self) -> Option<Value> {
        self.0.pop_first()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.0.len();
        (remaining, Some(remaining))
    }
}

impl DoubleEndedIterator for Drain {
    fn next_back(&mut self) -> Option<Value> {
        self.0.pop_last()
    }
}

impl ExactSizeIterator for Drain {}

impl IntoIterator for Box<dyn ListValue> {
    type Item = Value;
    type IntoIter = Drain;
    fn into_iter(self) -> Drain {
        Drain(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A range and the equal list compare structurally equal across backings.
    #[test]
    fn range_equals_equivalent_array_list() {
        let r: Box<dyn ListValue> =
            Box::new(RangeList::new(&Value::Int(0), &Value::Int(3), &Value::Int(1)).unwrap());
        let a: Box<dyn ListValue> =
            Box::new(ArrayList(vec![Value::Int(0), Value::Int(1), Value::Int(2)]));
        assert!(r == a);
    }

    /// `into_iter` yields a `DoubleEndedIterator`, so a range can be consumed
    /// from both ends and reports an exact remaining count along the way.
    #[test]
    fn drain_is_double_ended() {
        let list: Box<dyn ListValue> =
            Box::new(RangeList::new(&Value::Int(0), &Value::Int(4), &Value::Int(1)).unwrap());
        let mut drain = list.into_iter();
        assert_eq!(drain.len(), 4);
        assert_eq!(drain.next(), Some(Value::Int(0)));
        assert_eq!(drain.next_back(), Some(Value::Int(3)));
        assert_eq!(drain.len(), 2);
        assert_eq!(drain.next_back(), Some(Value::Int(2)));
        assert_eq!(drain.next(), Some(Value::Int(1)));
        assert_eq!(drain.next(), None);
    }
}
