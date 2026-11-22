//! The rules, RT001-RT007. Each matches on `syn` tree shapes, never on text.
//! `crates/tutor/tests/checker_cases.rs` holds each rule's positive, negative
//! and style cases (invariant 1: two correct styles give zero findings).

use crate::checker::{finding, FileCtx, Finding, Rule, RuleMeta, Severity};
use quote::ToTokens;
use std::collections::BTreeSet;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, BinOp, Expr, ExprPath, Lit, Token, UnOp};

pub fn all() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(FloatEqInAssert),
        Box::new(RangeIndexLoop),
        Box::new(AllocInLoop),
        Box::new(PartialCmpUnwrap),
        Box::new(EpsilonAbsTol),
        Box::new(StridedInnerLoop),
        Box::new(UnwrapInResultFn),
    ]
}

// ---------------------------------------------------------------- helpers

fn is_test_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        let p = a.path();
        if p.is_ident("test") {
            return true;
        }
        if p.is_ident("cfg") {
            return a.meta.to_token_stream().to_string().replace(' ', "").contains("cfg(test)");
        }
        false
    })
}

fn macro_name(m: &syn::Macro) -> String {
    m.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default()
}

/// Macros whose arguments are comma-separated expressions we look inside.
const EXPR_MACROS: &[&str] = &[
    "assert", "assert_eq", "assert_ne", "debug_assert", "debug_assert_eq", "debug_assert_ne", "vec", "println",
    "print", "eprintln", "eprint", "format", "write", "writeln", "panic", "dbg",
];

fn macro_args(m: &syn::Macro) -> Vec<Expr> {
    if !EXPR_MACROS.contains(&macro_name(m).as_str()) {
        return vec![];
    }
    // `vec![x; n]` uses a semicolon.
    if let Ok(e) = m.parse_body_with(|input: syn::parse::ParseStream| {
        let a: Expr = input.parse()?;
        let _: Token![;] = input.parse()?;
        let b: Expr = input.parse()?;
        Ok(vec![a, b])
    }) {
        return e;
    }
    m.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
        .map(|p| p.into_iter().collect())
        .unwrap_or_default()
}

fn unparen(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(p) => unparen(&p.expr),
        Expr::Group(g) => unparen(&g.expr),
        _ => e,
    }
}

fn single_ident(e: &Expr) -> Option<String> {
    match unparen(e) {
        Expr::Path(ExprPath { path, qself: None, .. }) if path.segments.len() == 1 => {
            Some(path.segments[0].ident.to_string())
        }
        _ => None,
    }
}

/// Syntactically a float: a float literal, `-lit`, `x as f64`, or a float
/// constant path such as `f64::NAN`.
fn is_float_expr(e: &Expr) -> bool {
    match unparen(e) {
        Expr::Lit(l) => matches!(l.lit, Lit::Float(_)),
        Expr::Unary(u) if matches!(u.op, UnOp::Neg(_)) => is_float_expr(&u.expr),
        Expr::Cast(c) => {
            let t = c.ty.to_token_stream().to_string();
            t == "f64" || t == "f32"
        }
        Expr::Path(p) => {
            let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
            segs.len() >= 2
                && segs.iter().any(|s| s == "f64" || s == "f32")
                && segs.last().is_some_and(|l| l.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
        }
        _ => false,
    }
}

fn contains_method(e: &Expr, name: &str) -> bool {
    struct V<'a> {
        name: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            if m.method == self.name {
                self.found = true;
            }
            visit::visit_expr_method_call(self, m);
        }
    }
    let mut v = V { name, found: false };
    v.visit_expr(e);
    v.found
}

fn mentions_ident(e: &Expr, id: &str) -> bool {
    struct V<'a> {
        id: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_expr_path(&mut self, p: &'ast ExprPath) {
            if p.path.segments.len() == 1 && p.path.segments[0].ident == self.id {
                self.found = true;
            }
        }
    }
    let mut v = V { id, found: false };
    v.visit_expr(e);
    v.found
}

