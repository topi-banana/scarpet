//! Lowering from the `scarpet-syntax` CST to the [`Doc`] IR.
//!
//! The traversal mirrors the shape of `strip_leading` in the parser: one arm
//! per `CstKind`, recursing into the same children. Each node's leading trivia
//! is emitted by [`expr`] (own-line comments) or lifted onto the preceding
//! token by [`child_after`] (same-line / trailing comments). Blank lines are
//! reconstructed as statement separators.

use scarpet_syntax::parser::{BinOp, Cst, CstKind, UnaryOp};

use crate::doc::{
    Doc, blank_line, concat, group, hardline, if_break, join, line, nest, nil, softline, space,
    text,
};
use crate::trivia::{has_blank_before, own_line_comments, same_line_comment};

/// Lower a whole program (the CST root) to a document.
pub fn program(root: &Cst) -> Doc {
    expr(root)
}

/// Lower a node, prefixing its own-line leading comments. Used wherever the
/// node is *not* preceded by a separator we can lift a trailing comment onto.
fn expr(cst: &Cst) -> Doc {
    concat([own_line_comments(&cst.leading), node_body(cst)])
}

/// Place `cst` after the separator `sep`. If `cst`'s leading begins with a
/// same-line (trailing) comment, emit it *before* `sep` so it hugs the previous
/// token, then hard-break; otherwise emit `sep` followed by any own-line
/// comments. This is what keeps `a; // note` on one line.
fn child_after(sep: Doc, cst: &Cst) -> Doc {
    match same_line_comment(&cst.leading) {
        Some(c) => concat([
            space(),
            text(c.to_string()),
            hardline(),
            own_line_comments(&cst.leading[1..]),
            node_body(cst),
        ]),
        None => concat([sep, own_line_comments(&cst.leading), node_body(cst)]),
    }
}

/// Lower a node's `kind`, ignoring its own leading (handled by the caller).
fn node_body(cst: &Cst) -> Doc {
    match &cst.kind {
        CstKind::Number(s) | CstKind::Str(s) | CstKind::Ident(s) => text(s.to_string()),
        CstKind::Unary { op, operand } => unary(*op, operand),
        CstKind::Binary { op, lhs, rhs } => match *op {
            BinOp::Comma => comma_chain(cst),
            BinOp::Semi => semi_chain(cst),
            BinOp::Arrow => arrow_chain(cst),
            BinOp::Get => tight(*op, lhs, rhs),
            _ => spaced(*op, lhs, rhs),
        },
        CstKind::Call { callee, args } => call(callee, args),
        CstKind::List(items) => collection("[", items, "]"),
        CstKind::Map(items) => collection("{", items, "}"),
        CstKind::Paren(inner) => paren(inner),
        CstKind::Empty => nil(),
    }
}

// ---- operators -----------------------------------------------------

/// A binary operator with no surrounding space (`a:b`).
fn tight(op: BinOp, lhs: &Cst, rhs: &Cst) -> Doc {
    concat([expr(lhs), text(bin_op_str(op)), expr(rhs)])
}

/// A binary operator spaced on each side, breakable before the RHS
/// (`a + b`, breaking to `a +` ⏎ `b` when it doesn't fit).
fn spaced(op: BinOp, lhs: &Cst, rhs: &Cst) -> Doc {
    // A trailing comment the parser attached to a *compound* RHS node sat
    // before the operator in source (`lhs // c` ⏎ `op rhs`): operator-leading
    // trivia binds to the RHS. Emit it before the operator so re-parsing
    // returns it to the same node — emitting it after would rebind it to the
    // RHS's leftmost leaf and break idempotency. (An atom RHS *is* its own
    // leftmost leaf, so the after-operator placement round-trips and reads
    // better; leave it to `child_after`.)
    if let Some(c) = same_line_comment(&rhs.leading)
        && !is_atom(rhs)
    {
        return concat([
            expr(lhs),
            space(),
            text(c.to_string()),
            hardline(),
            text(bin_op_str(op)),
            space(),
            own_line_comments(&rhs.leading[1..]),
            node_body(rhs),
        ]);
    }
    group(concat([
        expr(lhs),
        space(),
        text(bin_op_str(op)),
        child_after(line(), rhs),
    ]))
}

/// Whether `cst` is a leaf whose leftmost token is the node itself — so a
/// comment placed after a preceding operator re-parses back onto it.
fn is_atom(cst: &Cst) -> bool {
    matches!(
        &cst.kind,
        CstKind::Number(_) | CstKind::Str(_) | CstKind::Ident(_) | CstKind::Empty
    )
}

/// A prefix unary operator (`-x`, `!x`, `...xs`).
fn unary(op: UnaryOp, operand: &Cst) -> Doc {
    concat([text(unary_op_str(op)), expr(operand)])
}

// ---- chains --------------------------------------------------------

