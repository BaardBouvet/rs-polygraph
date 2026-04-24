/// Extract the column names from a segment's final RETURN or WITH clause.
fn segment_columns(seg: &[Clause]) -> Option<Vec<String>> {
    for clause in seg.iter().rev() {
        match clause {
            Clause::Return(r) => {
                if let ReturnItems::Explicit(items) = &r.items {
                    return Some(
                        items
                            .iter()
                            .map(|i| {
                                i.alias
                                    .clone()
                                    .or_else(|| {
                                        if let Expression::Variable(v) = &i.expression {
                                            Some(v.clone())
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or_default()
                            })
                            .collect(),
                    );
                }
                return None;
            }
            Clause::With(w) => {
                if let crate::ast::cypher::ReturnItems::Explicit(items) = &w.items {
                    return Some(
                        items
                            .iter()
                            .map(|i| {
                                i.alias
                                    .clone()
                                    .or_else(|| {
                                        if let Expression::Variable(v) = &i.expression {
                                            Some(v.clone())
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or_default()
                            })
                            .collect(),
                    );
                }
                return None;
            }
            _ => {}
        }
    }
    None
}

/// Semantic analysis pass: catches `VariableTypeConflict` and `VariableAlreadyBound`
/// before translation so openCypher constraints are enforced.
fn validate_semantics(query: &CypherQuery) -> Result<(), PolygraphError> {
    // ── UNION checks (InvalidClauseComposition, DifferentColumnsInUnion) ─────
    if query
        .clauses
        .iter()
        .any(|c| matches!(c, Clause::Union { .. }))
    {
        // Split into segments.
        let mut segments: Vec<Vec<Clause>> = Vec::new();
        let mut all_flags: Vec<bool> = Vec::new();
        let mut current_seg: Vec<Clause> = Vec::new();
        for clause in &query.clauses {
            if let Clause::Union { all } = clause {
                segments.push(std::mem::take(&mut current_seg));
                all_flags.push(*all);
            } else {
                current_seg.push(clause.clone());
            }
        }
        segments.push(current_seg);

        // InvalidClauseComposition: mixing UNION and UNION ALL is illegal.
        let has_union = all_flags.iter().any(|a| !a);
        let has_union_all = all_flags.iter().any(|a| *a);
        if has_union && has_union_all {
            return Err(PolygraphError::Translation {
                message: "InvalidClauseComposition: cannot mix UNION and UNION ALL".to_string(),
            });
        }

        // DifferentColumnsInUnion: all arms must project the same column names.
        let first_cols = segment_columns(&segments[0]);
        for seg in segments.iter().skip(1) {
            let cols = segment_columns(seg);
            if first_cols != cols {
                return Err(PolygraphError::Translation {
                    message:
                        "DifferentColumnsInUnion: UNION arms must return the same column names"
                            .to_string(),
                });
            }
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Node,
        Rel,
        Path,
        /// Scalar primitive or empty list or map — cannot be node/rel/path
        Scalar,
        /// Non-empty list of only variable expressions — e.g. `[r1, r2]`.
        /// Invalid as a node but valid as a relationship in `[rs*]` expansions.
        VarList,
    }

    let mut kinds: std::collections::HashMap<String, Kind> = std::collections::HashMap::new();
    let mut bound_vars: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_match = false;

    for clause in &query.clauses {
        match clause {
            Clause::With(w) => {
                // UnexpectedSyntax: pattern predicate directly used as WITH expression.
                if let crate::ast::cypher::ReturnItems::Explicit(items) = &w.items {
                    for item in items {
                        if matches!(&item.expression, Expression::PatternPredicate(_)) {
                            return Err(PolygraphError::Translation {
                                message: "UnexpectedSyntax: pattern predicate not allowed in WITH"
                                    .to_string(),
                            });
                        }
                    }
                }
                if let crate::ast::cypher::ReturnItems::Explicit(items) = &w.items {
                    // Collect the projected variable names for scope replacement.
                    let mut projected_names: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    let mut projected_kinds: std::collections::HashMap<String, Kind> =
                        std::collections::HashMap::new();

                    for item in items {
                        let name = item.alias.clone().or_else(|| {
                            if let Expression::Variable(v) = &item.expression {
                                Some(v.clone())
                            } else {
                                None
                            }
                        });
                        // NoExpressionAlias: non-variable expressions in WITH must have aliases.
                        if name.is_none() {
                            return Err(PolygraphError::Translation {
                                message: "NoExpressionAlias: expression in WITH must have an alias"
                                    .to_string(),
                            });
                        }
                        if let Some(var) = name {
                            projected_names.insert(var.clone());
                            let binding_kind = match &item.expression {
                                // Primitive scalars → definitely not node/rel/path
                                Expression::Literal(Literal::Integer(_))
                                | Expression::Literal(Literal::Float(_))
                                | Expression::Literal(Literal::String(_))
                                | Expression::Literal(Literal::Boolean(_)) => Some(Kind::Scalar),
                                // Maps → not node/rel/path
                                Expression::Map(_) => Some(Kind::Scalar),
                                Expression::List(elems) if elems.is_empty() => {
                                    // Empty list cannot be a node/rel/path
                                    Some(Kind::Scalar)
                                }
                                Expression::List(elems) => {
                                    let all_vars =
                                        elems.iter().all(|e| matches!(e, Expression::Variable(_)));
                                    if all_vars {
                                        // [r1, r2] — may be used as rel list in [rs*]
                                        Some(Kind::VarList)
                                    } else {
                                        // [1, 2] or mixed → scalar
                                        Some(Kind::Scalar)
                                    }
                                }
                                // Variable → propagate existing kind (pass-through).
                                Expression::Variable(v) => kinds.get(v.as_str()).cloned(),
                                _ => None, // unknown type — don't constrain
                            };
                            if let Some(k) = binding_kind {
                                projected_kinds.insert(var, k);
                            }
                        }
                    }

                    let projection_has_agg =
                        items.iter().any(|i| expr_contains_aggregate(&i.expression));

                    // UndefinedVariable check in ORDER BY (using pre-projection scope).
                    // Always check when bound_vars is non-empty (we have some scope context).
                    if let Some(ob) = &w.order_by {
                        if !bound_vars.is_empty() {
                            fn collect_free_vars_ob(expr: &Expression, vars: &mut Vec<String>) {
                                match expr {
                                    Expression::Variable(v) => vars.push(v.clone()),
                                    Expression::Property(base, _) => {
                                        collect_free_vars_ob(base, vars)
                                    }
                                    Expression::Aggregate(_) => {}
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
                                        collect_free_vars_ob(a, vars);
                                        collect_free_vars_ob(b, vars);
                                    }
                                    Expression::Not(e)
                                    | Expression::Negate(e)
                                    | Expression::IsNull(e)
                                    | Expression::IsNotNull(e) => collect_free_vars_ob(e, vars),
                                    Expression::FunctionCall { args, .. } => {
                                        for a in args {
                                            collect_free_vars_ob(a, vars);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            // Build the set of valid references for ORDER BY.
                            // When projection_has_agg: only projected items (expressions+aliases) are valid.
                            // When no aggregation: any current bound_var is valid.
                            let proj_aliases: std::collections::HashSet<&str> =
                                items.iter().filter_map(|i| i.alias.as_deref()).collect();
                            let non_agg_exprs: Vec<&Expression> = items
                                .iter()
                                .filter(|i| !expr_contains_aggregate(&i.expression))
                                .map(|i| &i.expression)
                                .collect();

                            for sort in &ob.items {
                                // For non-agg ORDER BY expressions: check free vars are in scope
                                if !expr_contains_aggregate(&sort.expression) {
                                    let mut sort_free = Vec::new();
                                    collect_free_vars_ob(&sort.expression, &mut sort_free);
                                    for v in sort_free {
                                        let covered = if projection_has_agg {
                                            // With aggregation: only projected non-agg items or aliases are valid
                                            proj_aliases.contains(v.as_str())
                                            || non_agg_exprs.iter().any(|e| {
                                                matches!(e, Expression::Variable(ev) if *ev == v)
                                                || matches!(e, Expression::Property(bx, _) if {
                                                    if let Expression::Variable(bv) = bx.as_ref() {
                                                        *bv == v
                                                    } else { false }
                                                })
                                            })
                                        } else {
                                            // No aggregation: any bound var is valid
                                            bound_vars.contains(&v)
                                                || projected_names.contains(&v)
                                                || proj_aliases.contains(v.as_str())
                                        };
                                        if !covered {
                                            return Err(PolygraphError::Translation {
                                                message: format!("UndefinedVariable: variable '{v}' not defined in ORDER BY"),
                                            });
                                        }
                                    }
                                }
                                // For aggregate ORDER BY expressions: check they're in the projection.
                                // (Handled by InvalidAggregation/AmbiguousAggregation checks below)
                            }
                        }
                    }

                    // InvalidAggregation: aggregate function in WITH ORDER BY.
                    // Only invalid when the WITH projection itself has no aggregates.
                    if let Some(ob) = &w.order_by {
                        if !projection_has_agg {
                            for sort in &ob.items {
                                if expr_contains_aggregate(&sort.expression) {
                                    return Err(PolygraphError::Translation {
                                        message:
                                            "InvalidAggregation: aggregate function in ORDER BY"
                                                .to_string(),
                                    });
                                }
                            }
                        }
                        // AmbiguousAggregationExpression: aggregate in ORDER BY where the ORDER BY
                        // item has free terms not covered by non-agg projection items or aliases.
                        if projection_has_agg {
                            let all_items_w: Vec<_> = items.iter().collect();
                            let non_agg_exprs: Vec<&Expression> = all_items_w
                                .iter()
                                .filter(|i| !expr_contains_aggregate(&i.expression))
                                .map(|i| &i.expression)
                                .collect();
                            // Aliases from any projection item (e.g. count(*) AS cnt → "cnt" is valid)
                            let proj_aliases_w: std::collections::HashSet<&str> = all_items_w
                                .iter()
                                .filter_map(|i| i.alias.as_deref())
                                .collect();
                            for sort in &ob.items {
                                if expr_contains_aggregate(&sort.expression) {
                                    if expr_has_free_var_outside_agg(&sort.expression) {
                                        let free_terms = atomic_free_terms(&sort.expression);
                                        let ambiguous = free_terms.iter().any(|ft| {
                                            if non_agg_exprs.contains(ft) {
                                                return false;
                                            }
                                            if let Expression::Variable(v) = ft {
                                                if proj_aliases_w.contains(v.as_str()) {
                                                    return false;
                                                }
                                            }
                                            true
                                        });
                                        if ambiguous {
                                            return Err(PolygraphError::Translation {
                                                message: "AmbiguousAggregationExpression: ORDER BY expression is ambiguous".to_string(),
                                            });
                                        }
                                    } else {
                                        // Aggregate in ORDER BY with no free vars outside agg
                                        // (pure aggregate like sum(x)). Must match a projected
                                        // expression or alias, otherwise UndefinedVariable.
                                        let alias_covers =
                                            if let Expression::Variable(v) = &sort.expression {
                                                proj_aliases_w.contains(v.as_str())
                                            } else {
                                                false
                                            };
                                        let expr_in_proj = all_items_w
                                            .iter()
                                            .any(|pi| pi.expression == sort.expression);
                                        if !alias_covers && !expr_in_proj {
                                            return Err(PolygraphError::Translation {
                                                message: "UndefinedVariable: aggregate in ORDER BY is not in the projection".to_string(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // AmbiguousAggregationExpression in WITH items.
                    {
                        let non_agg_items: Vec<&Expression> = items
                            .iter()
                            .filter(|i| !matches!(&i.expression, Expression::Aggregate(_)))
                            .filter(|i| !expr_contains_aggregate(&i.expression))
                            .map(|i| &i.expression)
                            .collect();
                        for item in items {
                            if !matches!(&item.expression, Expression::Aggregate(_))
                                && expr_contains_aggregate(&item.expression)
                                && expr_has_free_var_outside_agg(&item.expression)
                                && is_ambiguous_aggregation(&item.expression, &non_agg_items)
                            {
                                return Err(PolygraphError::Translation {
                                    message:
                                        "AmbiguousAggregationExpression: mix of aggregate and \
                                              non-aggregate in WITH expression"
                                            .to_string(),
                                });
                            }
                        }
                    }

                    // Scope replacement: after WITH, only projected variables are in scope.
                    // Always replace (even before MATCH) to handle WITH ... MATCH patterns.
                    bound_vars = projected_names;
                    kinds.retain(|k, _| projected_kinds.contains_key(k));
                    for (k, v) in projected_kinds {
                        kinds.insert(k, v);
                    }
                }
            }
            Clause::Return(r) => {
                // NoVariablesInScope: RETURN * with no bound variables from MATCH/WITH/UNWIND.
                // Only fire when there was at least one MATCH clause (tracked by seen_match).
                if matches!(&r.items, ReturnItems::All) && seen_match {
                    if bound_vars.is_empty() {
                        return Err(PolygraphError::Translation {
                            message: "NoVariablesInScope: RETURN * with no variables in scope"
                                .to_string(),
                        });
                    }
                }

                // UnexpectedSyntax: pattern predicate directly used as return expression.
                if let ReturnItems::Explicit(items) = &r.items {
                    for item in items {
                        if matches!(&item.expression, Expression::PatternPredicate(_)) {
                            return Err(PolygraphError::Translation {
                                message:
                                    "UnexpectedSyntax: pattern predicate not allowed in RETURN"
                                        .to_string(),
                            });
                        }
                    }
                }

                // InvalidArgumentType / UnexpectedSyntax: size() with invalid arg.
                if let ReturnItems::Explicit(items) = &r.items {
                    fn scan_invalid_size(
                        expr: &Expression,
                        kinds: &std::collections::HashMap<String, Kind>,
                    ) -> Option<&'static str> {
                        match expr {
                            Expression::FunctionCall { name, args, .. }
                                if name.eq_ignore_ascii_case("size") =>
                            {
                                if let Some(arg) = args.first() {
                                    match arg {
                                        Expression::PatternPredicate(_) => {
                                            return Some("UnexpectedSyntax")
                                        }
                                        Expression::Variable(v)
                                            if matches!(
                                                kinds.get(v.as_str()),
                                                Some(Kind::Path)
                                            ) =>
                                        {
                                            return Some("InvalidArgumentType")
                                        }
                                        _ => {}
                                    }
                                }
                                None
                            }
                            Expression::FunctionCall { args, .. } => {
                                args.iter().find_map(|a| scan_invalid_size(a, kinds))
                            }
                            Expression::Or(a, b)
                            | Expression::And(a, b)
                            | Expression::Add(a, b)
                            | Expression::Subtract(a, b)
                            | Expression::Multiply(a, b)
                            | Expression::Divide(a, b)
                            | Expression::Comparison(a, _, b) => {
                                scan_invalid_size(a, kinds).or_else(|| scan_invalid_size(b, kinds))
                            }
                            Expression::Not(e)
                            | Expression::Negate(e)
                            | Expression::IsNull(e)
                            | Expression::IsNotNull(e) => scan_invalid_size(e, kinds),
                            _ => None,
                        }
                    }
                    for item in items {
                        if let Some(err) = scan_invalid_size(&item.expression, &kinds) {
                            return Err(PolygraphError::Translation {
                                message: format!("{err}: invalid argument to size()"),
                            });
                        }
                    }
                }

                if let ReturnItems::Explicit(items) = &r.items {
                    // Collect the set of non-aggregate return expressions for grouping check.
                    let non_agg_items: Vec<&Expression> = items
                        .iter()
                        .filter(|i| !matches!(&i.expression, Expression::Aggregate(_)))
                        .filter(|i| !expr_contains_aggregate(&i.expression))
                        .map(|i| &i.expression)
                        .collect();

                    for item in items {
                        if let Expression::Aggregate(agg) = &item.expression {
                            // NestedAggregation: aggregate inside aggregate arg.
                            if agg_has_nested_aggregate(agg) {
                                return Err(PolygraphError::Translation {
                                    message: "NestedAggregation: aggregate inside \
                                              aggregate argument"
                                        .to_string(),
                                });
                            }
                        } else if !matches!(&item.expression, Expression::Aggregate(_))
                            && expr_contains_aggregate(&item.expression)
                            && expr_has_free_var_outside_agg(&item.expression)
                        {
                            if is_ambiguous_aggregation(&item.expression, &non_agg_items) {
                                return Err(PolygraphError::Translation {
                                    message: "AmbiguousAggregationExpression: mix of aggregate \
                                              and non-aggregate in RETURN expression"
                                        .to_string(),
                                });
                            }
                        }
                    }
                }

                // UndefinedVariable: RETURN references a variable not bound in MATCH/WITH/UNWIND.
                // Check when we have some scope context established, OR when the expression
                // contains variable references that would require a scope to be valid.
                if let ReturnItems::Explicit(items) = &r.items {
                    fn collect_free_vars(expr: &Expression, vars: &mut Vec<String>) {
                        match expr {
                            Expression::Variable(v) => vars.push(v.clone()),
                            Expression::Property(base, _) => collect_free_vars(base, vars),
                            Expression::Aggregate(_) => {}
                            Expression::Map(pairs) => {
                                for (_, v) in pairs {
                                    collect_free_vars(v, vars);
                                }
                            }
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
                                collect_free_vars(a, vars);
                                collect_free_vars(b, vars);
                            }
                            Expression::Not(e)
                            | Expression::Negate(e)
                            | Expression::IsNull(e)
                            | Expression::IsNotNull(e) => collect_free_vars(e, vars),
                            Expression::LabelCheck { variable, .. } => vars.push(variable.clone()),
                            Expression::FunctionCall { args, .. } => {
                                for a in args {
                                    collect_free_vars(a, vars);
                                }
                            }
                            _ => {}
                        }
                    }
                    for item in items {
                        let mut free_vars = Vec::new();
                        collect_free_vars(&item.expression, &mut free_vars);
                        if free_vars.is_empty() {
                            continue;
                        }
                        // Only check undefined vars when we have context OR the expr has vars
                        if seen_match || !bound_vars.is_empty() || !free_vars.is_empty() {
                            for v in free_vars {
                                // Check both kinds (from MATCH) and bound_vars (from WITH/UNWIND).
                                if !kinds.contains_key(&v) && !bound_vars.contains(&v) {
                                    return Err(PolygraphError::Translation {
                                        message: format!(
                                            "UndefinedVariable: variable '{v}' not defined"
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }

                // UndefinedVariable: DISTINCT RETURN can only ORDER BY projected expressions.
                // `RETURN DISTINCT a.name ORDER BY a.age` → a.age not projected, a not projected → error.
                // But `RETURN DISTINCT b ORDER BY b.name` → b is projected, so b.name is OK.
                if r.distinct {
                    if let (ReturnItems::Explicit(items), Some(ob)) = (&r.items, &r.order_by) {
                        let proj_aliases: std::collections::HashSet<&str> =
                            items.iter().filter_map(|i| i.alias.as_deref()).collect();
                        let proj_exprs: Vec<&Expression> =
                            items.iter().map(|i| &i.expression).collect();
                        // Projected variable names (directly projected, not through property)
                        let proj_vars: std::collections::HashSet<&str> = items
                            .iter()
                            .filter_map(|i| {
                                if let Expression::Variable(v) = &i.expression {
                                    Some(v.as_str())
                                } else {
                                    None
                                }
                            })
                            .chain(proj_aliases.iter().copied())
                            .collect();
                        for sort in &ob.items {
                            let is_projected = proj_exprs.iter().any(|pe| *pe == &sort.expression)
                                || if let Expression::Variable(v) = &sort.expression {
                                    proj_vars.contains(v.as_str())
                                } else {
                                    false
                                };
                            if !is_projected {
                                if let Expression::Property(base, _) = &sort.expression {
                                    if let Expression::Variable(v) = base.as_ref() {
                                        // Only error if the base variable is NOT projected directly
                                        // (i.e., `b` not in projection, and `b.property` is used)
                                        if !proj_vars.contains(v.as_str()) {
                                            if kinds.contains_key(v.as_str())
                                                || bound_vars.contains(v.as_str())
                                            {
                                                return Err(PolygraphError::Translation {
                                                    message: format!("UndefinedVariable: variable '{v}' not defined"),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // InvalidAggregation: aggregate in RETURN ORDER BY or in list comprehension.
                if let ReturnItems::Explicit(items) = &r.items {
                    fn contains_agg_in_list_comp(expr: &Expression) -> bool {
                        match expr {
                            Expression::ListComprehension {
                                projection: Some(p),
                                ..
                            } => expr_contains_aggregate(p),
                            Expression::Or(a, b)
                            | Expression::And(a, b)
                            | Expression::Add(a, b)
                            | Expression::Subtract(a, b)
                            | Expression::Multiply(a, b)
                            | Expression::Divide(a, b)
                            | Expression::Comparison(a, _, b) => {
                                contains_agg_in_list_comp(a) || contains_agg_in_list_comp(b)
                            }
                            Expression::Not(e)
                            | Expression::Negate(e)
                            | Expression::IsNull(e)
                            | Expression::IsNotNull(e) => contains_agg_in_list_comp(e),
                            Expression::List(elems) => elems.iter().any(contains_agg_in_list_comp),
                            Expression::FunctionCall { args, .. } => {
                                args.iter().any(contains_agg_in_list_comp)
                            }
                            _ => false,
                        }
                    }
                    for item in items {
                        if contains_agg_in_list_comp(&item.expression) {
                            return Err(PolygraphError::Translation {
                                message: "InvalidAggregation: aggregate inside list comprehension"
                                    .to_string(),
                            });
                        }
                    }
                }
                // InvalidAggregation / AmbiguousAggregation: aggregate in RETURN ORDER BY.
                if let Some(ob) = &r.order_by {
                    let projection_has_agg = if let ReturnItems::Explicit(items) = &r.items {
                        items.iter().any(|i| expr_contains_aggregate(&i.expression))
                    } else {
                        false
                    };
                    if !projection_has_agg {
                        for sort in &ob.items {
                            if expr_contains_aggregate(&sort.expression) {
                                return Err(PolygraphError::Translation {
                                    message: "InvalidAggregation: aggregate function in ORDER BY"
                                        .to_string(),
                                });
                            }
                        }
                    }
                    if projection_has_agg {
                        let all_items_r: Vec<_> = if let ReturnItems::Explicit(items) = &r.items {
                            items.iter().collect()
                        } else {
                            vec![]
                        };
                        let non_agg_exprs: Vec<&Expression> = all_items_r
                            .iter()
                            .filter(|i| !expr_contains_aggregate(&i.expression))
                            .map(|i| &i.expression)
                            .collect();
                        let proj_aliases: std::collections::HashSet<&str> = all_items_r
                            .iter()
                            .filter_map(|i| i.alias.as_deref())
                            .collect();
                        for sort in &ob.items {
                            if expr_contains_aggregate(&sort.expression)
                                && expr_has_free_var_outside_agg(&sort.expression)
                            {
                                let free_terms = atomic_free_terms(&sort.expression);
                                let ambiguous = free_terms.iter().any(|ft| {
                                    if non_agg_exprs.contains(ft) {
                                        return false;
                                    }
                                    if let Expression::Variable(v) = ft {
                                        if proj_aliases.contains(v.as_str()) {
                                            return false;
                                        }
                                    }
                                    true
                                });
                                if ambiguous {
                                    return Err(PolygraphError::Translation {
                                        message: "AmbiguousAggregationExpression: ORDER BY expression is ambiguous".to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            Clause::Match(m) => {
                seen_match = true;

                // Pre-register path variables so WHERE checks can detect path.prop access.
                for pattern in &m.pattern.0 {
                    if let Some(pv) = &pattern.variable {
                        if kinds.contains_key(pv.as_str()) {
                            return Err(PolygraphError::Translation {
                                message: format!("VariableAlreadyBound: '{pv}' is already bound"),
                            });
                        }
                        kinds.insert(pv.clone(), Kind::Path);
                        bound_vars.insert(pv.clone());
                    }
                }

                // InvalidAggregation: aggregate in WHERE.
                if let Some(wc) = &m.where_ {
                    if expr_contains_aggregate(&wc.expression) {
                        return Err(PolygraphError::Translation {
                            message: "InvalidAggregation: aggregate function in WHERE clause"
                                .to_string(),
                        });
                    }
                }

                // InvalidArgumentType: property access on a path variable.
                if let Some(wc) = &m.where_ {
                    fn check_path_prop(
                        expr: &Expression,
                        kinds: &std::collections::HashMap<String, Kind>,
                    ) -> bool {
                        match expr {
                            Expression::Property(base, _) => {
                                if let Expression::Variable(v) = base.as_ref() {
                                    if matches!(kinds.get(v.as_str()), Some(Kind::Path)) {
                                        return true;
                                    }
                                }
                                check_path_prop(base, kinds)
                            }
                            Expression::Or(a, b)
                            | Expression::And(a, b)
                            | Expression::Comparison(a, _, b) => {
                                check_path_prop(a, kinds) || check_path_prop(b, kinds)
                            }
                            Expression::Not(e)
                            | Expression::IsNull(e)
                            | Expression::IsNotNull(e) => check_path_prop(e, kinds),
                            _ => false,
                        }
                    }
                    if check_path_prop(&wc.expression, &kinds) {
                        return Err(PolygraphError::Translation {
                            message: "InvalidArgumentType: property access on a path variable"
                                .to_string(),
                        });
                    }
                }

                // Check RelationshipUniquenessViolation: same rel var twice in one pattern.
                for pattern in &m.pattern.0 {
                    let mut rel_vars_in_pattern: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    for elem in &pattern.elements {
                        if let PatternElement::Relationship(r) = elem {
                            if let Some(v) = &r.variable {
                                if !rel_vars_in_pattern.insert(v.clone()) {
                                    return Err(PolygraphError::Translation {
                                        message: format!(
                                            "RelationshipUniquenessViolation: \
                                             '{v}' used more than once in the same pattern"
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }

                for pattern in &m.pattern.0 {
                    // Path variables are already registered from the pre-scan above.

                    for elem in &pattern.elements {
                        match elem {
                            PatternElement::Node(n) => {
                                if let Some(v) = &n.variable {
                                    match kinds.get(v.as_str()) {
                                        Some(Kind::Rel) | Some(Kind::Path) => {
                                            return Err(PolygraphError::Translation {
                                                message: format!(
                                                    "VariableTypeConflict: '{v}' used as both \
                                                     relationship/path and node"
                                                ),
                                            });
                                        }
                                        Some(Kind::Scalar) | Some(Kind::VarList) => {
                                            return Err(PolygraphError::Translation {
                                                message: format!(
                                                    "VariableTypeConflict: '{v}' is a \
                                                     non-node value used as a node"
                                                ),
                                            });
                                        }
                                        _ => {
                                            kinds.insert(v.clone(), Kind::Node);
                                            bound_vars.insert(v.clone());
                                        }
                                    }
                                }
                            }
                            PatternElement::Relationship(r) => {
                                if let Some(v) = &r.variable {
                                    match kinds.get(v.as_str()) {
                                        Some(Kind::Node) | Some(Kind::Path) => {
                                            return Err(PolygraphError::Translation {
                                                message: format!(
                                                    "VariableTypeConflict: '{v}' used as both \
                                                     node/path and relationship"
                                                ),
                                            });
                                        }
                                        Some(Kind::Scalar) => {
                                            return Err(PolygraphError::Translation {
                                                message: format!(
                                                    "VariableTypeConflict: '{v}' is a scalar \
                                                     used as a relationship"
                                                ),
                                            });
                                        }
                                        // VarList ([r1, r2]) is allowed as rel in [rs*]
                                        _ => {
                                            kinds.insert(v.clone(), Kind::Rel);
                                            bound_vars.insert(v.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // UndefinedVariable: WHERE clause references an undefined variable.
                // This check runs AFTER pattern elements are registered.
                if let Some(wc) = &m.where_ {
                    fn check_undef_where(
                        expr: &Expression,
                        bound: &std::collections::HashSet<String>,
                        kinds: &std::collections::HashMap<String, Kind>,
                        found: &mut Option<String>,
                    ) {
                        if found.is_some() {
                            return;
                        }
                        match expr {
                            Expression::Variable(v) => {
                                if !bound.contains(v) && !kinds.contains_key(v.as_str()) {
                                    *found = Some(v.clone());
                                }
                            }
                            Expression::Property(base, _) => {
                                check_undef_where(base, bound, kinds, found)
                            }
                            Expression::Or(a, b)
                            | Expression::And(a, b)
                            | Expression::Xor(a, b)
                            | Expression::Add(a, b)
                            | Expression::Subtract(a, b)
                            | Expression::Multiply(a, b)
                            | Expression::Divide(a, b)
                            | Expression::Comparison(a, _, b) => {
                                check_undef_where(a, bound, kinds, found);
                                check_undef_where(b, bound, kinds, found);
                            }
                            Expression::Not(e)
                            | Expression::Negate(e)
                            | Expression::IsNull(e)
                            | Expression::IsNotNull(e) => check_undef_where(e, bound, kinds, found),
                            Expression::FunctionCall { args, .. } => {
                                for a in args {
                                    check_undef_where(a, bound, kinds, found);
                                }
                            }
                            Expression::PatternPredicate(pattern) => {
                                // Check for new named variables in pattern predicates.
                                for elem in &pattern.elements {
                                    match elem {
                                        PatternElement::Relationship(rp) => {
                                            if let Some(v) = &rp.variable {
                                                if !bound.contains(v.as_str())
                                                    && !kinds.contains_key(v.as_str())
                                                {
                                                    *found = Some(v.to_string());
                                                    return;
                                                }
                                            }
                                        }
                                        PatternElement::Node(np) => {
                                            if let Some(v) = &np.variable {
                                                if !bound.contains(v.as_str())
                                                    && !kinds.contains_key(v.as_str())
                                                {
                                                    *found = Some(v.to_string());
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    // InvalidArgumentType: bare node pattern `WHERE (n)` is not a boolean.
                    fn check_invalid_pattern(
                        expr: &Expression,
                        kinds: &std::collections::HashMap<String, Kind>,
                    ) -> bool {
                        match expr {
                            Expression::PatternPredicate(pattern) => !pattern
                                .elements
                                .iter()
                                .any(|e| matches!(e, PatternElement::Relationship(_))),
                            // Bare node/relationship variable used as a boolean predicate.
                            Expression::Variable(v) => {
                                matches!(kinds.get(v.as_str()), Some(Kind::Node) | Some(Kind::Rel))
                            }
                            // NOT can wrap an invalid predicate.
                            Expression::Not(e) => check_invalid_pattern(e, kinds),
                            Expression::Or(a, b) | Expression::And(a, b) => {
                                check_invalid_pattern(a, kinds) || check_invalid_pattern(b, kinds)
                            }
                            // IsNull/IsNotNull are valid uses of node/rel variables — do NOT recurse.
                            _ => false,
                        }
                    }
                    if check_invalid_pattern(&wc.expression, &kinds) {
                        return Err(PolygraphError::Translation {
                            message: "InvalidArgumentType: node/relationship variable cannot be used as a boolean predicate".to_string(),
                        });
                    }
                    let mut found_undef: Option<String> = None;
                    check_undef_where(&wc.expression, &bound_vars, &kinds, &mut found_undef);
                    if let Some(v) = found_undef {
                        return Err(PolygraphError::Translation {
                            message: format!("UndefinedVariable: variable '{v}' not defined"),
                        });
                    }
                }
            }
            Clause::Unwind(u) => {
                // Register the UNWIND variable as bound (type unknown — don't constrain kinds).
                bound_vars.insert(u.variable.clone());
            }
            Clause::Delete(d) => {
                // InvalidDelete: deleting a label predicate (n:Person) or rel-type (r:T)
                // or any non-entity expression is a SyntaxError/runtime error.
                for expr in &d.expressions {
                    match expr {
                        Expression::LabelCheck { .. } => {
                            return Err(PolygraphError::Translation {
                                message: "InvalidDelete: cannot delete a label predicate; \
                                          use REMOVE to remove labels"
                                    .to_string(),
                            });
                        }
                        Expression::Literal(_)
                        | Expression::Add(_, _)
                        | Expression::Subtract(_, _)
                        | Expression::Multiply(_, _)
                        | Expression::Divide(_, _) => {
                            return Err(PolygraphError::Translation {
                                message: "InvalidArgumentType: DELETE requires a node or \
                                          relationship, not a literal or arithmetic expression"
                                    .to_string(),
                            });
                        }
                        Expression::Variable(v) => {
                            if (seen_match || !bound_vars.is_empty())
                                && !bound_vars.contains(v.as_str())
                                && !kinds.contains_key(v.as_str())
                            {
                                return Err(PolygraphError::Translation {
                                    message: format!(
                                        "UndefinedVariable: variable '{v}' not defined"
                                    ),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            Clause::Set(s) => {
                // UndefinedVariable: SET RHS references a variable not in scope.
                for item in &s.items {
                    let (var_opt, rhs_opt): (Option<String>, Option<&Expression>) = match item {
                        crate::ast::cypher::SetItem::Property { key: _, value, .. } => {
                            (None, Some(value))
                        }
                        crate::ast::cypher::SetItem::NodeReplace { value, .. } => {
                            (None, Some(value))
                        }
                        crate::ast::cypher::SetItem::MergeMap { .. }
                        | crate::ast::cypher::SetItem::SetLabel { .. } => (None, None),
                    };
                    let _ = var_opt; // unused for now
                                     // TypeError: a PROPERTY value cannot be a map or a list-of-maps.
                                     // e.g. SET n.x = {k: v}  or  SET n.x = [{k: v}]
                                     // (Note: SET n = {map} is NodeReplace and IS VALID; only Property is invalid)
                    if let crate::ast::cypher::SetItem::Property { value, .. } = item {
                        let invalid = match value {
                            Expression::Map(_) => true,
                            Expression::List(items) => {
                                items.iter().any(|e| matches!(e, Expression::Map(_)))
                            }
                            _ => false,
                        };
                        if invalid {
                            return Err(PolygraphError::Translation {
                                message: "TypeError: InvalidPropertyType: property values cannot be maps or lists of maps".to_string(),
                            });
                        }
                    }
                    // SyntaxError: UnexpectedSyntax — patterns are not allowed inside
                    // SET RHS expressions (e.g. SET n.p = head((n)-[:R]->()).foo).
                    if let crate::ast::cypher::SetItem::Property { value, .. } = item {
                        fn contains_pattern_expr(e: &Expression) -> bool {
                            match e {
                                Expression::PatternComprehension { .. } => true,
                                Expression::FunctionCall { args, .. } => {
                                    args.iter().any(contains_pattern_expr)
                                }
                                Expression::Property(b, _) => contains_pattern_expr(b),
                                Expression::Subscript(a, b) => {
                                    contains_pattern_expr(a) || contains_pattern_expr(b)
                                }
                                Expression::ListSlice { list, start, end } => {
                                    contains_pattern_expr(list)
                                        || start.as_deref().map(contains_pattern_expr).unwrap_or(false)
                                        || end.as_deref().map(contains_pattern_expr).unwrap_or(false)
                                }
                                Expression::ListComprehension { list, predicate, projection, .. } => {
                                    contains_pattern_expr(list)
                                        || predicate.as_deref().map(contains_pattern_expr).unwrap_or(false)
                                        || projection.as_deref().map(contains_pattern_expr).unwrap_or(false)
                                }
                                Expression::QuantifierExpr { list, predicate, .. } => {
                                    contains_pattern_expr(list)
                                        || predicate.as_deref().map(contains_pattern_expr).unwrap_or(false)
                                }
                                Expression::Add(a, b)
                                | Expression::Subtract(a, b)
                                | Expression::Multiply(a, b)
                                | Expression::Divide(a, b)
                                | Expression::Comparison(a, _, b)
                                | Expression::And(a, b)
                                | Expression::Or(a, b) => {
                                    contains_pattern_expr(a) || contains_pattern_expr(b)
                                }
                                Expression::Not(e)
                                | Expression::Negate(e)
                                | Expression::IsNull(e)
                                | Expression::IsNotNull(e) => contains_pattern_expr(e),
                                Expression::List(items) => items.iter().any(contains_pattern_expr),
                                Expression::Map(pairs) => {
                                    pairs.iter().any(|(_, v)| contains_pattern_expr(v))
                                }
                                _ => false,
                            }
                        }
                        if contains_pattern_expr(value) {
                            return Err(PolygraphError::Translation {
                                message: "SyntaxError: UnexpectedSyntax: patterns are not allowed inside SET RHS expressions".to_string(),
                            });
                        }
                    }
                    if let Some(rhs) = rhs_opt {
                        fn collect_set_rhs_vars(expr: &Expression, vars: &mut Vec<String>) {
                            match expr {
                                Expression::Variable(v) => vars.push(v.clone()),
                                Expression::Property(base, _) => collect_set_rhs_vars(base, vars),
                                Expression::FunctionCall { args, .. } => {
                                    for a in args {
                                        collect_set_rhs_vars(a, vars);
                                    }
                                }
                                Expression::Add(a, b)
                                | Expression::Subtract(a, b)
                                | Expression::Multiply(a, b)
                                | Expression::Divide(a, b)
                                | Expression::Comparison(a, _, b) => {
                                    collect_set_rhs_vars(a, vars);
                                    collect_set_rhs_vars(b, vars);
                                }
                                Expression::Not(e)
                                | Expression::Negate(e)
                                | Expression::IsNull(e)
                                | Expression::IsNotNull(e) => collect_set_rhs_vars(e, vars),
                                _ => {}
                            }
                        }
                        if seen_match || !bound_vars.is_empty() {
                            let mut rhs_vars = Vec::new();
                            collect_set_rhs_vars(rhs, &mut rhs_vars);
                            for v in rhs_vars {
                                if !bound_vars.contains(v.as_str())
                                    && !kinds.contains_key(v.as_str())
                                {
                                    return Err(PolygraphError::Translation {
                                        message: format!(
                                            "UndefinedVariable: variable '{v}' not defined"
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            Clause::Create(c) => {
                // VariableAlreadyBound: a CREATE pattern variable was already introduced by
                // a preceding MATCH/WITH/UNWIND, or reused within the same CREATE in a way
                // that would re-bind it (adding labels/props, or standalone node pattern).
                //
                // NB: A bound variable CAN appear in a CREATE pattern as an endpoint of a
                // relationship without labels/props — e.g. `(a)-[:R]->(b)` when both `a`
                // and `b` were previously bound.  That is NOT VariableAlreadyBound.
                //
                // It IS an error when:
                //   1. The node element has labels (would add new labels to existing node), OR
                //   2. The node element has properties (would set properties on existing node), OR
                //   3. The pattern contains only a single element (standalone `(a)` creation).
                let mut create_vars: std::collections::HashSet<String> =
                    std::collections::HashSet::new();

                /// Collect variable names referenced in a property map literal.
                fn collect_prop_vars(
                    props: &Option<crate::ast::cypher::MapLiteral>,
                    vars: &mut Vec<String>,
                ) {
                    if let Some(map) = props {
                        for (_, v) in map {
                            collect_create_expr_vars(v, vars);
                        }
                    }
                }

                fn collect_create_expr_vars(expr: &Expression, vars: &mut Vec<String>) {
                    match expr {
                        Expression::Variable(v) => vars.push(v.clone()),
                        Expression::Property(base, _) => collect_create_expr_vars(base, vars),
                        Expression::FunctionCall { args, .. } => {
                            for a in args {
                                collect_create_expr_vars(a, vars);
                            }
                        }
                        Expression::Add(a, b)
                        | Expression::Subtract(a, b)
                        | Expression::Multiply(a, b)
                        | Expression::Divide(a, b) => {
                            collect_create_expr_vars(a, vars);
                            collect_create_expr_vars(b, vars);
                        }
                        _ => {}
                    }
                }

                for pattern in &c.pattern.0 {
                    let is_standalone = pattern.elements.len() == 1;
                    for elem in &pattern.elements {
                        match elem {
                            PatternElement::Node(n) => {
                                if let Some(v) = &n.variable {
                                    let already_bound = bound_vars.contains(v.as_str())
                                        || kinds.contains_key(v.as_str())
                                        || !create_vars.insert(v.clone());
                                    if already_bound {
                                        let adds_labels = !n.labels.is_empty();
                                        let adds_props = n.properties.is_some();
                                        if adds_labels || adds_props || is_standalone {
                                            return Err(PolygraphError::Translation {
                                                message: format!(
                                                    "VariableAlreadyBound: '{v}' is already bound"
                                                ),
                                            });
                                        }
                                        // else: valid endpoint reuse in a relationship pattern
                                    }
                                }
                                // UndefinedVariable in node properties
                                let mut prop_vars = Vec::new();
                                collect_prop_vars(&n.properties, &mut prop_vars);
                                for pv in prop_vars {
                                    if !bound_vars.contains(pv.as_str())
                                        && !kinds.contains_key(pv.as_str())
                                        && !create_vars.contains(pv.as_str())
                                    {
                                        return Err(PolygraphError::Translation {
                                            message: format!(
                                                "UndefinedVariable: variable '{pv}' not defined"
                                            ),
                                        });
                                    }
                                }
                            }
                            PatternElement::Relationship(r) => {
                                // NoSingleRelationshipType: CREATE must have exactly 1 rel type.
                                if r.rel_types.is_empty() {
                                    return Err(PolygraphError::Translation {
                                        message:
                                            "NoSingleRelationshipType: \
                                                  relationship in CREATE must have exactly one type"
                                                .to_string(),
                                    });
                                }
                                if r.rel_types.len() > 1 {
                                    return Err(PolygraphError::Translation {
                                        message:
                                            "NoSingleRelationshipType: \
                                                  relationship in CREATE cannot have multiple types"
                                                .to_string(),
                                    });
                                }
                                // RequiresDirectedRelationship: undirected in CREATE
                                if r.direction == crate::ast::cypher::Direction::Both {
                                    return Err(PolygraphError::Translation {
                                        message: "RequiresDirectedRelationship: \
                                                  relationship in CREATE must have a direction"
                                            .to_string(),
                                    });
                                }
                                // CreatingVarLength: variable-length rels in CREATE are invalid.
                                if r.range.is_some() {
                                    return Err(PolygraphError::Translation {
                                        message: "CreatingVarLength: \
                                                  variable-length relationship in CREATE \
                                                  is not supported"
                                            .to_string(),
                                    });
                                }
                                // VariableAlreadyBound for relationship variables
                                if let Some(rv) = &r.variable {
                                    if bound_vars.contains(rv.as_str())
                                        || kinds.contains_key(rv.as_str())
                                        || !create_vars.insert(rv.clone())
                                    {
                                        return Err(PolygraphError::Translation {
                                            message: format!(
                                                "VariableAlreadyBound: '{rv}' is already bound"
                                            ),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                // Register CREATE-created variables so subsequent clauses see them as bound.
                for v in create_vars {
                    bound_vars.insert(v);
                }
            }
            Clause::Merge(m) => {
                // Validate MERGE pattern — same rules as CREATE for structural errors.
                let mut merge_vars: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                let is_standalone = m.pattern.elements.len() == 1;
                for elem in &m.pattern.elements {
                    match elem {
                        PatternElement::Node(n) => {
                            // Null property values in MERGE are invalid (SemanticError).
                            if let Some(props) = &n.properties {
                                if props
                                    .iter()
                                    .any(|(_, v)| matches!(v, Expression::Literal(Literal::Null)))
                                {
                                    return Err(PolygraphError::Translation {
                                        message: "SemanticError: MergeReadOwnWrites: null property values are not allowed in MERGE".to_string(),
                                    });
                                }
                            }
                            if let Some(v) = &n.variable {
                                let already_bound = bound_vars.contains(v.as_str())
                                    || kinds.contains_key(v.as_str())
                                    || !merge_vars.insert(v.clone());
                                if already_bound {
                                    let adds_labels = !n.labels.is_empty();
                                    let adds_props = n.properties.is_some();
                                    if adds_labels || adds_props || is_standalone {
                                        return Err(PolygraphError::Translation {
                                            message: format!(
                                                "VariableAlreadyBound: '{v}' is already bound"
                                            ),
                                        });
                                    }
                                }
                            }
                        }
                        PatternElement::Relationship(r) => {
                            // Null property values in MERGE are invalid (SemanticError).
                            if let Some(props) = &r.properties {
                                if props
                                    .iter()
                                    .any(|(_, v)| matches!(v, Expression::Literal(Literal::Null)))
                                {
                                    return Err(PolygraphError::Translation {
                                        message: "SemanticError: MergeReadOwnWrites: null property values are not allowed in MERGE".to_string(),
                                    });
                                }
                            }
                            // NoSingleRelationshipType: MERGE must have exactly 1 rel type.
                            if r.rel_types.is_empty() {
                                return Err(PolygraphError::Translation {
                                    message: "NoSingleRelationshipType: \
                                              relationship in MERGE must have exactly one type"
                                        .to_string(),
                                });
                            }
                            if r.rel_types.len() > 1 {
                                return Err(PolygraphError::Translation {
                                    message: "NoSingleRelationshipType: \
                                              relationship in MERGE cannot have multiple types"
                                        .to_string(),
                                });
                            }
                            // CreatingVarLength: variable-length rels in MERGE are invalid.
                            if r.range.is_some() {
                                return Err(PolygraphError::Translation {
                                    message: "CreatingVarLength: \
                                              variable-length relationship in MERGE \
                                              is not supported"
                                        .to_string(),
                                });
                            }
                            // VariableAlreadyBound for relationship variables
                            if let Some(rv) = &r.variable {
                                if bound_vars.contains(rv.as_str())
                                    || kinds.contains_key(rv.as_str())
                                    || !merge_vars.insert(rv.clone())
                                {
                                    return Err(PolygraphError::Translation {
                                        message: format!(
                                            "VariableAlreadyBound: '{rv}' is already bound"
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
                // Register MERGE-introduced variables so subsequent RETURN/WITH clauses
                // do not get false UndefinedVariable errors.
                for v in merge_vars {
                    let kind = m
                        .pattern
                        .elements
                        .iter()
                        .find_map(|e| match e {
                            PatternElement::Node(n)
                                if n.variable.as_deref() == Some(v.as_str()) =>
                            {
                                Some(Kind::Node)
                            }
                            PatternElement::Relationship(r)
                                if r.variable.as_deref() == Some(v.as_str()) =>
                            {
                                Some(Kind::Rel)
                            }
                            _ => None,
                        })
                        .unwrap_or(Kind::Node);
                    kinds.insert(v.clone(), kind);
                    bound_vars.insert(v);
                }
                // Also register the path variable (e.g. `MERGE p = (a {num: 1})`).
                if let Some(pv) = &m.pattern.variable {
                    kinds.insert(pv.clone(), Kind::Path);
                    bound_vars.insert(pv.clone());
                }
                // UndefinedVariable in ON CREATE SET / ON MATCH SET actions.
                for action in &m.actions {
                    for item in &action.items {
                        // Check subject variable (the thing being SET)
                        let subject_var: &str = match item {
                            crate::ast::cypher::SetItem::Property { variable, .. } => {
                                variable.as_str()
                            }
                            crate::ast::cypher::SetItem::NodeReplace { variable, .. } => {
                                variable.as_str()
                            }
                            crate::ast::cypher::SetItem::MergeMap { variable, .. } => {
                                variable.as_str()
                            }
                            crate::ast::cypher::SetItem::SetLabel { variable, .. } => {
                                variable.as_str()
                            }
                        };
                        if !bound_vars.contains(subject_var) && !kinds.contains_key(subject_var) {
                            return Err(PolygraphError::Translation {
                                message: format!(
                                    "UndefinedVariable: variable '{subject_var}' not defined"
                                ),
                            });
                        }
                        // Also check RHS expression variables.
                        let rhs_opt: Option<&Expression> = match item {
                            crate::ast::cypher::SetItem::Property { value, .. } => Some(value),
                            crate::ast::cypher::SetItem::NodeReplace { value, .. } => Some(value),
                            _ => None,
                        };
                        if let Some(rhs) = rhs_opt {
                            fn collect_merge_action_vars(
                                expr: &Expression,
                                vars: &mut Vec<String>,
                            ) {
                                match expr {
                                    Expression::Variable(v) => vars.push(v.clone()),
                                    Expression::Property(base, _) => {
                                        collect_merge_action_vars(base, vars)
                                    }
                                    Expression::Add(a, b)
                                    | Expression::Subtract(a, b)
                                    | Expression::Multiply(a, b)
                                    | Expression::Divide(a, b) => {
                                        collect_merge_action_vars(a, vars);
                                        collect_merge_action_vars(b, vars);
                                    }
                                    _ => {}
                                }
                            }
                            let mut rhs_vars = Vec::new();
                            collect_merge_action_vars(rhs, &mut rhs_vars);
                            for v in rhs_vars {
                                if !bound_vars.contains(v.as_str())
                                    && !kinds.contains_key(v.as_str())
                                {
                                    return Err(PolygraphError::Translation {
                                        message: format!(
                                            "UndefinedVariable: variable '{v}' not defined"
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

// ── Translation state ─────────────────────────────────────────────────────────

/// Info stored per relationship variable for property access resolution.
#[derive(Clone)]
struct EdgeInfo {
    src: TermPattern,
    pred: NamedNode,
    /// For untyped relationships, the SPARQL variable bound to the predicate.
    pred_var: Option<Variable>,
    dst: TermPattern,
    /// In reification mode: the fresh variable used as the reification node.
    reif_var: Option<Variable>,
    /// A marker variable bound when the edge is in scope (for IS NULL checks on
    /// typed relationships).  For untyped, `pred_var` already serves this role.
    null_check_var: Option<Variable>,
    /// Synthetic edge-identity variable for relationship comparisons (r = r2).
    /// BIND'd to a canonical CONCAT(STR(s), "|", STR(p), "|", STR(o)) of the
    /// actual stored triple, so it is direction-invariant for undirected matches.
    eid_var: Option<Variable>,
    /// The `with_generation` counter at the time this edge was bound.  When the
    /// second occurrence of a relationship variable is processed, we compare this
    /// against the current `with_generation`; if equal, no WITH has occurred since
    /// the edge was first bound and the eid_var is still in scope.
    binding_generation: u64,
}

/// One triple "slot" tracking a stored-triple (subject, predicate, object) for isomorphism.
type EdgeIsoSlot = (TermPattern, spargebra::term::NamedNodePattern, TermPattern);

struct TranslationState {
    base_iri: String,
    counter: usize,
    /// Use SPARQL-star annotated triples (true) or RDF reification (false).
    rdf_star: bool,
    /// Tracks relationship variables → edge info for `r.prop` resolution.
    edge_map: std::collections::HashMap<String, EdgeInfo>,
    /// Aggregates captured while translating expressions (e.g. count(*) inside arithmetic).
    pending_aggs: Vec<(Variable, AggregateExpression)>,
    /// Pre-extends from temporal property expressions. Drained into the `extends` list
    /// before the main extend for each RETURN/WITH item, ensuring that intermediate
    /// BIND variables (needed to avoid SPARQL serialization precedence issues) come
    /// first in the generated SPARQL.
    pending_pre_extends: Vec<(Variable, SparExpr)>,
    /// Per-MATCH tracking: each inner Vec holds one or two stored-triple instances for
    /// one relationship hop.  Used to generate pairwise relationship-isomorphism FILTERs.
    iso_hops: Vec<Vec<EdgeIsoSlot>>,
    /// Variables that may be null (unbound) because they were introduced by an
    /// OPTIONAL MATCH that produced no match.  When such a variable is used as a
    /// node/edge in a subsequent mandatory MATCH, we add FILTER(BOUND(?var)) to
    /// prevent it from acting as a wildcard in the SPARQL JOIN.
    nullable_vars: std::collections::HashSet<String>,
    /// For each nullable variable, the set of RDF type / label / sentinel triples
    /// from its OPTIONAL MATCH pattern.  These are used to guard property-access
    /// OPTIONALs: instead of `OPTIONAL { ?n <prop> ?val }` (wildcard risk), we emit
    /// `OPTIONAL { ?n rdf:type X . ?n <prop> ?val }` so that when no X-typed nodes
    /// exist the pattern contributes nothing, preventing unbound ?n from matching all
    /// nodes.
    nullable_type_guards: std::collections::HashMap<String, Vec<TriplePattern>>,
    /// Variables assigned a literal-list value by a WITH clause.
    /// Used so that `UNWIND list_var AS x` can be expanded at compile time.
    with_list_vars: std::collections::HashMap<String, crate::ast::cypher::Expression>,
    /// Variables produced by `UNWIND outer_list AS x` where outer_list is a
    /// list-of-lists.  Maps the produced variable name to the outer list expression.
    /// Enables compile-time expansion of subsequent `UNWIND x AS y` clauses.
    unwind_list_source: std::collections::HashMap<String, crate::ast::cypher::Expression>,
    /// Named path variables → hop count (for fixed-length paths).
    /// Used to resolve `length(p)` at compile time.
    path_hops: std::collections::HashMap<String, u64>,
    /// Named path variables → ordered list of node SPARQL variables.
    /// Used to resolve `nodes(p)` at compile time.
    path_node_vars: std::collections::HashMap<String, Vec<Variable>>,
    /// Variables bound as node patterns in MATCH clauses.
    node_vars: std::collections::HashSet<String>,
    /// Projected columns collected during RETURN translation.
    projected_columns: Vec<ProjectedColumn>,
    /// Property expression substitutions active during WITH ORDER BY processing.
    /// Maps (base_var_name, property_key) → SPARQL variable that holds that property value.
    /// Populated before ORDER BY translation, cleared after.
    with_prop_subst: std::collections::HashMap<(String, String), Variable>,
    /// Aggregate expression → output variable mapping, populated during RETURN/WITH
    /// processing so that ORDER BY can reuse the same variables instead of creating
    /// new (unbound) fresh variables for aggregate expressions like count(*).
    agg_orderby_subst: std::collections::HashMap<String, Variable>,
    /// Alias name → projected SPARQL variable, populated during RETURN ORDER BY
    /// setup so that ORDER BY expressions referencing an alias (e.g. `ORDER BY n + 2`
    /// where `n` is a RETURN alias) resolve to the correct projected variable.
    return_alias_subst: std::collections::HashMap<String, Variable>,
    /// Whether the last RETURN used DISTINCT.
    return_distinct: bool,
    /// Variable-length relationship variables with their (lower, upper) hop bounds.
    /// Used to support `last(r)` / `head(r)` on bounded varlen paths.
    varlen_rel_scope: std::collections::HashMap<String, (u64, u64)>,
    /// Virtual map aliases from `head(collect({k: v})) AS alias`.
    /// Maps alias name → {key → SPARQL variable holding the min-aggregated value}.
    map_vars: std::collections::HashMap<String, std::collections::HashMap<String, Variable>>,
    /// Monotonically increasing counter, incremented on every WITH clause.
    /// Stored in each EdgeInfo so we can detect whether a WITH has occurred
    /// between when a relationship was first bound and when it is re-used.
    with_generation: u64,
    /// Deferred FILTER expressions accumulated during translate_relationship_pattern
    /// for re-used edges.  Applied to the combined MATCH pattern in
    /// translate_match_clause after all path patterns are joined (so the outer
    /// variables from preceding MATCH clauses are in scope for the FILTER).
    pending_match_filters: Vec<spargebra::algebra::Expression>,
    /// Expressions to bind to fresh variables (for IsNull/IsNotNull on complex
    /// boolean expressions). Each entry corresponds to a (var, expr) Extend binding
    /// that should be applied BEFORE the filter using the bound variable.
    pending_bind_checks: Vec<spargebra::algebra::Expression>,
    pending_bind_targets: Vec<Variable>,
    /// Variables from UNWIND lists that contained null (UNDEF) values.
    /// Used to add FILTER(BOUND(?v)) before aggregate GROUP patterns to work
    /// around an oxigraph bug where MAX/MIN over UNDEF+typed values returns null.
    unwind_null_vars: std::collections::HashSet<String>,
    /// Variables from UNWIND lists that contain BOTH null AND non-null values.
    /// Used to determine when FILTER(BOUND) is safe for DISTINCT GROUP_CONCAT.
    unwind_mixed_null_vars: std::collections::HashSet<String>,
    /// Variables that are statically known to be bound to `null` via a WITH clause
    /// like `WITH null AS x`.  Used to short-circuit subscript and property access
    /// on null, returning null instead of an error.
    null_vars: std::collections::HashSet<String>,
    /// Variables that are statically known to be bound to a compile-time integer.
    /// Populated when a WITH item evaluates to a literal integer (e.g., `size(lit_list)`).
    /// Used to resolve expressions like `range(0, numOfValues-1)`.
    const_int_vars: std::collections::HashMap<String, i64>,
    /// Subquery graph patterns generated by pattern comprehensions.
    /// Each entry is (result_var, subquery_pattern).  The caller joins these
    /// into the outer pattern after translating the containing expression.
    pending_subqueries: Vec<(Variable, GraphPattern)>,
    /// When true, write clauses (SET/REMOVE/MERGE/CREATE/DELETE) are silently skipped
    /// instead of returning an UnsupportedFeature error. Used by callers that handle
    /// write operations separately (e.g., the TCK test harness).
    skip_write_clauses: bool,
    /// Labels known for node variables from CREATE patterns (populated in skip_writes mode).
    /// Maps variable name → list of label strings.
    node_labels_from_create: std::collections::HashMap<String, Vec<String>>,
    /// Properties known for node/relationship variables from CREATE patterns (skip_writes mode).
    /// Maps variable name → {property_key → Expression value}.
    node_props_from_create:
        std::collections::HashMap<String, std::collections::HashMap<String, Expression>>,
    /// Tracks (variable, property_key) pairs that were SET by a SET clause in skip_writes mode.
    /// Used to distinguish SET-updated properties from CREATE-initialized ones.
    set_tracked_vars: std::collections::HashSet<(String, String)>,
    /// Labels being REMOVED in skip_writes mode. Maps variable name → set of label strings.
    /// When a label is in this map, it is skipped in MATCH pattern label constraints.
    remove_tracked_labels: std::collections::HashMap<String, std::collections::HashSet<String>>,
    /// WITH alias → string lexical value of the literal it was bound to.
    /// Used to fold expressions like `date(toString(alias))` at compile time.
    with_lit_vars: std::collections::HashMap<String, String>,
    /// Filter conditions emitted by translate_node_pattern_with_term when a
    /// property expression is too complex for an inline TermPattern object.
    /// The caller must drain and apply these after translating the pattern.
    pending_prop_filters: Vec<SparExpr>,
    /// Maps list UNWIND variable name → sort-key SPARQL variable name (`__sk_<var>`).
    ///
    /// When `UNWIND [...] AS x` expands a list of nested lists or mixed types, a
    /// parallel VALUES column `?__sk_x` is emitted, containing lexicographically
    /// sortable keys that encode Cypher's type ordering.  `apply_order_skip_limit`
    /// substitutes the sort-key variable in ORDER BY expressions that reference `x`.
    list_sort_key_vars: std::collections::HashMap<String, String>,
}