/// Generates the boilerplate shared by every rule visitor: test-context
/// tracking (skipped when `skip_tests`), and descent into macro arguments.
macro_rules! context_visitor {
    () => {
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            let was = self.in_test;
            self.in_test |= is_test_attr(&f.attrs);
            visit::visit_item_fn(self, f);
            self.in_test = was;
        }
        context_visitor!(no_item_fn);
    };
    (no_item_fn) => {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let was = self.in_test;
            self.in_test |= is_test_attr(&m.attrs);
            visit::visit_item_mod(self, m);
            self.in_test = was;
        }
        fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
            let was = self.in_test;
            self.in_test |= is_test_attr(&f.attrs);
            visit::visit_impl_item_fn(self, f);
            self.in_test = was;
        }
    };
}

// ---------------------------------------------------------------- RT001

pub struct FloatEqInAssert;

impl Rule for FloatEqInAssert {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: "RT001",
            name: "float-eq-in-assert",
            kc: "floating-point-comparison",
            severity: Severity::Warn,
            fp_risk: "Syntax only, no types: flags an assertion only when one side is visibly a float \
                (a float literal, `as f64`, or an `f64::CONST`). `assert_eq!(a, b)` with two float \
                variables is missed (clippy::float_cmp, pedantic, has the types). A value that is exact \
                by construction (small integers, powers of two) is a false positive: say so with \
                `x.to_bits() == y.to_bits()`, which this rule accepts.",
            measured_by: "nothing to measure: this is a property of the test, not of the run",
            hints: [
                "Would this assertion still pass if the computation rounded differently in its last digit, \
                 for example after reordering a sum?",
                "Floating-point results carry rounding error, so an exact comparison tests the rounding, not \
                 the mathematics. A test must state how close is close enough, and why.",
                "Compare with a stated tolerance: `assert!((x - want).abs() <= tol)` with `tol` derived from \
                 an error bound (e.g. `8.0 * f64::EPSILON * want.abs()`), or `to_bits()` if exactness is the \
                 specification.",
            ],
            answer_tokens: &["abs()", "tol", "EPSILON", "to_bits"],
        }
    }
    fn check(&self, ctx: &FileCtx) -> Vec<Finding> {
        struct V<'c, 'f> {
            r: &'c FloatEqInAssert,
            ctx: &'c FileCtx<'f>,
            out: Vec<Finding>,
            in_test: bool,
        }
        impl<'ast> Visit<'ast> for V<'_, '_> {
            context_visitor!();
            fn visit_macro(&mut self, m: &'ast syn::Macro) {
                let name = macro_name(m);
                let args = macro_args(m);
                match name.as_str() {
                    "assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne" if args.len() >= 2 => {
                        if is_float_expr(&args[0]) || is_float_expr(&args[1]) {
                            self.out.push(finding(
                                self.r,
                                self.ctx,
                                m.path.span(),
                                format!("`{name}!` compares floats exactly"),
                            ));
                        }
                    }
                    "assert" | "debug_assert" if !args.is_empty() => {
                        struct B(bool);
                        impl<'a> Visit<'a> for B {
                            fn visit_expr_binary(&mut self, b: &'a syn::ExprBinary) {
                                if matches!(b.op, BinOp::Eq(_) | BinOp::Ne(_))
                                    && (is_float_expr(&b.left) || is_float_expr(&b.right))
                                {
                                    self.0 = true;
                                }
                                visit::visit_expr_binary(self, b);
                            }
                        }
                        let mut b = B(false);
                        b.visit_expr(&args[0]);
                        if b.0 {
                            self.out.push(finding(
                                self.r,
                                self.ctx,
                                m.path.span(),
                                format!("`{name}!` uses `==`/`!=` on a float"),
                            ));
                        }
                    }
                    _ => {}
                }
                for a in &args {
                    self.visit_expr(a);
                }
            }
        }
        let mut v = V { r: self, ctx, out: vec![], in_test: ctx.test_file };
        v.visit_file(ctx.file);
        v.out
    }
}

// ---------------------------------------------------------------- RT002

pub struct RangeIndexLoop;

