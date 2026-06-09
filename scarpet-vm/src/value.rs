mod access;
mod add;
mod arithmetic;
mod compare;
mod container;
mod list;
mod mul;
mod query;

pub use container::ValueContainer;
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
}
