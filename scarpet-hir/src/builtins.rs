//! What the analysis knows about Scarpet's built-in functions.
//!
//! Two tiers, because the upstream reference supplies two very different
//! amounts of information:
//!
//! - [`is_known_builtin`] answers "does this name exist" from
//!   `names::BUILTIN_NAMES`, generated from `docs/scarpet/Full.md`. The
//!   reference records signatures like `min(arg, ...)` but no types, so that is
//!   all it can answer — and it is exactly what the
//!   [`UnknownFunction`](crate::LintCode::UnknownFunction) lint needs. Without
//!   this list the lint would fire on the several hundred real builtins the VM
//!   has not implemented yet.
//! - [`builtin`] is a hand-written table of the arities and return types of the
//!   core language functions. Nothing upstream can generate it; a name absent
//!   here is still *known*, just untyped.
//!
//! The VM's own registry is not reusable as a schema: it stores
//! `Rc<dyn Function>` with no arity or type field, and each implementation
//! checks its own argument count internally.
//!
//! # Special forms
//!
//! Scarpet has no control-flow syntax. `if`, `for`, `while` and friends are
//! ordinary calls whose implementations receive *unevaluated* arguments, which
//! makes them special forms indistinguishable from functions at the call site.
//! [`Special`] names the ones that need code in the inference walk — either
//! because their result depends on their arguments' types, or because they bind
//! `_` and `_i` around a body.

mod names;

use crate::ty::Ty;

/// A coarse type shape, restricted to what a `const` table can hold.
///
/// Not [`Ty`] itself: `Ty` is recursive and boxed, so it cannot appear in a
/// `static` table. [`TyShape::to_ty`] widens back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TyShape {
    Any,
    Bool,
    Int,
    Num,
    Str,
    List,
    Map,
    Null,
}

impl TyShape {
    pub fn to_ty(self) -> Ty {
        match self {
            TyShape::Any => Ty::Unknown,
            TyShape::Bool => Ty::Bool,
            TyShape::Int => Ty::Int,
            TyShape::Num => Ty::Num,
            TyShape::Str => Ty::Str,
            TyShape::List => Ty::list(Ty::Unknown),
            TyShape::Map => Ty::map(Ty::Unknown, Ty::Unknown),
            TyShape::Null => Ty::Null,
        }
    }
}

/// How many arguments a builtin accepts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arity {
    Exact(u8),
    /// Inclusive on both ends.
    Range(u8, u8),
    AtLeast(u8),
}

impl Arity {
    /// Whether `count` arguments satisfy this arity.
    pub fn accepts(self, count: usize) -> bool {
        let count = u8::try_from(count).unwrap_or(u8::MAX);
        match self {
            Arity::Exact(n) => count == n,
            Arity::Range(low, high) => (low..=high).contains(&count),
            Arity::AtLeast(n) => count >= n,
        }
    }

    /// A human-readable description for a `wrong-arg-count` message.
    pub fn describe(self) -> String {
        match self {
            Arity::Exact(1) => "1 argument".to_owned(),
            Arity::Exact(n) => format!("{n} arguments"),
            Arity::Range(low, high) => format!("{low} to {high} arguments"),
            Arity::AtLeast(n) => format!("at least {n} arguments"),
        }
    }
}

/// A form that needs a hand-written arm in the inference walk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Special {
    /// `if(cond, expr, cond?, expr?, …, default?)` — the result is the join of
    /// the branches.
    If,
    /// `for(list, expr)` — binds `_` and `_i`, returns the count of successful
    /// iterations.
    For,
    /// `map(list, expr)` — binds `_` and `_i`, returns a list of the results.
    Map,
    /// `filter(list, expr)` — binds `_` and `_i`, returns a sublist.
    Filter,
    /// `first(list, expr)` — binds `_` and `_i`, returns an element or null.
    First,
    /// `all(list, expr)` — binds `_` and `_i`, returns a boolean.
    All,
    /// `reduce(list, expr, initial)` — binds `_a`, `_` and `_i`.
    Reduce,
    /// `while(cond, limit?, expr)` — binds `_`, returns the last body value or
    /// null.
    While,
    /// `loop(num, expr, exit?)` — binds `_`.
    Loop,
    /// `c_for(init, condition, increment, body)` — binds nothing.
    CFor,
    /// `call(name, args…)` — dispatch by a runtime string.
    Call,
    /// `var(expr)` — a variable addressed by a runtime name.
    Var,
    /// `outer(name)` — legal only in a signature, where lowering has already
    /// consumed it.
    Outer,
    /// `sort(list)` / `sort(values…)`.
    Sort,
}