impl Rule for RangeIndexLoop {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: "RT002",
            name: "range-index-loop",
            kc: "iterators-over-indices",
            severity: Severity::Advice,
            fp_risk: "Flags `for i in 0..x.len()` only when `i` is used for nothing but indexing one or \
                two slices. Loops indexing three or more slices are exempt (a four-way `zip` is harder to \
                read than indices). LLVM often removes the bounds checks anyway, so this is about intent \
                and not, by itself, speed. Skipped in test code.",
            measured_by: "nothing deterministic: a bounds check's cost is a timing (unit 31 shows one, labelled)",
            hints: [
                "Is the number `i` itself ever needed in this loop, or only the elements it points at?",
                "When an index only selects elements, the loop is really a walk over the elements. Iterators \
                 say that directly, cannot go out of bounds, and give the compiler the lengths up front.",
                "Write `for (o, v) in out.iter_mut().zip(x) { ... }`, or `x.iter().enumerate()` if you do \
                 need the index.",
            ],
            answer_tokens: &["iter", "zip", "enumerate"],
        }
    }
    fn check(&self, ctx: &FileCtx) -> Vec<Finding> {
        struct V<'c, 'f> {
            r: &'c RangeIndexLoop,
            ctx: &'c FileCtx<'f>,
            out: Vec<Finding>,
            in_test: bool,
        }
        struct Uses<'a> {
            var: &'a str,
            bare: usize,
            bases: BTreeSet<String>,
        }
        impl<'ast> Visit<'ast> for Uses<'_> {
            fn visit_expr_index(&mut self, ix: &'ast syn::ExprIndex) {
                if single_ident(&ix.index).as_deref() == Some(self.var) {
                    self.bases.insert(ix.expr.to_token_stream().to_string());
                    self.visit_expr(&ix.expr);
                } else {
                    visit::visit_expr_index(self, ix);
                }
            }
            fn visit_expr_path(&mut self, p: &'ast ExprPath) {
                if p.path.segments.len() == 1 && p.path.segments[0].ident == self.var {
                    self.bare += 1;
                }
            }
            fn visit_macro(&mut self, m: &'ast syn::Macro) {
                let args = macro_args(m);
                if args.is_empty() && m.tokens.to_string().split(|c: char| !c.is_alphanumeric() && c != '_').any(|t| t == self.var) {
                    self.bare += 1; // used inside a macro we cannot parse: be conservative
                }
                for a in &args {
                    self.visit_expr(a);
                }
            }
        }
        impl<'ast> Visit<'ast> for V<'_, '_> {
            context_visitor!();
            fn visit_expr_for_loop(&mut self, f: &'ast syn::ExprForLoop) {
                if !self.in_test {
                    if let (syn::Pat::Ident(pi), Expr::Range(rg)) = (&*f.pat, unparen(&f.expr)) {
                        let starts_at_zero = match rg.start.as_deref().map(unparen) {
                            None => true,
                            Some(Expr::Lit(l)) => l.lit.to_token_stream().to_string() == "0",
                            _ => false,
                        };
                        let len_of = match rg.end.as_deref().map(unparen) {
                            Some(Expr::MethodCall(mc)) if mc.method == "len" && mc.args.is_empty() => {
                                Some(mc.receiver.to_token_stream().to_string())
                            }
                            _ => None,
                        };
                        if let (true, Some(x)) = (starts_at_zero, len_of) {
                            let var = pi.ident.to_string();
                            let mut u = Uses { var: &var, bare: 0, bases: BTreeSet::new() };
                            u.visit_block(&f.body);
                            if u.bare == 0 && !u.bases.is_empty() && u.bases.len() <= 2 && u.bases.contains(&x) {
                                let list: Vec<String> = u.bases.iter().map(|b| format!("`{}`", b.replace(' ', ""))).collect();
                                self.out.push(finding(
                                    self.r,
                                    self.ctx,
                                    f.for_token.span,
                                    format!("`{var}` only indexes {}; the loop can iterate over the elements", list.join(" and ")),
                                ));
                            }
                        }
                    }
                }
                visit::visit_expr_for_loop(self, f);
            }
        }
        let mut v = V { r: self, ctx, out: vec![], in_test: ctx.test_file };
        v.visit_file(ctx.file);
        v.out
    }
}

// ---------------------------------------------------------------- RT003

pub struct AllocInLoop;

