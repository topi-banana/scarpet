use regex::Regex;

use super::Value;
use crate::error::VmError;

impl Value {
    /// Scarpet `:` element access (`base.get(key)` via the original
    /// `ContainerValueInterface`). Defined only on the container types: a `List`
    /// indexes by a number coerced to a `long` — an empty list is `null`,
    /// otherwise the index wraps modulo the length (the original
    /// `ListValue.normalizeIndex`, so `-1` is the last element and out-of-range
    /// indices cycle rather than fail); a `Map` looks the key up (absent →
    /// `null`). Every other type is not a container, so `:` yields `null` — note
    /// strings are NOT indexed by `:` in the original.
    pub fn scarpet_get(&self, key: &Value) -> Result<Value, VmError> {
        match self {
            Value::List(items) => {
                if items.is_empty() {
                    return Ok(Value::Null);
                }
                // Wrap out-of-range / negative indices modulo the length. The
                // normalised index is in range (the list is non-empty), so a
                // lazy backing computes the element directly without realising
                // its neighbours.
                let normalized = Self::normalize_list_index(key, items.len())?;
                Ok(items.get(normalized).unwrap_or(Value::Null))
            }
            Value::Map(entries) => {
                for (k, v) in entries {
                    if k.scarpet_eq(key) {
                        return Ok(v.clone());
                    }
                }
                Ok(Value::Null)
            }
            // Non-containers (string / number / null / bool) yield null.
            _ => Ok(Value::Null),
        }
    }

    /// Coerce `key` to a `List` index normalised modulo `len`, wrapping negative
    /// and out-of-range indices like the original `ListValue.normalizeIndex` (`-1`
    /// is the last element, indices cycle rather than fail). `len` must be
    /// non-zero — every caller handles the empty list first, where there is no
    /// slot to address. Shared by the `:` read ([`scarpet_get`]) and write
    /// ([`element_mut`], and through it [`scarpet_put`]) paths.
    ///
    /// [`scarpet_get`]: Value::scarpet_get
    /// [`element_mut`]: Value::element_mut
    /// [`scarpet_put`]: Value::scarpet_put
    fn normalize_list_index(key: &Value, len: usize) -> Result<usize, VmError> {
        let idx = match key.as_number()? {
            Value::Int(i) => i,
            // `getLong` truncates a double toward zero, like a `(long)` cast.
            Value::Double(d) => d as i64,
            // `as_number` only ever yields an `Int` or a `Double`.
            _ => return Err(VmError::ExpectedNumber),
        };
        Ok(idx.rem_euclid(len as i64) as usize)
    }

    /// Scarpet `:` element assignment — the write counterpart of [`scarpet_get`],
    /// matching the original `LContainerValue` → `container.put`. On a `List` the
    /// key is a numeric index, normalised modulo the length exactly as
    /// [`scarpet_get`] reads it, and the element is replaced in place; a lazy
    /// `range` is immutable ([`ImmutableList`]) and an empty list has no slot to
    /// write ([`IndexOutOfRange`]). On a `Map` the key is inserted, or its value
    /// updated when the key is already present. A non-container value (string /
    /// number / null) cannot be assigned into ([`NotAContainer`]).
    ///
    /// [`scarpet_get`]: Value::scarpet_get
    /// [`ImmutableList`]: VmError::ImmutableList
    /// [`IndexOutOfRange`]: VmError::IndexOutOfRange
    /// [`NotAContainer`]: VmError::NotAContainer
    pub fn scarpet_put(&mut self, key: &Value, value: Value) -> Result<(), VmError> {
        match self {
            // A list slot is exactly the writable place `element_mut` lends, so
            // reuse it rather than repeat the empty-check and index handling.
            Value::List(_) => {
                *self.element_mut(key)? = value;
                Ok(())
            }
            Value::Map(entries) => {
                for (k, v) in entries.iter_mut() {
                    if k.scarpet_eq(key) {
                        *v = value;
                        return Ok(());
                    }
                }
                entries.push((key.clone(), value));
                Ok(())
            }
            // Non-containers (string / number / null / bool) cannot be written to.
            _ => Err(VmError::NotAContainer),
        }
    }

