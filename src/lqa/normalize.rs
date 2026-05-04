//! LQA normalisation rules — spec-anchored desugaring transformations.
//!
//! Each function in this module implements one desugaring rule from the
//! openCypher 9 specification.  Rules are cited with
//! `// NORMALIZATION(openCypher 9 §X.Y):` comments.
//!
//! Phase 3 implements two proof-of-concept rules:
//!
//! 1. [`simple_case_to_searched`] — converts the "simple" CASE form
//!    (`CASE expr WHEN value THEN result …`) into the canonical "searched"
//!    form (`CASE WHEN expr = value THEN result …`).
//!    openCypher 9 §6.2.1
//!
//! 2. [`desugar_implicit_alias`] — makes implicit column aliases explicit.
//!    A `RETURN expr` with no `AS name` receives a generated alias
//!    `?gen_N` where N is a monotonically increasing counter.
//!    openCypher 9 §4.5.1
//!
//! Phase 4 will add:
//! - list/pattern/map comprehension normalisation
//! - variable scoping and shadowing resolution
//! - type-annotation pass (type inference from spec §2.1)
//! - quantifier tautology folding (moved from the ad-hoc translator)

use crate::lqa::expr::{CmpOp, Expr};
use crate::lqa::op::{Op, ProjItem};

// ── Rule 1: Simple CASE → Searched CASE ───────────────────────────────────────

/// Desugar "simple CASE" to "searched CASE".
///
/// openCypher 9 §6.2.1 defines two CASE forms:
///
/// ```text
/// Simple:   CASE x  WHEN v1 THEN r1  WHEN v2 THEN r2  [ELSE d]  END
/// Searched: CASE    WHEN x=v1 THEN r1  WHEN x=v2 THEN r2  [ELSE d]  END
/// ```
///
/// The searched form is the canonical representation used throughout the LQA.
/// The subject expression `x` is cloned into each WHEN branch as the LHS of an
/// equality comparison.  This is valid because Cypher equality semantics
/// (`null = anything → null`) are preserved by the generated comparisons.
///
/// # NORMALIZATION(openCypher 9 §6.2.1):
/// "The only difference between the simple form and the general form is that
/// the general form does not need a start expression."
///
/// # Examples
///
/// ```text
/// CASE n.status WHEN 'a' THEN 1 WHEN 'b' THEN 2 ELSE 0 END
/// →
/// CASE WHEN n.status = 'a' THEN 1 WHEN n.status = 'b' THEN 2 ELSE 0 END
/// ```
pub fn simple_case_to_searched(subject: Expr, branches: Vec<(Expr, Expr)>, else_expr: Option<Expr>) -> Expr {
    // NORMALIZATION(openCypher 9 §6.2.1): expand each branch to a
    // `subject = value` comparison.  The subject is cloned once per branch.
    let searched_branches = branches
        .into_iter()
        .map(|(when_val, then_expr)| {
            let cond = Expr::Comparison(
                CmpOp::Eq,
                Box::new(subject.clone()),
                Box::new(when_val),
            );
            (cond, then_expr)
        })
        .collect();

    Expr::CaseSearched {
        branches: searched_branches,
        else_expr: else_expr.map(Box::new),
    }
}

// ── Rule 2: Implicit alias generation ─────────────────────────────────────────

/// State for generating fresh alias names.
///
/// In openCypher, `RETURN expr` without an `AS alias` generates an
/// implementation-defined column name.  The LQA represents this as a
/// generated alias `?gen_0`, `?gen_1`, … to make the binding names explicit
/// and stable within a single normalisation pass.
pub struct AliasGen {
    counter: usize,
}

impl AliasGen {
    /// Create a new alias generator starting from counter 0.
    pub fn new() -> Self {
        AliasGen { counter: 0 }
    }

    /// Return the next generated alias name.
    pub fn next(&mut self) -> String {
        let name = format!("?gen_{}", self.counter);
        self.counter += 1;
        name
    }
}

impl Default for AliasGen {
    fn default() -> Self {
        Self::new()
    }
}

