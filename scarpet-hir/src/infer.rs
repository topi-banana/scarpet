//! Type inference over the [`Hir`] arena.
//!
//! Bottom-up abstract interpretation with hand-written arms for the special
//! forms, wrapped in a fixpoint. Not bidirectional: Scarpet has no type
//! annotations, so there is never an expected type to push down.
//!
//! # The fixpoint
//!
//! Two unknowns depend on each other. A function's return type depends on its
//! parameters' types, and its parameters' types come from its call sites — which
//! for a recursive function include its own body. So both are seeded at
//! [`Ty::Never`] and raised together until they stop moving:
//!
//! ```text
//! fib(n) -> if(n < 2, n, fib(n-1) + fib(n-2)); print('fib = ' + fib(10))
//!
//! round 1  n from `fib(10)` is int; body = if(bool, int, never + never)
//!          = join(int, never) = int          -> fib returns int
//! round 2  body = if(bool, int, int + int) = int, unchanged -> converged
//! ```
//!
//! Raising from the bottom is what makes this work: seeding at `Unknown`
//! instead would be absorbing, and every recursive function would stay unknown
//! forever. Convergence is guaranteed because the lattice is finite —
//! [`MAX_DEPTH`](crate::ty::MAX_DEPTH) and [`MAX_UNION`](crate::ty::MAX_UNION)
//! bound how large a type can grow — and [`MAX_ITERATIONS`] is a backstop for
//! any rule that turns out not to be monotone.
//!
//! Diagnostics and per-expression types are recorded only on a final pass after
//! the fixpoint settles, so nothing is reported from an intermediate state.
//!
//! # What is deliberately not modelled
//!
//! - **Slot aliasing.** `b = a = 1` gives `b` *`a`'s storage slot* in the VM, so
//!   a later write to one is visible through the other. Tracking that needs
//!   union-find over slots and would change essentially no inferred type, since
//!   both sides start from the same right-hand side.
//! - **Branch merging.** Assignment is flow-sensitive and last-write-wins, so
//!   after `if(c, a = 1, a = 'x')` the analysis reports `a` as `string` rather
//!   than `int | string`.
//! - **`outer(x)` capture types.** A capture shares its slot with the defining
//!   scope in both directions, resolved when the definition runs; the body sees
//!   it as `unknown`.
//! - **`global_` variables.** Cross-scope by name prefix, never diagnosed,
//!   always `unknown`.

mod ops;

use std::collections::{HashMap, HashSet};

use crate::builtins::{self, RetRule, Signature, Special};
use crate::diagnostic::{Diagnostic, LintCode};
use crate::hir::{AssignOp, BinOp, Expr, ExprId, FnId, Hir, Name, PrefixOp};
use crate::span::Span;
use crate::ty::{Ty, join, join_all};

/// How many times the fixpoint may re-run before giving up and widening the
/// functions that are still moving. A backstop, not the expected exit: the
/// lattice is finite, so a monotone pass settles well inside this.
const MAX_ITERATIONS: usize = 8;

/// How deep the expression walk may recurse before answering `unknown`.
///
/// Lowering flattens `;` and `,`, so depth tracks genuine expression nesting
/// rather than statement count — but arithmetic still left-nests, and a
/// machine-generated `1 + 1 + …` chain has no natural bound. Past this the
/// analysis degrades instead of overflowing the stack.
const MAX_DEPTH: u32 = 512;