/// How a builtin's return type is computed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RetRule {
    Fixed(TyShape),
    /// `list<shape>`.
    ListOf(TyShape),
    /// The type of argument `n`.
    SameAs(u8),
    /// The type of the last argument — `print(expr)` and `print(player, expr)`
    /// both hand back the message.
    SameAsLast,
    /// Computed by the [`Special`] handler.
    Form(Special),
    /// Known to exist, but its result is not modelled.
    Unknown,
}

/// What the analysis knows about one builtin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Signature {
    pub arity: Arity,
    pub ret: RetRule,
}

impl Signature {
    /// The special form this builtin is, if any.
    pub fn special(self) -> Option<Special> {
        match self.ret {
            RetRule::Form(special) => Some(special),
            _ => None,
        }
    }
}

const fn sig(arity: Arity, ret: RetRule) -> Signature {
    Signature { arity, ret }
}

const fn form(arity: Arity, special: Special) -> Signature {
    Signature {
        arity,
        ret: RetRule::Form(special),
    }
}

/// The typed signature of a core builtin, or `None` for the documented long
/// tail — mostly the Minecraft API, whose return types are not modelled.
///
/// A plain `match` rather than a map: it needs no initialisation, works in
/// `wasm` without a lazy static, and is what code generation would emit anyway.
pub fn builtin(name: &str) -> Option<Signature> {
    use Arity::*;
    use RetRule::*;
    use TyShape as S;

    let signature = match name {
        // --- values and conversion ---
        "type" => sig(Exact(1), Fixed(S::Str)),
        "str" => sig(AtLeast(1), Fixed(S::Str)),
        "bool" => sig(Exact(1), Fixed(S::Bool)),
        // `number(expr)` yields null when the argument does not convert, which
        // `RetRule` cannot spell; leaving it unknown beats claiming `number`.
        "number" => sig(Exact(1), Unknown),
        "length" => sig(Exact(1), Fixed(S::Int)),
        "copy" => sig(Exact(1), SameAs(0)),
        "print" => sig(Range(1, 2), SameAsLast),

        // --- arithmetic ---
        "abs" | "round" | "floor" | "ceil" | "sqrt" | "relu" | "fact" => {
            sig(Exact(1), Fixed(S::Num))
        }
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "ln" | "log" | "log10" => {
            sig(Exact(1), Fixed(S::Num))
        }
        "atan2" => sig(Exact(2), Fixed(S::Num)),
        "min" | "max" => sig(AtLeast(1), Unknown),
        "rand" => sig(Range(1, 2), Unknown),

        // --- containers ---
        "range" => sig(Range(1, 3), ListOf(S::Num)),
        "keys" | "values" | "pairs" => sig(Exact(1), ListOf(S::Any)),
        "sort" => form(AtLeast(1), Special::Sort),
        "sort_key" => sig(Exact(2), SameAs(0)),
        "slice" => sig(Range(2, 3), SameAs(0)),
        "put" => sig(Range(2, 4), Unknown),
        "get" | "delete" => sig(AtLeast(1), Unknown),
        "has" => sig(AtLeast(1), Fixed(S::Bool)),

        // --- strings ---
        "split" => sig(Range(1, 2), ListOf(S::Str)),
        "join" => sig(AtLeast(2), Fixed(S::Str)),
        "lower" | "upper" | "title" => sig(Exact(1), Fixed(S::Str)),
        "replace" | "replace_first" => sig(Range(2, 3), Fixed(S::Str)),
        "format" => sig(AtLeast(1), Unknown),

        // --- control flow, all of it ordinary calls ---
        "if" => form(AtLeast(2), Special::If),
        "for" => form(Exact(2), Special::For),
        "map" => form(Exact(2), Special::Map),
        "filter" => form(Exact(2), Special::Filter),
        "first" => form(Exact(2), Special::First),
        "all" => form(Exact(2), Special::All),
        "reduce" => form(Exact(3), Special::Reduce),
        "while" => form(Range(2, 3), Special::While),
        "loop" => form(Range(2, 3), Special::Loop),
        "c_for" => form(Exact(4), Special::CFor),
        "call" => form(AtLeast(1), Special::Call),
        "var" => form(Exact(1), Special::Var),
        "outer" => form(Exact(1), Special::Outer),

        _ => return None,
    };
    Some(signature)
}