/// A `,`-separated chain, flat (`a, b, c`) or one-per-line when wide.
fn comma_chain(cst: &Cst) -> Doc {
    group(comma_separated(&flatten_left(BinOp::Comma, cst)))
}

/// A `;`-separated statement sequence.
fn semi_chain(cst: &Cst) -> Doc {
    statement_seq(&flatten_left(BinOp::Semi, cst))
}

/// A one-per-line, `;`-terminated statement sequence, preserving blank-line
/// separators. Shared by the top level and paren blocks. (A trailing `;` is
/// dropped on re-parse, so synthesizing one is non-destructive.)
fn statement_seq(stmts: &[&Cst]) -> Doc {
    let mut parts = Vec::new();
    for (i, s) in stmts.iter().enumerate() {
        if i == 0 {
            parts.push(expr(s));
        } else {
            let sep = if has_blank_before(&s.leading) {
                blank_line()
            } else {
                hardline()
            };
            parts.push(child_after(sep, s));
        }
        parts.push(text(";"));
    }
    concat(parts)
}

/// A right-associative `->` chain (`a -> b -> c`). The arrow stays on the
/// signature line; the RHS handles its own breaking.
fn arrow_chain(cst: &Cst) -> Doc {
    let parts = flatten_right(BinOp::Arrow, cst);
    join(parts.into_iter().map(expr), text(" -> "))
}

// ---- calls / collections / parens ----------------------------------

/// A call `callee(args)`.
fn call(callee: &Cst, args: &[Cst]) -> Doc {
    concat([expr(callee), collection("(", args, ")")])
}

/// A delimited, comma-separated sequence (`(...)`, `[...]`, `{...}`). Laid out
/// flat if it fits, else one item per line with a trailing comma.
fn collection(open: &'static str, items: &[Cst], close: &'static str) -> Doc {
    if items.is_empty() {
        return concat([text(open), text(close)]);
    }
    let body = comma_separated(&items.iter().collect::<Vec<_>>());
    group(concat([
        text(open),
        nest(concat([softline(), body, if_break(text(","), nil())])),
        softline(),
        text(close),
    ]))
}

/// Join items with `, ` (breakable after each comma), lifting trailing comments
/// of the 2nd+ items onto the preceding comma.
fn comma_separated(items: &[&Cst]) -> Doc {
    let mut parts = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if i == 0 {
            parts.push(expr(item));
        } else {
            parts.push(text(","));
            parts.push(child_after(line(), item));
        }
    }
    concat(parts)
}

/// A parenthesized node. Parens are always preserved. A `;`-chain inside is a
/// statement block (one statement per line); anything else is an expression
/// paren that may break softly.
fn paren(inner: &Cst) -> Doc {
    if is_semi_chain(inner) {
        let stmts = flatten_left(BinOp::Semi, inner);
        group(concat([
            text("("),
            nest(concat([hardline(), statement_seq(&stmts)])),
            hardline(),
            text(")"),
        ]))
    } else {
        group(concat([
            text("("),
            nest(concat([softline(), expr(inner)])),
            softline(),
            text(")"),
        ]))
    }
}

fn is_semi_chain(cst: &Cst) -> bool {
    matches!(
        &cst.kind,
        CstKind::Binary {
            op: BinOp::Semi,
            ..
        }
    )
}

// ---- chain flattening ----------------------------------------------

/// Collect the operands of a left-nested chain of `op` (`((a op b) op c)` →
/// `[a, b, c]`). The tree is never rebuilt — this only drives layout.
fn flatten_left<'a, 's>(op: BinOp, cst: &'a Cst<'s>) -> Vec<&'a Cst<'s>> {
    fn go<'a, 's>(op: BinOp, cst: &'a Cst<'s>, out: &mut Vec<&'a Cst<'s>>) {
        if let CstKind::Binary { op: o, lhs, rhs } = &cst.kind
            && *o == op
        {
            go(op, lhs.as_ref(), out);
            out.push(rhs.as_ref());
            return;
        }
        out.push(cst);
    }
    let mut out = Vec::new();
    go(op, cst, &mut out);
    out
}

/// Collect the operands of a right-nested chain of `op` (`a op (b op c)` →
/// `[a, b, c]`).
fn flatten_right<'a, 's>(op: BinOp, cst: &'a Cst<'s>) -> Vec<&'a Cst<'s>> {
    let mut out = Vec::new();
    let mut cur = cst;
    loop {
        if let CstKind::Binary { op: o, lhs, rhs } = &cur.kind
            && *o == op
        {
            out.push(lhs.as_ref());
            cur = rhs.as_ref();
            continue;
        }
        out.push(cur);
        break;
    }
    out
}

// ---- glyphs --------------------------------------------------------

fn bin_op_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::Pow => "^",
        BinOp::Eq => "==",
        BinOp::NotEq => "!=",
        BinOp::Lt => "<",
        BinOp::LtEq => "<=",
        BinOp::Gt => ">",
        BinOp::GtEq => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::Match => "~",
        BinOp::Get => ":",
        BinOp::Assign => "=",
        BinOp::AddAssign => "+=",
        BinOp::Swap => "<>",
        BinOp::Arrow => "->",
        BinOp::Semi => ";",
        BinOp::Comma => ",",
    }
}

