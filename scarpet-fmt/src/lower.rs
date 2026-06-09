//! Lowering from the `scarpet-syntax` CST to the [`Doc`] IR.
//!
//! The traversal mirrors the shape of `strip_leading` in the parser: one arm
//! per `CstKind`, recursing into the same children. Each node's leading trivia
//! is emitted by [`Lowerer::expr`] (own-line comments) or lifted onto the
//! preceding token by [`Lowerer::child_after`] (same-line / trailing comments).
//! Blank lines are reconstructed as statement separators.

use scarpet_syntax::parser::{BinOp, Cst, CstKind, Trivia, UnaryOp};

use crate::doc::{
    Doc, blank_lines, comment, concat, group, hardline, if_break, join, line, nest, nil, softline,
    space, text,
};
use crate::trivia::{blank_lines_before, has_comment, own_line_comments, same_line_comment};
use crate::{BinopSeparator, BraceStyle, Config, TrailingComma};

/// Lower a whole program (the CST root) to a document.
pub fn program(root: &Cst, config: &Config) -> Doc {
    Lowerer { config }.expr(root)
}

/// Threads the [`Config`] through the recursive lowering so layout knobs (like
/// [`BraceStyle`]) are reachable at every node.
struct Lowerer<'a> {
    config: &'a Config,
}