/// Whether `name` is any function the upstream reference documents.
///
/// Broader than [`builtin`]: it covers the whole Minecraft API and the
/// `__on_*` event hooks, so the `unknown-function` lint stays quiet about code
/// this analysis simply has no types for.
pub fn is_known_builtin(name: &str) -> bool {
    names::BUILTIN_NAMES.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generated_list_is_sorted_and_searchable() {
        assert!(names::BUILTIN_NAMES.windows(2).all(|w| w[0] < w[1]));
        for name in [
            "sort",
            "map",
            "filter",
            "reduce",
            "print",
            "if",
            "for",
            "system_info",
        ] {
            assert!(is_known_builtin(name), "{name} should be documented");
        }
        assert!(!is_known_builtin("definitely_not_a_builtin"));
    }

    /// Anything with a hand-written signature must also be in the generated
    /// list, or `unknown-function` would contradict `wrong-arg-count`.
    #[test]
    fn every_typed_builtin_is_also_a_known_name() {
        for name in [
            "type",
            "str",
            "bool",
            "number",
            "length",
            "copy",
            "print",
            "abs",
            "round",
            "floor",
            "ceil",
            "sqrt",
            "relu",
            "fact",
            "min",
            "max",
            "rand",
            "range",
            "keys",
            "values",
            "pairs",
            "sort",
            "sort_key",
            "slice",
            "put",
            "get",
            "delete",
            "has",
            "split",
            "join",
            "lower",
            "upper",
            "title",
            "replace",
            "replace_first",
            "format",
            "if",
            "for",
            "map",
            "filter",
            "first",
            "all",
            "reduce",
            "while",
            "loop",
            "c_for",
            "call",
            "var",
            "outer",
            "sin",
            "cos",
            "tan",
            "asin",
            "acos",
            "atan",
            "atan2",
            "ln",
            "log",
            "log10",
        ] {
            assert!(builtin(name).is_some(), "{name} should be typed");
            assert!(is_known_builtin(name), "{name} should be documented");
        }
    }

    #[test]
    fn arities_accept_what_they_should() {
        assert!(Arity::Exact(2).accepts(2));
        assert!(!Arity::Exact(2).accepts(3));
        assert!(Arity::Range(1, 3).accepts(1));
        assert!(Arity::Range(1, 3).accepts(3));
        assert!(!Arity::Range(1, 3).accepts(4));
        assert!(Arity::AtLeast(1).accepts(99));
        assert!(!Arity::AtLeast(1).accepts(0));
        assert_eq!(Arity::Exact(1).describe(), "1 argument");
        assert_eq!(Arity::Range(2, 3).describe(), "2 to 3 arguments");
    }

    #[test]
    fn special_forms_are_reachable_through_their_signature() {
        assert_eq!(
            builtin("if").and_then(Signature::special),
            Some(Special::If)
        );
        assert_eq!(builtin("str").and_then(Signature::special), None);
    }
}