/// Everything one analysis learned about a program.
pub(crate) struct Inference {
    /// Parallel to the arena: the type of every expression.
    pub tys: Vec<Ty>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Infer types for `hir` and collect its diagnostics.
pub(crate) fn infer(hir: &Hir) -> Inference {
    let mut ctx = Ctx::new(hir);
    for _ in 0..MAX_ITERATIONS {
        ctx.changed = false;
        ctx.pass();
        if !ctx.changed {
            break;
        }
    }
    // One last walk, this time writing down what it finds.
    ctx.recording = true;
    ctx.pass();
    Inference {
        tys: ctx.tys,
        diagnostics: ctx.diagnostics,
    }
}

/// Scarpet's scope is flat and per-call — the VM keeps one map per invocation
/// and has no block scoping at all — so one `HashMap` per function body is a
/// faithful model rather than a simplification.
#[derive(Default)]
struct Scope {
    /// Every name written so far, and the type of what was written. Absence is
    /// what [`ReadBeforeWrite`](LintCode::ReadBeforeWrite) reports.
    vars: HashMap<Name, Ty>,
    /// Set once `var(<non-literal>)` is seen. Any name could then have been
    /// written, so unresolved reads must widen instead of being accused of
    /// being uninitialised.
    dynamic: bool,
}

struct Ctx<'h> {
    hir: &'h Hir,
    tys: Vec<Ty>,
    diagnostics: Vec<Diagnostic>,
    /// Every definition in the program, by name. Scarpet's function namespace is
    /// flat and global — a definition nested in another body is still visible
    /// everywhere, and the last one wins — so this is collected up front. Doing
    /// it up front is also what makes forward references and mutual recursion
    /// work without spurious `unknown-function`.
    fns: HashMap<Name, FnId>,
    /// Function names brought in by `import('module', 'name', …)`. They are
    /// defined in another file this analysis cannot see, so they are known
    /// without being typed.
    imported: HashSet<Name>,
    /// Whether some `import` did not spell out the names it brings in — either
    /// `import('module')` on its own or a non-literal argument. Anything the
    /// module defines is then in scope, so `unknown-function` has to stand down
    /// for the whole file.
    wildcard_import: bool,
    /// Per function, the join of every call site's argument types.
    fn_params: Vec<Vec<Ty>>,
    /// Per function, the join of everything its `...rest` binder has collected.
    fn_rest: Vec<Ty>,
    /// Per function, the join of everything its body has returned.
    fn_ret: Vec<Ty>,
    scope: Scope,
    depth: u32,
    /// Whether this pass writes types and diagnostics down.
    recording: bool,
    /// Whether this pass raised anything — the fixpoint's exit condition.
    changed: bool,
}

