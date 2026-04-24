// ── Peephole optimizations ────────────────────────────────────────────────────

/// Detect `WITH … collect(X) AS list … / UNWIND list AS item` pairs and rewrite
/// them into simple projections (`WITH … X AS item …`).  This eliminates the
/// need for runtime list iteration which SPARQL 1.1 cannot express.
/// Return `true` if `expr` contains a property access (`x.key`) where `x` is
/// one of the deleted variables. Used to decide whether a DELETE clause is
/// safe to skip (when only metadata like `type(r)` is accessed).
fn expr_accesses_deleted_prop(
    expr: &Expression,
    deleted_vars: &std::collections::HashSet<String>,
) -> bool {
    match expr {
        Expression::Property(base, _) => {
            if let Expression::Variable(v) = base.as_ref() {
                return deleted_vars.contains(v.as_str());
            }
            expr_accesses_deleted_prop(base, deleted_vars)
        }
        Expression::FunctionCall { name, args, .. } => {
            // labels(n) / id(n) / etc. on a deleted var is unsafe too.
            let is_unsafe_fn = matches!(
                name.to_lowercase().as_str(),
                "labels" | "id" | "elementid" | "properties" | "keys"
            );
            if is_unsafe_fn {
                if let Some(Expression::Variable(v)) = args.first() {
                    if deleted_vars.contains(v.as_str()) {
                        return true;
                    }
                }
            }
            args.iter()
                .any(|a| expr_accesses_deleted_prop(a, deleted_vars))
        }
        Expression::Or(a, b)
        | Expression::And(a, b)
        | Expression::Xor(a, b)
        | Expression::Comparison(a, _, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b)
        | Expression::Modulo(a, b)
        | Expression::Power(a, b) => {
            expr_accesses_deleted_prop(a, deleted_vars)
                || expr_accesses_deleted_prop(b, deleted_vars)
        }
        Expression::Not(e)
        | Expression::Negate(e)
        | Expression::IsNull(e)
        | Expression::IsNotNull(e) => expr_accesses_deleted_prop(e, deleted_vars),
        Expression::List(elems) => elems
            .iter()
            .any(|e| expr_accesses_deleted_prop(e, deleted_vars)),
        Expression::Map(pairs) => pairs
            .iter()
            .any(|(_, e)| expr_accesses_deleted_prop(e, deleted_vars)),
        Expression::Aggregate(agg) => {
            let inner = match agg {
                AggregateExpr::Count { expr, .. } => expr.as_deref(),
                AggregateExpr::Sum { expr, .. }
                | AggregateExpr::Avg { expr, .. }
                | AggregateExpr::Min { expr, .. }
                | AggregateExpr::Max { expr, .. }
                | AggregateExpr::Collect { expr, .. } => Some(expr.as_ref()),
            };
            inner.map_or(false, |e| expr_accesses_deleted_prop(e, deleted_vars))
        }
        _ => false,
    }
}

fn eliminate_collect_unwind(clauses: &[Clause]) -> Vec<Clause> {
    let mut result: Vec<Clause> = Vec::new();
    let mut i = 0;
    while i < clauses.len() {
        // Look for WITH(..., collect(X) AS list_var, ...) followed by UNWIND list_var AS item
        if let (Clause::With(w), Some(Clause::Unwind(u))) = (&clauses[i], clauses.get(i + 1)) {
            if let Expression::Variable(unwind_list_var) = &u.expression {
                if let crate::ast::cypher::ReturnItems::Explicit(items) = &w.items {
                    // Find a collect() aggregate aliased to the UNWIND source variable.
                    let collect_idx = items.iter().position(|item| {
                        if let Expression::Aggregate(AggregateExpr::Collect { .. }) =
                            &item.expression
                        {
                            item.alias.as_deref() == Some(unwind_list_var.as_str())
                        } else {
                            false
                        }
                    });

                    if let Some(ci) = collect_idx {
                        if let Expression::Aggregate(AggregateExpr::Collect { expr, .. }) =
                            &items[ci].expression
                        {
                            // Build new WITH items:
                            //  - remove the collect() item
                            //  - add the inner expression aliased to the UNWIND output var
                            let mut new_items: Vec<crate::ast::cypher::ReturnItem> = items
                                .iter()
                                .enumerate()
                                .filter(|(idx, _)| *idx != ci)
                                .map(|(_, item)| item.clone())
                                .collect();
                            new_items.push(crate::ast::cypher::ReturnItem {
                                expression: *expr.clone(),
                                alias: Some(u.variable.clone()),
                            });

                            // Emit the modified WITH (now aggregate-free for this item).
                            result.push(Clause::With(crate::ast::cypher::WithClause {
                                distinct: w.distinct,
                                items: crate::ast::cypher::ReturnItems::Explicit(new_items),
                                where_: w.where_.clone(),
                                order_by: w.order_by.clone(),
                                skip: w.skip.clone(),
                                limit: w.limit.clone(),
                            }));
                            i += 2; // skip both WITH and UNWIND
                            continue;
                        }
                    }
                }
            }
        }
        result.push(clauses[i].clone());
        i += 1;
    }
    result
}