impl Rule for AllocInLoop {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: "RT003",
            name: "alloc-in-loop",
            kc: "preallocation",
            severity: Severity::Advice,
            fp_risk: "Points at calls that usually allocate (`vec![..]` with elements, `Vec::with_capacity`, \
                `to_vec`, `to_owned`, `to_string`, `collect`, `format!`, `Box::new`) inside a loop body. \
                `Vec::new()` and `vec![]` are not flagged: they do not allocate. A loop that must return one \
                owned value per iteration, or that runs a handful of times, is a false positive. Whether it \
                allocates in fact is measured, not assumed. Skipped in test code.",
            measured_by: "explain: allocation counts from a counting global allocator (examples/measure.rs)",
            hints: [
                "How many times does this line run when the loop runs a million times, and what does each run \
                 ask the operating system's allocator for?",
                "Each of these calls requests fresh heap memory. In a hot loop that cost and the cache misses \
                 it causes can dominate the arithmetic; a buffer allocated once can be reused.",
                "Allocate the buffer before the loop (`let mut tmp = vec![0.0; n];`, or a field of a workspace \
                 struct) and overwrite it each iteration with `copy_from_slice` or an index loop.",
            ],
            answer_tokens: &["before the loop", "workspace", "copy_from_slice", "reuse"],
        }
    }
    fn check(&self, ctx: &FileCtx) -> Vec<Finding> {
        struct V<'c, 'f> {
            r: &'c AllocInLoop,
            ctx: &'c FileCtx<'f>,
            out: Vec<Finding>,
            in_test: bool,
            depth: usize,
        }
        impl V<'_, '_> {
            fn flag(&mut self, span: proc_macro2::Span, what: &str) {
                if self.depth > 0 && !self.in_test {
                    self.out.push(finding(self.r, self.ctx, span, format!("`{what}` inside a loop allocates on every iteration")));
                }
            }
        }
        impl<'ast> Visit<'ast> for V<'_, '_> {
            context_visitor!(no_item_fn);
            fn visit_expr_for_loop(&mut self, f: &'ast syn::ExprForLoop) {
                self.visit_expr(&f.expr); // evaluated once
                self.depth += 1;
                self.visit_block(&f.body);
                self.depth -= 1;
            }
            fn visit_expr_while(&mut self, w: &'ast syn::ExprWhile) {
                self.depth += 1;
                visit::visit_expr_while(self, w);
                self.depth -= 1;
            }
            fn visit_expr_loop(&mut self, l: &'ast syn::ExprLoop) {
                self.depth += 1;
                visit::visit_expr_loop(self, l);
                self.depth -= 1;
            }
            fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
                // A nested fn item does not run per iteration of an enclosing loop.
                let (was, d) = (self.in_test, self.depth);
                self.in_test |= is_test_attr(&f.attrs);
                self.depth = 0;
                visit::visit_item_fn(self, f);
                self.in_test = was;
                self.depth = d;
            }
            fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
                let name = m.method.to_string();
                if ["to_vec", "to_owned", "to_string", "collect"].contains(&name.as_str()) {
                    self.flag(m.method.span(), &format!(".{name}()"));
                }
                visit::visit_expr_method_call(self, m);
            }
            fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
                if let Expr::Path(p) = &*c.func {
                    let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
                    let tail = segs.iter().rev().take(2).rev().cloned().collect::<Vec<_>>().join("::");
                    if ["Vec::with_capacity", "Box::new", "String::with_capacity"].contains(&tail.as_str()) {
                        self.flag(c.func.span(), &tail);
                    }
                }
                visit::visit_expr_call(self, c);
            }
            fn visit_macro(&mut self, m: &'ast syn::Macro) {
                let name = macro_name(m);
                let args = macro_args(m);
                if (name == "vec" && !args.is_empty()) || name == "format" {
                    self.flag(m.path.span(), &format!("{name}!"));
                }
                for a in &args {
                    self.visit_expr(a);
                }
            }
        }
        let mut v = V { r: self, ctx, out: vec![], in_test: ctx.test_file, depth: 0 };
        v.visit_file(ctx.file);
        v.out
    }
}

// ---------------------------------------------------------------- RT004

pub struct PartialCmpUnwrap;

