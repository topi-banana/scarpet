//! Type rules for Scarpet's operators.
//!
//! A deliberate port of `scarpet-vm`'s `value/*.rs`, rule for rule and **in the
//! VM's match order** — several arms are asymmetric, so the order is part of the
//! semantics. Each function names its source file; when the VM's behaviour
//! changes, the two must move together. `scarpet-hir` does not depend on
//! `scarpet-vm` (a compiler must not depend on an interpreter), so the coupling
//! is by comment rather than by code.
//!
//! # Totality
//!
//! Almost nothing here is an error. Scarpet defines every operator on every pair
//! of values by falling back to text: `1 + [1, 2]` is `"1[1, 2]"`, `'hello' - 1`
//! is `"hello"`, `1 / [1, 2]` is `"1/[1, 2]"`. That fallback is what the
//! [`StringFallback`](crate::LintCode::StringFallback) lint exists to point at;
//! the rules below just report the type it produces.
//!
//! # Two intentional divergences from `scarpet-vm`
//!
//! - **`Bool` in arithmetic.** The VM's `AddAssign` has no `Bool` arm, so
//!   `true + 1` falls through to `"true1"`. Upstream fabric-carpet's
//!   `BooleanValue` *is* a `NumericValue` and adds numerically, so these rules
//!   follow upstream and treat `Bool` as `Int`. The lint never fires on a `Bool`
//!   operand, so neither reading produces a false warning.
//! - **`*` on unhandled pairs.** `scarpet-vm/src/value/mul.rs` ends in
//!   `_ => todo!()`, so `null * null` aborts the process. Upstream joins the two
//!   as text, so these rules say `string`.

use crate::ty::{Ty, join, join_all};

/// Bottom propagates: an operand that cannot produce a value means the operator
/// cannot either.
fn bottom(a: &Ty, b: &Ty) -> bool {
    matches!(a, Ty::Never) || matches!(b, Ty::Never)
}

/// Apply an operator to each member of a union operand and join the results.
///
/// Without this, `(int | null) + int` would find no arithmetic arm that covers
/// the whole left side and fall all the way through to `string` — which is
/// wrong (it is `int` whenever the left side is an `int`) and, worse, would make
/// the [`StringFallback`](crate::LintCode::StringFallback) lint accuse perfectly
/// ordinary code. Distributing gives `int | string`, which is the truth.
///
/// Terminates because a normalised union never contains a union.
fn distribute(a: &Ty, b: &Ty, op: fn(&Ty, &Ty) -> Ty) -> Option<Ty> {
    match (a, b) {
        (Ty::Union(members), _) => Some(join_all(members.iter().map(|member| op(member, b)))),
        (_, Ty::Union(members)) => Some(join_all(members.iter().map(|member| op(a, member)))),
        _ => None,
    }
}

/// The numeric view of a type, following the VM's `as_number`: numbers pass
/// through and a boolean is an integer. `None` for anything else — the VM raises
/// `ExpectedNumber` where that matters.
fn as_number(ty: &Ty) -> Option<Ty> {
    match ty {
        Ty::Int | Ty::Bool => Some(Ty::Int),
        Ty::Double => Some(Ty::Double),
        Ty::Num => Some(Ty::Num),
        Ty::Union(members) => {
            let mut acc = Ty::Never;
            for member in members {
                acc = join(&acc, &as_number(member)?);
            }
            Some(acc)
        }
        _ => None,
    }
}

/// The result of an arithmetic operator on two numeric operands: integers stay
/// integers, anything touching a double is a double, and a `number` on either
/// side keeps the result a `number`.
fn arith(a: &Ty, b: &Ty) -> Ty {
    match (a, b) {
        (Ty::Int, Ty::Int) => Ty::Int,
        (Ty::Double, _) | (_, Ty::Double) => Ty::Double,
        _ => Ty::Num,
    }
}

/// A list operand broadcasts: two equal-length lists combine pairwise, and a
/// list with a scalar applies the scalar to each element. Either way the result
/// is a list, and its element type is the operator applied one level down.
///
/// Terminates because [`Ty::elem`] is strictly shallower and bottoms out at a
/// non-list, whose `elem` is `Unknown`.
fn broadcast(a: &Ty, b: &Ty, op: fn(&Ty, &Ty) -> Ty) -> Ty {
    let elem = if b.is_list() {
        op(&a.elem(), &b.elem())
    } else {
        op(&a.elem(), b)
    };
    Ty::list(elem)
}

/// `+` — `scarpet-vm/src/value/add.rs`, `AddAssign`.
pub(crate) fn add(a: &Ty, b: &Ty) -> Ty {
    if bottom(a, b) {
        return Ty::Never;
    }
    if let Some(ty) = distribute(a, b, add) {
        return ty;
    }
    // A list on the left wins before any numeric arm can apply.
    if a.is_list() {
        return broadcast(a, b, add);
    }
    if let (Some(x), Some(y)) = (as_number(a), as_number(b)) {
        return arith(&x, &y);
    }
    // A string on the left concatenates with anything.
    if a.is_string() {
        return Ty::Str;
    }
    // Every remaining pair of *known* types joins as text.
    if a.is_known() && b.is_known() {
        return Ty::Str;
    }
    Ty::Unknown
}