// ── Free helpers ──────────────────────────────────────────────────────────────

/// Collect all named variable strings from a MATCH pattern list (nodes and edges).
fn collect_pattern_vars(pattern_list: &crate::ast::cypher::PatternList) -> Vec<String> {
    let mut vars = Vec::new();
    for pattern in &pattern_list.0 {
        for elem in &pattern.elements {
            match elem {
                PatternElement::Node(n) => {
                    if let Some(v) = &n.variable {
                        vars.push(v.clone());
                    }
                }
                PatternElement::Relationship(r) => {
                    if let Some(v) = &r.variable {
                        vars.push(v.clone());
                    }
                }
            }
        }
    }
    vars
}

/// Returns true if `expr` references `variable.key` (the specific property access).
/// Used to detect circular SET assignments.
fn expr_references_prop(expr: &Expression, variable: &str, key: &str) -> bool {
    match expr {
        Expression::Property(base, k) => {
            if k == key {
                if let Expression::Variable(v) = base.as_ref() {
                    if v == variable {
                        return true;
                    }
                }
            }
            expr_references_prop(base, variable, key)
        }
        Expression::Variable(_) | Expression::Literal(_) => false,
        Expression::IsNull(e)
        | Expression::IsNotNull(e)
        | Expression::Not(e)
        | Expression::Negate(e) => expr_references_prop(e, variable, key),
        Expression::Or(a, b)
        | Expression::And(a, b)
        | Expression::Xor(a, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b)
        | Expression::Modulo(a, b)
        | Expression::Power(a, b)
        | Expression::Comparison(a, _, b) => {
            expr_references_prop(a, variable, key) || expr_references_prop(b, variable, key)
        }
        Expression::List(items) => items.iter().any(|e| expr_references_prop(e, variable, key)),
        Expression::Map(pairs) => pairs
            .iter()
            .any(|(_, v)| expr_references_prop(v, variable, key)),
        Expression::FunctionCall { args, .. } => {
            args.iter().any(|e| expr_references_prop(e, variable, key))
        }
        _ => false,
    }
}

/// Returns true if `expr` references a variable with the given name.
fn expr_references_var(expr: &Expression, name: &str) -> bool {
    match expr {
        Expression::Variable(v) => v == name,
        Expression::Property(base, _) => expr_references_var(base, name),
        Expression::IsNull(e)
        | Expression::IsNotNull(e)
        | Expression::Not(e)
        | Expression::Negate(e) => expr_references_var(e, name),
        Expression::Or(a, b)
        | Expression::And(a, b)
        | Expression::Xor(a, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b)
        | Expression::Modulo(a, b)
        | Expression::Power(a, b)
        | Expression::Comparison(a, _, b) => {
            expr_references_var(a, name) || expr_references_var(b, name)
        }
        Expression::List(items) => items.iter().any(|e| expr_references_var(e, name)),
        Expression::Map(pairs) => pairs.iter().any(|(_, v)| expr_references_var(v, name)),
        Expression::FunctionCall { args, .. } => args.iter().any(|e| expr_references_var(e, name)),
        Expression::Aggregate(agg) => match agg {
            AggregateExpr::Count { expr, .. } => expr
                .as_ref()
                .map_or(false, |e| expr_references_var(e, name)),
            AggregateExpr::Sum { expr, .. }
            | AggregateExpr::Avg { expr, .. }
            | AggregateExpr::Min { expr, .. }
            | AggregateExpr::Max { expr, .. }
            | AggregateExpr::Collect { expr, .. } => expr_references_var(expr, name),
        },
        _ => false,
    }
}

