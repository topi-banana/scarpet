//! Static analysis for Scarpet: a high-level IR and a type inference pass.
//!
//! ```text
//! source → scarpet-syntax::parse → rowan tree → lower → Hir → infer → Ty + Diagnostic
//! ```
//!
//! Sibling to `scarpet-fmt`, not built on it: the formatter consumes the
//! lossless `Cst`, while this crate consumes the typed rowan layer
//! (`scarpet_syntax::nodes`), because that is the only view of a program that
//! carries source positions. Everything here is `wasm`-clean and free of file
//! I/O, so the playground, a language server, and a future compiler can all
//! depend on it.
//!
//! ```
//! let analysis = scarpet_hir::analyze("fib(n) -> if(n < 2, n, fib(n-1) + fib(n-2)); fib(10)");
//! assert_eq!(analysis.ty(analysis.hir().unwrap().root()).to_string(), "int");
//! assert!(analysis.diagnostics().is_empty());
//! ```
//!
//! # What the analysis is for
//!
//! Scarpet is very nearly total. `1 + [1, 2]` is `"1[1, 2]"`, `'hello' - 1` is
//! `"hello"`, and indexing a non-container yields `null` rather than failing. A
//! checker built to find type *errors* would find almost none. So, in order of
//! value, this crate exists to:
//!
//! 1. **Describe.** Say that an expression is a `string`. That powers the
//!    playground's type view, and hover and inlay hints in an editor. This is
//!    the product.
//! 2. **Check names and arities.** Real, common, statically decidable, and it
//!    needs no lattice at all.
//! 3. **Warn about the totality.** Point at the arithmetic that silently
//!    becomes text. See [`LintCode::StringFallback`].
//!
//! # Guarantees
//!
//! [`analyze`] is **total**: it never fails and never panics. A parse error
//! becomes a [`Diagnostic`] and leaves [`Analysis::hir`] empty; everything else
//! degrades to [`Ty::Unknown`] rather than guessing.
//!
//! Four things are knowingly not modelled, and each is a decision rather than a
//! gap: storage-slot aliasing (`b = a = 1` shares one slot in the VM), branch
//! merging (assignment is flow-sensitive and last-write-wins), `outer(x)`
//! capture types, and `global_` variables. The `infer` module's header explains
//! why in each case; `README.md` lists them alongside the rest.
//!
//! # Relationship to `scarpet-vm`
//!
//! None, by design: a compiler must not depend on an interpreter. The operator
//! rules in `infer::ops` are a deliberate port of the VM's `value/*.rs`, with
//! each rule naming its source file, so the two are kept honest by review rather
//! than by a dependency edge.

pub mod builtins;
pub mod diagnostic;
mod dump;
pub mod hir;
mod infer;
mod lower;
pub mod span;
pub mod ty;

pub use diagnostic::{Diagnostic, LintCode, Related, Severity};
pub use dump::walk;
pub use hir::{AssignOp, BinOp, Expr, ExprId, FnDef, FnId, Hir, MapEntry, Name, PrefixOp, SeqSep};
pub use span::Span;
pub use ty::{OpaqueTy, Ty, join, join_all};

use scarpet_syntax::syntax::SyntaxNode;

/// Everything one analysis learned about a program.
#[derive(Clone, Debug)]
pub struct Analysis {
    hir: Option<Hir>,
    tys: Vec<Ty>,
    diagnostics: Vec<Diagnostic>,
}

/// Parse and type-analyse `src`.
///
/// Total: a syntax error becomes a [`LintCode::Parse`] diagnostic and leaves
/// [`hir`](Analysis::hir) empty. Nothing here returns a `Result`, which is what
/// lets a front end render analysis findings and parse errors through one path
/// instead of two.
pub fn analyze(src: &str) -> Analysis {
    match scarpet_syntax::parser::parse(src) {
        Ok(tree) => analyze_syntax(&tree),
        Err(error) => Analysis {
            hir: None,
            tys: Vec::new(),
            diagnostics: vec![Diagnostic::from_parse_error(&error)],
        },
    }
}