/// `-` — `scarpet-vm/src/value/add.rs`, `SubAssign`. Unlike `+` there is no
/// string arm: a non-numeric pair deletes the right side's text from the left's,
/// which is still a string.
pub(crate) fn sub(a: &Ty, b: &Ty) -> Ty {
    if bottom(a, b) {
        return Ty::Never;
    }
    if let Some(ty) = distribute(a, b, sub) {
        return ty;
    }
    if a.is_list() {
        return broadcast(a, b, sub);
    }
    if let (Some(x), Some(y)) = (as_number(a), as_number(b)) {
        return arith(&x, &y);
    }
    if a.is_known() && b.is_known() {
        return Ty::Str;
    }
    Ty::Unknown
}

/// `*` — `scarpet-vm/src/value/mul.rs`, `MulAssign`. The only operator whose
/// list arm is symmetric: `2 * [1, 2]` scales the list just as `[1, 2] * 2`
/// does.
pub(crate) fn mul(a: &Ty, b: &Ty) -> Ty {
    if bottom(a, b) {
        return Ty::Never;
    }
    if let Some(ty) = distribute(a, b, mul) {
        return ty;
    }
    if let (Some(x), Some(y)) = (as_number(a), as_number(b)) {
        return arith(&x, &y);
    }
    if a.is_list() {
        return broadcast(a, b, mul);
    }
    if b.is_list() {
        return Ty::list(mul(a, &b.elem()));
    }
    // `'ab' * 3` repeats, `'a' * 'b'` joins as "a.b", and every other known pair
    // reaches the VM's `todo!()` — upstream joins those as text too.
    if a.is_known() && b.is_known() {
        return Ty::Str;
    }
    Ty::Unknown
}

/// `/` — `scarpet-vm/src/value/mul.rs`, `DivAssign`. Two numbers **always**
/// divide as doubles, so `4 / 2` is `2.0`, and only a list on the *left*
/// broadcasts.
pub(crate) fn div(a: &Ty, b: &Ty) -> Ty {
    if bottom(a, b) {
        return Ty::Never;
    }
    if let Some(ty) = distribute(a, b, div) {
        return ty;
    }
    if a.is_list() {
        return broadcast(a, b, div);
    }
    if as_number(a).is_some() && as_number(b).is_some() {
        return Ty::Double;
    }
    if a.is_known() && b.is_known() {
        return Ty::Str;
    }
    Ty::Unknown
}

/// `%` — `scarpet-vm/src/value/arithmetic.rs`. Coerces both sides through
/// `as_number` and raises `ExpectedNumber` when that fails, so the result is
/// always numeric where it exists at all.
pub(crate) fn rem(a: &Ty, b: &Ty) -> Ty {
    if bottom(a, b) {
        return Ty::Never;
    }
    if let Some(ty) = distribute(a, b, rem) {
        return ty;
    }
    match (as_number(a), as_number(b)) {
        (Some(x), Some(y)) => arith(&x, &y),
        // A non-numeric operand is a run-time error, reported as a diagnostic;
        // the expression's type is still the one it would have had.
        _ => Ty::Num,
    }
}

/// `^` — `scarpet-vm/src/value/arithmetic.rs`. Always a double.
pub(crate) fn pow(a: &Ty, b: &Ty) -> Ty {
    if bottom(a, b) {
        return Ty::Never;
    }
    Ty::Double
}

/// Unary `-` and `+` — `scarpet-vm/src/value/arithmetic.rs`. Both coerce through
/// `as_number`, so a boolean becomes an integer.
pub(crate) fn unary_number(a: &Ty) -> Ty {
    match a {
        Ty::Never => Ty::Never,
        other => as_number(other).unwrap_or(Ty::Num),
    }
}

/// `:` — `scarpet-vm/src/value/access.rs`, `scarpet_get`.
///
/// A list index wraps with `rem_euclid`, so it never runs out of range and the
/// only way to get `null` is an empty container. A **non-container is not an
/// error**: it simply reads as `null`, strings included.
///
/// Takes only the container: what a `:` produces depends on what it reads from,
/// never on the address. (`[1, 'a']:0` is `int | string`, not `int`, because
/// element types are not tracked per index.)
pub(crate) fn get(base: &Ty) -> Ty {
    match base {
        Ty::Never => Ty::Never,
        Ty::Unknown => Ty::Unknown,
        Ty::List(elem) => join(elem, &Ty::Null),
        Ty::Map(_, value) => join(value, &Ty::Null),
        Ty::Union(members) => join_all(members.iter().map(get)),
        _ => Ty::Null,
    }
}