/// Returns true if `expr` contains a numeric arithmetic operator (Modulo, Multiply, Divide,
/// Subtract, Power, or Negate) that is applied directly to `var` or to an expression
/// containing `var`. Used to detect `InvalidArgumentType` at compile time.
fn predicate_uses_numeric_arithmetic(expr: &Expression, var: &str) -> bool {
    match expr {
        Expression::Modulo(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b)
        | Expression::Subtract(a, b)
        | Expression::Power(a, b) => {
            expr_references_var(a, var)
                || expr_references_var(b, var)
                || predicate_uses_numeric_arithmetic(a, var)
                || predicate_uses_numeric_arithmetic(b, var)
        }
        Expression::Negate(e) => {
            expr_references_var(e, var) || predicate_uses_numeric_arithmetic(e, var)
        }
        Expression::Comparison(a, _, b) => {
            predicate_uses_numeric_arithmetic(a, var) || predicate_uses_numeric_arithmetic(b, var)
        }
        Expression::Or(a, b) | Expression::And(a, b) | Expression::Xor(a, b) => {
            predicate_uses_numeric_arithmetic(a, var) || predicate_uses_numeric_arithmetic(b, var)
        }
        _ => false,
    }
}

/// Substitute every occurrence of variable `var` with `replacement` in `expr`.
///
/// Used to expand quantifier predicates over literal lists: given
/// `all(x IN [1, 2, 3] WHERE x > 0)`, substitute x→1, x→2, x→3 and AND the
/// results together.  Inner quantifiers/comprehensions that shadow `var` are
/// left unchanged.
fn substitute_var_in_expr(expr: &Expression, var: &str, replacement: &Expression) -> Expression {
    match expr {
        Expression::Variable(v) if v.as_str() == var => replacement.clone(),
        // Unary
        Expression::Not(e) => {
            Expression::Not(Box::new(substitute_var_in_expr(e, var, replacement)))
        }
        Expression::Negate(e) => {
            Expression::Negate(Box::new(substitute_var_in_expr(e, var, replacement)))
        }
        Expression::IsNull(e) => {
            Expression::IsNull(Box::new(substitute_var_in_expr(e, var, replacement)))
        }
        Expression::IsNotNull(e) => {
            Expression::IsNotNull(Box::new(substitute_var_in_expr(e, var, replacement)))
        }
        // Binary logical
        Expression::Or(a, b) => Expression::Or(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        Expression::And(a, b) => Expression::And(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        Expression::Xor(a, b) => Expression::Xor(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        // Binary arithmetic / comparison
        Expression::Add(a, b) => Expression::Add(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        Expression::Subtract(a, b) => Expression::Subtract(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        Expression::Multiply(a, b) => Expression::Multiply(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        Expression::Divide(a, b) => Expression::Divide(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        Expression::Modulo(a, b) => Expression::Modulo(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        Expression::Power(a, b) => Expression::Power(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        Expression::Comparison(a, op, b) => Expression::Comparison(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            op.clone(),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        // Property access: substitute in the base object
        Expression::Property(base, key) => Expression::Property(
            Box::new(substitute_var_in_expr(base, var, replacement)),
            key.clone(),
        ),
        Expression::Subscript(a, b) => Expression::Subscript(
            Box::new(substitute_var_in_expr(a, var, replacement)),
            Box::new(substitute_var_in_expr(b, var, replacement)),
        ),
        Expression::ListSlice { list, start, end } => Expression::ListSlice {
            list: Box::new(substitute_var_in_expr(list, var, replacement)),
            start: start
                .as_ref()
                .map(|e| Box::new(substitute_var_in_expr(e, var, replacement))),
            end: end
                .as_ref()
                .map(|e| Box::new(substitute_var_in_expr(e, var, replacement))),
        },
        // Function call: substitute all args
        Expression::FunctionCall {
            name,
            distinct,
            args,
        } => Expression::FunctionCall {
            name: name.clone(),
            distinct: *distinct,
            args: args
                .iter()
                .map(|a| substitute_var_in_expr(a, var, replacement))
                .collect(),
        },
        // List literal: substitute element-wise
        Expression::List(items) => Expression::List(
            items
                .iter()
                .map(|i| substitute_var_in_expr(i, var, replacement))
                .collect(),
        ),
        // Nested quantifier: stop at inner variable shadowing
        Expression::QuantifierExpr {
            kind,
            variable: inner_var,
            list,
            predicate,
        } if inner_var.as_str() != var => Expression::QuantifierExpr {
            kind: kind.clone(),
            variable: inner_var.clone(),
            list: Box::new(substitute_var_in_expr(list, var, replacement)),
            predicate: predicate
                .as_ref()
                .map(|p| Box::new(substitute_var_in_expr(p, var, replacement))),
        },
        // List comprehension: stop at inner variable shadowing
        Expression::ListComprehension {
            variable: lc_var,
            list,
            predicate,
            projection,
        } if lc_var.as_str() != var => Expression::ListComprehension {
            variable: lc_var.clone(),
            list: Box::new(substitute_var_in_expr(list, var, replacement)),
            predicate: predicate
                .as_ref()
                .map(|p| Box::new(substitute_var_in_expr(p, var, replacement))),
            projection: projection
                .as_ref()
                .map(|p| Box::new(substitute_var_in_expr(p, var, replacement))),
        },
        Expression::CaseExpression {
            operand,
            whens,
            else_expr,
        } => Expression::CaseExpression {
            operand: operand
                .as_ref()
                .map(|o| Box::new(substitute_var_in_expr(o, var, replacement))),
            whens: whens
                .iter()
                .map(|(w, t)| {
                    (
                        substitute_var_in_expr(w, var, replacement),
                        substitute_var_in_expr(t, var, replacement),
                    )
                })
                .collect(),
            else_expr: else_expr
                .as_ref()
                .map(|e| Box::new(substitute_var_in_expr(e, var, replacement))),
        },
        // Everything else (literals, map, label check, etc.) is left unchanged
        _ => expr.clone(),
    }
}

/// Returns true if the expression references any variable in `nullable`.
fn expr_uses_nullable(expr: &Expression, nullable: &std::collections::HashSet<String>) -> bool {
    match expr {
        Expression::Variable(v) => nullable.contains(v),
        Expression::Property(base, _) => expr_uses_nullable(base, nullable),
        Expression::IsNull(e)
        | Expression::IsNotNull(e)
        | Expression::Not(e)
        | Expression::Negate(e) => expr_uses_nullable(e, nullable),
        Expression::Or(a, b)
        | Expression::And(a, b)
        | Expression::Xor(a, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b)
        | Expression::Modulo(a, b)
        | Expression::Power(a, b)
        | Expression::Comparison(a, _, b) => {
            expr_uses_nullable(a, nullable) || expr_uses_nullable(b, nullable)
        }
        Expression::FunctionCall { args, .. } => {
            args.iter().any(|a| expr_uses_nullable(a, nullable))
        }
        Expression::List(items) => items.iter().any(|i| expr_uses_nullable(i, nullable)),
        Expression::LabelCheck { variable, .. } => nullable.contains(variable),
        Expression::PatternPredicate(_) => false,
        Expression::ExistsSubquery { .. } => false,
        Expression::Aggregate(_) | Expression::Literal(_) | Expression::Map(_) => false,
        Expression::CaseExpression {
            operand,
            whens,
            else_expr,
        } => {
            operand
                .as_ref()
                .map_or(false, |e| expr_uses_nullable(e, nullable))
                || whens.iter().any(|(w, t)| {
                    expr_uses_nullable(w, nullable) || expr_uses_nullable(t, nullable)
                })
                || else_expr
                    .as_ref()
                    .map_or(false, |e| expr_uses_nullable(e, nullable))
        }
        Expression::QuantifierExpr {
            list, predicate, ..
        } => {
            expr_uses_nullable(list, nullable)
                || predicate
                    .as_ref()
                    .map_or(false, |e| expr_uses_nullable(e, nullable))
        }
        Expression::Subscript(a, b) => {
            expr_uses_nullable(a, nullable) || expr_uses_nullable(b, nullable)
        }
        Expression::ListSlice { list, start, end } => {
            expr_uses_nullable(list, nullable)
                || start
                    .as_ref()
                    .map_or(false, |e| expr_uses_nullable(e, nullable))
                || end
                    .as_ref()
                    .map_or(false, |e| expr_uses_nullable(e, nullable))
        }
        Expression::ListComprehension {
            list,
            predicate,
            projection,
            ..
        } => {
            expr_uses_nullable(list, nullable)
                || predicate
                    .as_ref()
                    .map_or(false, |e| expr_uses_nullable(e, nullable))
                || projection
                    .as_ref()
                    .map_or(false, |e| expr_uses_nullable(e, nullable))
        }
        Expression::PatternComprehension {
            predicate,
            projection,
            ..
        } => {
            predicate
                .as_ref()
                .map_or(false, |e| expr_uses_nullable(e, nullable))
                || expr_uses_nullable(projection, nullable)
        }
    }
}

/// Extract type/sentinel guard triples for a specific SPARQL variable from a graph pattern.
///
/// Returns all TriplePatterns from the pattern (recursively) where:
///   - the subject is the given variable
///   - the predicate is `rdf:type` or a sentinel predicate (ends with `/__node`)
///
/// These triples are used to guard property-access OPTIONALs for nullable variables,
/// preventing wildcard expansion when the class doesn't exist.
fn extract_type_guards(pattern: &GraphPattern, var_name: &str) -> Vec<TriplePattern> {
    let mut guards = Vec::new();
    collect_type_guards_rec(pattern, var_name, &mut guards);
    guards
}

fn collect_type_guards_rec(pattern: &GraphPattern, var_name: &str, out: &mut Vec<TriplePattern>) {
    match pattern {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                if let TermPattern::Variable(s) = &tp.subject {
                    if s.as_str() == var_name {
                        let pred_str = match &tp.predicate {
                            spargebra::term::NamedNodePattern::NamedNode(nn) => {
                                nn.as_str().to_owned()
                            }
                            spargebra::term::NamedNodePattern::Variable(v) => v.as_str().to_owned(),
                        };
                        if pred_str == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
                            || pred_str.ends_with("/__node")
                        {
                            out.push(tp.clone());
                        }
                    }
                }
            }
        }
        GraphPattern::LeftJoin { left, right, .. } => {
            collect_type_guards_rec(left, var_name, out);
            collect_type_guards_rec(right, var_name, out);
        }
        GraphPattern::Join { left, right } => {
            collect_type_guards_rec(left, var_name, out);
            collect_type_guards_rec(right, var_name, out);
        }
        GraphPattern::Filter { inner, .. } | GraphPattern::Project { inner, .. } => {
            collect_type_guards_rec(inner, var_name, out);
        }
        _ => {}
    }
}

/// Build an empty BGP for use as the identity element in joins.
fn empty_bgp() -> GraphPattern {
    GraphPattern::Bgp { patterns: vec![] }
}

/// Build an OPTIONAL pattern that guards against nullable subject wildcard expansion.
///
/// When `guard_triples` is non-empty (the type/sentinel triples from the OPTIONAL MATCH
/// that introduced `?subj`), we produce:
///
///   `OPTIONAL { ?subj rdf:type X . ?subj <pred> ?obj }`
fn nullable_subject_optional(
    tp: TriplePattern,
    subj_var: Variable,
    guard_triples: Vec<TriplePattern>,
) -> GraphPattern {
    if !guard_triples.is_empty() {
        let mut all_patterns = guard_triples;
        all_patterns.push(tp);
        GraphPattern::Bgp {
            patterns: all_patterns,
        }
    } else {
        let mut project_vars = vec![subj_var.clone()];
        if let TermPattern::Variable(obj_var) = &tp.object {
            project_vars.push(obj_var.clone());
        }
        let inner = GraphPattern::Filter {
            inner: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
            expr: SparExpr::Bound(subj_var),
        };
        GraphPattern::Project {
            inner: Box::new(inner),
            variables: project_vars,
        }
    }
}

/// Extract the (value_str, datatype_str) from a SPARQL literal expression.
fn extract_lit_num(expr: &SparExpr) -> Option<(&str, &str)> {
    if let SparExpr::Literal(lit) = expr {
        let ds = lit.datatype().as_str();
        if ds == XSD_INTEGER || ds == XSD_DOUBLE {
            return Some((lit.value(), ds));
        }
    }
    None
}

/// Try to constant-fold a binary arithmetic expression.
/// Returns the folded literal if both operands are numeric literals.
fn try_const_fold_arith(op: char, la: &SparExpr, lb: &SparExpr) -> Option<SparExpr> {
    let (lv, ld) = extract_lit_num(la)?;
    let (rv, rd) = extract_lit_num(lb)?;

    if ld == XSD_INTEGER && rd == XSD_INTEGER {
        let l: i64 = lv.parse().ok()?;
        let r: i64 = rv.parse().ok()?;
        let result = match op {
            '+' => l.checked_add(r)?,
            '-' => l.checked_sub(r)?,
            '*' => l.checked_mul(r)?,
            '/' => {
                if r == 0 {
                    return None;
                }
                // Cypher truncates toward zero
                l / r
            }
            '%' => {
                if r == 0 {
                    return None;
                }
                l % r
            }
            _ => return None,
        };
        return Some(SparExpr::Literal(SparLit::new_typed_literal(
            result.to_string(),
            NamedNode::new_unchecked(XSD_INTEGER),
        )));
    }

    // Mixed integer/double → double
    let ld_ok = ld == XSD_INTEGER || ld == XSD_DOUBLE;
    let rd_ok = rd == XSD_INTEGER || rd == XSD_DOUBLE;
    if ld_ok && rd_ok {
        let l: f64 = lv.parse().ok()?;
        let r: f64 = rv.parse().ok()?;
        let result: f64 = match op {
            '+' => l + r,
            '-' => l - r,
            '*' => l * r,
            '/' => l / r,
            '%' => l % r,
            _ => return None,
        };
        if !result.is_finite() {
            return None;
        }
        // If both were integers but one is xsd:double, output is double
        let out_type = if ld == XSD_DOUBLE || rd == XSD_DOUBLE {
            XSD_DOUBLE
        } else {
            XSD_INTEGER
        };
        let val_str = if out_type == XSD_DOUBLE {
            format!("{result:?}")
        } else {
            (result as i64).to_string()
        };
        return Some(SparExpr::Literal(SparLit::new_typed_literal(
            val_str,
            NamedNode::new_unchecked(out_type),
        )));
    }

    None
}

/// Format a float value in Cypher / Neo4j compatible style:
/// - `-0.0` normalises to `0.0`
/// - small-magnitude values (exponent -6 to +9) use decimal notation
/// - very large / very small values use `1.23e4` / `1.23e-5` style
fn cypher_float_str(f: f64) -> String {
    // Neg-zero normalisation
    if f == 0.0 {
        return "0.0".to_string();
    }
    // Use Rust Debug ("{:?}") as the base, which gives the shortest round-trip decimal.
    let s = format!("{f:?}");
    // Handle cases where Debug gives scientific notation like "1e-5" or "1.5e300".
    if let Some(e_pos) = s.to_lowercase().find('e') {
        let mantissa = &s[..e_pos];
        let exp_str = &s[e_pos + 1..];
        if let Ok(exp) = exp_str.parse::<i32>() {
            // Cypher decimal range: -6 ≤ exp ≤ 9
            if exp >= -6 && exp <= 9 {
                // Expand to decimal. Build mantissa digits.
                let neg = mantissa.starts_with('-');
                let mant_abs = if neg { &mantissa[1..] } else { mantissa };
                let (int_part, frac_part) = if let Some(d) = mant_abs.find('.') {
                    (&mant_abs[..d], &mant_abs[d + 1..])
                } else {
                    (mant_abs, "")
                };
                // Combine digits without decimal point
                let all_digits = format!("{}{}", int_part, frac_part);
                // Number of digits that belong to the integer part at position 0:
                // int_part has int_part.len() digits; shift by exp
                let int_len = int_part.len() as i32 + exp;
                let result = if int_len >= all_digits.len() as i32 {
                    // All digits are in integer part, add trailing zeros + ".0"
                    let zeros = (int_len - all_digits.len() as i32) as usize;
                    format!(
                        "{}{}{}.0",
                        if neg { "-" } else { "" },
                        all_digits,
                        "0".repeat(zeros),
                    )
                } else if int_len <= 0 {
                    // All digits are in fractional part, add leading zeros
                    let leading = (-int_len) as usize;
                    format!(
                        "{}0.{}{}",
                        if neg { "-" } else { "" },
                        "0".repeat(leading),
                        all_digits,
                    )
                } else {
                    // Split into integer and fractional
                    let (i_digits, f_digits) = all_digits.split_at(int_len as usize);
                    if f_digits.is_empty() {
                        format!("{}{}.0", if neg { "-" } else { "" }, i_digits)
                    } else {
                        format!("{}{}.{}", if neg { "-" } else { "" }, i_digits, f_digits)
                    }
                };
                return result;
            }
        }
    }
    // Already in decimal notation or exponent out of range — return as-is,
    // but ensure there's always a decimal point + at least one fractional digit.
    if !s.contains('.') && !s.to_lowercase().contains('e') {
        return format!("{s}.0");
    }
    s
}

/// Generate a canonical string key for an aggregate expression, used to
/// match ORDER BY aggregate expressions to already-computed RETURN aggregates.
fn agg_expr_key(agg: &crate::ast::cypher::AggregateExpr) -> String {
    use crate::ast::cypher::{AggregateExpr, Expression, Literal};
    fn expr_key(e: &Expression) -> String {
        match e {
            Expression::Variable(v) => v.clone(),
            Expression::Literal(Literal::Integer(n)) => n.to_string(),
            Expression::Literal(Literal::String(s)) => format!("'{s}'"),
            _ => format!("{e:?}"),
        }
    }
    match agg {
        AggregateExpr::Count {
            distinct,
            expr: None,
        } => format!("count_{}", if *distinct { "d_star" } else { "star" }),
        AggregateExpr::Count {
            distinct,
            expr: Some(e),
        } => format!("count_{}{}", if *distinct { "d_" } else { "" }, expr_key(e)),
        AggregateExpr::Sum { distinct, expr } => format!(
            "sum_{}{}",
            if *distinct { "d_" } else { "" },
            expr_key(expr)
        ),
        AggregateExpr::Avg { distinct, expr } => format!(
            "avg_{}{}",
            if *distinct { "d_" } else { "" },
            expr_key(expr)
        ),
        AggregateExpr::Min { distinct, expr } => format!(
            "min_{}{}",
            if *distinct { "d_" } else { "" },
            expr_key(expr)
        ),
        AggregateExpr::Max { distinct, expr } => format!(
            "max_{}{}",
            if *distinct { "d_" } else { "" },
            expr_key(expr)
        ),
        AggregateExpr::Collect { distinct, expr } => format!(
            "collect_{}{}",
            if *distinct { "d_" } else { "" },
            expr_key(expr)
        ),
    }
}

/// Serialize a list of expressions to a string like `[1, 2, 'foo']`.
fn serialize_list_literal(elems: &[Expression]) -> String {
    let parts: Vec<String> = elems.iter().map(serialize_list_element).collect();
    format!("[{}]", parts.join(", "))
}

/// Constant-fold `base ^ exponent` when both are numeric literals.
/// Power in Cypher always returns double.
fn try_const_fold_pow(base: &SparExpr, exp: &SparExpr) -> Option<SparExpr> {
    let (bv, bd) = extract_lit_num(base)?;
    let (ev, _ed) = extract_lit_num(exp)?;
    let b: f64 = bv.parse().ok()?;
    let e: f64 = ev.parse().ok()?;
    let result = b.powf(e);
    if !result.is_finite() {
        return None;
    }
    let _ = bd; // suppress unused variable
    Some(SparExpr::Literal(SparLit::new_typed_literal(
        format!("{result:?}"),
        NamedNode::new_unchecked(XSD_DOUBLE),
    )))
}

/// Returns true if `s` looks like a compile-time temporal value string:
/// a date (`YYYY-…`) or a time/localtime (`HH:…`).
fn is_temporal_lit_str(s: &str) -> bool {
    let b = s.as_bytes();
    // Date: 4-digit year followed by '-'
    if b.len() >= 5 && b[4] == b'-' && b[..4].iter().all(|c| c.is_ascii_digit()) {
        return true;
    }
    // Time / localtime: 2-digit hour followed by ':'
    if b.len() >= 3 && b[2] == b':' && b[..2].iter().all(|c| c.is_ascii_digit()) {
        return true;
    }
    false
}

/// Returns true if `s` is a plain date string (exactly `YYYY-MM-DD`, length 10).
fn is_date_only_lit_str(s: &str) -> bool {
    s.len() == 10 && s.as_bytes().get(4) == Some(&b'-')
}

/// Build a SPARQL expression that computes `temporal - duration`.
///
/// Oxigraph supports `temporal - xsd:yearMonthDuration` and
/// `temporal - xsd:dayTimeDuration` but NOT `temporal - xsd:duration`.
/// We work around this by splitting the duration into its yearMonth and
/// dayTime string parts via `STRDT(REPLACE(…), …)` and subtracting each:
///
/// ```sparql
/// COALESCE(temporal - STRDT(REPLACE(STR(dur), "^P(([0-9.]*Y)?([0-9.]*M)?).*", "P$1"), ymd), temporal)
///   - STRDT(REPLACE(STR(dur), "^P([0-9.]*Y)?([0-9.]*M)?", "P"), dtd)
/// ```
///
/// The outer `COALESCE` handles the case where the DT part reduces to `"P"`
/// (i.e., the duration has no day/time components), which would make `STRDT`
/// return UNDEF for an invalid bare `"P"` dayTimeDuration.
///
/// When `is_date` is `true` (i.e., the LHS is an `xs:date`), the time
/// portion of the dayTimeDuration is stripped via `STRBEFORE(dt_str, "T")`
/// so that hours/minutes/seconds do NOT bleed into the date result.
/// (Oxigraph converts date→dateTime at midnight before subtracting, which
/// causes an off-by-one-day error otherwise.)
fn temporal_subtract_sparql(temporal: SparExpr, dur: SparExpr, is_date: bool) -> SparExpr {
    use spargebra::algebra::Function;

    let ymd_nn = NamedNode::new_unchecked(XSD_YEAR_MONTH_DUR);
    let dtd_nn = NamedNode::new_unchecked(XSD_DAY_TIME_DUR);

    // STR(dur) — two copies needed (one per REPLACE call)
    let dur_str_ym = SparExpr::FunctionCall(Function::Str, vec![dur.clone()]);
    let dur_str_dt = SparExpr::FunctionCall(Function::Str, vec![dur]);

    // REPLACE(STR(dur), "^P(([0-9.]*Y)?([0-9.]*M)?).*", "P$1") → yearMonth part string
    let ym_pat = SparExpr::Literal(SparLit::new_simple_literal(
        "^P(([0-9.]*Y)?([0-9.]*M)?).*",
    ));
    let ym_repl = SparExpr::Literal(SparLit::new_simple_literal("P$1"));
    let ym_str = SparExpr::FunctionCall(Function::Replace, vec![dur_str_ym, ym_pat, ym_repl]);

    // STRDT(ym_str, xsd:yearMonthDuration)
    let ym_dur = SparExpr::FunctionCall(
        Function::StrDt,
        vec![ym_str, SparExpr::NamedNode(ymd_nn)],
    );

    // REPLACE(STR(dur), "^P([0-9.]*Y)?([0-9.]*M)?", "P") → dayTime part string
    let dt_pat = SparExpr::Literal(SparLit::new_simple_literal("^P([0-9.]*Y)?([0-9.]*M)?"));
    let dt_repl = SparExpr::Literal(SparLit::new_simple_literal("P"));
    let dt_str_full = SparExpr::FunctionCall(Function::Replace, vec![dur_str_dt, dt_pat, dt_repl]);

    // For xs:date arithmetic, Oxigraph implements date - dayTimeDuration by converting
    // the date to a dateTime at midnight and subtracting, so hours/minutes/seconds cause
    // an off-by-one-day error.  Strip the time part by taking STRBEFORE(dt_str, "T").
    let dt_str = if is_date {
        let t_lit = SparExpr::Literal(SparLit::new_simple_literal("T"));
        SparExpr::FunctionCall(Function::StrBefore, vec![dt_str_full, t_lit])
    } else {
        dt_str_full
    };

    // STRDT(dt_str, xsd:dayTimeDuration)
    let dt_dur = SparExpr::FunctionCall(
        Function::StrDt,
        vec![dt_str, SparExpr::NamedNode(dtd_nn)],
    );

    // step1 = COALESCE(temporal - ym_dur, temporal)
    // Handles: time - yearMonthDuration is UNDEF (time has no year/month), so COALESCE falls
    // back to temporal unchanged; also handles empty "P" yearMonthDuration → STRDT returns UNDEF.
    let step1 = SparExpr::Coalesce(vec![
        SparExpr::Subtract(Box::new(temporal.clone()), Box::new(ym_dur)),
        temporal,
    ]);

    // COALESCE(step1 - dt_dur, step1)
    // Handles: "P" dayTimeDuration (no time component) → STRDT returns UNDEF → COALESCE keeps step1.
    SparExpr::Coalesce(vec![
        SparExpr::Subtract(Box::new(step1.clone()), Box::new(dt_dur)),
        step1,
    ])
}