impl Rule for PartialCmpUnwrap {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: "RT004",
            name: "partial-cmp-unwrap",
            kc: "nan-and-total-order",
            severity: Severity::Warn,
            fp_risk: "Flags `.partial_cmp(..).unwrap()` and `.expect(..)` on any type. For a type whose \
                `partial_cmp` never returns `None` (all integers) it is only noise; for floats it is a panic \
                on the first NaN. If NaN is impossible by construction the panic is unreachable, but the fix \
                costs nothing.",
            measured_by: "a test with a NaN input (the panic is the measurement)",
            hints: [
                "What does `partial_cmp` return when one of the two floats is NaN, and what does `unwrap` do \
                 with that?",
                "NaN is not less than, equal to, or greater than anything, so `partial_cmp` returns `None` and \
                 `unwrap` panics. A sort or a max over measured data then crashes on the first missing value.",
                "Use `a.total_cmp(b)` (IEEE 754 total order: NaNs sort after +inf), e.g. \
                 `v.sort_by(|a, b| a.total_cmp(b))`, or filter NaNs out first and say so.",
            ],
            answer_tokens: &["total_cmp", "total order"],
        }
    }
    fn check(&self, ctx: &FileCtx) -> Vec<Finding> {
        struct V<'c, 'f> {
            r: &'c PartialCmpUnwrap,
            ctx: &'c FileCtx<'f>,
            out: Vec<Finding>,
            in_test: bool,
        }
        impl<'ast> Visit<'ast> for V<'_, '_> {
            context_visitor!();
            fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
                if m.method == "unwrap" || m.method == "expect" {
                    if let Expr::MethodCall(inner) = unparen(&m.receiver) {
                        if inner.method == "partial_cmp" {
                            self.out.push(finding(
                                self.r,
                                self.ctx,
                                inner.method.span(),
                                format!("`.partial_cmp(..).{}()` panics if either value is NaN", m.method),
                            ));
                        }
                    }
                }
                visit::visit_expr_method_call(self, m);
            }
            fn visit_macro(&mut self, m: &'ast syn::Macro) {
                for a in &macro_args(m) {
                    self.visit_expr(a);
                }
            }
        }
        let mut v = V { r: self, ctx, out: vec![], in_test: ctx.test_file };
        v.visit_file(ctx.file);
        v.out
    }
}

// ---------------------------------------------------------------- RT005

pub struct EpsilonAbsTol;

impl Rule for EpsilonAbsTol {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: "RT005",
            name: "epsilon-as-absolute-tolerance",
            kc: "floating-point-comparison",
            severity: Severity::Warn,
            fp_risk: "Flags a comparison `<`, `<=`, `>`, `>=` with a bare `f64::EPSILON` or `f32::EPSILON` \
                (not multiplied by anything) on one side and an `.abs()` on the other. If the compared values \
                are known to lie in [0.5, 2], an absolute EPSILON is a correct one-ulp test and this is a \
                false positive. `EPSILON * scale` is never flagged.",
            measured_by: "nothing to measure: the error is in the tolerance's units",
            hints: [
                "`f64::EPSILON` is about 2.2e-16. How far apart are two neighbouring floats near 1000.0, and \
                 near 1e-20?",
                "EPSILON is the gap between 1.0 and the next float, a *relative* size. Near 1000 neighbouring \
                 floats are 1.1e-13 apart, so `< EPSILON` demands exact equality; near 1e-20 it accepts \
                 values that differ by a factor of a million.",
                "Scale it: `(a - b).abs() <= k * f64::EPSILON * a.abs().max(b.abs())` with `k` from an error \
                 bound, plus an absolute floor for values near zero; or a tolerance derived for the problem.",
            ],
            answer_tokens: &["scale", "max(", "relative"],
        }
    }
    fn check(&self, ctx: &FileCtx) -> Vec<Finding> {
        fn bare_eps(e: &Expr) -> bool {
            match unparen(e) {
                Expr::Path(p) => {
                    let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
                    segs.last().is_some_and(|l| l == "EPSILON")
                        && (segs.len() == 1 || segs.iter().any(|s| s == "f64" || s == "f32"))
                }
                _ => false,
            }
        }
        struct V<'c, 'f> {
            r: &'c EpsilonAbsTol,
            ctx: &'c FileCtx<'f>,
            out: Vec<Finding>,
            in_test: bool,
        }
        impl<'ast> Visit<'ast> for V<'_, '_> {
            context_visitor!();
            fn visit_expr_binary(&mut self, b: &'ast syn::ExprBinary) {
                if matches!(b.op, BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_))
                    && ((bare_eps(&b.right) && contains_method(&b.left, "abs"))
                        || (bare_eps(&b.left) && contains_method(&b.right, "abs")))
                {
                    self.out.push(finding(
                        self.r,
                        self.ctx,
                        b.op.span(),
                        "`EPSILON` used as an absolute tolerance".to_string(),
                    ));
                }
                visit::visit_expr_binary(self, b);
            }
            fn visit_macro(&mut self, m: &'ast syn::Macro) {
                for a in &macro_args(m) {
                    self.visit_expr(a);
                }
            }
        }
        let mut v = V { r: self, ctx, out: vec![], in_test: ctx.test_file };
        v.visit_file(ctx.file);
        v.out
    }
}