    /// A mutable borrow of the element addressed by `key`, for walking by
    /// reference into a nested container assignment target (`x:0:1 = …`). Unlike
    /// [`scarpet_get`] it borrows the element in place rather than cloning, so a
    /// write through the result lands in the original. A list index is normalised
    /// modulo the length exactly as [`scarpet_get`] reads it; an empty list, a
    /// lazy `range` ([`ImmutableList`]), an absent map key ([`IndexOutOfRange`]),
    /// or a non-container ([`NotAContainer`]) all error.
    ///
    /// [`scarpet_get`]: Value::scarpet_get
    /// [`ImmutableList`]: VmError::ImmutableList
    /// [`IndexOutOfRange`]: VmError::IndexOutOfRange
    /// [`NotAContainer`]: VmError::NotAContainer
    pub fn element_mut(&mut self, key: &Value) -> Result<&mut Value, VmError> {
        match self {
            Value::List(items) => {
                if items.is_empty() {
                    return Err(VmError::IndexOutOfRange);
                }
                // A non-empty list always normalises in range; only a lazy backing
                // refuses to lend a slot.
                let normalized = Self::normalize_list_index(key, items.len())?;
                items.get_mut(normalized).ok_or(VmError::ImmutableList)
            }
            Value::Map(entries) => {
                for (k, v) in entries.iter_mut() {
                    if k.scarpet_eq(key) {
                        return Ok(v);
                    }
                }
                // A missing key has no element to walk into mid-path.
                Err(VmError::IndexOutOfRange)
            }
            _ => Err(VmError::NotAContainer),
        }
    }