fn unary_op_str(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Neg => "-",
        UnaryOp::Pos => "+",
        UnaryOp::Not => "!",
        UnaryOp::Unpack => "...",
    }
}

#[cfg(test)]
mod tests {
    use crate::{Config, format_source};

    fn fmt(src: &str) -> String {
        format_source(src, &Config::default()).unwrap()
    }

    #[test]
    fn arithmetic_spacing() {
        assert_eq!(fmt("2+3*4"), "2 + 3 * 4\n");
        assert_eq!(fmt("2 + 3 - 1"), "2 + 3 - 1\n");
    }

    #[test]
    fn power_chain() {
        assert_eq!(fmt("2^3^2"), "2 ^ 3 ^ 2\n");
    }

    #[test]
    fn get_is_tight_match_is_spaced() {
        assert_eq!(fmt("a:b:c"), "a:b:c\n");
        // `~` is spaced per the chosen style.
        assert_eq!(fmt("a~b"), "a ~ b\n");
    }

    #[test]
    fn unary_prefixes() {
        assert_eq!(fmt("-x"), "-x\n");
        assert_eq!(fmt("!x"), "!x\n");
        assert_eq!(fmt("...xs"), "...xs\n");
        assert_eq!(fmt("-foo:0"), "-foo:0\n");
    }

    #[test]
    fn assignment_is_right_assoc_and_spaced() {
        assert_eq!(fmt("a=b=5"), "a = b = 5\n");
    }

    #[test]
    fn comma_chain_flat() {
        assert_eq!(fmt("a,b,c"), "a, b, c\n");
    }

    #[test]
    fn semi_chain_one_statement_per_line() {
        assert_eq!(fmt("a;b;c"), "a;\nb;\nc;\n");
    }

    #[test]
    fn arrow_is_spaced() {
        assert_eq!(fmt("a->b"), "a -> b\n");
        assert_eq!(fmt("a->b->c"), "a -> b -> c\n");
    }

    #[test]
    fn empty_call() {
        assert_eq!(fmt("f()"), "f()\n");
    }

    #[test]
    fn calls_stay_flat_when_short() {
        assert_eq!(fmt("print('hi')"), "print('hi')\n");
        assert_eq!(fmt("f(a,b,c)"), "f(a, b, c)\n");
    }

    #[test]
    fn nested_call_flat() {
        assert_eq!(fmt("print(format('x','y'))"), "print(format('x', 'y'))\n");
    }

    #[test]
    fn list_and_map_flat() {
        assert_eq!(fmt("[1,2,3]"), "[1, 2, 3]\n");
        assert_eq!(fmt("{'a'->1,'b'->2}"), "{'a' -> 1, 'b' -> 2}\n");
    }

    #[test]
    fn parens_are_preserved() {
        assert_eq!(fmt("(a+b)"), "(a + b)\n");
        assert_eq!(fmt("((x))"), "((x))\n");
    }

    #[test]
    fn paren_with_semi_chain_is_a_block() {
        assert_eq!(fmt("foo()->(a;b)"), "foo() -> (\n    a;\n    b;\n)\n");
    }

    #[test]
    fn long_collection_breaks_one_per_line_with_trailing_comma() {
        let nums = [
            "1111111111",
            "2222222222",
            "3333333333",
            "4444444444",
            "5555555555",
            "6666666666",
            "7777777777",
            "8888888888",
            "9999999999",
        ];
        let src = format!("[{}]", nums.join(", "));
        let mut expected = String::from("[\n");
        for n in nums {
            expected.push_str(&format!("    {n},\n"));
        }
        expected.push_str("]\n");
        assert_eq!(fmt(&src), expected);
    }

    // ---- trivia ----------------------------------------------------

    #[test]
    fn header_comment_kept_on_own_line() {
        assert_eq!(fmt("// hi\nx"), "// hi\nx\n");
    }

    #[test]
    fn blank_line_between_statements_collapses_to_one() {
        assert_eq!(fmt("a;\n\n\nb"), "a;\n\nb;\n");
    }

    #[test]
    fn single_break_is_not_a_blank_line() {
        assert_eq!(fmt("a;\nb"), "a;\nb;\n");
    }

    #[test]
    fn trailing_comment_stays_on_previous_line() {
        // option 2: a same-line comment hugs the preceding token.
        assert_eq!(fmt("a; // note\nb"), "a; // note\nb;\n");
        assert_eq!(fmt("a + // mid\nb"), "a + // mid\nb\n");
    }

    #[test]
    fn own_line_comment_between_statements() {
        assert_eq!(fmt("a;\n// note\nb"), "a;\n// note\nb;\n");
    }
}
