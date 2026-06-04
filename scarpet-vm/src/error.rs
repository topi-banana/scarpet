/// An error raised while evaluating Scarpet code.
#[derive(Debug)]
pub enum VmError {
    /// A [`ValueContainer`](crate::value::ValueContainer)'s lock was poisoned: a
    /// thread panicked while holding it, so the wrapped value may be left in an
    /// inconsistent state. `std::sync::Mutex::lock` reports this through a
    /// `PoisonError` whose guard borrows the lock for `'_`; we drop that detail
    /// and surface this variant instead.
    PoisonedLock,
    /// fabric-carpet's `MapValue.compareTo` throws "Cannot compare with a map
    /// value"; the relational operators (`<`, `<=`, `>`, `>=`) surface it here.
    /// Map *equality* (`==` / `!=`) is structural, so it stays fine.
    IncomparableMap,
    /// An arithmetic operator that coerces to a number (`%`, `^`, unary `-` /
    /// `+`) got a non-numeric operand. The original `NumericValue.asNumber`
    /// throws "Operand has to be of a numeric type"; strings are not parsed.
    ExpectedNumber,
    /// The modulo operator `%` was given a zero divisor (the original `mod`
    /// raises an `ArithmeticException`).
    DivisionByZero,
    /// A map literal / `m(...)` entry was a list whose length was not 2, so it
    /// is not a key/value pair (the original `MapValue.put` throws "Map
    /// constructor requires elements that have two items").
    MapEntryNotPair,
}
