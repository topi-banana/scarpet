use super::Value;
use crate::error::VmError;

impl Value {
    /// Coerce to a number, mirroring the original `NumericValue.asNumber`: an
    /// `Int` / `Double` passes through and a `Bool` becomes its `0` / `1` `Int`
    /// (the original `BooleanValue` is a numeric `0` / `1`). Anything else —
    /// including a numeric-looking string, which the original does NOT parse —
    /// is rejected with `ExpectedNumber` ("Operand has to be of a numeric type").
    pub(super) fn as_number(&self) -> Result<Value, VmError> {
        match self {
            Value::Int(_) | Value::Double(_) => Ok(self.clone()),
            Value::Bool(b) => Ok(Value::Int(*b as i64)),
            _ => Err(VmError::ExpectedNumber),
        }
    }

    /// Scarpet `%`: both operands coerce to numbers, then floored modulo (the
    /// original `NumericValue.mod`). Two integers stay integral via `floorMod`,
    /// whose result takes the divisor's sign; otherwise it is `x - floor(x/y)*y`
    /// as a double. A zero divisor raises `DivisionByZero`.
    pub fn scarpet_rem(&self, other: &Value) -> Result<Value, VmError> {
        match (self.as_number()?, other.as_number()?) {
            (Value::Int(a), Value::Int(b)) => {
                if b == 0 {
                    return Err(VmError::DivisionByZero);
                }
                // `floorMod` as `((a % b) + b) % b`: its sign follows `b`, unlike
                // Rust's `%` (sign of `a`) or `rem_euclid` (always non-negative).
                Ok(Value::Int(((a % b) + b) % b))
            }
            (a, b) => {
                let (x, y) = (a.as_double(), b.as_double());
                if y == 0.0 {
                    return Err(VmError::DivisionByZero);
                }
                Ok(Value::Double(x - (x / y).floor() * y))
            }
        }
    }

    /// Scarpet `^`: both operands coerce to a double and the result is always a
    /// `Double` (the original wraps `Math.pow` in a fresh `NumericValue`).
    pub fn scarpet_pow(&self, other: &Value) -> Result<Value, VmError> {
        let base = self.as_number()?.as_double();
        let exp = other.as_number()?.as_double();
        Ok(Value::Double(base.powf(exp)))
    }

    /// Scarpet unary `-`: coerce to a number and negate, keeping the numeric
    /// type (the original `NumericValue.opposite`). `i64::MIN` wraps, as the
    /// original `long` negation does.
    pub fn scarpet_neg(&self) -> Result<Value, VmError> {
        Ok(match self.as_number()? {
            Value::Int(i) => Value::Int(i.wrapping_neg()),
            Value::Double(d) => Value::Double(-d),
            // `as_number` only ever yields an `Int` or a `Double`.
            other => other,
        })
    }

    /// Scarpet unary `+`: coerce to a number and otherwise leave it untouched
    /// (the original maps it straight to `NumericValue.asNumber`).
    pub fn scarpet_pos(&self) -> Result<Value, VmError> {
        self.as_number()
    }

    /// Scarpet unary `!`: the negation of truthiness as a bool (the original
    /// returns `Value.FALSE` / `Value.TRUE`). Never fails.
    pub fn scarpet_not(&self) -> Value {
        Value::Bool(!self.is_true())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scarpet_pos_coerces_to_number() {
        assert_eq!(Value::Int(5).scarpet_pos().unwrap(), Value::Int(5));
        assert_eq!(
            Value::Double(2.5).scarpet_pos().unwrap(),
            Value::Double(2.5)
        );
        // A bool coerces to its 0 / 1 int.
        assert_eq!(Value::Bool(true).scarpet_pos().unwrap(), Value::Int(1));
    }

    /// A numeric-looking string is NOT parsed (unlike a literal); coercion fails.
    #[test]
    fn scarpet_pos_rejects_non_numbers() {
        assert!(matches!(
            Value::String("5".to_owned()).scarpet_pos(),
            Err(VmError::ExpectedNumber)
        ));
        assert!(matches!(
            Value::Null.scarpet_pos(),
            Err(VmError::ExpectedNumber)
        ));
        assert!(matches!(
            Value::list(vec![]).scarpet_pos(),
            Err(VmError::ExpectedNumber)
        ));
    }

    #[test]
    fn scarpet_neg_keeps_numeric_type() {
        assert_eq!(Value::Int(5).scarpet_neg().unwrap(), Value::Int(-5));
        assert_eq!(
            Value::Double(2.5).scarpet_neg().unwrap(),
            Value::Double(-2.5)
        );
        // A bool coerces first, so `-true` is `-1`.
        assert_eq!(Value::Bool(true).scarpet_neg().unwrap(), Value::Int(-1));
    }

    #[test]
    fn scarpet_neg_rejects_non_numbers() {
        assert!(matches!(
            Value::String("x".to_owned()).scarpet_neg(),
            Err(VmError::ExpectedNumber)
        ));
    }

    /// Two ints stay integral and floorMod's sign follows the divisor.
    #[test]
    fn scarpet_rem_floors_with_divisor_sign() {
        assert_eq!(
            Value::Int(5).scarpet_rem(&Value::Int(3)).unwrap(),
            Value::Int(2)
        );
        assert_eq!(
            Value::Int(-5).scarpet_rem(&Value::Int(3)).unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            Value::Int(5).scarpet_rem(&Value::Int(-3)).unwrap(),
            Value::Int(-1)
        );
    }

    /// A double operand promotes the modulo to a double.
    #[test]
    fn scarpet_rem_promotes_to_double() {
        assert_eq!(
            Value::Double(5.5).scarpet_rem(&Value::Int(2)).unwrap(),
            Value::Double(1.5)
        );
    }

    #[test]
    fn scarpet_rem_by_zero_is_an_error() {
        assert!(matches!(
            Value::Int(1).scarpet_rem(&Value::Int(0)),
            Err(VmError::DivisionByZero)
        ));
        assert!(matches!(
            Value::Double(1.0).scarpet_rem(&Value::Double(0.0)),
            Err(VmError::DivisionByZero)
        ));
    }

    /// Power always yields a double, even for integer operands.
    #[test]
    fn scarpet_pow_is_always_double() {
        assert_eq!(
            Value::Int(2).scarpet_pow(&Value::Int(10)).unwrap(),
            Value::Double(1024.0)
        );
        assert_eq!(
            Value::Int(2).scarpet_pow(&Value::Int(3)).unwrap(),
            Value::Double(8.0)
        );
    }

    #[test]
    fn scarpet_not_negates_truthiness() {
        assert_eq!(Value::Bool(true).scarpet_not(), Value::Bool(false));
        assert_eq!(Value::Int(0).scarpet_not(), Value::Bool(true));
        assert_eq!(Value::Null.scarpet_not(), Value::Bool(true));
        // A non-empty string is truthy, so `!` gives false.
        assert_eq!(
            Value::String("x".to_owned()).scarpet_not(),
            Value::Bool(false)
        );
    }
}