impl<'h> Ctx<'h> {
    fn new(hir: &'h Hir) -> Ctx<'h> {
        let mut fns = HashMap::new();
        for (index, def) in hir.fns().iter().enumerate() {
            fns.insert(def.name.clone(), FnId::from_index(index));
        }

        // Scarpet apps split across files with `import`, so a call to a name
        // this file never defines is routine. Collecting the imported names up
        // front is what keeps `unknown-function` from accusing every one of
        // them.
        let mut imported = HashSet::new();
        let mut wildcard_import = false;
        for id in hir.ids() {
            let Expr::Call { callee, args, .. } = hir.expr(id) else {
                continue;
            };
            if &**callee != "import" {
                continue;
            }
            let mut all_named = args.len() > 1;
            for arg in args.iter().skip(1) {
                match hir.expr(*arg) {
                    Expr::Str(name) => {
                        imported.insert(Name::from(&**name));
                    }
                    _ => all_named = false,
                }
            }
            wildcard_import |= !all_named;
        }

        Ctx {
            hir,
            tys: vec![Ty::Unknown; hir.len()],
            diagnostics: Vec::new(),
            fns,
            imported,
            wildcard_import,
            fn_params: hir
                .fns()
                .iter()
                .map(|def| vec![Ty::Never; def.params.len()])
                .collect(),
            fn_rest: vec![Ty::Never; hir.fns().len()],
            fn_ret: vec![Ty::Never; hir.fns().len()],
            scope: Scope::default(),
            depth: 0,
            recording: false,
            changed: false,
        }
    }

    /// One walk of the whole program: the top-level expression, then every
    /// function body in its own scope.
    fn pass(&mut self) {
        if self.recording {
            self.diagnostics.clear();
        }
        self.enter_scope(Scope::default());
        self.infer_expr(self.hir.root());
        for index in 0..self.hir.fns().len() {
            self.infer_fn(FnId::from_index(index));
        }
    }

    fn enter_scope(&mut self, scope: Scope) -> Scope {
        self.depth = 0;
        std::mem::replace(&mut self.scope, scope)
    }

    fn infer_fn(&mut self, id: FnId) {
        let hir = self.hir;
        let Some(def) = hir.fn_def(id) else {
            return;
        };
        let mut scope = Scope::default();
        for (index, param) in def.params.iter().enumerate() {
            let ty = self.fn_params[id.index()]
                .get(index)
                .cloned()
                .unwrap_or(Ty::Never);
            scope.vars.insert(param.clone(), ty);
        }
        // A capture shares the defining scope's slot, resolved when the
        // definition runs; from inside the body its type is not knowable.
        for capture in &def.captures {
            scope.vars.insert(capture.clone(), Ty::Unknown);
        }
        if let Some(rest) = &def.rest {
            scope
                .vars
                .insert(rest.clone(), Ty::list(self.fn_rest[id.index()].clone()));
        }

        let outer = self.enter_scope(scope);
        let ty = self.infer_expr(def.body);
        self.enter_scope(outer);
        self.raise_fn_ret(id, ty);
    }

    /// Raise a slot to include `ty`, remembering whether that moved it.
    fn raise(changed: &mut bool, slot: &mut Ty, ty: &Ty) {
        let next = join(slot, ty);
        if next != *slot {
            *slot = next;
            *changed = true;
        }
    }

    fn raise_fn_ret(&mut self, id: FnId, ty: Ty) {
        let changed = &mut self.changed;
        Ctx::raise(changed, &mut self.fn_ret[id.index()], &ty);
    }

    // ----------------------------------------------------------------
    // Expressions
    // ----------------------------------------------------------------

    fn infer_expr(&mut self, id: ExprId) -> Ty {
        if self.depth >= MAX_DEPTH {
            return Ty::Unknown;
        }
        self.depth += 1;
        let ty = self.infer_expr_inner(id);
        self.depth -= 1;
        if self.recording {
            self.tys[id.index()] = ty.clone();
        }
        ty
    }

    fn infer_expr_inner(&mut self, id: ExprId) -> Ty {
        // `hir` is a shared reference field, so copying it out keeps the
        // borrows below independent of `&mut self`.
        let hir = self.hir;
        match hir.expr(id) {
            Expr::Missing => Ty::Unknown,
            Expr::Int(_) => Ty::Int,
            Expr::Double(_) => Ty::Double,
            Expr::Str(_) => Ty::Str,
            Expr::Name(name) => self.read_var(name, hir.span(id)),
            Expr::List(items) => {
                let elems: Vec<Ty> = items.iter().map(|item| self.infer_expr(*item)).collect();
                Ty::list(join_all(elems))
            }
            Expr::Map(entries) => {
                let mut keys = Vec::with_capacity(entries.len());
                let mut values = Vec::with_capacity(entries.len());
                for entry in entries {
                    let key = self.infer_expr(entry.key);
                    match entry.value {
                        Some(value) => {
                            keys.push(key);
                            values.push(self.infer_expr(value));
                        }
                        // A bare entry must evaluate to a two-element list, so
                        // both halves are that list's element type.
                        None => {
                            self.check_map_entry(entry.key);
                            keys.push(key.elem());
                            values.push(key.elem());
                        }
                    }
                }
                Ty::map(join_all(keys), join_all(values))
            }
            Expr::Prefix { op, operand } => self.infer_prefix(*op, *operand),
            Expr::Binary { op, lhs, rhs } => self.infer_binary(*op, *lhs, *rhs, hir.span(id)),
            Expr::Assign { op, target, value } => self.infer_assign(*op, *target, *value),
            Expr::Seq { exprs, .. } => {
                let mut last = Ty::Null;
                for expr in exprs {
                    last = self.infer_expr(*expr);
                }
                last
            }
            // A definition evaluates to the function's *name as a string*; its
            // body is walked separately, from the flat list of definitions.
            Expr::Define(_) => Ty::Str,
            Expr::Call {
                callee,
                callee_span,
                args,
            } => self.infer_call(callee, *callee_span, args),
        }
    }

    fn infer_prefix(&mut self, op: PrefixOp, operand: ExprId) -> Ty {
        let ty = self.infer_expr(operand);
        match op {
            PrefixOp::Neg | PrefixOp::Pos => {
                self.check_number(&ty, self.hir.span(operand), op.as_str());
                ops::unary_number(&ty)
            }
            PrefixOp::Not => Ty::Bool,
            // The VM tags the container and never reads the tag back, so the
            // spread marker is transparent to the type.
            PrefixOp::Spread => ty,
        }
    }

    fn infer_binary(&mut self, op: BinOp, lhs: ExprId, rhs: ExprId, span: Span) -> Ty {
        let a = self.infer_expr(lhs);
        let b = self.infer_expr(rhs);
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                let result = match op {
                    BinOp::Add => ops::add(&a, &b),
                    BinOp::Sub => ops::sub(&a, &b),
                    BinOp::Mul => ops::mul(&a, &b),
                    _ => ops::div(&a, &b),
                };
                self.check_string_fallback(op, &a, &b, &result, span);
                result
            }
            BinOp::Rem | BinOp::Pow => {
                self.check_number(&a, self.hir.span(lhs), op.as_str());
                self.check_number(&b, self.hir.span(rhs), op.as_str());
                if op == BinOp::Rem {
                    ops::rem(&a, &b)
                } else {
                    ops::pow(&a, &b)
                }
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => Ty::Bool,
            BinOp::And | BinOp::Or => Ty::Bool,
            BinOp::Get => {
                // A list index must be a number; a map key need not be.
                if a.is_list() {
                    self.check_number(&b, self.hir.span(rhs), ":");
                }
                ops::get(&a)
            }
            BinOp::Match => ops::matches(&a),
        }
    }

