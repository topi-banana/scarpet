//! The type lattice.
//!
//! # What it models
//!
//! Scarpet's `type()` reports only seven names — `null`, `bool`, `number`,
//! `string`, `list`, `iterator`, `map`. [`Ty`] is finer in two places where the
//! extra precision pays for itself and coarser in one where it does not:
//!
//! - [`Int`](Ty::Int) and [`Double`](Ty::Double) are separate, because `/` and
//!   `^` always produce a double while `+` on two integers does not, and a
//!   reader wants to see that. [`Num`](Ty::Num) is the join of the two and is
//!   what `type()` calls `number`.
//! - [`Undef`](Ty::Undef) is separate from [`Null`](Ty::Null) even though
//!   `type()` collapses them, because "read before anything wrote it" is the
//!   whole basis of the [`ReadBeforeWrite`](crate::LintCode::ReadBeforeWrite)
//!   lint.
//! - A lazy `range` is *not* distinguished from a realised list. The VM keeps
//!   them apart only so `type()` can answer `iterator`; nothing else in the
//!   language treats them differently, and a whole variant for one builtin's
//!   `type()` string is not worth it. Known imprecision, recorded here.
//!
//! # Join, and why there is no meet
//!
//! [`join`] is the least upper bound: the type of an expression when control
//! could have produced either operand — the branches of an `if`, a variable
//! written twice. There is deliberately no `meet` and no narrowing. Narrowing
//! refines a type inside a branch guarded by a predicate, and Scarpet has no
//! syntactic control flow, no type guards, and no exhaustiveness to check;
//! nothing in the language ever asks "what is `x` given that this test passed".
//!
//! [`Unknown`](Ty::Unknown) is the top: it absorbs everything, and in a language
//! this total "I don't know" is a legitimate and common answer rather than a
//! failure. [`Never`](Ty::Never) is the bottom and the identity of [`join`]; it
//! exists so the recursion fixpoint in the inference walk has somewhere to
//! ascend *from*. A finished analysis reports `Never` only for a function whose
//! body cannot produce a value at all.
//!
//! # Bounds
//!
//! `Ty` is recursive, so an unbounded nesting depth would mean an unbounded
//! `Drop` recursion — and corpus files do contain deeply nested literals. Every
//! container is built through [`Ty::list`] and [`Ty::map`], which widen past
//! [`MAX_DEPTH`]; every union is built through [`Ty::union`], which widens past
//! [`MAX_UNION`]. Together they make the lattice finite, which is also what
//! makes the fixpoint guaranteed to converge.

use std::fmt;

/// How deeply containers may nest before the element type widens to
/// [`Unknown`](Ty::Unknown).
pub const MAX_DEPTH: u32 = 6;

/// How many members a union may hold before it widens to
/// [`Unknown`](Ty::Unknown).
pub const MAX_UNION: usize = 4;

/// A type known by name but not by structure.
///
/// Scarpet's Minecraft-facing values plus `task`. The analysis never constructs
/// these from expressions yet — they are here so the builtin signature table can
/// name them as it grows, without a lattice change.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum OpaqueTy {
    Block,
    Entity,
    Nbt,
    Screen,
    Text,
    Task,
}

impl OpaqueTy {
    pub const fn as_str(self) -> &'static str {
        match self {
            OpaqueTy::Block => "block",
            OpaqueTy::Entity => "entity",
            OpaqueTy::Nbt => "nbt",
            OpaqueTy::Screen => "screen",
            OpaqueTy::Text => "text",
            OpaqueTy::Task => "task",
        }
    }
}

/// An inferred type.
///
/// Build containers with [`Ty::list`] / [`Ty::map`] and alternatives with
/// [`Ty::union`] or [`join`] — the variants are public so callers can match on
/// them, but constructing `List`/`Map`/`Union` directly bypasses the bounds that
/// keep the lattice finite.
///
/// The derived [`Ord`] is not a subtype relation; it is the canonical order
/// [`Ty::union`] sorts by, so **the declaration order below is the order union
/// members are printed in**. It is arranged to read the way a Scarpet
/// programmer would write it — `int | null`, `string | null` — which is why
/// `Undef` and `Null` sit at the end rather than next to the other scalars.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Ty {
    /// Top: no information.
    Unknown,
    /// Bottom: no value can arrive here. The identity of [`join`].
    Never,
    Bool,
    Int,
    Double,
    /// `Int | Double` — what `type()` calls `number`.
    Num,
    Str,
    List(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    Opaque(OpaqueTy),
    /// A variable read before anything wrote it.
    Undef,
    Null,
    /// A normalised alternative: sorted, deduplicated, two or more members,
    /// never containing `Unknown` or `Never`.
    Union(Vec<Ty>),
}