/// Analyse an already-parsed tree.
///
/// Produces no parse diagnostics. For a language server that keeps syntax trees
/// across keystrokes and does not want to re-parse to re-analyse.
pub fn analyze_syntax(root: &SyntaxNode) -> Analysis {
    let hir = lower::lower(root);
    let result = infer::infer(&hir);
    Analysis {
        hir: Some(hir),
        tys: result.tys,
        diagnostics: result.diagnostics,
    }
}

impl Analysis {
    /// The lowered program, or `None` when the source did not parse.
    pub fn hir(&self) -> Option<&Hir> {
        self.hir.as_ref()
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Whether anything at [`Severity::Error`] was reported.
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    /// The inferred type of `id`.
    ///
    /// Total: an id from a different `Analysis` reads as
    /// [`Unknown`](Ty::Unknown) rather than panicking, because a front end that
    /// re-analyses on every keystroke will inevitably hold a stale one.
    pub fn ty(&self, id: ExprId) -> &Ty {
        self.tys.get(id.index()).unwrap_or(&Ty::Unknown)
    }

    /// The innermost expression covering `offset`, and its type — the entry
    /// point for hover and inlay hints.
    pub fn ty_at(&self, offset: u32) -> Option<(ExprId, &Ty)> {
        let id = self.hir.as_ref()?.expr_at(offset)?;
        Some((id, self.ty(id)))
    }

    /// An indented, type-annotated rendering of the whole program.
    pub fn dump(&self) -> String {
        match &self.hir {
            Some(hir) => dump::dump(hir, &self.tys),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The type of the whole program.
    fn ty_of(src: &str) -> String {
        let analysis = analyze(src);
        let hir = analysis.hir().expect("parsed");
        analysis.ty(hir.root()).to_string()
    }

    fn codes(src: &str) -> Vec<LintCode> {
        analyze(src)
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect()
    }

    #[test]
    fn literals_and_operators() {
        assert_eq!(ty_of("1"), "int");
        assert_eq!(ty_of("1.5"), "double");
        assert_eq!(ty_of("'hi'"), "string");
        assert_eq!(ty_of("1 + 2"), "int");
        // Division is always a double, exponentiation too.
        assert_eq!(ty_of("4 / 2"), "double");
        assert_eq!(ty_of("2 ^ 10"), "double");
        assert_eq!(ty_of("1 < 2"), "bool");
        assert_eq!(ty_of("!1"), "bool");
        assert_eq!(ty_of("[1, 2]"), "list<int>");
        assert_eq!(ty_of("[1, 'a']"), "list<int | string>");
        assert_eq!(ty_of("{'a' -> 1}"), "map<string, int>");
        assert_eq!(ty_of("null"), "null");
        assert_eq!(ty_of("true"), "bool");
    }

    #[test]
    fn a_sequence_takes_its_last_value() {
        assert_eq!(ty_of("1; 'two'"), "string");
        assert_eq!(ty_of("a = 1; a"), "int");
    }

    #[test]
    fn assignment_is_flow_sensitive() {
        assert_eq!(ty_of("a = 1; a = 'two'; a"), "string");
        assert_eq!(ty_of("a = 1; a += 2; a"), "int");
        // `+=` follows the `+` rule, so it can change the type.
        assert_eq!(ty_of("a = 1; a += 'x'; a"), "string");
    }

    #[test]
    fn destructuring_binds_element_types() {
        assert_eq!(ty_of("[a, b] = [1, 2]; a"), "int");
        assert_eq!(ty_of("[a, ...rest] = [1, 2, 3]; rest"), "list<int>");
    }

    /// `if` joins its branches, and can fall through to null without a default.
    #[test]
    fn if_joins_its_branches() {
        assert_eq!(ty_of("if(1, 'a')"), "string | null");
        assert_eq!(ty_of("if(1, 'a', 'b')"), "string");
        assert_eq!(ty_of("if(1, 'a', 2, 'b', 'c')"), "string");
        assert_eq!(ty_of("if(1, 1, 2, 'b')"), "int | string | null");
    }

    #[test]
    fn iterating_forms_bind_the_underscore_variables() {
        // `for` returns the count of truthy iterations.
        assert_eq!(ty_of("for([1, 2], _)"), "int");
        assert_eq!(ty_of("map(['a'], _)"), "list<string>");
        assert_eq!(ty_of("filter([1, 2], _)"), "list<int>");
        assert_eq!(ty_of("first([1, 2], _)"), "int | null");
        assert_eq!(ty_of("all([1, 2], _)"), "bool");
        assert_eq!(ty_of("reduce([1, 2], _a + _, 0)"), "int");
        // `_` leaks out of the loop, exactly as it does in the VM.
        assert_eq!(ty_of("for(['a'], _); _"), "string");
    }

    #[test]
    fn builtin_return_types() {
        assert_eq!(ty_of("type(1)"), "string");
        assert_eq!(ty_of("str(1)"), "string");
        assert_eq!(ty_of("length([1])"), "int");
        assert_eq!(ty_of("range(10)"), "list<number>");
        assert_eq!(ty_of("split('a', 'b')"), "list<string>");
        // `print` hands back its message.
        assert_eq!(ty_of("print('hi')"), "string");
        assert_eq!(ty_of("copy([1])"), "list<int>");
        assert_eq!(ty_of("sort([1, 2])"), "list<int>");
    }

    #[test]
    fn a_definition_evaluates_to_its_own_name() {
        assert_eq!(ty_of("f(x) -> x"), "string");
    }

    /// The headline result: a recursive function's return type, which needs both
    /// halves of the fixpoint — call sites flowing in, the body flowing out.
    #[test]
    fn recursion_converges() {
        let src = "fib(n) -> if(n < 2, n, fib(n-1) + fib(n-2)); fib(10)";
        assert_eq!(ty_of(src), "int");

        // Mutual recursion converges the same way.
        let src = "even(n) -> if(n == 0, true, odd(n - 1)); \
                   odd(n) -> if(n == 0, false, even(n - 1)); \
                   even(10)";
        assert_eq!(ty_of(src), "bool");
    }

    /// A function that can only call itself never produces a value, and the
    /// bottom of the lattice says exactly that.
    #[test]
    fn a_function_that_cannot_return_is_never() {
        assert_eq!(ty_of("f() -> f(); f()"), "never");
    }

    #[test]
    fn user_definitions_shadow_builtins() {
        // The user's `type` takes two arguments; the builtin takes one.
        assert_eq!(ty_of("type(a, b) -> a; type(1, 2)"), "int");
        assert_eq!(codes("type(a, b) -> a; type(1)"), [LintCode::WrongArgCount]);
    }

    #[test]
    fn forward_references_and_nested_definitions_resolve() {
        assert_eq!(ty_of("f() -> g(); g() -> 1; f()"), "int");
        // A definition nested in another body is still global.
        assert_eq!(
            ty_of("outer_fn() -> (inner() -> 2); outer_fn(); inner()"),
            "int"
        );
    }

    #[test]
    fn arity_is_checked_for_users_and_typed_builtins() {
        assert_eq!(codes("f(a, b) -> a; f(1)"), [LintCode::WrongArgCount]);
        assert!(codes("f(a, ...rest) -> a; f(1, 2, 3)").is_empty());
        assert_eq!(codes("type(1, 2)"), [LintCode::WrongArgCount]);
        assert!(codes("min(1, 2, 3, 4)").is_empty());
    }

    /// What a `wrong-arg-count` actually says. Worth pinning: no corpus file
    /// trips this path, so nothing else covers the wording or the label.
    #[test]
    fn a_wrong_arg_count_names_the_signature_and_the_definition() {
        let message = |src| {
            let analysis = analyze(src);
            let [diagnostic] = analysis.diagnostics() else {
                panic!("expected exactly one finding for {src}");
            };
            (diagnostic.message.clone(), diagnostic.related.clone())
        };

        let (text, related) = message("f(a, b) -> a; f(1)");
        assert_eq!(text, "`f` takes 2 arguments, but 1 were given");
        assert_eq!(related.len(), 1, "the definition is pointed at");
        assert_eq!(related[0].message, "`f` is defined here");

        // The other two phrasings a definition can produce.
        assert_eq!(
            message("f(a) -> a; f(1, 2)").0,
            "`f` takes 1 argument, but 2 were given"
        );
        assert_eq!(
            message("f(a, ...rest) -> a; f()").0,
            "`f` takes at least 1 arguments, but 0 were given"
        );
        // A builtin is described from its own arity table, with no definition
        // to point at.
        let (text, related) = message("type(1, 2)");
        assert_eq!(text, "`type` takes 1 argument, but 2 were given");
        assert!(related.is_empty());
    }

    #[test]
    fn unknown_functions_are_reported_but_documented_ones_are_not() {
        assert_eq!(codes("no_such_function(1)"), [LintCode::UnknownFunction]);
        // Documented but untyped — the whole Minecraft API lands here.
        assert!(codes("block_state('stone')").is_empty());
    }

    /// Scarpet apps span files. A name `import` brings in is defined somewhere
    /// this analysis cannot see, so it must not be accused.
    #[test]
    fn imported_names_are_known() {
        assert!(codes("import('math', 'euclidean'); euclidean(1, 2)").is_empty());
        assert_eq!(
            codes("import('math', 'euclidean'); manhattan(1, 2)"),
            [LintCode::UnknownFunction]
        );
        // An import that does not spell out its names brings in everything.
        assert!(codes("import('math'); anything(1)").is_empty());
        assert!(
            !codes("m = 'math'; n = 'f'; import(m, n); anything(1)")
                .contains(&LintCode::UnknownFunction)
        );
    }

    /// `f(...args)` spreads a list into the call, so the written argument count
    /// says nothing about how many actually arrive.
    #[test]
    fn a_spread_argument_suspends_the_arity_check() {
        assert!(codes("f(a, b) -> a; f(...[1, 2])").is_empty());
        assert!(codes("args = [1]; type(...args)").is_empty());
        // A spread alongside plain arguments is just as uncountable.
        assert!(codes("f(a, b) -> a; rest = [2]; f(1, ...rest)").is_empty());
    }

    /// A union operand distributes, so a value that is *sometimes* a number
    /// still gets arithmetic — and the string-fallback lint stays quiet.
    #[test]
    fn operators_distribute_over_unions() {
        let src = "a = if(1, 1); a + 1";
        assert_eq!(ty_of(src), "int | string");
        assert!(codes(src).is_empty());
    }

    #[test]
    fn expected_number_fires_only_on_proof() {
        assert_eq!(codes("'a' % 2"), [LintCode::ExpectedNumber]);
        assert_eq!(codes("-'a'"), [LintCode::ExpectedNumber]);
        assert_eq!(codes("[1, 2]:'a'"), [LintCode::ExpectedNumber]);
        // A boolean coerces, and an unknown is never accused.
        assert!(codes("true % 2").is_empty());
        assert!(codes("f(x) -> x % 2; f(1)").is_empty());
    }

    /// Building a message with `+` is idiomatic Scarpet, so it must stay quiet;
    /// arithmetic that silently becomes text must not.
    #[test]
    fn string_fallback_is_narrow() {
        assert!(codes("print('x = ' + 1)").is_empty());
        assert!(codes("'a' * 3").is_empty());
        assert_eq!(codes("1 + [1, 2]"), [LintCode::StringFallback]);
        assert_eq!(codes("1 + null"), [LintCode::StringFallback]);
    }

    #[test]
    fn read_before_write_is_suppressed_where_it_would_be_noise() {
        assert_eq!(codes("a"), [LintCode::ReadBeforeWrite]);
        assert!(codes("a = 1; a").is_empty());
        // Parameters, captures, `global_`, and the underscore convention.
        assert!(codes("f(a) -> a; f(1)").is_empty());
        assert!(codes("o = 1; f(outer(o)) -> o; f()").is_empty());
        assert!(codes("global_x").is_empty());
        assert!(codes("_i").is_empty());
        // A dynamic write could have named anything.
        assert!(codes("var('x' + 1) = 2; a").is_empty());
    }

    #[test]
    fn a_bare_map_entry_must_be_a_pair() {
        assert!(codes("{[1, 2]}").is_empty());
        assert_eq!(codes("{[1, 2, 3]}"), [LintCode::MapEntryNotPair]);
    }

    #[test]
    fn a_parse_error_becomes_a_diagnostic_rather_than_a_failure() {
        let analysis = analyze("[1, 2");
        assert!(analysis.hir().is_none());
        assert!(analysis.has_errors());
        assert_eq!(analysis.diagnostics()[0].code, LintCode::Parse);
        assert_eq!(analysis.dump(), "");
    }

    #[test]
    fn ty_at_finds_the_expression_under_a_cursor() {
        let src = "a = 'hi'; length(a)";
        let analysis = analyze(src);
        // Offset 17 is the `a` inside `length(...)`.
        let (_, ty) = analysis.ty_at(17).expect("a node covers the argument");
        assert_eq!(ty.to_string(), "string");
        // A stale id from another analysis reads as unknown, never panics.
        let other = analyze("1");
        let root = other.hir().expect("parsed").root();
        assert_eq!(analyze("").ty(root), &Ty::Unknown);
    }

    #[test]
    fn the_dump_is_an_indented_tree() {
        let dump = analyze("a = 1 + 2").dump();
        let lines: Vec<&str> = dump.lines().collect();
        assert!(lines[0].starts_with("Assign `=`"));
        assert!(lines[0].ends_with("int"));
        assert!(lines[1].starts_with("  Name a"));
        assert!(lines[2].starts_with("  Bin `+`"));
        assert!(lines[3].starts_with("    Int 1"));
    }

    /// Every expression must be reachable from the root, function bodies
    /// included, or a tree view would silently hide code.
    #[test]
    fn the_walk_covers_the_whole_arena() {
        let analysis = analyze("f(x) -> x + 1; f(2)");
        let hir = analysis.hir().expect("parsed");
        assert_eq!(walk(hir).len(), hir.len());
    }

    #[test]
    fn analysis_is_deterministic() {
        let src = "f(n) -> if(n, n, f(n - 1)); g = f(3); [g, g]";
        assert_eq!(analyze(src).dump(), analyze(src).dump());
    }
}

/// Run the whole `example/` corpus through the analysis to prove it is total.
///
/// Skips quietly when the submodule is not checked out, so a green `cargo test`
/// locally does not prove this ran — confirm with
/// `git submodule update --init --recursive`.
#[cfg(test)]
mod corpus {
    use super::*;
    use std::path::{Path, PathBuf};

    fn corpus_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("example")
    }

    fn walk_sc(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_sc(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("sc") {
                out.push(path);
            }
        }
    }

    /// No `KNOWN_BAD` list is needed: [`analyze`] is total, so a file the parser
    /// rejects simply yields one diagnostic and no HIR, which is a valid result
    /// to assert about.
    #[test]
    fn the_analysis_is_total_over_the_corpus() {
        let root = corpus_root();
        if !root.is_dir() {
            eprintln!(
                "skipping corpus test: {} absent (run `git submodule update --init`)",
                root.display()
            );
            return;
        }
        let mut files = Vec::new();
        walk_sc(&root, &mut files);
        files.sort();
        assert!(!files.is_empty(), "the corpus should not be empty");

        for file in &files {
            let Ok(src) = std::fs::read_to_string(file) else {
                continue;
            };
            let name = file.display();
            let analysis = analyze(&src);
            let Some(hir) = analysis.hir() else {
                assert!(
                    !analysis.diagnostics().is_empty(),
                    "{name}: no HIR and no diagnostic"
                );
                continue;
            };

            assert_eq!(
                hir.len(),
                analysis.tys.len(),
                "{name}: type table is ragged"
            );
            for id in hir.ids() {
                let span = hir.span(id);
                assert!(span.start <= span.end, "{name}: reversed span {span:?}");
                assert!(
                    span.end as usize <= src.len(),
                    "{name}: {span:?} escapes the source"
                );
                for child in hir.children(id) {
                    assert!(child.index() < hir.len(), "{name}: dangling child");
                }
            }
            for def in hir.fns() {
                assert!(def.body.index() < hir.len(), "{name}: dangling body");
            }
            for diagnostic in analysis.diagnostics() {
                assert!(
                    diagnostic.span.end as usize <= src.len(),
                    "{name}: diagnostic escapes the source"
                );
            }
            assert!(!analysis.dump().is_empty(), "{name}: empty dump");
        }
    }
}