impl Lowerer<'_> {
    /// Lower a node, prefixing its own-line leading comments. Used wherever the
    /// node is *not* preceded by a separator we can lift a trailing comment onto.
    fn expr(&self, cst: &Cst) -> Doc {
        concat([own_line_comments(&cst.leading), self.node_body(cst)])
    }

    /// Place `cst` after the separator `sep`. If `cst`'s leading begins with a
    /// same-line (trailing) comment, emit it *before* `sep` so it hugs the
    /// previous token, then hard-break; otherwise emit `sep` followed by any
    /// own-line comments. This is what keeps `a; // note` on one line.
    fn child_after(&self, sep: Doc, cst: &Cst) -> Doc {
        match same_line_comment(&cst.leading) {
            Some(c) => concat([
                space(),
                comment(c.to_string()),
                hardline(),
                own_line_comments(&cst.leading[1..]),
                self.node_body(cst),
            ]),
            None => concat([sep, own_line_comments(&cst.leading), self.node_body(cst)]),
        }
    }

    /// Lower a node's `kind`, ignoring its own leading (handled by the caller).
    fn node_body(&self, cst: &Cst) -> Doc {
        match &cst.kind {
            CstKind::Number(s) | CstKind::Str(s) | CstKind::Ident(s) => text(s.to_string()),
            CstKind::Unary { op, operand } => self.unary(*op, operand),
            CstKind::Binary { op, lhs, rhs } => match *op {
                BinOp::Comma => self.comma_chain(cst),
                BinOp::Semi => self.semi_chain(cst),
                BinOp::Arrow => self.arrow_chain(cst),
                BinOp::Get => self.tight(*op, lhs, rhs),
                _ => self.spaced(*op, lhs, rhs),
            },
            CstKind::Call { callee, args } => self.call(callee, args),
            CstKind::List(items) => self.collection("[", items, "]", false),
            CstKind::Map(items) => self.collection("{", items, "}", false),
            CstKind::Paren(inner) => self.paren(inner, false),
            CstKind::Empty => nil(),
        }
    }

    // ---- operators -----------------------------------------------------

    /// A binary operator with no surrounding space (`a:b`).
    fn tight(&self, op: BinOp, lhs: &Cst, rhs: &Cst) -> Doc {
        concat([self.expr(lhs), text(bin_op_str(op)), self.expr(rhs)])
    }

    /// A binary operator spaced on each side, breakable before the RHS
    /// (`a + b`, breaking to `a +` ⏎ `b` when it doesn't fit).
    fn spaced(&self, op: BinOp, lhs: &Cst, rhs: &Cst) -> Doc {
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
                self.expr(lhs),
                space(),
                comment(c.to_string()),
                hardline(),
                text(bin_op_str(op)),
                space(),
                own_line_comments(&rhs.leading[1..]),
                self.node_body(rhs),
            ]);
        }
        let op_doc = text(bin_op_str(op));
        // `Front` moves the operator to the head of the wrapped line. It only
        // applies to the non-assignment operators (an assignment's RHS reads
        // better hugging the `=` line) and only when the RHS carries no leading
        // comment: the operator's leading trivia binds to the RHS on re-parse,
        // so a comment there would rebind to a different anchor under `Front` and
        // break idempotency. A comment leaves it to the `Back` path below, where
        // `child_after` already lifts both comment shapes non-destructively. The
        // flat rendering is byte-identical to `Back` (`a op b`), so the
        // break/flat fit decision — and thus the corpus output — is unchanged.
        if self.config.binop_separator == BinopSeparator::Front
            && !is_assign_like(op)
            && !has_comment(&rhs.leading)
        {
            return group(concat([
                self.expr(lhs),
                line(),
                op_doc,
                space(),
                self.expr(rhs),
            ]));
        }
        group(concat([
            self.expr(lhs),
            space(),
            op_doc,
            self.child_after(line(), rhs),
        ]))
    }

    /// A prefix unary operator (`-x`, `!x`, `...xs`).
    fn unary(&self, op: UnaryOp, operand: &Cst) -> Doc {
        concat([text(unary_op_str(op)), self.expr(operand)])
    }

    // ---- chains --------------------------------------------------------

    /// A `,`-separated chain, flat (`a, b, c`) or one-per-line when wide.
    fn comma_chain(&self, cst: &Cst) -> Doc {
        group(self.comma_separated(&flatten_left(BinOp::Comma, cst)))
    }

    /// A `;`-separated statement sequence.
    fn semi_chain(&self, cst: &Cst) -> Doc {
        self.statement_seq(&flatten_left(BinOp::Semi, cst))
    }

    /// A one-per-line, `;`-terminated statement sequence, preserving blank-line
    /// separators. Shared by the top level and paren blocks. (A trailing `;` is
    /// dropped on re-parse, so synthesizing one is non-destructive.)
    fn statement_seq(&self, stmts: &[&Cst]) -> Doc {
        let mut parts = Vec::new();
        for (i, s) in stmts.iter().enumerate() {
            if i == 0 {
                parts.push(self.expr(s));
            } else {
                parts.push(self.child_after(self.blank_separator(&s.leading), s));
            }
            parts.push(text(";"));
        }
        concat(parts)
    }

    /// The separator before a statement: a plain [`hardline`] when no blank line
    /// is wanted, else `n` [`blank_lines`]. The source's blank-line count is
    /// clamped into `[blank_lines_lower_bound, blank_lines_upper_bound]` —
    /// truncating long runs (the upper bound, default 1) and inserting blanks
    /// between adjacent statements (the lower bound, default 0). The
    /// `.min(upper).max(lower)` order lets the lower bound win under a
    /// misconfigured `lower > upper`, which the CLI rejects anyway.
    ///
    /// Note this is bypassed when the statement's line begins with a trailing
    /// comment lifted onto the previous token: [`Lowerer::child_after`] then
    /// drops the separator entirely, so the lower bound is not enforced across
    /// such a comment (its leading break run is empty, so the count is 0 too).
    fn blank_separator(&self, leading: &[Trivia]) -> Doc {
        let n = blank_lines_before(leading)
            .min(self.config.blank_lines_upper_bound)
            .max(self.config.blank_lines_lower_bound);
        if n == 0 { hardline() } else { blank_lines(n) }
    }

    /// A right-associative `->` chain (`a -> b -> c`). The arrow stays on the
    /// signature line; the RHS handles its own breaking. The final operand is the
    /// body, lowered via [`Lowerer::arrow_body`] so its opening delimiter can hug
    /// the arrow.
    fn arrow_chain(&self, cst: &Cst) -> Doc {
        let parts = flatten_right(BinOp::Arrow, cst);
        let last = parts.len() - 1;
        let docs = parts.into_iter().enumerate().map(|(i, p)| {
            if i == last {
                self.arrow_body(p)
            } else {
                self.expr(p)
            }
        });
        join(docs, text(" -> "))
    }

    /// Lower the right-hand side of a `->` (a lambda/function body, or a map
    /// value). Its opening delimiter hugs the arrow, so under
    /// [`BraceStyle::NextLine`] it moves onto its own line. Mirrors
    /// [`Lowerer::expr`] for the leading-comment handling.
    fn arrow_body(&self, cst: &Cst) -> Doc {
        let body = match &cst.kind {
            CstKind::Paren(inner) => self.paren(inner, true),
            CstKind::List(items) => self.collection("[", items, "]", true),
            CstKind::Map(items) => self.collection("{", items, "}", true),
            _ => return self.expr(cst),
        };
        concat([own_line_comments(&cst.leading), body])
    }

    // ---- calls / collections / parens ----------------------------------

    /// A call `callee(args)`. The argument list hugs the callee, so under
    /// [`BraceStyle::NextLine`] its `(` moves onto its own line.
    fn call(&self, callee: &Cst, args: &[Cst]) -> Doc {
        concat([self.expr(callee), self.collection("(", args, ")", true)])
    }

    /// A delimited, comma-separated sequence (`(...)`, `[...]`, `{...}`). Laid
    /// out flat if it fits, else one item per line with a trailing comma. `hug`
    /// says whether the opening delimiter hugs a preceding head (see
    /// [`Lowerer::open_delim`]).
    fn collection(&self, open: &'static str, items: &[Cst], close: &'static str, hug: bool) -> Doc {
        if items.is_empty() {
            return concat([text(open), text(close)]);
        }
        let body = self.comma_separated(&items.iter().collect::<Vec<_>>());
        group(concat([
            self.open_delim(open, hug),
            nest(concat([softline(), body, self.trailing_comma()])),
            softline(),
            text(close),
        ]))
    }

    /// Join items with `, ` (breakable after each comma), lifting trailing
    /// comments of the 2nd+ items onto the preceding comma.
    fn comma_separated(&self, items: &[&Cst]) -> Doc {
        let mut parts = Vec::new();
        for (i, item) in items.iter().enumerate() {
            if i == 0 {
                parts.push(self.expr(item));
            } else {
                parts.push(text(","));
                parts.push(self.child_after(line(), item));
            }
        }
        concat(parts)
    }

    /// A parenthesized node. Parens are always preserved. A `;`-chain inside is a
    /// statement block (one statement per line); anything else is an expression
    /// paren that may break softly. `hug` says whether the `(` hugs a preceding
    /// head (true for an arrow body — see [`Lowerer::open_delim`]).
    fn paren(&self, inner: &Cst, hug: bool) -> Doc {
        if is_semi_chain(inner) {
            let stmts = flatten_left(BinOp::Semi, inner);
            group(concat([
                self.open_delim("(", hug),
                nest(concat([hardline(), self.statement_seq(&stmts)])),
                hardline(),
                text(")"),
            ]))
        } else {
            group(concat([
                self.open_delim("(", hug),
                nest(concat([softline(), self.expr(inner)])),
                softline(),
                text(")"),
            ]))
        }
    }

    /// The opening delimiter `open`. When it `hug`s a preceding head (a callee or
    /// a `->`) and [`BraceStyle::NextLine`] is set, a soft break is emitted before
    /// it so it starts a fresh line once the enclosing group breaks; otherwise it
    /// is emitted bare.
    ///
    /// A block reached *after* a separator that already breaks — an operator's
    /// `line()` (`x =` ⏎ `[…]`) or a comma/statement break — is not hugging, so it
    /// passes `hug = false` and keeps the original layout. That avoids stacking a
    /// second break on top of the separator's, which would leave a blank line.
    /// (The soft break is nil while flat, so a block that fits is unaffected and
    /// the break/flat choice is unchanged.)
    fn open_delim(&self, open: &'static str, hug: bool) -> Doc {
        if hug && self.config.brace_style == BraceStyle::NextLine {
            concat([softline(), text(open)])
        } else {
            text(open)
        }
    }

    /// The trailing comma after the last item of a [`Lowerer::collection`], per
    /// [`TrailingComma`]: only when broken (`Vertical`), in both layouts
    /// (`Always`, so a flat `[1, 2,]` keeps its comma), or never (`Never`).
    /// `Always` emits an unconditional comma rather than an `if_break`, so it
    /// shows in the flat rendering too — that keeps the flat/broken fit decision,
    /// and thus the output, idempotent.
    fn trailing_comma(&self) -> Doc {
        match self.config.trailing_comma {
            TrailingComma::Vertical => if_break(text(","), nil()),
            TrailingComma::Always => text(","),
            TrailingComma::Never => nil(),
        }
    }
}