    fn infer_assign(&mut self, op: AssignOp, target: ExprId, value: ExprId) -> Ty {
        let value_ty = self.infer_expr(value);
        match op {
            AssignOp::Assign => {
                self.bind(target, &value_ty);
                value_ty
            }
            AssignOp::Add => {
                let current = self.infer_expr(target);
                let result = ops::add(&current, &value_ty);
                self.bind(target, &result);
                result
            }
            // `<>` exchanges the two sides' bindings and hands back the right
            // side's value.
            AssignOp::Swap => {
                let current = self.infer_expr(target);
                self.bind(target, &value_ty);
                self.bind(value, &current);
                value_ty
            }
        }
    }

    /// Bind an assignment target to `ty`, descending into a destructuring
    /// pattern.
    fn bind(&mut self, target: ExprId, ty: &Ty) {
        if self.depth >= MAX_DEPTH {
            return;
        }
        self.depth += 1;
        self.bind_inner(target, ty);
        self.depth -= 1;
        if self.recording {
            self.tys[target.index()] = ty.clone();
        }
    }

    fn bind_inner(&mut self, target: ExprId, ty: &Ty) {
        let hir = self.hir;
        match hir.expr(target) {
            Expr::Name(name) => self.write_var(name.clone(), ty.clone()),
            // `[a, b] = pair` — every binder takes the element type.
            Expr::List(items) => {
                let elem = ty.elem();
                for item in items {
                    self.bind(*item, &elem);
                }
            }
            // `[a, ...rest] = xs` — the rest binder collects a list.
            Expr::Prefix {
                op: PrefixOp::Spread,
                operand,
            } => self.bind(*operand, &Ty::list(ty.clone())),
            // `var('x') = 1` names its target at run time.
            Expr::Call { callee, args, .. } if &**callee == "var" => {
                match args.first().and_then(|arg| self.literal_str(*arg)) {
                    Some(name) => self.write_var(Name::from(name), ty.clone()),
                    None => self.scope.dynamic = true,
                }
            }
            // A computed place — `a:0`, `if(c, a, b) = 1`. Nothing new is bound;
            // the sub-expressions still need types.
            _ => {
                self.infer_expr(target);
            }
        }
    }

    // ----------------------------------------------------------------
    // Variables
    // ----------------------------------------------------------------

    fn read_var(&mut self, name: &str, span: Span) -> Ty {
        if let Some(ty) = self.scope.vars.get(name) {
            return ty.clone();
        }
        if let Some(ty) = constant(name) {
            return ty;
        }
        // Names the analysis cannot follow: anything after a dynamic write, the
        // `global_` namespace, and the underscore convention that every
        // iterating form injects.
        if self.scope.dynamic || name.starts_with("global_") || name.starts_with('_') {
            return Ty::Unknown;
        }
        if self.recording {
            self.diagnostics.push(
                Diagnostic::warning(
                    span,
                    LintCode::ReadBeforeWrite,
                    format!("`{name}` is read before anything writes it"),
                )
                .with_help("an unwritten variable reads as `null`"),
            );
        }
        Ty::Undef
    }