// ---------------------------------------------------------------- RT006

pub struct StridedInnerLoop;

impl Rule for StridedInnerLoop {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: "RT006",
            name: "strided-inner-loop",
            kc: "memory-layout",
            severity: Severity::Advice,
            fp_risk: "Inside two nested `for` loops, flags `a[inner][outer]` and a flat index whose product \
                term contains the inner loop variable (`a[inner * n + outer]`). It cannot know the layout the \
                author intended: if the data is stored transposed on purpose, or `n` is tiny, it is a false \
                positive. Loops over other iterators than simple patterns are not seen. Skipped in test code.",
            measured_by: "not graded: cache effects are timings (unit 31 has a labelled, non-graded benchmark)",
            hints: [
                "As the inner loop advances by one, how many elements apart in memory are two consecutive \
                 accesses?",
                "A `Vec` is one contiguous block. In a row-major `i * n + j` layout, consecutive `j` are \
                 neighbours, consecutive `i` are `n` elements apart. Each far jump can miss the cache, and \
                 the hardware prefetcher favours unit stride.",
                "Swap the loops so the innermost variable is the one added without a multiplier \
                 (`for i { for j { a[i * n + j] } }`), or store the data in the order you traverse it.",
            ],
            answer_tokens: &["swap", "innermost", "for i { for j"],
        }
    }
    fn check(&self, ctx: &FileCtx) -> Vec<Finding> {
        struct V<'c, 'f> {
            r: &'c StridedInnerLoop,
            ctx: &'c FileCtx<'f>,
            out: Vec<Finding>,
            in_test: bool,
            vars: Vec<String>,
        }
        fn product_has(e: &Expr, inner: &str) -> bool {
            match unparen(e) {
                Expr::Binary(b) if matches!(b.op, BinOp::Mul(_)) => {
                    single_ident(&b.left).as_deref() == Some(inner) || single_ident(&b.right).as_deref() == Some(inner)
                }
                Expr::Binary(b) if matches!(b.op, BinOp::Add(_)) => product_has(&b.left, inner) || product_has(&b.right, inner),
                _ => false,
            }
        }
        impl<'ast> Visit<'ast> for V<'_, '_> {
            context_visitor!();
            fn visit_expr_for_loop(&mut self, f: &'ast syn::ExprForLoop) {
                self.visit_expr(&f.expr);
                let name = match &*f.pat {
                    syn::Pat::Ident(pi) => pi.ident.to_string(),
                    _ => String::from("_"),
                };
                self.vars.push(name);
                self.visit_block(&f.body);
                self.vars.pop();
            }
            fn visit_expr_index(&mut self, ix: &'ast syn::ExprIndex) {
                if self.vars.len() >= 2 && !self.in_test {
                    let inner = self.vars.last().unwrap().clone();
                    let outers = &self.vars[..self.vars.len() - 1];
                    // a[inner][outer]
                    if let Expr::Index(first) = unparen(&ix.expr) {
                        let i1 = single_ident(&first.index);
                        let i2 = single_ident(&ix.index);
                        if i1.as_deref() == Some(inner.as_str()) && i2.as_ref().is_some_and(|o| outers.contains(o)) {
                            self.out.push(finding(
                                self.r,
                                self.ctx,
                                ix.bracket_token.span.join(),
                                format!("inner loop variable `{inner}` selects the row: consecutive iterations jump between rows"),
                            ));
                        }
                    }
                    // a[inner * n + outer]
                    if let Expr::Binary(b) = unparen(&ix.index) {
                        if matches!(b.op, BinOp::Add(_))
                            && product_has(&ix.index, &inner)
                            && outers.iter().any(|o| mentions_ident(&ix.index, o))
                        {
                            self.out.push(finding(
                                self.r,
                                self.ctx,
                                ix.bracket_token.span.join(),
                                format!("inner loop variable `{inner}` is multiplied by a stride: consecutive iterations are far apart in memory"),
                            ));
                        }
                    }
                }
                visit::visit_expr_index(self, ix);
            }
            fn visit_macro(&mut self, m: &'ast syn::Macro) {
                for a in &macro_args(m) {
                    self.visit_expr(a);
                }
            }
        }
        let mut v = V { r: self, ctx, out: vec![], in_test: ctx.test_file, vars: vec![] };
        v.visit_file(ctx.file);
        v.out
    }
}

