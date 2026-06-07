use std::{
    cmp::Ordering,
    ops::{Add, AddAssign, Deref, DerefMut, Div, DivAssign, Mul, MulAssign, Sub, SubAssign},
    sync::{Arc, Mutex},
};

use regex::Regex;

use crate::error::VmError;

mod list;

pub use list::{ListValue, RangeList};

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
/// A lazy `iterator` such as `range` is not a separate variant: it is a
/// [`List`](Value::List) over a lazy [`ListValue`] backing, since it behaves as
/// a list and only `type()` ("iterator") tells the two apart — matching the
/// original `AbstractListValue` hierarchy.
///
/// Types that exist in the original but are not carried here yet, because the VM
/// lacks the machinery for them:
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
    /// A list. Either a realised list `[...]` / `l(...)` (the original
    /// `ListValue`) or a lazy one such as `range` (`LazyListValue`); both behave
    /// as a list and only `type()` tells them apart. See [`ListValue`] for the
    /// backing, and [`Value::list`] for the realised constructor.
    List(Box<dyn ListValue>),
    /// A map `{...}` / `m(...)` (`MapValue`). The original is an unordered hash
    /// map, but since keys may be arbitrary values we keep it here simply as a
    /// sequence of key/value pairs.
    Map(Vec<(Value, Value)>),
}

impl Value {
    /// A realised [`List`](Value::List) from its elements — the everyday list
    /// constructor, wrapping the eager [`ArrayList`](list::ArrayList) backing. A
    /// lazy list such as `range` wraps its own backing instead; see [`RangeList`].
    pub fn list(items: Vec<Value>) -> Value {
        Value::List(Box::new(list::ArrayList(items)))
    }

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
            // A realised list reports "list", a lazy one (a `range`) "iterator".
            Value::List(items) => items.type_name(),
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
                let inner = (0..items.len())
                    .filter_map(|i| items.get(i))
                    .map(|item| item.to_scarpet_string())
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
    fn as_double(&self) -> f64 {
        match self {
            Value::Bool(b) => *b as i64 as f64,
            Value::Int(i) => *i as f64,
            Value::Double(d) => *d,
            _ => f64::NAN,
        }
    }

    /// Coerce to a number, mirroring the original `NumericValue.asNumber`: an
    /// `Int` / `Double` passes through and a `Bool` becomes its `0` / `1` `Int`
    /// (the original `BooleanValue` is a numeric `0` / `1`). Anything else —
    /// including a numeric-looking string, which the original does NOT parse —
    /// is rejected with `ExpectedNumber` ("Operand has to be of a numeric type").
    fn as_number(&self) -> Result<Value, VmError> {
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
                let idx = match key.as_number()? {
                    Value::Int(i) => i,
                    // `getLong` truncates a double toward zero, like a `(long)` cast.
                    Value::Double(d) => d as i64,
                    // `as_number` only ever yields an `Int` or a `Double`.
                    _ => return Ok(Value::Null),
                };
                // Wrap out-of-range / negative indices modulo the length. The
                // normalised index is in range (the list is non-empty), so a
                // lazy backing computes the element directly without realising
                // its neighbours.
                let normalized = idx.rem_euclid(items.len() as i64) as usize;
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
            Value::List(items) => {
                if items.is_empty() {
                    return Err(VmError::IndexOutOfRange);
                }
                let idx = match key.as_number()? {
                    Value::Int(i) => i,
                    // `getLong` truncates a double toward zero, like a `(long)` cast.
                    Value::Double(d) => d as i64,
                    // `as_number` only ever yields an `Int` or a `Double`.
                    _ => return Err(VmError::ExpectedNumber),
                };
                // The list is non-empty, so the normalised index is always in
                // range; only a lazy backing can refuse the write.
                let normalized = idx.rem_euclid(items.len() as i64) as usize;
                if items.set(normalized, value) {
                    Ok(())
                } else {
                    Err(VmError::ImmutableList)
                }
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

    /// A list renders with `getString` semantics: `[1, 2]`, no quotes on elements.
    #[test]
    fn to_scarpet_string_renders_list_without_quotes() {
        let list = Value::list(vec![Value::Int(1), Value::String("a".to_owned())]);

        assert_eq!(list.to_scarpet_string(), "[1, a]");
    }

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
    fn value_container_get_delegates() {
        let list = ValueContainer::new(Value::list(vec![Value::Int(7), Value::Int(8)]));
        let got = list.scarpet_get(&ValueContainer::int(1)).unwrap();
        assert_eq!(*got.lock().unwrap(), Value::Int(8));
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

    #[test]
    fn value_container_match_delegates() {
        let list = ValueContainer::new(Value::list(vec![Value::Int(5), Value::Int(6)]));
        let got = list.scarpet_match(&ValueContainer::int(6)).unwrap();
        assert_eq!(*got.lock().unwrap(), Value::Int(1));
    }
}