/// `~` — `scarpet-vm/src/value/access.rs`, `scarpet_match`.
///
/// A list yields the index of the value or `null`; a map yields the key if it is
/// present; `null` matches to `null`; anything else runs the right side as a
/// regular expression over the two sides' text, yielding the match, its capture
/// groups, or `null`. As with [`get`], only the left side decides the shape of
/// the result.
pub(crate) fn matches(base: &Ty) -> Ty {
    match base {
        Ty::Never => Ty::Never,
        Ty::Unknown => Ty::Unknown,
        Ty::List(_) => join(&Ty::Int, &Ty::Null),
        Ty::Map(key, _) => join(key, &Ty::Null),
        Ty::Null | Ty::Undef => Ty::Null,
        Ty::Union(members) => join_all(members.iter().map(matches)),
        _ => Ty::union([Ty::Str, Ty::list(Ty::Str), Ty::Null]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each of these mirrors an assertion in `scarpet-vm`'s own tests.
    #[test]
    fn arithmetic_matches_the_vm() {
        assert_eq!(add(&Ty::Int, &Ty::Int), Ty::Int);
        assert_eq!(add(&Ty::Int, &Ty::Double), Ty::Double);
        // `4 / 2` is 2.0 — division is always a double.
        assert_eq!(div(&Ty::Int, &Ty::Int), Ty::Double);
        // `2 ^ 10` is a double too.
        assert_eq!(pow(&Ty::Int, &Ty::Int), Ty::Double);
        assert_eq!(rem(&Ty::Int, &Ty::Int), Ty::Int);
        assert_eq!(rem(&Ty::Int, &Ty::Double), Ty::Double);
    }

    /// `1 + [1, 2]` is `"1[1, 2]"`; `[1, 2, 3] + 1` is `[2, 3, 4]`.
    #[test]
    fn lists_broadcast_only_from_the_left_except_for_multiplication() {
        assert_eq!(add(&Ty::list(Ty::Int), &Ty::Int), Ty::list(Ty::Int));
        assert_eq!(add(&Ty::Int, &Ty::list(Ty::Int)), Ty::Str);
        assert_eq!(div(&Ty::Int, &Ty::list(Ty::Int)), Ty::Str);
        // `*` scales from either side.
        assert_eq!(mul(&Ty::Int, &Ty::list(Ty::Int)), Ty::list(Ty::Int));
        assert_eq!(mul(&Ty::list(Ty::Int), &Ty::Int), Ty::list(Ty::Int));
        // Two lists combine pairwise.
        assert_eq!(
            add(&Ty::list(Ty::Int), &Ty::list(Ty::Double)),
            Ty::list(Ty::Double)
        );
    }

    /// `'hello' - 1` is `"hello"`; `'5' + 1` is `"51"`. Strings never parse as
    /// numbers.
    #[test]
    fn strings_never_become_numbers() {
        assert_eq!(add(&Ty::Str, &Ty::Int), Ty::Str);
        assert_eq!(add(&Ty::Int, &Ty::Str), Ty::Str);
        assert_eq!(sub(&Ty::Str, &Ty::Int), Ty::Str);
        assert_eq!(mul(&Ty::Str, &Ty::Str), Ty::Str);
        assert_eq!(as_number(&Ty::Str), None);
    }

    /// Upstream's `BooleanValue` is a number, so `true + 1` is arithmetic.
    #[test]
    fn booleans_are_numbers() {
        assert_eq!(add(&Ty::Bool, &Ty::Int), Ty::Int);
        assert_eq!(unary_number(&Ty::Bool), Ty::Int);
    }

    /// The VM's `mul` reaches `todo!()` here; upstream joins the two as text.
    #[test]
    fn multiplication_of_an_unhandled_pair_is_text() {
        assert_eq!(mul(&Ty::Null, &Ty::Null), Ty::Str);
    }

    #[test]
    fn an_unknown_operand_gives_up_rather_than_guessing() {
        assert_eq!(add(&Ty::Int, &Ty::Unknown), Ty::Unknown);
        assert_eq!(sub(&Ty::Unknown, &Ty::Int), Ty::Unknown);
        // Except where the result does not depend on the operands.
        assert_eq!(pow(&Ty::Unknown, &Ty::Unknown), Ty::Double);
    }

    #[test]
    fn bottom_propagates() {
        assert_eq!(add(&Ty::Never, &Ty::Int), Ty::Never);
        assert_eq!(mul(&Ty::Str, &Ty::Never), Ty::Never);
        assert_eq!(get(&Ty::Never), Ty::Never);
    }

    #[test]
    fn access_is_total() {
        assert_eq!(get(&Ty::list(Ty::Str)).to_string(), "string | null");
        assert_eq!(get(&Ty::map(Ty::Str, Ty::Int)).to_string(), "int | null");
        // Indexing a non-container is not an error; it reads as null.
        assert_eq!(get(&Ty::Str), Ty::Null);
        assert_eq!(get(&Ty::Int), Ty::Null);
    }

    #[test]
    fn matching_depends_on_the_container() {
        assert_eq!(matches(&Ty::list(Ty::Str)).to_string(), "int | null");
        assert_eq!(
            matches(&Ty::map(Ty::Str, Ty::Int)).to_string(),
            "string | null"
        );
        assert_eq!(matches(&Ty::Null), Ty::Null);
    }
}
