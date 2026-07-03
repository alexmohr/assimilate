// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_ast;
extern crate rustc_hir;
extern crate rustc_middle;

use clippy_utils::{diagnostics::span_lint_and_help, is_in_test, res::MaybeDef};
use rustc_ast::LitKind;
use rustc_hir::{
    BinOpKind, Expr, ExprKind, HirId, ImplItemKind, ItemKind, LangItem, Lit, Node, Pat,
    PatExprKind, PatKind,
};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty::{Ty, TyCtxt};

dylint_linting::declare_late_lint! {
    /// ### What it does
    /// Flags `==`/`!=` comparisons and `match` expressions that branch on
    /// equality with a string literal, where the compared value has type
    /// `&str` or `String`.
    ///
    /// ### Why is this bad?
    /// This project's style guide forbids driving control flow off of raw
    /// string comparisons: a typo in the literal silently falls through
    /// instead of failing to compile, and the set of valid values isn't
    /// documented anywhere the compiler can check. Parse the string into an
    /// enum at the boundary and match on that instead.
    ///
    /// ### Known problems
    /// Only catches direct `==`/`!=` comparisons and `match` patterns
    /// against string literals; it won't catch equivalent logic expressed
    /// through method calls (e.g. `s.eq("foo")`) or `HashMap` lookups.
    ///
    /// ### Example
    /// ```rust
    /// fn handle(status: &str) {
    ///     if status == "active" {
    ///         // ...
    ///     }
    /// }
    /// ```
    /// Use instead:
    /// ```rust
    /// enum Status {
    ///     Active,
    ///     Inactive,
    /// }
    ///
    /// fn handle(status: Status) {
    ///     if status == Status::Active {
    ///         // ...
    ///     }
    /// }
    /// ```
    pub NO_STRING_CONTROL_FLOW,
    Warn,
    "control flow driven by string literal comparisons instead of enums"
}

impl<'tcx> LateLintPass<'tcx> for NoStringControlFlow {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        if expr.span.from_expansion()
            || is_in_test(cx.tcx, expr.hir_id)
            || is_in_str_conversion_fn(cx.tcx, expr.hir_id)
        {
            return;
        }

        if let ExprKind::Binary(op, lhs, rhs) = expr.kind
            && matches!(op.node, BinOpKind::Eq | BinOpKind::Ne)
        {
            check_string_literal_comparison(cx, expr, lhs, rhs);
        }

        if let ExprKind::Match(scrutinee, arms, _) = expr.kind
            && is_str_or_string(cx, cx.typeck_results().expr_ty_adjusted(scrutinee))
            && arms.iter().any(|arm| pattern_has_string_literal(arm.pat))
        {
            span_lint_and_help(
                cx,
                NO_STRING_CONTROL_FLOW,
                expr.span,
                "matching on a string literal to drive control flow",
                None,
                "parse this into an enum at the boundary and match on that instead",
            );
        }
    }
}

fn check_string_literal_comparison<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &Expr<'tcx>,
    lhs: &'tcx Expr<'tcx>,
    rhs: &'tcx Expr<'tcx>,
) {
    let other = if is_string_literal(lhs) {
        rhs
    } else if is_string_literal(rhs) {
        lhs
    } else {
        return;
    };

    if is_str_or_string(cx, cx.typeck_results().expr_ty_adjusted(other)) {
        span_lint_and_help(
            cx,
            NO_STRING_CONTROL_FLOW,
            expr.span,
            "comparing a string against a literal to drive control flow",
            None,
            "parse this into an enum at the boundary and compare that instead",
        );
    }
}

fn is_string_literal(expr: &Expr<'_>) -> bool {
    matches!(
        expr.kind,
        ExprKind::Lit(Lit {
            node: LitKind::Str(..),
            ..
        })
    )
}

fn pattern_has_string_literal(pat: &Pat<'_>) -> bool {
    match pat.kind {
        PatKind::Expr(pat_expr) => {
            matches!(pat_expr.kind, PatExprKind::Lit { lit, .. } if matches!(lit.node, LitKind::Str(..)))
        }
        PatKind::Or(pats) => pats.iter().any(pattern_has_string_literal),
        _ => false,
    }
}

fn is_str_or_string<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> bool {
    let ty = ty.peel_refs();
    ty.is_str() || ty.is_lang_item(cx, LangItem::String)
}

/// Whether `hir_id` sits inside a `from`, `from_str`, `try_from`, or
/// `deserialize` function.
///
/// These are the sanctioned boundary-conversion functions (`From<&str>`,
/// `FromStr`, `TryFrom<&str>`, `serde::Deserialize::deserialize`) that turn
/// an external string/JSON representation into an enum; the string
/// comparisons they contain are the intended target of the conversion, not
/// the ad-hoc control flow this lint is meant to catch elsewhere in the
/// codebase.
fn is_in_str_conversion_fn(tcx: TyCtxt<'_>, hir_id: HirId) -> bool {
    tcx.hir_parent_iter(hir_id)
        .find_map(|(_, node)| match node {
            Node::Item(item) => match item.kind {
                ItemKind::Fn { ident, .. } => Some(ident),
                _ => None,
            },
            Node::ImplItem(impl_item) => match impl_item.kind {
                ImplItemKind::Fn(..) => Some(impl_item.ident),
                _ => None,
            },
            _ => None,
        })
        .is_some_and(|ident| {
            matches!(
                ident.name.as_str(),
                "from" | "from_str" | "try_from" | "deserialize"
            )
        })
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}