impl Ty {
    /// `list<elem>`, truncating the element past [`MAX_DEPTH`].
    pub fn list(elem: Ty) -> Ty {
        Ty::List(Box::new(clamp_depth(elem, MAX_DEPTH - 1)))
    }

    /// `map<key, value>`, truncating either side past [`MAX_DEPTH`].
    pub fn map(key: Ty, value: Ty) -> Ty {
        Ty::Map(
            Box::new(clamp_depth(key, MAX_DEPTH - 1)),
            Box::new(clamp_depth(value, MAX_DEPTH - 1)),
        )
    }

    /// The normalised alternative of `members`.
    ///
    /// Flattens nested unions, drops `Never`, collapses `Int`/`Double` into
    /// `Num`, sorts and deduplicates, and widens to `Unknown` when any member is
    /// `Unknown` or when more than [`MAX_UNION`] survive.
    pub fn union(members: impl IntoIterator<Item = Ty>) -> Ty {
        let mut flat = Vec::new();
        let mut stack: Vec<Ty> = members.into_iter().collect();
        stack.reverse();
        while let Some(ty) = stack.pop() {
            match ty {
                // Top absorbs: an alternative including "anything" is anything.
                Ty::Unknown => return Ty::Unknown,
                // Bottom contributes no value, so it drops out.
                Ty::Never => {}
                Ty::Union(inner) => stack.extend(inner.into_iter().rev()),
                other => flat.push(other),
            }
        }

        // `Num` is the join of `Int` and `Double`; keeping all three would let
        // `number | int` exist, which says nothing `number` does not.
        let has_num =
            flat.contains(&Ty::Num) || (flat.contains(&Ty::Int) && flat.contains(&Ty::Double));
        if has_num {
            flat.retain(|ty| !matches!(ty, Ty::Int | Ty::Double | Ty::Num));
            flat.push(Ty::Num);
        }

        flat.sort();
        flat.dedup();
        match flat.len() {
            0 => Ty::Never,
            1 => flat.pop().expect("length checked"),
            n if n > MAX_UNION => Ty::Unknown,
            _ => Ty::Union(flat),
        }
    }

    /// How deeply containers nest inside this type. Bounded by [`MAX_DEPTH`] for
    /// any type built through the constructors above.
    pub fn depth(&self) -> u32 {
        match self {
            Ty::List(elem) => 1 + elem.depth(),
            Ty::Map(key, value) => 1 + key.depth().max(value.depth()),
            Ty::Union(members) => members.iter().map(Ty::depth).max().unwrap_or(0),
            _ => 0,
        }
    }

    /// Whether every value of this type is a number.
    pub fn is_number(&self) -> bool {
        match self {
            Ty::Int | Ty::Double | Ty::Num => true,
            Ty::Union(members) => members.iter().all(Ty::is_number),
            _ => false,
        }
    }

    /// Whether this type says anything at all. `Unknown` and `Never` do not, so
    /// a lint that needs proof must check this first.
    pub fn is_known(&self) -> bool {
        !matches!(self, Ty::Unknown | Ty::Never)
    }

    /// Whether every value of this type is a list.
    pub fn is_list(&self) -> bool {
        match self {
            Ty::List(_) => true,
            Ty::Union(members) => members.iter().all(Ty::is_list),
            _ => false,
        }
    }

    /// Whether every value of this type is a map.
    pub fn is_map(&self) -> bool {
        match self {
            Ty::Map(_, _) => true,
            Ty::Union(members) => members.iter().all(Ty::is_map),
            _ => false,
        }
    }