/// Make implicit aliases explicit in a list of projection items.
///
/// In Cypher, `RETURN n.name, n.age AS age` has an implicit alias for the
/// first item.  openCypher 9 §4.5.1 says the implicit alias is the text of
/// the expression (implementation-specific); we use the generated `?gen_N`
/// convention.
///
/// # NORMALIZATION(openCypher 9 §4.5.1):
/// "If the expression does not use an alias, the name of the column is
/// implementation-specific."
///
/// Items that already have an alias are returned unchanged.
/// Items without an alias receive a fresh `?gen_N` alias from `gen`.
pub fn desugar_implicit_alias(items: Vec<(Expr, Option<String>)>, gen: &mut AliasGen) -> Vec<ProjItem> {
    // NORMALIZATION(openCypher 9 §4.5.1): assign a stable generated name to
    // any projection item that does not carry an explicit user-visible alias.
    items
        .into_iter()
        .map(|(expr, alias)| {
            let alias = alias.unwrap_or_else(|| gen.next());
            ProjItem { expr, alias }
        })
        .collect()
}

// ── Rule 3: Flatten nested NOT ───────────────────────────────────────────────

/// Simplify `NOT(NOT(expr))` → `expr`.
///
/// # NORMALIZATION(openCypher 9 §6.1 Boolean operators):
/// Double negation is an identity in two-valued logic.  In three-valued
/// Cypher logic it is also an identity: `NOT(NOT(null)) = NOT(null) = null`.
pub fn flatten_double_not(expr: Expr) -> Expr {
    if let Expr::Not(inner) = expr {
        if let Expr::Not(inner2) = *inner {
            // Double NOT — return the inner expression (already normalised).
            return flatten_double_not(*inner2);
        }
        // Single NOT — normalise the inner expression and rebuild.
        return Expr::Not(Box::new(flatten_double_not(*inner)));
    }
    expr
}

// ── Rule 4: Constant-fold IS NULL / IS NOT NULL ───────────────────────────────

/// Constant-fold `IS NULL` and `IS NOT NULL` on known-non-null literals.
///
/// # NORMALIZATION(openCypher 9 §6.1):
/// A literal value that is not `null` is definitively not null.
/// `IS NULL(42)` → `false`, `IS NOT NULL('hi')` → `true`.
pub fn fold_null_checks(expr: Expr) -> Expr {
    match expr {
        Expr::IsNull(ref inner) => {
            if let Expr::Literal(ref lit) = **inner {
                return if matches!(lit, crate::lqa::expr::Literal::Null) {
                    Expr::bool(true)
                } else {
                    Expr::bool(false)
                };
            }
            expr
        }
        Expr::IsNotNull(ref inner) => {
            if let Expr::Literal(ref lit) = **inner {
                return if matches!(lit, crate::lqa::expr::Literal::Null) {
                    Expr::bool(false)
                } else {
                    Expr::bool(true)
                };
            }
            expr
        }
        other => other,
    }
}

/// Recursively apply [`flatten_double_not`] and [`fold_null_checks`] to an
/// expression tree.
///
/// Rules are applied in order:
/// 1. `flatten_double_not` — removes `NOT(NOT(…))` pairs first so that inner
///    expressions are exposed for constant folding.
/// 2. `fold_null_checks` — constant-folds `IS NULL` / `IS NOT NULL` on
///    literals after double-NOT removal reveals them.
///
/// This is intentionally shallow in Phase 3 — it normalises the top-level
/// expression and its immediate children.  A full recursive traversal will
/// be added in Phase 4 alongside the type-annotation pass.
pub fn normalize_expr(expr: Expr) -> Expr {
    // Apply double-NOT removal first: this may expose a foldable IS NULL.
    let expr = flatten_double_not(expr);
    // Then constant-fold null checks on the (now de-nested) expression.
    fold_null_checks(expr)
}