/// Whether `cst` is a leaf whose leftmost token is the node itself — so a
/// comment placed after a preceding operator re-parses back onto it.
fn is_atom(cst: &Cst) -> bool {
    matches!(
        &cst.kind,
        CstKind::Number(_) | CstKind::Str(_) | CstKind::Ident(_) | CstKind::Empty
    )
}

/// Whether `op` is an assignment-like operator (`=`, `+=`, `<>`). These keep the
/// operator in tail position regardless of [`BinopSeparator`], so their RHS hugs
/// the operator line (`x =` ⏎ `value`) rather than dropping the `=` below.
fn is_assign_like(op: BinOp) -> bool {
    matches!(op, BinOp::Assign | BinOp::AddAssign | BinOp::Swap)
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
    use crate::{BinopSeparator, BraceStyle, Config, TrailingComma, format_source};

    fn fmt(src: &str) -> String {
        format_source(src, &Config::default()).unwrap()
    }

    fn fmt_nl(src: &str) -> String {
        format_source(
            src,
            &Config {
                brace_style: BraceStyle::NextLine,
                ..Config::default()
            },
        )
        .unwrap()
    }

    fn fmt_comment_width(src: &str, width: usize) -> String {
        format_source(
            src,
            &Config {
                comment_width: Some(width),
                ..Config::default()
            },
        )
        .unwrap()
    }

    fn fmt_tc(src: &str, trailing_comma: TrailingComma) -> String {
        format_source(
            src,
            &Config {
                trailing_comma,
                ..Config::default()
            },
        )
        .unwrap()
    }

    fn fmt_bs(src: &str, binop_separator: BinopSeparator) -> String {
        format_source(
            src,
            &Config {
                binop_separator,
                ..Config::default()
            },
        )
        .unwrap()
    }

    fn fmt_bl(src: &str, upper: usize, lower: usize) -> String {
        format_source(
            src,
            &Config {
                blank_lines_upper_bound: upper,
                blank_lines_lower_bound: lower,
                ..Config::default()
            },
        )
        .unwrap()
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

    // ---- trailing comma --------------------------------------------

    #[test]
    fn trailing_comma_vertical_matches_the_default() {
        // `Vertical` is the default: a comma only when the collection breaks.
        assert_eq!(fmt_tc("[1, 2, 3]", TrailingComma::Vertical), "[1, 2, 3]\n");
        assert_eq!(fmt("[1, 2, 3]"), "[1, 2, 3]\n");
    }

    #[test]
    fn trailing_comma_always_adds_to_flat_collections() {
        assert_eq!(fmt_tc("[1, 2, 3]", TrailingComma::Always), "[1, 2, 3,]\n");
        assert_eq!(fmt_tc("f(a, b)", TrailingComma::Always), "f(a, b,)\n");
        assert_eq!(
            fmt_tc("{'a' -> 1, 'b' -> 2}", TrailingComma::Always),
            "{'a' -> 1, 'b' -> 2,}\n"
        );
    }

    #[test]
    fn trailing_comma_always_leaves_empty_collections_bare() {
        // The empty-collection early return keeps a stray comma out of `[]`/`f()`.
        assert_eq!(fmt_tc("[]", TrailingComma::Always), "[]\n");
        assert_eq!(fmt_tc("f()", TrailingComma::Always), "f()\n");
    }

    #[test]
    fn trailing_comma_never_drops_it_when_broken() {
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
        for (i, n) in nums.iter().enumerate() {
            // `Never` drops only the trailing comma; the separators between
            // items remain, so every line but the last keeps its comma.
            let sep = if i + 1 < nums.len() { "," } else { "" };
            expected.push_str(&format!("    {n}{sep}\n"));
        }
        expected.push_str("]\n");
        assert_eq!(fmt_tc(&src, TrailingComma::Never), expected);
    }

    // ---- binop separator -------------------------------------------

    #[test]
    fn binop_separator_defaults_to_back() {
        // `Back` is the default, and a flat expression is identical either way.
        assert_eq!(fmt("a + b"), "a + b\n");
        assert_eq!(fmt_bs("a + b", BinopSeparator::Back), "a + b\n");
        assert_eq!(fmt_bs("a + b", BinopSeparator::Front), "a + b\n");
    }

    #[test]
    fn binop_separator_back_breaks_operator_at_tail() {
        // Too wide to fit: `Back` leaves the operator at the end of the line.
        let a = "a".repeat(60);
        let b = "b".repeat(60);
        let src = format!("{a} + {b}");
        assert_eq!(fmt_bs(&src, BinopSeparator::Back), format!("{a} +\n{b}\n"));
    }

    #[test]
    fn binop_separator_front_breaks_operator_at_head() {
        // The same expression under `Front`: the operator leads the wrapped line.
        let a = "a".repeat(60);
        let b = "b".repeat(60);
        let src = format!("{a} + {b}");
        assert_eq!(fmt_bs(&src, BinopSeparator::Front), format!("{a}\n+ {b}\n"));
    }

    #[test]
    fn binop_separator_front_keeps_assignment_back() {
        // Assignment-like operators stay `Back` even under `Front`, so the RHS
        // keeps hugging the operator line rather than dropping the `=` below.
        let a = "a".repeat(60);
        let b = "b".repeat(60);
        assert_eq!(
            fmt_bs(&format!("{a} = {b}"), BinopSeparator::Front),
            format!("{a} =\n{b}\n")
        );
        assert_eq!(
            fmt_bs(&format!("{a} += {b}"), BinopSeparator::Front),
            format!("{a} +=\n{b}\n")
        );
    }

    #[test]
    fn binop_separator_front_breaks_only_the_wide_group_in_a_chain() {
        // `((a + b) + c)`: the outer chain overflows and breaks, but the inner
        // `a + b` still fits, so only the outer operator leads a new line. Each
        // operator sits in its own group, so the break is per-group.
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let c = "c".repeat(40);
        let src = format!("{a} + {b} + {c}");
        assert_eq!(
            fmt_bs(&src, BinopSeparator::Front),
            format!("{a} + {b}\n+ {c}\n")
        );
    }

    #[test]
    fn binop_separator_front_inside_assignment() {
        // `x = a + b`: the outer `=` stays `Back` (RHS on its own line), and the
        // inner `+` chain breaks `Front`.
        let a = "a".repeat(50);
        let b = "b".repeat(50);
        let src = format!("x = {a} + {b}");
        assert_eq!(
            fmt_bs(&src, BinopSeparator::Front),
            format!("x =\n{a}\n+ {b}\n")
        );
    }

    #[test]
    fn binop_separator_front_is_idempotent() {
        // Formatting the broken `Front` output again must be a no-op.
        let a = "a".repeat(60);
        let b = "b".repeat(60);
        let once = fmt_bs(&format!("{a} + {b}"), BinopSeparator::Front);
        assert_eq!(fmt_bs(&once, BinopSeparator::Front), once);
    }

    // ---- blank lines -----------------------------------------------

    #[test]
    fn blank_lines_upper_bound_truncates_long_runs() {
        // Three blank lines collapse to the upper bound of two...
        assert_eq!(fmt_bl("a;\n\n\n\nb", 2, 0), "a;\n\n\nb;\n");
        // ...and to a single one under the default upper bound of one.
        assert_eq!(fmt_bl("a;\n\n\n\nb", 1, 0), "a;\n\nb;\n");
    }

    #[test]
    fn blank_lines_upper_bound_keeps_runs_within_the_cap() {
        // A run already at or under the cap is left untouched.
        assert_eq!(fmt_bl("a;\n\nb", 2, 0), "a;\n\nb;\n");
    }

    #[test]
    fn blank_lines_lower_bound_inserts_between_adjacent_statements() {
        // Adjacent statements get a blank line forced between them.
        assert_eq!(fmt_bl("a;\nb", 1, 1), "a;\n\nb;\n");
    }

    #[test]
    fn blank_lines_lower_bound_applies_inside_paren_blocks() {
        // A `(a;b)` block lowers through the same statement sequence, so the
        // lower bound inserts a blank between its statements too.
        assert_eq!(
            fmt_bl("foo()->(a;b)", 1, 1),
            "foo() -> (\n    a;\n\n    b;\n)\n"
        );
    }

    #[test]
    fn blank_lines_bounds_are_idempotent() {
        // Both truncation and insertion reach a fixpoint in one reformat.
        let upper = fmt_bl("a;\n\n\n\nb", 2, 0);
        assert_eq!(fmt_bl(&upper, 2, 0), upper);
        let lower = fmt_bl("a;\nb", 1, 1);
        assert_eq!(fmt_bl(&lower, 1, 1), lower);
    }

    // ---- brace style -----------------------------------------------

    #[test]
    fn next_line_keeps_short_blocks_inline() {
        // A block that fits on one line is unaffected by the brace style.
        assert_eq!(fmt_nl("f(a, b, c)"), "f(a, b, c)\n");
        assert_eq!(fmt_nl("[1, 2, 3]"), "[1, 2, 3]\n");
        assert_eq!(fmt_nl("{'a' -> 1, 'b' -> 2}"), "{'a' -> 1, 'b' -> 2}\n");
        assert_eq!(fmt_nl("(a + b)"), "(a + b)\n");
    }

    #[test]
    fn next_line_breaks_function_body_open_paren() {
        // A `;`-body always breaks; the opening paren moves onto its own line.
        assert_eq!(fmt_nl("foo()->(a;b)"), "foo() ->\n(\n    a;\n    b;\n)\n");
    }

    #[test]
    fn next_line_breaks_call_open_paren() {
        // A call hugs its callee, so the `(` moves onto its own line.
        let args = [
            "aaaaaaaaaa",
            "bbbbbbbbbb",
            "cccccccccc",
            "dddddddddd",
            "eeeeeeeeee",
            "ffffffffff",
            "gggggggggg",
            "hhhhhhhhhh",
            "iiiiiiiiii",
        ];
        let src = format!("call_something({})", args.join(", "));
        let mut expected = String::from("call_something\n(\n");
        for a in args {
            expected.push_str(&format!("    {a},\n"));
        }
        expected.push_str(")\n");
        assert_eq!(fmt_nl(&src), expected);
    }

    #[test]
    fn next_line_keeps_operator_rhs_without_a_blank_line() {
        // The `=` already breaks its RHS onto a fresh line; brace style must not
        // stack a second break (which would leave a blank line). A literal
        // assigned to a variable therefore lays out the same under either style.
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
        let src = format!("data = [{}]", nums.join(", "));
        let mut expected = String::from("data =\n[\n");
        for n in nums {
            expected.push_str(&format!("    {n},\n"));
        }
        expected.push_str("]\n");
        assert_eq!(fmt_nl(&src), expected);
        assert_eq!(fmt_nl(&src), fmt(&src));
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

    #[test]
    fn comment_width_wraps_own_line_comments() {
        assert_eq!(
            fmt_comment_width("// one two three four\nx", 12),
            "// one two\n// three\n// four\nx\n"
        );
    }

    #[test]
    fn comment_width_wraps_trailing_comments() {
        assert_eq!(
            fmt_comment_width("a; // one two three\nb", 14),
            "a; // one two\n// three\nb;\n"
        );
    }

    #[test]
    fn disabled_comment_width_keeps_comments_verbatim() {
        assert_eq!(
            fmt("// one two three four\nx"),
            "// one two three four\nx\n"
        );
    }
}