    /// Record a write. Flow-sensitive and last-write-wins, which is what a
    /// reader expects walking the program top to bottom.
    fn write_var(&mut self, name: Name, ty: Ty) {
        self.scope.vars.insert(name, ty);
    }

    fn literal_str(&self, id: ExprId) -> Option<&'h str> {
        match self.hir.expr(id) {
            Expr::Str(text) => Some(text),
            _ => None,
        }
    }

    // ----------------------------------------------------------------
    // Calls
    // ----------------------------------------------------------------

    fn infer_call(&mut self, callee: &Name, callee_span: Span, args: &[ExprId]) -> Ty {
        // A user definition shadows a builtin of the same name, exactly as it
        // does in the VM's single flat function table.
        if let Some(&id) = self.fns.get(callee) {
            return self.infer_user_call(id, callee, callee_span, args);
        }
        if let Some(signature) = builtins::builtin(callee) {
            if self.recording && self.countable(args) && !signature.arity.accepts(args.len()) {
                self.diagnostics.push(wrong_arg_count(
                    callee,
                    callee_span,
                    args.len(),
                    &signature.arity.describe(),
                ));
            }
            return match signature.special() {
                Some(special) => self.special_form(special, args),
                None => self.builtin_result(signature, args),
            };
        }
        // Not typed here, and possibly not known at all — either way the
        // arguments still need types.
        let known = builtins::is_known_builtin(callee)
            || self.wildcard_import
            || self.imported.contains(callee);
        for arg in args {
            self.infer_expr(*arg);
        }
        if self.recording && !known {
            self.diagnostics.push(
                Diagnostic::error(
                    callee_span,
                    LintCode::UnknownFunction,
                    format!("no function named `{callee}`"),
                )
                .with_help("define it, or check the spelling against the Scarpet reference"),
            );
        }
        Ty::Unknown
    }

    fn infer_user_call(
        &mut self,
        id: FnId,
        callee: &Name,
        callee_span: Span,
        args: &[ExprId],
    ) -> Ty {
        let arg_tys: Vec<Ty> = args.iter().map(|arg| self.infer_expr(*arg)).collect();

        // Every call site widens what the parameters may hold. This is the half
        // of the fixpoint that flows *into* a function.
        for (index, ty) in arg_tys.iter().enumerate() {
            let changed = &mut self.changed;
            match self.fn_params[id.index()].get_mut(index) {
                Some(slot) => Ctx::raise(changed, slot, ty),
                None => Ctx::raise(changed, &mut self.fn_rest[id.index()], ty),
            }
        }

        // `self.hir` is a shared reference, so the definition it lends out lives
        // independently of the `&mut self` the diagnostic push needs.
        if self.recording
            && self.countable(args)
            && let Some(def) = self.hir.fn_def(id)
            && !def.accepts(args.len())
        {
            self.diagnostics.push(
                wrong_arg_count(callee, callee_span, args.len(), &def.describe_arity())
                    .with_related(def.name_span, format!("`{callee}` is defined here")),
            );
        }
        self.fn_ret[id.index()].clone()
    }

    fn builtin_result(&mut self, signature: Signature, args: &[ExprId]) -> Ty {
        let arg_tys: Vec<Ty> = args.iter().map(|arg| self.infer_expr(*arg)).collect();
        match signature.ret {
            RetRule::Fixed(shape) => shape.to_ty(),
            RetRule::ListOf(shape) => Ty::list(shape.to_ty()),
            RetRule::SameAs(index) => arg_tys.get(index as usize).cloned().unwrap_or(Ty::Unknown),
            RetRule::SameAsLast => arg_tys.last().cloned().unwrap_or(Ty::Unknown),
            RetRule::Form(_) => Ty::Unknown,
            RetRule::Unknown => Ty::Unknown,
        }
    }

    /// The forms that need code: their result depends on their arguments'
    /// types, or they bind `_` and friends around a body.
    fn special_form(&mut self, special: Special, args: &[ExprId]) -> Ty {
        match special {
            Special::If => self.infer_if(args),
            Special::For | Special::Map | Special::Filter | Special::First | Special::All => {
                let list = self.infer_arg(args, 0);
                self.bind_iteration_vars(&list);
                let body = self.infer_arg(args, 1);
                match special {
                    // `for` counts the iterations whose body was truthy.
                    Special::For => Ty::Int,
                    Special::Map => Ty::list(body),
                    Special::Filter => Ty::list(list.elem()),
                    Special::First => join(&list.elem(), &Ty::Null),
                    _ => Ty::Bool,
                }
            }
            Special::Reduce => {
                let list = self.infer_arg(args, 0);
                // The accumulator's seed is the third argument, so it is
                // evaluated before the body can read `_a`.
                let initial = self.infer_arg(args, 2);
                self.write_var(Name::from("_a"), initial.clone());
                self.bind_iteration_vars(&list);
                let body = self.infer_arg(args, 1);
                join(&body, &initial)
            }
            // `while(cond, expr)` / `while(cond, limit, expr)` and
            // `loop(num, expr, exit?)` both inject `_` as the iteration count
            // and yield the last body value, or null if the body never ran.
            Special::While => {
                self.write_var(Name::from("_"), Ty::Int);
                let mut last = Ty::Null;
                for arg in args {
                    last = self.infer_expr(*arg);
                }
                join(&last, &Ty::Null)
            }
            Special::Loop => {
                self.infer_arg(args, 0);
                self.write_var(Name::from("_"), Ty::Int);
                let body = self.infer_arg(args, 1);
                for arg in args.iter().skip(2) {
                    self.infer_expr(*arg);
                }
                join(&body, &Ty::Null)
            }
            Special::CFor => {
                for arg in args {
                    self.infer_expr(*arg);
                }
                Ty::Unknown
            }
            // Dispatch by a run-time string. A literal name resolves; anything
            // else is beyond static reach.
            Special::Call => {
                for arg in args {
                    self.infer_expr(*arg);
                }
                match args
                    .first()
                    .and_then(|arg| self.literal_str(*arg))
                    .and_then(|name| self.fns.get(name))
                {
                    Some(&id) => self.fn_ret[id.index()].clone(),
                    None => Ty::Unknown,
                }
            }
            // `var('x')` is a variable named at run time. A literal name is just
            // a read; anything else could touch any name, so the whole scope
            // stops being trustworthy.
            Special::Var => match args.first().and_then(|arg| self.literal_str(*arg)) {
                Some(name) => {
                    let name = name.to_owned();
                    self.infer_arg(args, 0);
                    self.read_var(&name, Span::default())
                }
                None => {
                    for arg in args {
                        self.infer_expr(*arg);
                    }
                    self.scope.dynamic = true;
                    Ty::Unknown
                }
            },
            // Legal only in a signature, where lowering already consumed it.
            Special::Outer => {
                for arg in args {
                    self.infer_expr(*arg);
                }
                Ty::Unknown
            }
            Special::Sort => {
                let tys: Vec<Ty> = args.iter().map(|arg| self.infer_expr(*arg)).collect();
                match tys.as_slice() {
                    [only] if only.is_list() => only.clone(),
                    _ => Ty::list(join_all(tys)),
                }
            }
        }
    }

    /// `if(cond, expr, cond?, expr?, …, default?)`.
    ///
    /// The VM pops a condition and then a branch; when no branch is left, the
    /// value it just evaluated as a "condition" *is* the result. So the branches
    /// are the odd indices, an odd-length argument list ends in a default, and
    /// an even-length one can run out of conditions and yield null.
    fn infer_if(&mut self, args: &[ExprId]) -> Ty {
        let tys: Vec<Ty> = args.iter().map(|arg| self.infer_expr(*arg)).collect();
        // `Option` rather than seeding with a lattice point: `Undef` would be
        // wrong (it is a real type, not an identity) and `Never` would be right
        // but hides an argument-less `if`.
        let mut result: Option<Ty> = None;
        let mut index = 1;
        while index < tys.len() {
            result = Some(match result {
                Some(acc) => join(&acc, &tys[index]),
                None => tys[index].clone(),
            });
            index += 2;
        }
        let tail = if tys.len() % 2 == 1 {
            tys.last().cloned()
        } else {
            Some(Ty::Null)
        };
        match (result, tail) {
            (Some(acc), Some(tail)) => join(&acc, &tail),
            (Some(acc), None) => acc,
            (None, tail) => tail.unwrap_or(Ty::Null),
        }
    }

    /// Bind `_` and `_i` the way every iterating form does — and leave them
    /// bound afterwards, matching the VM, where `for` deliberately leaks them so
    /// an accumulator like `s += _` works.
    fn bind_iteration_vars(&mut self, list: &Ty) {
        self.write_var(Name::from("_"), list.elem());
        self.write_var(Name::from("_i"), Ty::Int);
    }

    fn infer_arg(&mut self, args: &[ExprId], index: usize) -> Ty {
        match args.get(index) {
            Some(arg) => self.infer_expr(*arg),
            None => Ty::Unknown,
        }
    }

    // ----------------------------------------------------------------
    // Lints
    // ----------------------------------------------------------------

    /// Whether the argument count is a static fact.
    ///
    /// `f(...args)` spreads a list into the call, so how many arguments arrive
    /// depends on how long that list is at run time. Checking arity against the
    /// written count would then be wrong — the corpus does exactly this, e.g.
    /// `_getHeadNbt(...page:_)` for a two-parameter function.
    fn countable(&self, args: &[ExprId]) -> bool {
        !args.iter().any(|arg| {
            matches!(
                self.hir.expr(*arg),
                Expr::Prefix {
                    op: PrefixOp::Spread,
                    ..
                }
            )
        })
    }

    /// An operator that coerces through `as_number` was handed something that
    /// provably is not one. `Unknown` and `Bool` are never accused — `Bool`
    /// coerces, and accusing `Unknown` would be guessing.
    fn check_number(&mut self, ty: &Ty, operand: Span, op: &str) {
        if !self.recording || ty.could_be_number() {
            return;
        }
        self.diagnostics.push(Diagnostic::error(
            operand,
            LintCode::ExpectedNumber,
            format!("`{op}` needs a number, but this is `{ty}`"),
        ));
    }

    /// An arithmetic operator that will silently join its operands as text.
    ///
    /// Narrow on purpose. Scarpet's `+` between a string and a number is the
    /// *idiomatic* way to build a message — `print('x = ' + x)` is everywhere —
    /// so flagging it would bury the signal. What is worth reporting is
    /// arithmetic where **neither** side is a string and the result is one
    /// anyway: `1 + [1, 2]` is `"1[1, 2]"`, `count + null` is `"5null"`. Those
    /// are almost always a mistake.
    fn check_string_fallback(&mut self, op: BinOp, a: &Ty, b: &Ty, result: &Ty, span: Span) {
        if !self.recording
            || *result != Ty::Str
            || a.is_string()
            || b.is_string()
            || !(a.is_number() || b.is_number())
        {
            return;
        }
        self.diagnostics.push(
            Diagnostic::warning(
                span,
                LintCode::StringFallback,
                format!(
                    "`{}` between `{a}` and `{b}` has no arithmetic meaning, \
                     so it joins them as text",
                    op.as_str()
                ),
            )
            .with_help("convert explicitly with `str(...)` if that is intended"),
        );
    }

    /// A bare map entry has to evaluate to a key/value pair. Only a literal list
    /// is checked — anything else needs values the analysis does not have.
    fn check_map_entry(&mut self, key: ExprId) {
        if !self.recording {
            return;
        }
        let hir = self.hir;
        let Expr::List(items) = hir.expr(key) else {
            return;
        };
        if items.len() == 2 {
            return;
        }
        self.diagnostics.push(
            Diagnostic::error(
                hir.span(key),
                LintCode::MapEntryNotPair,
                format!(
                    "a map entry without `->` must be a two-element list, but this has {}",
                    items.len()
                ),
            )
            .with_help("write it as `key -> value`"),
        );
    }
}

/// The `wrong-arg-count` finding, shared by user definitions and typed
/// builtins. A free function so a caller can attach a `defined here` label
/// while still holding a definition borrowed out of the arena.
fn wrong_arg_count(callee: &str, span: Span, got: usize, expected: &str) -> Diagnostic {
    Diagnostic::error(
        span,
        LintCode::WrongArgCount,
        format!("`{callee}` takes {expected}, but {got} were given"),
    )
}

/// Scarpet's built-in constants, which are ordinary identifiers.
fn constant(name: &str) -> Option<Ty> {
    match name {
        "null" => Some(Ty::Null),
        "true" | "false" => Some(Ty::Bool),
        "pi" | "euler" => Some(Ty::Double),
        _ => None,
    }
}