    /// Whether every value of this type is a string.
    pub fn is_string(&self) -> bool {
        match self {
            Ty::Str => true,
            Ty::Union(members) => members.iter().all(Ty::is_string),
            _ => false,
        }
    }

    /// Whether any value of this type could be a number — the negation is what
    /// [`ExpectedNumber`](crate::LintCode::ExpectedNumber) needs, since `Bool`
    /// coerces and `Unknown` must never be accused.
    pub fn could_be_number(&self) -> bool {
        match self {
            Ty::Int | Ty::Double | Ty::Num | Ty::Bool | Ty::Unknown | Ty::Never => true,
            Ty::Union(members) => members.iter().any(Ty::could_be_number),
            _ => false,
        }
    }

    /// The element type of a list, or `Unknown` for anything else.
    pub fn elem(&self) -> Ty {
        match self {
            Ty::List(elem) => (**elem).clone(),
            Ty::Union(members) => Ty::union(members.iter().map(Ty::elem)),
            _ => Ty::Unknown,
        }
    }

    /// The value type of a map, or `Unknown` for anything else.
    pub fn value(&self) -> Ty {
        match self {
            Ty::Map(_, value) => (**value).clone(),
            Ty::Union(members) => Ty::union(members.iter().map(Ty::value)),
            _ => Ty::Unknown,
        }
    }

    /// The key type of a map, or `Unknown` for anything else.
    pub fn key(&self) -> Ty {
        match self {
            Ty::Map(key, _) => (**key).clone(),
            Ty::Union(members) => Ty::union(members.iter().map(Ty::key)),
            _ => Ty::Unknown,
        }
    }
}

/// Replace any container nested deeper than `budget` with
/// [`Unknown`](Ty::Unknown), keeping the outermost levels.
///
/// Truncating rather than widening wholesale is what makes the bound a *fixed
/// point*: collapsing an over-deep type to a shallow one would let the next
/// `Ty::list` grow it again, so nesting would oscillate instead of settling and
/// the inference fixpoint would never converge.
///
/// The recursion is bounded by `budget`, not by the input, because the `budget
/// == 0` arm returns without descending.
fn clamp_depth(ty: Ty, budget: u32) -> Ty {
    match ty {
        Ty::List(_) | Ty::Map(_, _) if budget == 0 => Ty::Unknown,
        Ty::List(elem) => Ty::List(Box::new(clamp_depth(*elem, budget - 1))),
        Ty::Map(key, value) => Ty::Map(
            Box::new(clamp_depth(*key, budget - 1)),
            Box::new(clamp_depth(*value, budget - 1)),
        ),
        // Re-normalise: truncation can make two members equal, or turn one into
        // `Unknown`, which absorbs the rest.
        Ty::Union(members) => Ty::union(
            members
                .into_iter()
                .map(|member| clamp_depth(member, budget)),
        ),
        other => other,
    }
}

