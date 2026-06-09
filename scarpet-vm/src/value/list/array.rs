use super::ListValue;
use crate::value::Value;

/// A realised list — the backing of a list literal `[...]` / `l(...)` and the
/// result of any operation that must materialise its elements. The original
/// `ListValue`.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrayList(pub Vec<Value>);

impl ListValue for ArrayList {
    fn type_name(&self) -> &'static str {
        "list"
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn get(&self, index: usize) -> Option<Value> {
        self.0.get(index).cloned()
    }
    fn get_mut(&mut self, index: usize) -> Option<&mut Value> {
        self.0.get_mut(index)
    }
    fn pop_first(&mut self) -> Option<Value> {
        (!self.0.is_empty()).then(|| self.0.remove(0))
    }
    fn pop_last(&mut self) -> Option<Value> {
        self.0.pop()
    }
    fn clone_box(&self) -> Box<dyn ListValue> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The eager backing pops from both ends, shrinking until it reports `None`.
    #[test]
    fn array_list_pops_from_both_ends() {
        let mut a = ArrayList(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        assert_eq!(a.pop_first(), Some(Value::Int(1)));
        assert_eq!(a.pop_last(), Some(Value::Int(3)));
        assert_eq!(a.pop_first(), Some(Value::Int(2)));
        assert_eq!(a.pop_first(), None);
    }
}