// ---------------------------------------------------------------- RT007

pub struct UnwrapInResultFn;

impl Rule for UnwrapInResultFn {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: "RT007",
            name: "unwrap-in-result-fn",
            kc: "result-error-handling",
            severity: Severity::Advice,
            fp_risk: "Flags `.unwrap()` / `.expect(..)` in the body of a function whose declared return type \
                is named `Result`, outside tests. An unwrap that cannot fail (a `try_into` of a length just \
                checked) is a false positive; so is one inside a closure that returns something else. A \
                type alias for Result under another name is missed.",
            measured_by: "a test that triggers the failure: a panic instead of an `Err`",
            hints: [
                "This function already has a way to report failure to its caller. What happens to the caller \
                 if this line fails instead?",
                "`unwrap` turns a recoverable error into a panic that unwinds the whole computation; a caller \
                 sweeping over parameters cannot then skip the bad case and continue.",
                "Propagate it: `let v = thing?;` (with a `From` conversion or `.map_err(..)` to your error \
                 type), or return a specific variant such as `Err(OdeError::NonFinite { t })`.",
            ],
            answer_tokens: &["`?`", "?;", "map_err", "Err("],
        }
    }
    fn check(&self, ctx: &FileCtx) -> Vec<Finding> {
        fn returns_result(sig: &syn::Signature) -> bool {
            match &sig.output {
                syn::ReturnType::Type(_, t) => match &**t {
                    syn::Type::Path(p) => p.path.segments.last().is_some_and(|s| s.ident == "Result"),
                    _ => false,
                },
                syn::ReturnType::Default => false,
            }
        }
        struct Calls<'c, 'f> {
            r: &'c UnwrapInResultFn,
            ctx: &'c FileCtx<'f>,
            out: Vec<Finding>,
        }
        impl<'ast> Visit<'ast> for Calls<'_, '_> {
            fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
                if m.method == "unwrap" || m.method == "expect" {
                    let after_partial_cmp = matches!(unparen(&m.receiver), Expr::MethodCall(i) if i.method == "partial_cmp");
                    if !after_partial_cmp {
                        self.out.push(finding(
                            self.r,
                            self.ctx,
                            m.method.span(),
                            format!("`.{}()` in a function that returns `Result`", m.method),
                        ));
                    }
                }
                visit::visit_expr_method_call(self, m);
            }
            fn visit_item_fn(&mut self, _: &'ast syn::ItemFn) {} // nested fns are checked on their own
            fn visit_macro(&mut self, m: &'ast syn::Macro) {
                for a in &macro_args(m) {
                    self.visit_expr(a);
                }
            }
        }
        struct V<'c, 'f> {
            r: &'c UnwrapInResultFn,
            ctx: &'c FileCtx<'f>,
            out: Vec<Finding>,
            in_test: bool,
        }
        impl V<'_, '_> {
            fn body(&mut self, sig: &syn::Signature, block: &syn::Block) {
                if !self.in_test && returns_result(sig) {
                    let mut c = Calls { r: self.r, ctx: self.ctx, out: vec![] };
                    c.visit_block(block);
                    self.out.extend(c.out);
                }
            }
        }
        impl<'ast> Visit<'ast> for V<'_, '_> {
            fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
                let was = self.in_test;
                self.in_test |= is_test_attr(&f.attrs);
                self.body(&f.sig, &f.block);
                visit::visit_item_fn(self, f);
                self.in_test = was;
            }
            fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
                let was = self.in_test;
                self.in_test |= is_test_attr(&f.attrs);
                self.body(&f.sig, &f.block);
                visit::visit_impl_item_fn(self, f);
                self.in_test = was;
            }
            fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
                let was = self.in_test;
                self.in_test |= is_test_attr(&m.attrs);
                visit::visit_item_mod(self, m);
                self.in_test = was;
            }
        }
        let mut v = V { r: self, ctx, out: vec![], in_test: ctx.test_file };
        v.visit_file(ctx.file);
        v.out
    }
}