/// Apply expression normalisation to all predicates and projections in an
/// operator tree.
///
/// Phase 3 scope: normalises only `Selection` predicates and `Projection`
/// items.  Full recursive tree normalisation is Phase 4.
pub fn normalize_op(op: Op) -> Op {
    match op {
        Op::Selection { inner, predicate } => Op::Selection {
            inner,
            predicate: normalize_expr(predicate),
        },
        Op::Projection { inner, items, distinct } => {
            let items = items
                .into_iter()
                .map(|pi| ProjItem { expr: normalize_expr(pi.expr), alias: pi.alias })
                .collect();
            Op::Projection { inner, items, distinct }
        }
        other => other,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lqa::expr::{Expr, Literal, CmpOp};

    #[test]
    fn simple_case_one_branch() {
        // CASE n WHEN 1 THEN 'a' END
        let subject = Expr::var("n");
        let branches = vec![(Expr::int(1), Expr::str("a"))];
        let result = simple_case_to_searched(subject, branches, None);
        match result {
            Expr::CaseSearched { branches, else_expr } => {
                assert_eq!(branches.len(), 1);
                assert!(else_expr.is_none());
                assert!(matches!(branches[0].0, Expr::Comparison(CmpOp::Eq, _, _)));
            }
            other => panic!("expected CaseSearched, got {other:?}"),
        }
    }

    #[test]
    fn simple_case_multiple_branches_with_else() {
        // CASE status WHEN 'active' THEN 1 WHEN 'inactive' THEN 0 ELSE -1 END
        let subject = Expr::var("status");
        let branches = vec![
            (Expr::str("active"), Expr::int(1)),
            (Expr::str("inactive"), Expr::int(0)),
        ];
        let result = simple_case_to_searched(subject, branches, Some(Expr::int(-1)));
        match result {
            Expr::CaseSearched { branches, else_expr } => {
                assert_eq!(branches.len(), 2);
                assert!(else_expr.is_some());
                // Each branch should compare subject = value
                for (cond, _) in &branches {
                    assert!(matches!(cond, Expr::Comparison(CmpOp::Eq, _, _)));
                }
            }
            other => panic!("expected CaseSearched, got {other:?}"),
        }
    }

    #[test]
    fn simple_case_clones_subject_per_branch() {
        // Verify that the subject is cloned independently for each branch.
        let subject = Expr::var("x");
        let branches = vec![
            (Expr::int(1), Expr::bool(true)),
            (Expr::int(2), Expr::bool(false)),
            (Expr::int(3), Expr::bool(true)),
        ];
        let result = simple_case_to_searched(subject, branches, None);
        if let Expr::CaseSearched { branches, .. } = result {
            assert_eq!(branches.len(), 3);
            // Subject appears as LHS in each comparison; must be independent clones.
            for (cond, _) in &branches {
                if let Expr::Comparison(_, lhs, _) = cond {
                    assert!(matches!(**lhs, Expr::Variable { ref name, .. } if name == "x"));
                }
            }
        }
    }

    #[test]
    fn alias_gen_sequential() {
        let mut gen = AliasGen::new();
        assert_eq!(gen.next(), "?gen_0");
        assert_eq!(gen.next(), "?gen_1");
        assert_eq!(gen.next(), "?gen_2");
    }

    #[test]
    fn desugar_implicit_alias_fills_missing() {
        let mut gen = AliasGen::new();
        let items = vec![
            (Expr::var("n"), None),
            (Expr::var("m"), Some("other".to_string())),
        ];
        let result = desugar_implicit_alias(items, &mut gen);
        assert_eq!(result[0].alias, "?gen_0");
        assert_eq!(result[1].alias, "other");
    }

    #[test]
    fn desugar_implicit_alias_preserves_explicit() {
        let mut gen = AliasGen::new();
        let items = vec![
            (Expr::var("a"), Some("aa".to_string())),
            (Expr::var("b"), Some("bb".to_string())),
        ];
        let result = desugar_implicit_alias(items, &mut gen);
        assert_eq!(result[0].alias, "aa");
        assert_eq!(result[1].alias, "bb");
        // Counter should not have advanced since all had explicit aliases.
        assert_eq!(gen.counter, 0);
    }

    #[test]
    fn flatten_double_not_removes_pair() {
        let expr = Expr::Not(Box::new(Expr::Not(Box::new(Expr::bool(true)))));
        let result = flatten_double_not(expr);
        assert_eq!(result, Expr::bool(true));
    }

    #[test]
    fn flatten_triple_not_simplified() {
        // NOT(NOT(NOT(x))) = NOT(x)
        let inner = Expr::var("x");
        let expr = Expr::Not(Box::new(Expr::Not(Box::new(Expr::Not(Box::new(inner))))));
        let result = flatten_double_not(expr);
        assert!(matches!(result, Expr::Not(inner) if matches!(*inner, Expr::Variable { .. })));
    }

    #[test]
    fn flatten_single_not_unchanged() {
        let expr = Expr::Not(Box::new(Expr::bool(false)));
        let result = flatten_double_not(expr);
        assert!(matches!(result, Expr::Not(_)));
    }

    #[test]
    fn fold_is_null_on_null_literal() {
        let expr = Expr::IsNull(Box::new(Expr::Literal(Literal::Null)));
        let result = fold_null_checks(expr);
        assert_eq!(result, Expr::bool(true));
    }

    #[test]
    fn fold_is_null_on_integer_literal() {
        let expr = Expr::IsNull(Box::new(Expr::int(42)));
        let result = fold_null_checks(expr);
        assert_eq!(result, Expr::bool(false));
    }

    #[test]
    fn fold_is_not_null_on_null_literal() {
        let expr = Expr::IsNotNull(Box::new(Expr::Literal(Literal::Null)));
        let result = fold_null_checks(expr);
        assert_eq!(result, Expr::bool(false));
    }

    #[test]
    fn fold_is_not_null_on_string_literal() {
        let expr = Expr::IsNotNull(Box::new(Expr::str("hello")));
        let result = fold_null_checks(expr);
        assert_eq!(result, Expr::bool(true));
    }

    #[test]
    fn fold_is_null_on_variable_unchanged() {
        let expr = Expr::IsNull(Box::new(Expr::var("n")));
        let result = fold_null_checks(expr);
        // Variable is runtime — can't fold
        assert!(matches!(result, Expr::IsNull(_)));
    }

    #[test]
    fn normalize_expr_double_not_null_check() {
        // NOT(NOT(IS NULL(null))) = IS NULL(null) = true
        let inner = Expr::IsNull(Box::new(Expr::Literal(Literal::Null)));
        let expr = Expr::Not(Box::new(Expr::Not(Box::new(inner))));
        let result = normalize_expr(expr);
        // After double-NOT removal and null fold
        assert_eq!(result, Expr::bool(true));
    }

    #[test]
    fn normalize_op_selection_normalises_predicate() {
        // NOT(NOT(IS NULL(?n))) in a Selection should be folded.
        let predicate = Expr::Not(Box::new(Expr::Not(Box::new(
            Expr::IsNull(Box::new(Expr::Literal(Literal::Null))),
        ))));
        let op = Op::Selection {
            inner: Box::new(Op::Unit),
            predicate,
        };
        let normalised = normalize_op(op);
        if let Op::Selection { predicate, .. } = normalised {
            assert_eq!(predicate, Expr::bool(true));
        } else {
            panic!("expected Selection");
        }
    }

    #[test]
    fn normalize_op_projection_normalises_items() {
        use crate::lqa::op::ProjItem;
        let items = vec![ProjItem {
            expr: Expr::Not(Box::new(Expr::Not(Box::new(Expr::bool(false))))),
            alias: "x".into(),
        }];
        let op = Op::Projection { inner: Box::new(Op::Unit), items, distinct: false };
        let normalised = normalize_op(op);
        if let Op::Projection { items, .. } = normalised {
            assert_eq!(items[0].expr, Expr::bool(false));
        } else {
            panic!("expected Projection");
        }
    }
}