/// The least upper bound of two types.
///
/// `join(Never, t) == t` and `join(Unknown, _) == Unknown`. Containers join
/// pointwise, so `list<int>` and `list<string>` become `list<int | string>`
/// rather than a two-member union of lists — that keeps unions shallow and reads
/// better.
pub fn join(a: &Ty, b: &Ty) -> Ty {
    match (a, b) {
        (Ty::Never, other) | (other, Ty::Never) => other.clone(),
        (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
        (Ty::List(x), Ty::List(y)) => Ty::list(join(x, y)),
        (Ty::Map(k1, v1), Ty::Map(k2, v2)) => Ty::map(join(k1, k2), join(v1, v2)),
        _ if a == b => a.clone(),
        _ => Ty::union([a.clone(), b.clone()]),
    }
}

/// The least upper bound of any number of types; `Never` when there are none.
pub fn join_all(types: impl IntoIterator<Item = Ty>) -> Ty {
    types.into_iter().fold(Ty::Never, |acc, ty| join(&acc, &ty))
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Unknown => f.write_str("unknown"),
            Ty::Never => f.write_str("never"),
            Ty::Undef => f.write_str("undef"),
            Ty::Null => f.write_str("null"),
            Ty::Bool => f.write_str("bool"),
            Ty::Int => f.write_str("int"),
            Ty::Double => f.write_str("double"),
            Ty::Num => f.write_str("number"),
            Ty::Str => f.write_str("string"),
            Ty::List(elem) => write!(f, "list<{elem}>"),
            Ty::Map(key, value) => write!(f, "map<{key}, {value}>"),
            Ty::Opaque(opaque) => f.write_str(opaque.as_str()),
            Ty::Union(members) => {
                for (index, member) in members.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{member}")?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_is_the_identity_of_join() {
        assert_eq!(join(&Ty::Never, &Ty::Int), Ty::Int);
        assert_eq!(join(&Ty::Str, &Ty::Never), Ty::Str);
        assert_eq!(join(&Ty::Never, &Ty::Never), Ty::Never);
        assert_eq!(join_all([]), Ty::Never);
    }

    #[test]
    fn unknown_absorbs() {
        assert_eq!(join(&Ty::Unknown, &Ty::Int), Ty::Unknown);
        assert_eq!(Ty::union([Ty::Int, Ty::Unknown, Ty::Str]), Ty::Unknown);
    }

    #[test]
    fn int_and_double_collapse_to_number() {
        assert_eq!(join(&Ty::Int, &Ty::Double), Ty::Num);
        assert_eq!(join(&Ty::Int, &Ty::Num), Ty::Num);
        assert_eq!(Ty::union([Ty::Double, Ty::Num, Ty::Int]), Ty::Num);
        // The collapse must not swallow unrelated members.
        assert_eq!(
            Ty::union([Ty::Int, Ty::Double, Ty::Str]).to_string(),
            "number | string"
        );
    }

    #[test]
    fn unions_are_normalised() {
        assert_eq!(Ty::union([Ty::Str]), Ty::Str);
        assert_eq!(Ty::union([Ty::Str, Ty::Str]), Ty::Str);
        // Nested unions flatten, and the order is canonical.
        let nested = Ty::union([Ty::union([Ty::Str, Ty::Null]), Ty::Bool]);
        assert_eq!(nested, Ty::union([Ty::Bool, Ty::Null, Ty::Str]));
    }

    #[test]
    fn a_wide_union_widens_to_unknown() {
        let members = [Ty::Null, Ty::Bool, Ty::Str, Ty::Int, Ty::list(Ty::Str)];
        assert!(members.len() > MAX_UNION);
        assert_eq!(Ty::union(members), Ty::Unknown);
    }

    #[test]
    fn containers_join_pointwise() {
        assert_eq!(
            join(&Ty::list(Ty::Int), &Ty::list(Ty::Str)).to_string(),
            "list<int | string>"
        );
        assert_eq!(
            join(&Ty::map(Ty::Str, Ty::Int), &Ty::map(Ty::Str, Ty::Null)).to_string(),
            "map<string, int | null>"
        );
    }

    /// The bound is what makes `Drop` safe and the fixpoint finite.
    #[test]
    fn nesting_is_capped() {
        let mut ty = Ty::Int;
        for _ in 0..100 {
            ty = Ty::list(ty);
        }
        assert!(ty.depth() <= MAX_DEPTH);
        assert_eq!(
            ty.to_string(),
            "list<list<list<list<list<list<unknown>>>>>>"
        );
    }

    #[test]
    fn display_uses_scarpet_facing_names() {
        assert_eq!(Ty::Num.to_string(), "number");
        assert_eq!(Ty::Str.to_string(), "string");
        assert_eq!(Ty::list(Ty::Int).to_string(), "list<int>");
        assert_eq!(Ty::Opaque(OpaqueTy::Block).to_string(), "block");
        assert_eq!(Ty::union([Ty::Int, Ty::Null]).to_string(), "int | null");
    }

    #[test]
    fn predicates_do_not_accuse_unknown() {
        assert!(Ty::Unknown.could_be_number());
        assert!(Ty::Bool.could_be_number());
        assert!(!Ty::Str.could_be_number());
        assert!(!Ty::Unknown.is_known());
        assert!(Ty::list(Ty::Int).is_list());
        assert_eq!(Ty::list(Ty::Int).elem(), Ty::Int);
        assert_eq!(Ty::Str.elem(), Ty::Unknown);
    }
}
