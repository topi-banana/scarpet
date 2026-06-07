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
    /// The right side of `~` against a string / number was not a valid regular
    /// expression (the original `Value.in` throws "Incorrect matching pattern").
    InvalidPattern,
    /// A call named a function that is neither a builtin nor user-defined.
    UnknownFunction,
    /// A function was called with the wrong number of arguments for its
    /// parameter list.
    WrongArgCount,
    /// A function definition used a parameter that is not a plain variable
    /// (literal patterns, `...rest`, `outer(x)` are not modelled yet).
    UnsupportedParameter,
    /// The right-hand side of a destructuring assignment (`[a, b] = …`) was not a
    /// list, so it cannot be unpacked.
    ExpectedList,
    /// A destructuring assignment had more values than its pattern could bind
    /// (the original `=` raises "Too many values to unpack").
    TooManyValuesToUnpack,
    /// A destructuring assignment had fewer values than its pattern required (the
    /// original `=` raises "Too few values to unpack").
    TooFewValuesToUnpack,
    /// An assignment target that cannot be assigned to: a literal or computed
    /// element in a destructuring pattern (`[a, 1] = …`), where the original
    /// `assertAssignable` throws.
    NotAssignable,
    /// An element assignment (`x:k = …`) whose base is not a container — a
    /// string / number / null has nothing to write into.
    NotAContainer,
    /// An element assignment into a lazy, immutable list (`range(3):0 = …`),
    /// which has no stored elements to overwrite.
    ImmutableList,
    /// An element assignment into an empty list (`[]:0 = …`), which has no slot
    /// to write — `:` reading wraps modulo the length, but an empty list cannot.
    IndexOutOfRange,
    /// Writing a `print` line to the VM's configured standard output failed: the
    /// [`Write`](std::io::Write)r in [`GlobalState`](crate::GlobalState) returned
    /// an I/O error. The process's stdout can fail (a broken pipe); the
    /// playground's in-memory capture buffer never does.
    StdoutWrite,
}
