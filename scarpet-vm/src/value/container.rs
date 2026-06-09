use std::cmp::Ordering;
use std::ops::{Add, Deref, DerefMut, Div, Mul, Sub};
use std::sync::{Arc, Mutex};

use super::Value;
use crate::error::VmError;

/// A shared, thread-safe slot holding a [`Value`] (the original keeps every value
/// behind an `Arc<Mutex<…>>` so a `task` thread can be handed one). The variant
/// records whether the value carries an `...` unpack marker: a [`Single`] is an
/// ordinary value, while an [`Expand`] is one produced by the unary `...`
/// operator, to be spread into its elements when collected into an argument list
/// or a list / map literal.
///
/// [`Single`]: ValueContainer::Single
/// [`Expand`]: ValueContainer::Expand
#[derive(Clone, Debug)]
pub enum ValueContainer {
    Single(Arc<Mutex<Value>>),
    Expand(Arc<Mutex<Value>>),
}

/// The lock guard returned by [`ValueContainer::lock`], carrying the same
/// `Single` / `Expand` tag as the container it came from. It derefs to the
/// guarded [`Value`] (like the [`MutexGuard`](std::sync::MutexGuard) it wraps),
/// so the unpack tag stays available without getting in the way of reading or
/// mutating the value through it.
#[derive(Debug)]
pub enum ValueContainerGuard<'lock> {
    Single(std::sync::MutexGuard<'lock, Value>),
    Expand(std::sync::MutexGuard<'lock, Value>),
}

impl Deref for ValueContainerGuard<'_> {
    type Target = Value;
    fn deref(&self) -> &Value {
        match self {
            ValueContainerGuard::Single(guard) | ValueContainerGuard::Expand(guard) => guard,
        }
    }
}

impl DerefMut for ValueContainerGuard<'_> {
    fn deref_mut(&mut self) -> &mut Value {
        match self {
            ValueContainerGuard::Single(guard) | ValueContainerGuard::Expand(guard) => guard,
        }
    }
}

impl ValueContainer {
    pub fn new(value: Value) -> Self {
        Self::Single(Arc::new(Mutex::new(value)))
    }
    pub fn expand(value: Value) -> Self {
        Self::Expand(Arc::new(Mutex::new(value)))
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
    pub fn lock(&self) -> Result<ValueContainerGuard<'_>, VmError> {
        Ok(match self {
            Self::Single(v) => {
                ValueContainerGuard::Single(v.lock().map_err(|_| VmError::PoisonedLock)?)
            }
            Self::Expand(v) => {
                ValueContainerGuard::Expand(v.lock().map_err(|_| VmError::PoisonedLock)?)
            }
        })
    }
    /// `==`: whether the two values are Scarpet-equal ([`Value::scarpet_eq`]).
    /// Clones each side out of its lock first, so `x == x` (the same container
    /// on both sides) cannot deadlock — matching the arithmetic `Add` impl.
    pub fn scarpet_eq(&self, rhs: &ValueContainer) -> Result<bool, VmError> {
        let lhs = self.lock()?.clone();
        let rhs = rhs.lock()?.clone();
        Ok(lhs.scarpet_eq(&rhs))
    }
    /// The `compareTo` ordering of the two values ([`Value::scarpet_compare`]),
    /// from which `<`, `<=`, `>`, `>=` read a boolean. `Err` only for a map.
    pub fn scarpet_compare(&self, rhs: &ValueContainer) -> Result<Ordering, VmError> {
        let lhs = self.lock()?.clone();
        let rhs = rhs.lock()?.clone();
        lhs.scarpet_compare(&rhs)
    }
    /// `%` ([`Value::scarpet_rem`]). Clones each side out of its lock first, so
    /// `x % x` cannot deadlock.
    pub fn scarpet_rem(&self, rhs: &ValueContainer) -> Result<ValueContainer, VmError> {
        let lhs = self.lock()?.clone();
        let rhs = rhs.lock()?.clone();
        Ok(ValueContainer::new(lhs.scarpet_rem(&rhs)?))
    }
    /// `^` ([`Value::scarpet_pow`]).
    pub fn scarpet_pow(&self, rhs: &ValueContainer) -> Result<ValueContainer, VmError> {
        let lhs = self.lock()?.clone();
        let rhs = rhs.lock()?.clone();
        Ok(ValueContainer::new(lhs.scarpet_pow(&rhs)?))
    }
    /// Unary `-` ([`Value::scarpet_neg`]).
    pub fn scarpet_neg(&self) -> Result<ValueContainer, VmError> {
        Ok(ValueContainer::new(self.lock()?.scarpet_neg()?))
    }
    /// Unary `+` ([`Value::scarpet_pos`]).
    pub fn scarpet_pos(&self) -> Result<ValueContainer, VmError> {
        Ok(ValueContainer::new(self.lock()?.scarpet_pos()?))
    }
    /// Unary `!` ([`Value::scarpet_not`]).
    pub fn scarpet_not(&self) -> Result<ValueContainer, VmError> {
        Ok(ValueContainer::new(self.lock()?.scarpet_not()))
    }
    /// `:` element access ([`Value::scarpet_get`]). Clones each side out of its
    /// lock first, so `x:x` cannot deadlock.
    pub fn scarpet_get(&self, key: &ValueContainer) -> Result<ValueContainer, VmError> {
        let base = self.lock()?.clone();
        let key = key.lock()?.clone();
        Ok(ValueContainer::new(base.scarpet_get(&key)?))
    }
    /// `~` match ([`Value::scarpet_match`]). Clones each side out of its lock
    /// first, so `x ~ x` cannot deadlock.
    pub fn scarpet_match(&self, right: &ValueContainer) -> Result<ValueContainer, VmError> {
        let left = self.lock()?.clone();
        let right = right.lock()?.clone();
        Ok(ValueContainer::new(left.scarpet_match(&right)?))
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

    /// The `ValueContainer` wrappers lock, clone, and delegate to `Value`.
    #[test]
    fn value_container_comparisons_delegate() {
        assert!(
            ValueContainer::int(1)
                .scarpet_eq(&ValueContainer::int(1))
                .unwrap()
        );
        let ord = ValueContainer::int(1)
            .scarpet_compare(&ValueContainer::int(2))
            .unwrap();
        assert_eq!(ord, Ordering::Less);
    }

    /// The arithmetic / unary `ValueContainer` wrappers delegate to `Value`.
    #[test]
    fn value_container_arithmetic_delegates() {
        let rem = ValueContainer::int(5)
            .scarpet_rem(&ValueContainer::int(3))
            .unwrap();
        assert_eq!(*rem.lock().unwrap(), Value::Int(2));
        let neg = ValueContainer::int(2).scarpet_neg().unwrap();
        assert_eq!(*neg.lock().unwrap(), Value::Int(-2));
    }

    #[test]
    fn value_container_get_delegates() {
        let list = ValueContainer::new(Value::list(vec![Value::Int(7), Value::Int(8)]));
        let got = list.scarpet_get(&ValueContainer::int(1)).unwrap();
        assert_eq!(*got.lock().unwrap(), Value::Int(8));
    }

    #[test]
    fn value_container_match_delegates() {
        let list = ValueContainer::new(Value::list(vec![Value::Int(5), Value::Int(6)]));
        let got = list.scarpet_match(&ValueContainer::int(6)).unwrap();
        assert_eq!(*got.lock().unwrap(), Value::Int(1));
    }
}