    /// Scarpet `~` match (`left.in(right)`). On a `List` it is the index of the
    /// first element equal to `right` (else null); on a `Map` it is the key
    /// itself when present (else null); `null` / `undef` is always null. On a
    /// string or number, `right` is compiled as a regex and searched against the
    /// left's string form: no match is null, no capture groups yields the whole
    /// match, one group yields that group, and several yield a list of the group
    /// strings (the original `Value.in`). A bad pattern is `InvalidPattern`.
    pub fn scarpet_match(&self, right: &Value) -> Result<Value, VmError> {
        match self {
            Value::List(items) => {
                for i in 0..items.len() {
                    if let Some(item) = items.get(i)
                        && item.scarpet_eq(right)
                    {
                        return Ok(Value::Int(i as i64));
                    }
                }
                Ok(Value::Null)
            }
            Value::Map(entries) => {
                for (k, _) in entries {
                    if k.scarpet_eq(right) {
                        return Ok(right.clone());
                    }
                }
                Ok(Value::Null)
            }
            // `NullValue.in` is overridden to always yield null (no regex).
            Value::Undef | Value::Null => Ok(Value::Null),
            // Strings and numbers match `right` as a regex on their string forms.
            _ => {
                let re =
                    Regex::new(&right.to_scarpet_string()).map_err(|_| VmError::InvalidPattern)?;
                let haystack = self.to_scarpet_string();
                let Some(caps) = re.captures(&haystack) else {
                    return Ok(Value::Null);
                };
                let group = |i: usize| {
                    caps.get(i)
                        .map_or(Value::Null, |m| Value::String(m.as_str().to_owned()))
                };
                // `caps.len()` counts group 0 (the whole match) plus the capture
                // groups, mirroring the original `groupCount()` + 1.
                match caps.len() - 1 {
                    0 => Ok(group(0)),
                    1 => Ok(group(1)),
                    n => Ok(Value::list((1..=n).map(group).collect())),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scarpet_get_list_indexes() {
        let list = Value::list(vec![Value::Int(10), Value::Int(20), Value::Int(30)]);
        assert_eq!(list.scarpet_get(&Value::Int(0)).unwrap(), Value::Int(10));
        assert_eq!(list.scarpet_get(&Value::Int(2)).unwrap(), Value::Int(30));
    }

    /// Out-of-range and negative indices wrap modulo the length (`-1` is last).
    #[test]
    fn scarpet_get_list_wraps_index() {
        let list = Value::list(vec![Value::Int(10), Value::Int(20), Value::Int(30)]);
        assert_eq!(list.scarpet_get(&Value::Int(-1)).unwrap(), Value::Int(30));
        assert_eq!(list.scarpet_get(&Value::Int(3)).unwrap(), Value::Int(10));
        assert_eq!(list.scarpet_get(&Value::Int(-4)).unwrap(), Value::Int(30));
    }

    /// A double index truncates toward zero before wrapping.
    #[test]
    fn scarpet_get_list_truncates_double_index() {
        let list = Value::list(vec![Value::Int(10), Value::Int(20), Value::Int(30)]);
        assert_eq!(
            list.scarpet_get(&Value::Double(1.9)).unwrap(),
            Value::Int(20)
        );
    }

    #[test]
    fn scarpet_get_empty_list_is_null() {
        assert_eq!(
            Value::list(vec![]).scarpet_get(&Value::Int(0)).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn scarpet_get_list_non_numeric_key_errors() {
        let list = Value::list(vec![Value::Int(10)]);
        assert!(matches!(
            list.scarpet_get(&Value::String("x".to_owned())),
            Err(VmError::ExpectedNumber)
        ));
    }

    #[test]
    fn scarpet_get_map_looks_up_key() {
        let map = Value::Map(vec![
            (Value::Int(1), Value::String("a".to_owned())),
            (Value::String("k".to_owned()), Value::Int(99)),
        ]);
        assert_eq!(
            map.scarpet_get(&Value::Int(1)).unwrap(),
            Value::String("a".to_owned())
        );
        assert_eq!(
            map.scarpet_get(&Value::String("k".to_owned())).unwrap(),
            Value::Int(99)
        );
        // An absent key yields null.
        assert_eq!(map.scarpet_get(&Value::Int(2)).unwrap(), Value::Null);
        // Key matching uses scarpet_eq, so 1.0 finds the int key 1.
        assert_eq!(
            map.scarpet_get(&Value::Double(1.0)).unwrap(),
            Value::String("a".to_owned())
        );
    }

    /// `:` on a non-container (string / number / null) yields null.
    #[test]
    fn scarpet_get_non_container_is_null() {
        assert_eq!(
            Value::String("abc".to_owned())
                .scarpet_get(&Value::Int(1))
                .unwrap(),
            Value::Null
        );
        assert_eq!(
            Value::Int(5).scarpet_get(&Value::Int(0)).unwrap(),
            Value::Null
        );
        assert_eq!(
            Value::Null.scarpet_get(&Value::Int(0)).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn scarpet_match_list_returns_index() {
        let list = Value::list(vec![Value::Int(10), Value::Int(20), Value::Int(30)]);
        assert_eq!(list.scarpet_match(&Value::Int(20)).unwrap(), Value::Int(1));
        assert_eq!(list.scarpet_match(&Value::Int(99)).unwrap(), Value::Null);
    }

    #[test]
    fn scarpet_match_map_returns_key_when_present() {
        let map = Value::Map(vec![(Value::String("a".to_owned()), Value::Int(1))]);
        assert_eq!(
            map.scarpet_match(&Value::String("a".to_owned())).unwrap(),
            Value::String("a".to_owned())
        );
        assert_eq!(
            map.scarpet_match(&Value::String("z".to_owned())).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn scarpet_match_string_regex_groups() {
        let s = Value::String("a1b2".to_owned());
        // No capture groups: the whole match.
        assert_eq!(
            Value::String("hello".to_owned())
                .scarpet_match(&Value::String("l+".to_owned()))
                .unwrap(),
            Value::String("ll".to_owned())
        );
        // One group: that group.
        assert_eq!(
            s.scarpet_match(&Value::String("([a-z])".to_owned()))
                .unwrap(),
            Value::String("a".to_owned())
        );
        // Several groups: a list of the group strings.
        assert_eq!(
            s.scarpet_match(&Value::String("([a-z])([0-9])".to_owned()))
                .unwrap(),
            Value::list(vec![
                Value::String("a".to_owned()),
                Value::String("1".to_owned()),
            ])
        );
        // No match: null.
        assert_eq!(
            Value::String("hello".to_owned())
                .scarpet_match(&Value::String("z".to_owned()))
                .unwrap(),
            Value::Null
        );
    }

    #[test]
    fn scarpet_match_null_is_always_null() {
        assert_eq!(
            Value::Null
                .scarpet_match(&Value::String("x".to_owned()))
                .unwrap(),
            Value::Null
        );
    }

    #[test]
    fn scarpet_match_invalid_pattern_errors() {
        let s = Value::String("hello".to_owned());
        assert!(matches!(
            s.scarpet_match(&Value::String("(".to_owned())),
            Err(VmError::InvalidPattern)
        ));
    }
}
