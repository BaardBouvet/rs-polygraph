impl TranslationState {
    fn translate_clause_sequence(
        &mut self,
        clauses: &[Clause],
    ) -> Result<GraphPattern, PolygraphError> {
        // Accumulate extra BGP triples emitted during expression translation.
        let mut extra_triples: Vec<TriplePattern> = Vec::new();
        // The pattern is built left-to-right over clauses.
        let mut current = empty_bgp();
        // Collects filters to apply at the end of each scope.
        let mut pending_filters: Vec<SparExpr> = Vec::new();
        // The output variables of the most recent WITH clause (used to build
        // sub-select scope boundaries when nullable variables must be checked).
        let mut last_with_vars: Option<Vec<Variable>> = None;

        // In skip_writes mode, pre-scan SET and REMOVE clauses so that MATCH patterns
        // can be adjusted to find nodes/relationships after updates have been applied.
        if self.skip_write_clauses {
            // First pass: populate node_props_from_create from CREATE clauses so that
            // self-referential SET expressions (e.g. a.nums = a.nums + [4, 5]) can
            // be statically folded in the second pass below.
            for clause in clauses {
                if let Clause::Create(c) = clause {
                    for pat in &c.pattern.0 {
                        for elem in &pat.elements {
                            if let PatternElement::Node(n) = elem {
                                if let Some(v) = &n.variable {
                                    if let Some(props) = &n.properties {
                                        let prop_map: std::collections::HashMap<
                                            String,
                                            Expression,
                                        > = props
                                            .iter()
                                            .map(|(k, vv)| (k.clone(), vv.clone()))
                                            .collect();
                                        // Only insert; the main loop will overwrite later.
                                        self.node_props_from_create
                                            .entry(v.clone())
                                            .or_insert(prop_map);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            for clause in clauses {
                match clause {
                    Clause::Set(s) => {
                        for item in &s.items {
                            if let crate::ast::cypher::SetItem::Property {
                                variable,
                                key,
                                value,
                            } = item
                            {
                                if !expr_references_prop(value, variable, key) {
                                    self.set_tracked_vars
                                        .insert((variable.clone(), key.clone()));
                                    self.node_props_from_create
                                        .entry(variable.clone())
                                        .or_default()
                                        .insert(key.clone(), value.clone());
                                } else {
                                    // Self-referential SET (e.g. a.nums = a.nums + [4, 5]).
                                    // If the current value is known, fold the expression statically.
                                    let current = self
                                        .node_props_from_create
                                        .get(variable.as_str())
                                        .and_then(|m| m.get(key.as_str()))
                                        .cloned();
                                    if let Some(cur) = current {
                                        if let Some(folded) =
                                            fold_set_list_concat(value, variable, key, &cur)
                                        {
                                            self.set_tracked_vars
                                                .insert((variable.clone(), key.clone()));
                                            self.node_props_from_create
                                                .entry(variable.clone())
                                                .or_default()
                                                .insert(key.clone(), folded);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Clause::Remove(r) => {
                        for item in &r.items {
                            if let crate::ast::cypher::RemoveItem::Label { variable, labels } = item
                            {
                                for label in labels {
                                    self.remove_tracked_labels
                                        .entry(variable.clone())
                                        .or_default()
                                        .insert(label.clone());
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        for clause in clauses {
            match clause {
                Clause::Match(m) => {
                    if m.optional {
                        // For OPTIONAL MATCH, use a local extra buffer so that
                        let mut opt_extra: Vec<TriplePattern> = Vec::new();
                        let (match_pattern, opt_filter, where_extra) =
                            self.translate_match_clause(m, &mut opt_extra)?;
                        // Merge where_extra into the right-hand side of the LeftJoin
                        // as OPTIONAL LeftJoins for proper null semantics inside the
                        // optional scope.
                        let mut right = match_pattern;
                        for tp in where_extra {
                            right = GraphPattern::LeftJoin {
                                left: Box::new(right),
                                right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                expression: None,
                            };
                        }
                        // Also merge opt_extra (now unused, _extra param) into right.
                        if !opt_extra.is_empty() {
                            right = join_patterns(
                                right,
                                GraphPattern::Bgp {
                                    patterns: opt_extra,
                                },
                            );
                        }
                        // Extract type-guard triples BEFORE moving `right` into the LeftJoin.
                        // These will be used later for nullable property-access OPTIONALs.
                        for v in collect_pattern_vars(&m.pattern) {
                            let guards = extract_type_guards(&right, &v);
                            if !guards.is_empty() {
                                self.nullable_type_guards.insert(v, guards);
                            }
                        }
                        current = GraphPattern::LeftJoin {
                            left: Box::new(current),
                            right: Box::new(right),
                            expression: opt_filter,
                        };
                        // Mark variables introduced by this OPTIONAL MATCH as
                        // possibly-null so downstream mandatory MATCHes can add
                        // FILTER(BOUND(?var)) guards.
                        for v in collect_pattern_vars(&m.pattern) {
                            self.nullable_vars.insert(v);
                        }
                        // Also mark path variables from OPTIONAL MATCH as nullable.
                        for pattern in &m.pattern.0 {
                            if let Some(path_var) = &pattern.variable {
                                self.nullable_vars.insert(path_var.clone());
                            }
                        }
                    } else {
                        // Before joining, emit FILTER(BOUND(?v)) for any pattern
                        // variable that might be null from a prior OPTIONAL MATCH.
                        let pattern_vars = collect_pattern_vars(&m.pattern);
                        let nullable_used: Vec<String> = pattern_vars
                            .iter()
                            .filter(|v| self.nullable_vars.contains(*v))
                            .cloned()
                            .collect();
                        if !nullable_used.is_empty() {
                            // Flush pending filters before creating the scope boundary.
                            current = apply_filters(current, pending_filters.drain(..));
                            // Add FILTER(BOUND(?v)) for each nullable variable used.
                            let bound_filter = nullable_used
                                .iter()
                                .map(|v| SparExpr::Bound(Variable::new_unchecked(v.clone())))
                                .reduce(|a, b| SparExpr::And(Box::new(a), Box::new(b)))
                                .expect("non-empty");
                            current = GraphPattern::Filter {
                                expr: bound_filter,
                                inner: Box::new(current),
                            };
                            // Wrap in a sub-select scope boundary so the filter is
                            // evaluated BEFORE the outer join with the next MATCH
                            // pattern.  Without this, SPARQL 1.1 re-applies filters
                            // after all joins, allowing unbound variables to act as
                            // wildcards.  Use `last_with_vars` (the WITH-declared
                            // output vars) as the projection list.
                            if let Some(ref wv) = last_with_vars {
                                current = GraphPattern::Project {
                                    inner: Box::new(current),
                                    variables: wv.clone(),
                                };
                            }
                            for v in &nullable_used {
                                self.nullable_vars.remove(v);
                            }
                        }
                        let (match_pattern, opt_filter, where_extra) =
                            self.translate_match_clause(m, &mut extra_triples)?;
                        current = join_patterns(current, match_pattern);
                        // Apply deferred sameTerm filters for re-used relationship variables.
                        // Applied here (after JOIN) so outer-scope variables from preceding
                        // MATCH clauses are in scope for the sameTerm comparisons.
                        let reuse_filters = std::mem::take(&mut self.pending_match_filters);
                        current = apply_filters(current, reuse_filters.into_iter());
                        // WHERE-clause property-access triples applied as OPTIONAL so
                        // that missing properties evaluate to null (Cypher semantics)
                        // rather than filtering the row out entirely.
                        for tp in where_extra {
                            current = GraphPattern::LeftJoin {
                                left: Box::new(current),
                                right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                expression: None,
                            };
                        }
                        if let Some(f) = opt_filter {
                            pending_filters.push(f);
                        }
                    }
                }
                Clause::With(w) => {
                    // A new WITH clause increases the generation so that re-used
                    // relationship variables from MATCH clauses before this WITH are
                    // not mistakenly treated as still in scope for eid-filter reuse.
                    self.with_generation += 1;
                    // Flush any pending extra triples.
                    if !extra_triples.is_empty() {
                        let extra = GraphPattern::Bgp {
                            patterns: std::mem::take(&mut extra_triples),
                        };
                        current = join_patterns(current, extra);
                    }
                    // Flush pending filters.
                    current = apply_filters(current, pending_filters.drain(..));

                    // WITH acts as a projection/aggregation scope boundary.
                    if let crate::ast::cypher::ReturnItems::Explicit(ref items) = w.items {
                        // Filter out List/Map-valued items that can't be expressed
                        // as SPARQL expressions.  These are tracked in with_list_vars
                        // for later UNWIND expansion.
                        let translatable_items: Vec<_> = items
                            .iter()
                            .filter(|item| {
                                !matches!(
                                    &item.expression,
                                    Expression::List(_) | Expression::Map(_)
                                )
                            })
                            .cloned()
                            .collect();
                        // Also collect List-valued items that need VALUES bindings.
                        let list_items: Vec<_> = items
                            .iter()
                            .filter(|item| matches!(&item.expression, Expression::List(_)))
                            .cloned()
                            .collect();

                        let as_return = ReturnClause {
                            distinct: w.distinct,
                            items: crate::ast::cypher::ReturnItems::Explicit(translatable_items),
                            order_by: None,
                            skip: None,
                            limit: None,
                        };
                        let (
                            with_triples,
                            mut project_vars,
                            need_distinct,
                            aggregates,
                            extends,
                            post_extends,
                        ) = self.translate_return_clause(&as_return, &mut extra_triples)?;

                        // For List-valued WITH items, emit VALUES bindings and add
                        // to the project vars list.
                        let _list_values_patterns: Vec<GraphPattern> = Vec::new();
                        for li in &list_items {
                            let var_name = li.alias.as_deref().unwrap_or("__list");
                            let var = Variable::new_unchecked(var_name.to_string());
                            if let Expression::List(elems) = &li.expression {
                                // Serialize each element as a ground term for VALUES.
                                let mut bindings: Vec<Vec<Option<GroundTerm>>> = Vec::new();
                                for elem in elems {
                                    match elem {
                                        Expression::List(inner) => {
                                            // Nested list: serialize as string.
                                            let parts: Vec<String> = inner
                                                .iter()
                                                .filter_map(|e| match e {
                                                    Expression::Literal(Literal::Integer(n)) => {
                                                        Some(n.to_string())
                                                    }
                                                    Expression::Literal(Literal::String(s)) => {
                                                        Some(format!("'{s}'"))
                                                    }
                                                    _ => None,
                                                })
                                                .collect();
                                            let encoded = format!("[{}]", parts.join(", "));
                                            bindings.push(vec![Some(GroundTerm::Literal(
                                                SparLit::new_simple_literal(encoded),
                                            ))]);
                                        }
                                        _ => {
                                            if let Ok(gt) = self.expr_to_ground_term(elem) {
                                                if let Ok(ground) = term_pattern_to_ground(gt) {
                                                    bindings.push(vec![Some(ground)]);
                                                }
                                            }
                                        }
                                    }
                                }
                                // Bind the list variable to a serialized string
                                // representation so RETURN * can project it.
                                let serialized = serialize_list_literal(elems);
                                // Add Extend to bind list var to serialized string.
                                current = GraphPattern::Extend {
                                    inner: Box::new(current),
                                    variable: var.clone(),
                                    expression: SparExpr::Literal(SparLit::new_simple_literal(
                                        serialized,
                                    )),
                                };
                            }
                            if let Some(ref mut pvars) = project_vars {
                                pvars.push(var);
                                // When there's no aggregation, also project any node/edge
                                // variables embedded in the list so they remain in scope
                                // after the WITH boundary for downstream use (e.g.
                                // `labels(list[0])` when `list = [a, 1]`,
                                // `type(list[0])` when `list = [r, 1]`).
                                if aggregates.is_empty() {
                                    if let Expression::List(elems) = &li.expression {
                                        for elem in elems {
                                            if let Expression::Variable(v) = elem {
                                                let vv = Variable::new_unchecked(v.clone());
                                                if !pvars.contains(&vv) {
                                                    pvars.push(vv);
                                                }
                                                // For edge variables, also project pred_var,
                                                // eid_var, src and dst so type(r), r.prop etc.
                                                // continue to work after the WITH boundary.
                                                if let Some(edge) =
                                                    self.edge_map.get(v.as_str()).cloned()
                                                {
                                                    if let TermPattern::Variable(sv) = &edge.src {
                                                        if !pvars.contains(sv) {
                                                            pvars.push(sv.clone());
                                                        }
                                                    }
                                                    if let TermPattern::Variable(dv) = &edge.dst {
                                                        if !pvars.contains(dv) {
                                                            pvars.push(dv.clone());
                                                        }
                                                    }
                                                    if let Some(pv) = &edge.pred_var {
                                                        if !pvars.contains(pv) {
                                                            pvars.push(pv.clone());
                                                        }
                                                    }
                                                    if let Some(ev) = &edge.eid_var {
                                                        if !pvars.contains(ev) {
                                                            pvars.push(ev.clone());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Also handle Map-valued items: bind each key as a separate variable
                        // so property access like `map.key` can be resolved at translation time.
                        let map_items: Vec<_> = items
                            .iter()
                            .filter(|item| matches!(&item.expression, Expression::Map(_)))
                            .cloned()
                            .collect();
                        for mi in &map_items {
                            let alias = mi.alias.as_deref().unwrap_or("__map");
                            if let Expression::Map(pairs) = &mi.expression {
                                let mut key_vars: std::collections::HashMap<String, Variable> =
                                    Default::default();
                                for (key, val_expr) in pairs {
                                    let key_var =
                                        Variable::new_unchecked(format!("{alias}__{key}"));
                                    // Bind the key variable to the value (or leave unbound for null).
                                    match val_expr {
                                        Expression::Literal(Literal::Null) => {
                                            // null → leave key_var unbound (not added to Extend)
                                        }
                                        _ => {
                                            if let Ok(sparql_expr) =
                                                self.translate_expr(val_expr, &mut extra_triples)
                                            {
                                                current = GraphPattern::Extend {
                                                    inner: Box::new(current),
                                                    variable: key_var.clone(),
                                                    expression: sparql_expr,
                                                };
                                            }
                                            // If val is a nested map, recursively register inner key vars
                                            // so that `alias.key.innerkey` chains are resolvable.
                                            if let Expression::Map(inner_pairs) = val_expr {
                                                let inner_alias = format!("{alias}__{key}");
                                                let mut inner_key_vars: std::collections::HashMap<
                                                    String,
                                                    Variable,
                                                > = Default::default();
                                                for (ik, iv) in inner_pairs {
                                                    let iv_var = Variable::new_unchecked(format!(
                                                        "{inner_alias}__{ik}"
                                                    ));
                                                    if !matches!(
                                                        iv,
                                                        Expression::Literal(Literal::Null)
                                                    ) {
                                                        if let Ok(sv) = self
                                                            .translate_expr(iv, &mut extra_triples)
                                                        {
                                                            current = GraphPattern::Extend {
                                                                inner: Box::new(current),
                                                                variable: iv_var.clone(),
                                                                expression: sv,
                                                            };
                                                        }
                                                    }
                                                    inner_key_vars
                                                        .insert(ik.clone(), iv_var.clone());
                                                    if let Some(ref mut pvars) = project_vars {
                                                        if !matches!(
                                                            iv,
                                                            Expression::Literal(Literal::Null)
                                                        ) {
                                                            pvars.push(iv_var);
                                                        }
                                                    }
                                                }
                                                self.map_vars.insert(inner_alias, inner_key_vars);
                                            }
                                        }
                                    }
                                    key_vars.insert(key.clone(), key_var.clone());
                                    if let Some(ref mut pvars) = project_vars {
                                        // Only add non-null vars (null ones remain unbound)
                                        if !matches!(val_expr, Expression::Literal(Literal::Null)) {
                                            pvars.push(key_var);
                                        }
                                    }
                                }
                                self.map_vars.insert(alias.to_string(), key_vars);
                            }
                        }

                        // Property-access triples in WITH must be OPTIONAL.
                        // When the triple's predicate is rdf:reifies, the NEXT triple
                        // (the actual property access on the reification node) MUST live
                        // in the SAME OPTIONAL block; otherwise the reification variable
                        // is unbound in the second OPTIONAL and acts as a wildcard.
                        {
                            const RDF_REIFIES: &str =
                                "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
                            let mut i = 0;
                            while i < with_triples.len() {
                                let tp = with_triples[i].clone();
                                let is_reifies = matches!(
                                    &tp.predicate,
                                    spargebra::term::NamedNodePattern::NamedNode(nn)
                                        if nn.as_str() == RDF_REIFIES
                                );
                                let group_end = if is_reifies && i + 1 < with_triples.len() {
                                    i + 2
                                } else {
                                    i + 1
                                };
                                let group: Vec<TriplePattern> = with_triples[i..group_end].to_vec();
                                i = group_end;
                                current = GraphPattern::LeftJoin {
                                    left: Box::new(current),
                                    right: Box::new(GraphPattern::Bgp { patterns: group }),
                                    expression: None,
                                };
                            }
                        }
                        // Flush extra triples from expression translation.
                        if !extra_triples.is_empty() {
                            const RDF_REIFIES: &str =
                                "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
                            let drained: Vec<TriplePattern> = extra_triples.drain(..).collect();
                            let mut i = 0;
                            while i < drained.len() {
                                let tp = drained[i].clone();
                                let is_reifies = matches!(
                                    &tp.predicate,
                                    spargebra::term::NamedNodePattern::NamedNode(nn)
                                        if nn.as_str() == RDF_REIFIES
                                );
                                let group_end = if is_reifies && i + 1 < drained.len() {
                                    i + 2
                                } else {
                                    i + 1
                                };
                                let group: Vec<TriplePattern> = drained[i..group_end].to_vec();
                                i = group_end;
                                current = GraphPattern::LeftJoin {
                                    left: Box::new(current),
                                    right: Box::new(GraphPattern::Bgp { patterns: group }),
                                    expression: None,
                                };
                            }
                        }

                        // Detect alias-to-variable-name conflicts (e.g. `a.name AS a`).
                        // The Property branch uses a fresh var when alias == base_var,
                        // so the project_vars contain the fresh name, not the alias.
                        // Build a rename map: alias → fresh_var for these cases.
                        // IMPORTANT: only compare translatable_items (not list items),
                        // since pvars is built from translatable_items first, then list
                        // vars are appended. Zipping all items against pvars would
                        // misalign when list items appear earlier in the original WITH.
                        let mut outer_renames: Vec<(Variable, Variable)> = Vec::new();
                        if let crate::ast::cypher::ReturnItems::Explicit(ref ti) = as_return.items {
                            if let Some(ref pvars) = project_vars {
                                for (item, pvar) in ti.iter().zip(pvars.iter()) {
                                    if let Some(ref alias) = item.alias {
                                        if alias != pvar.as_str() {
                                            // Projected var differs from the alias —
                                            // need to rename after sub-select.
                                            outer_renames.push((
                                                Variable::new_unchecked(alias.clone()),
                                                pvar.clone(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }

                        // ── BIND-conflict resolution ─────────────────────────────────────
                        // SPARQL 1.1 §18.2.4 forbids BIND(expr AS ?v) when ?v is already
                        // in scope.  This arises when WITH renames a variable to a name
                        // that is already bound by the preceding MATCH pattern, e.g.:
                        //   MATCH (a)-[r]->(b) WITH a AS b, b AS tmp
                        // Fix: for each conflicting extend target, insert an inner Extend
                        // that renames the original binding to a fresh "shadow" variable,
                        // then wrap in a sub-Project that hides the original name.  The
                        // outer BIND can then legally use that name.
                        {
                            let current_scope = bound_vars_of_pattern(&current);
                            let mut shadows: std::collections::HashMap<String, Variable> =
                                Default::default();
                            for (target, _) in &extends {
                                let name = target.as_str();
                                if current_scope.contains(name) {
                                    let shadow = self.fresh_var(&format!("shadow_{name}"));
                                    shadows.insert(name.to_string(), shadow);
                                }
                            }
                            if !shadows.is_empty() {
                                // Add inner Extend nodes to bind shadow vars.
                                for (orig_name, shadow) in &shadows {
                                    let orig_var = Variable::new_unchecked(orig_name.clone());
                                    current = GraphPattern::Extend {
                                        inner: Box::new(current),
                                        variable: shadow.clone(),
                                        expression: SparExpr::Variable(orig_var),
                                    };
                                }
                                // Wrap in a sub-Project: expose shadow vars, hide originals.
                                let mut shadow_proj_vars: Vec<Variable> = current_scope
                                    .iter()
                                    .filter(|n| !shadows.contains_key(*n))
                                    .map(|n| Variable::new_unchecked(n.clone()))
                                    .collect();
                                for shadow in shadows.values() {
                                    shadow_proj_vars.push(shadow.clone());
                                }
                                current = GraphPattern::Project {
                                    inner: Box::new(current),
                                    variables: shadow_proj_vars,
                                };
                            }
                            for (var, expr) in &extends {
                                // Rewrite any source variable that was shadowed.
                                let resolved_expr = match expr {
                                    SparExpr::Variable(v) if shadows.contains_key(v.as_str()) => {
                                        SparExpr::Variable(shadows[v.as_str()].clone())
                                    }
                                    _ => expr.clone(),
                                };
                                current = GraphPattern::Extend {
                                    inner: Box::new(current),
                                    variable: var.clone(),
                                    expression: resolved_expr,
                                };
                            }
                        }

                        // Join in any correlated subqueries from pattern comprehensions (WITH clause).
                        current = self.drain_pending_subqueries(current);

                        // Apply aggregation (GROUP BY).
                        if !aggregates.is_empty() {
                            // Work around oxigraph bug: MAX/MIN/SUM over VALUES with UNDEF
                            // returns null. Add FILTER(BOUND(?v)) for UNWIND null vars.
                            if !self.unwind_null_vars.is_empty() {
                                for null_var_name in self.unwind_null_vars.clone() {
                                    let bound_var = Variable::new_unchecked(null_var_name);
                                    current = GraphPattern::Filter {
                                        inner: Box::new(current),
                                        expr: SparExpr::Bound(bound_var),
                                    };
                                }
                            }
                            let group_vars: Vec<Variable> = project_vars
                                .as_ref()
                                .map(|vs| {
                                    vs.iter()
                                        .filter(|v| !aggregates.iter().any(|(av, _)| av == *v))
                                        .filter(|v| !post_extends.iter().any(|(pv, _)| pv == *v))
                                        .cloned()
                                        .collect()
                                })
                                .unwrap_or_default();
                            current = GraphPattern::Group {
                                inner: Box::new(current),
                                variables: group_vars,
                                aggregates: aggregates.clone(),
                            };
                        }

                        for (var, expr) in &post_extends {
                            current = GraphPattern::Extend {
                                inner: Box::new(current),
                                variable: var.clone(),
                                expression: expr.clone(),
                            };
                        }

                        // Apply WITH WHERE clause BEFORE projection so that both old-scope
                        // variables (e.g. ?a from MATCH) and newly-bound aliases (from Extends)
                        // are in scope. After Project, old variables are no longer visible.
                        if let Some(ref wc) = w.where_ {
                            let filter_expr =
                                self.translate_expr(&wc.expression, &mut extra_triples)?;
                            // Apply any property-access triples needed by WHERE as OPTIONAL joins.
                            // Group rdf:reifies triples with their following property-access triple
                            // into a single OPTIONAL block.
                            if !extra_triples.is_empty() {
                                const RDF_REIFIES: &str =
                                    "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
                                let drained: Vec<TriplePattern> = extra_triples.drain(..).collect();
                                let mut i = 0;
                                while i < drained.len() {
                                    let tp = drained[i].clone();
                                    let is_reifies = matches!(
                                        &tp.predicate,
                                        spargebra::term::NamedNodePattern::NamedNode(nn)
                                            if nn.as_str() == RDF_REIFIES
                                    );
                                    let group_end = if is_reifies && i + 1 < drained.len() {
                                        i + 2
                                    } else {
                                        i + 1
                                    };
                                    let group: Vec<TriplePattern> = drained[i..group_end].to_vec();
                                    i = group_end;
                                    current = GraphPattern::LeftJoin {
                                        left: Box::new(current),
                                        right: Box::new(GraphPattern::Bgp { patterns: group }),
                                        expression: None,
                                    };
                                }
                            }
                            current = self.apply_pending_binds(current);
                            current = GraphPattern::Filter {
                                inner: Box::new(current),
                                expr: filter_expr,
                            };
                        }

                        if need_distinct {
                            current = GraphPattern::Distinct {
                                inner: Box::new(current),
                            };
                        }

                        // Project to WITH output variables.
                        // For conflicting renames, project the fresh var.
                        if let Some(ref vars) = project_vars {
                            let mut inner_vars: Vec<Variable> = vars
                                .iter()
                                .map(|v| {
                                    outer_renames
                                        .iter()
                                        .find(|(alias, _)| alias == v)
                                        .map(|(_, fresh)| fresh.clone())
                                        .unwrap_or_else(|| v.clone())
                                })
                                .collect();
                            // Also project eid_var, src, and dst for relationship variables
                            // so that relationship identity comparisons (a = b) work
                            // correctly after the WITH scope boundary.
                            // Uses sameTerm(src_a, src_b) etc., which works for blank nodes.
                            if let crate::ast::cypher::ReturnItems::Explicit(ref wit_items) =
                                w.items
                            {
                                for item in wit_items {
                                    if let Expression::Variable(var_name) = &item.expression {
                                        let alias =
                                            item.alias.as_deref().unwrap_or(var_name.as_str());
                                        // Also project source variables from node_props_from_create
                                        // so that property bindings like n.num = ?x survive the
                                        // WITH projection boundary (skip_writes mode).
                                        if let Some(prop_map) =
                                            self.node_props_from_create.get(alias).cloned()
                                        {
                                            for (_, val_expr) in &prop_map {
                                                fn collect_expr_vars(
                                                    e: &Expression,
                                                    out: &mut Vec<Variable>,
                                                ) {
                                                    match e {
                                                        Expression::Variable(v) => {
                                                            let sv =
                                                                Variable::new_unchecked(v.clone());
                                                            if !out.contains(&sv) {
                                                                out.push(sv);
                                                            }
                                                        }
                                                        Expression::Add(l, r)
                                                        | Expression::Subtract(l, r)
                                                        | Expression::Multiply(l, r)
                                                        | Expression::Divide(l, r)
                                                        | Expression::Modulo(l, r) => {
                                                            collect_expr_vars(l, out);
                                                            collect_expr_vars(r, out);
                                                        }
                                                        Expression::FunctionCall {
                                                            args, ..
                                                        } => {
                                                            for a in args {
                                                                collect_expr_vars(a, out);
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                collect_expr_vars(val_expr, &mut inner_vars);
                                            }
                                        }
                                        if let Some(edge) = self.edge_map.get(alias).cloned() {
                                            // Skip projecting a src/dst variable if it is
                                            // already the SOURCE of one of the extends —
                                            // i.e., it is being renamed to a different
                                            // name.  The rename captures its value in the
                                            // new variable, so projecting the old name here
                                            // would leak the raw SPARQL var into the next
                                            // scope and incorrectly constrain subsequent
                                            // MATCH patterns.
                                            let renamed_away = |v: &Variable| {
                                                extends.iter().any(|(_, expr)| {
                                                    matches!(expr, SparExpr::Variable(src) if src == v)
                                                })
                                            };
                                            // Project src variable
                                            if let TermPattern::Variable(sv) = &edge.src {
                                                if !inner_vars.contains(sv) && !renamed_away(sv) {
                                                    inner_vars.push(sv.clone());
                                                }
                                            }
                                            // Project dst variable
                                            if let TermPattern::Variable(dv) = &edge.dst {
                                                if !inner_vars.contains(dv) && !renamed_away(dv) {
                                                    inner_vars.push(dv.clone());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            // Pre-translate ORDER BY edge-property accesses BEFORE the WITH
                            // sub-SELECT so that RDF-star reification triples are inside the
                            // sub-SELECT where src/dst variables are in scope.
                            // The result variable is added to inner_vars so it's projected,
                            // and stored in with_prop_subst for apply_order_skip_limit to reuse.
                            let original_inner_vars = inner_vars.clone();
                            if w.order_by.is_some() {
                                if let Some(ob) = w.order_by.as_ref() {
                                    for sort_item in &ob.items {
                                        if let Expression::Property(base_expr, key) =
                                            &sort_item.expression
                                        {
                                            if let Expression::Variable(var_name) =
                                                base_expr.as_ref()
                                            {
                                                if self.edge_map.contains_key(var_name.as_str())
                                                    && !self.with_prop_subst.contains_key(&(
                                                        var_name.clone(),
                                                        key.clone(),
                                                    ))
                                                {
                                                    let mut ob_extra = Vec::new();
                                                    match self.translate_expr(
                                                        &sort_item.expression,
                                                        &mut ob_extra,
                                                    ) {
                                                        Ok(SparExpr::Variable(prop_v)) => {
                                                            for tp in ob_extra {
                                                                current = GraphPattern::LeftJoin {
                                                                    left: Box::new(current),
                                                                    right: Box::new(
                                                                        GraphPattern::Bgp {
                                                                            patterns: vec![tp],
                                                                        },
                                                                    ),
                                                                    expression: None,
                                                                };
                                                            }
                                                            self.with_prop_subst.insert(
                                                                (var_name.clone(), key.clone()),
                                                                prop_v.clone(),
                                                            );
                                                            inner_vars.push(prop_v);
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            let with_needs_outer_project =
                                inner_vars.len() > original_inner_vars.len();
                            // For any sort-by-list-variable in ORDER BY, add the sort key
                            // variable to the inner projection so ORDER BY (which runs outside
                            // the sub-SELECT) can reference it. The outer Project below will
                            // hide these extra variables from subsequent clauses.
                            if let Some(ob) = w.order_by.as_ref() {
                                for sort_item in &ob.items {
                                    if let Expression::Variable(v) = &sort_item.expression {
                                        if let Some(sk_name) =
                                            self.list_sort_key_vars.get(v.as_str()).cloned()
                                        {
                                            let sk_var = Variable::new_unchecked(sk_name);
                                            if !inner_vars.contains(&sk_var) {
                                                inner_vars.push(sk_var);
                                            }
                                        }
                                    }
                                }
                            }
                            let with_needs_outer_project = with_needs_outer_project
                                || inner_vars.len() > original_inner_vars.len();
                            current = GraphPattern::Project {
                                inner: Box::new(current),
                                variables: inner_vars,
                            };
                            // Apply outer renames after the sub-select.
                            for (alias, fresh) in &outer_renames {
                                current = GraphPattern::Extend {
                                    inner: Box::new(current),
                                    variable: alias.clone(),
                                    expression: SparExpr::Variable(fresh.clone()),
                                };
                            }
                            // Build property substitutions for ORDER BY expressions:
                            // properties that were already projected in this WITH can be
                            // looked up by their output variable instead of re-fetching them.
                            if w.order_by.is_some() {
                                if let crate::ast::cypher::ReturnItems::Explicit(ref ti) =
                                    as_return.items
                                {
                                    for (item, pvar) in ti.iter().zip(vars.iter()) {
                                        if let Expression::Property(base, key) = &item.expression {
                                            if let Expression::Variable(base_var) = base.as_ref() {
                                                self.with_prop_subst.insert(
                                                    (base_var.clone(), key.clone()),
                                                    pvar.clone(),
                                                );
                                            }
                                        }
                                        // Also map aggregate expressions so ORDER BY can reuse them.
                                        if let Expression::Aggregate(agg_expr) = &item.expression {
                                            let key = agg_expr_key(agg_expr);
                                            self.agg_orderby_subst.insert(key, pvar.clone());
                                        }
                                    }
                                }
                            }
                            // Collect extra_triples count before ORDER BY.
                            let extra_before_ob = extra_triples.len();
                            // Apply ORDER BY / SKIP / LIMIT from WITH clause.
                            current = self.apply_order_skip_limit(
                                current,
                                w.order_by.as_ref(),
                                w.skip.as_ref(),
                                w.limit.as_ref(),
                                &mut extra_triples,
                            )?;
                            // Flush any property-access triples from ORDER BY as OPTIONAL
                            // LeftJoins. This handles `WITH a ORDER BY a.x LIMIT n`.
                            let ob_triples: Vec<TriplePattern> =
                                extra_triples.drain(extra_before_ob..).collect();
                            for tp in ob_triples {
                                current = GraphPattern::LeftJoin {
                                    left: Box::new(current),
                                    right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                    expression: None,
                                };
                            }
                            // Clear ORDER BY property/aggregate substitutions.
                            self.with_prop_subst.clear();
                            self.agg_orderby_subst.clear();
                            // If ORDER BY edge-property vars were added to the inner sub-SELECT,
                            // wrap with an outer Project to hide them from the WITH output scope.
                            if with_needs_outer_project {
                                current = GraphPattern::Project {
                                    inner: Box::new(current),
                                    variables: original_inner_vars,
                                };
                            }
                        } else {
                            current = GraphPattern::Project {
                                inner: Box::new(current),
                                variables: Vec::new(), // no vars — filtered by outer scope
                            };
                            // Apply outer renames after the sub-select.
                            for (alias, fresh) in &outer_renames {
                                current = GraphPattern::Extend {
                                    inner: Box::new(current),
                                    variable: alias.clone(),
                                    expression: SparExpr::Variable(fresh.clone()),
                                };
                            }
                            // Collect extra_triples count before ORDER BY.
                            let extra_before_ob = extra_triples.len();
                            current = self.apply_order_skip_limit(
                                current,
                                w.order_by.as_ref(),
                                w.skip.as_ref(),
                                w.limit.as_ref(),
                                &mut extra_triples,
                            )?;
                            let ob_triples: Vec<TriplePattern> =
                                extra_triples.drain(extra_before_ob..).collect();
                            for tp in ob_triples {
                                current = GraphPattern::LeftJoin {
                                    left: Box::new(current),
                                    right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                    expression: None,
                                };
                            }
                            self.with_prop_subst.clear();
                            self.agg_orderby_subst.clear();
                        }
                    }
                    // NOTE: WITH's WHERE clause is now handled BEFORE the projection above,
                    // so old-scope variables are still accessible. No action needed here.
                    // Update nullable_vars based on WITH output items.
                    {
                        let mut new_nullable: std::collections::HashSet<String> =
                            Default::default();
                        let mut with_vars: Vec<Variable> = Vec::new();
                        if let crate::ast::cypher::ReturnItems::Explicit(ref items) = w.items {
                            for item in items {
                                let alias = match &item.alias {
                                    Some(a) => a.clone(),
                                    None => {
                                        if let Expression::Variable(v) = &item.expression {
                                            v.clone()
                                        } else {
                                            continue;
                                        }
                                    }
                                };
                                with_vars.push(Variable::new_unchecked(alias.clone()));
                                if expr_uses_nullable(&item.expression, &self.nullable_vars) {
                                    new_nullable.insert(alias);
                                }
                            }
                        }
                        self.nullable_vars = new_nullable;
                        last_with_vars = if with_vars.is_empty() {
                            None
                        } else {
                            Some(with_vars.clone())
                        };
                        // Remove from unwind_null_vars any variable projected away by this WITH.
                        // After WITH, only the output aliases are in scope; applying
                        // FILTER(BOUND(?old_unwind_var)) after projection would always
                        // evaluate to false (the var is no longer bound), dropping all rows.
                        {
                            let with_output_names: std::collections::HashSet<&str> =
                                with_vars.iter().map(|v| v.as_str()).collect();
                            self.unwind_null_vars
                                .retain(|v| with_output_names.contains(v.as_str()));
                        }
                        // Track literal-list-valued WITH items for compile-time
                        // UNWIND expansion.  Save the current map BEFORE clearing so
                        // pass-through variable aliases can re-register their bindings.
                        let prev_list_vars = std::mem::take(&mut self.with_list_vars);
                        // Also clear the double-UNWIND source tracking — WITH creates a new scope.
                        self.unwind_list_source.clear();
                        // Save and reset null_vars for this new WITH scope.
                        let prev_null_vars = std::mem::take(&mut self.null_vars);
                        // Save and reset const_int_vars for this new WITH scope.
                        let prev_const_ints = std::mem::take(&mut self.const_int_vars);
                        // Save and reset with_lit_vars for this new WITH scope.
                        let prev_lit_vars = std::mem::take(&mut self.with_lit_vars);
                        if let crate::ast::cypher::ReturnItems::Explicit(ref items) = w.items {
                            for item in items {
                                let alias = match &item.alias {
                                    Some(a) => a.clone(),
                                    None => {
                                        if let Expression::Variable(v) = &item.expression {
                                            v.clone()
                                        } else {
                                            continue;
                                        }
                                    }
                                };
                                match &item.expression {
                                    Expression::List(_) => {
                                        self.with_list_vars.insert(alias, item.expression.clone());
                                    }
                                    Expression::Literal(crate::ast::cypher::Literal::Null) => {
                                        // WITH null AS x → x is known to be null
                                        self.null_vars.insert(alias);
                                    }
                                    Expression::Literal(crate::ast::cypher::Literal::Integer(
                                        n,
                                    )) => {
                                        // WITH <integer> AS x → x is a compile-time integer
                                        self.const_int_vars.insert(alias, *n);
                                    }
                                    Expression::Variable(v) => {
                                        // Look up from the SAVED pre-clear map so pass-through
                                        // aliases like `WITH inputList, ...` keep their binding.
                                        if let Some(existing) =
                                            prev_list_vars.get(v.as_str()).cloned()
                                        {
                                            self.with_list_vars.insert(alias.clone(), existing);
                                        }
                                        // Propagate null-var status for pass-through aliases.
                                        if prev_null_vars.contains(v.as_str()) {
                                            self.null_vars.insert(alias.clone());
                                        }
                                        // Propagate const-int status for pass-through aliases.
                                        if let Some(n) = prev_const_ints.get(v.as_str()).copied() {
                                            self.const_int_vars.insert(alias.clone(), n);
                                        }
                                        // Propagate lit-var status for pass-through aliases.
                                        if let Some(s) = prev_lit_vars.get(v.as_str()).cloned() {
                                            self.with_lit_vars.insert(alias, s);
                                        }
                                    }
                                    Expression::FunctionCall { name, args, .. }
                                        if name.eq_ignore_ascii_case("size") =>
                                    {
                                        // size(literal_list) → compile-time integer
                                        if let Some(arg) = args.first() {
                                            let count_opt =
                                                count_list_elements(arg).or_else(|| {
                                                    if let Expression::Variable(v) = arg {
                                                        prev_list_vars
                                                            .get(v.as_str())
                                                            .and_then(count_list_elements)
                                                    } else {
                                                        None
                                                    }
                                                });
                                            if let Some(count) = count_opt {
                                                self.const_int_vars.insert(alias, count as i64);
                                            }
                                        }
                                    }
                                    Expression::FunctionCall {
                                        name: fname,
                                        args: fargs,
                                        ..
                                    } => {
                                        // Evaluate temporal constructors to compile-time string
                                        // values so that `date(toString(alias))` can be folded.
                                        let lower = fname.to_lowercase();
                                        let base = lower
                                            .strip_suffix(".transaction")
                                            .or_else(|| lower.strip_suffix(".statement"))
                                            .or_else(|| lower.strip_suffix(".realtime"))
                                            .unwrap_or(lower.as_str());
                                        let lit_opt: Option<String> = match fargs.first() {
                                            Some(Expression::Map(pairs)) => match base {
                                                "date" => temporal_date_from_map(pairs),
                                                "localtime" => temporal_localtime_from_map(pairs),
                                                "time" => temporal_time_from_map(pairs),
                                                "localdatetime" => {
                                                    temporal_localdatetime_from_map(pairs)
                                                }
                                                "datetime" => temporal_datetime_from_map(pairs),
                                                "duration" => temporal_duration_from_map(pairs),
                                                _ => None,
                                            },
                                            Some(Expression::Literal(
                                                crate::ast::cypher::Literal::String(s),
                                            )) => match base {
                                                "date" => temporal_parse_date(s),
                                                "localtime" => temporal_parse_localtime(s),
                                                "time" => temporal_parse_time(s),
                                                "localdatetime" => temporal_parse_localdatetime(s),
                                                "datetime" => temporal_parse_datetime(s),
                                                "duration" => temporal_parse_duration(s),
                                                _ => None,
                                            },
                                            _ => None,
                                        };
                                        // Also evaluate duration.between/inMonths/inDays/inSeconds
                                        // when both arguments are WITH-bound literals.
                                        let lit_opt = lit_opt.or_else(|| {
                                            if base == "duration.between"
                                                || base == "duration.inmonths"
                                                || base == "duration.indays"
                                                || base == "duration.inseconds"
                                            {
                                                if fargs.len() >= 2 {
                                                    // Helper to get a literal string from a temporal expression.
                                                    let eval_temporal = |e: &Expression| -> Option<String> {
                                                        match e {
                                                            Expression::Literal(crate::ast::cypher::Literal::String(s)) => Some(s.clone()),
                                                            Expression::Variable(v) => {
                                                                prev_lit_vars.get(v.as_str())
                                                                    .or_else(|| self.with_lit_vars.get(v.as_str()))
                                                                    .cloned()
                                                            }
                                                            Expression::FunctionCall { name: fn_name, args: fn_args, .. } => {
                                                                let fn_lower = fn_name.to_lowercase();
                                                                match (fn_lower.as_str(), fn_args.first()) {
                                                                    ("localdatetime", Some(Expression::Literal(crate::ast::cypher::Literal::String(s)))) => temporal_parse_localdatetime(s),
                                                                    ("localdatetime", Some(Expression::Map(pairs))) => temporal_localdatetime_from_map(pairs),
                                                                    ("datetime", Some(Expression::Literal(crate::ast::cypher::Literal::String(s)))) => temporal_parse_datetime(s),
                                                                    ("datetime", Some(Expression::Map(pairs))) => temporal_datetime_from_map(pairs),
                                                                    ("date", Some(Expression::Literal(crate::ast::cypher::Literal::String(s)))) => temporal_parse_date(s),
                                                                    ("date", Some(Expression::Map(pairs))) => temporal_date_from_map(pairs),
                                                                    ("time", Some(Expression::Literal(crate::ast::cypher::Literal::String(s)))) => temporal_parse_time(s),
                                                                    ("time", Some(Expression::Map(pairs))) => temporal_time_from_map(pairs),
                                                                    ("localtime", Some(Expression::Literal(crate::ast::cypher::Literal::String(s)))) => temporal_parse_localtime(s),
                                                                    ("localtime", Some(Expression::Map(pairs))) => temporal_localtime_from_map(pairs),
                                                                    _ => None,
                                                                }
                                                            }
                                                            _ => None,
                                                        }
                                                    };
                                                    let lhs = eval_temporal(&fargs[0])?;
                                                    let rhs = eval_temporal(&fargs[1])?;
                                                    match base {
                                                        "duration.between" => temporal_duration_between(&lhs, &rhs),
                                                        "duration.inmonths" => temporal_duration_in_months(&lhs, &rhs),
                                                        "duration.indays" => temporal_duration_in_days(&lhs, &rhs),
                                                        "duration.inseconds" => temporal_duration_in_seconds(&lhs, &rhs),
                                                        _ => None,
                                                    }
                                                } else { None }
                                            } else { None }
                                        });
                                        if let Some(s) = lit_opt {
                                            self.with_lit_vars.insert(alias, s);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Clause::Return(r) => {
                    // Flush pending extra triples before projection.
                    if !extra_triples.is_empty() {
                        let extra = GraphPattern::Bgp {
                            patterns: std::mem::take(&mut extra_triples),
                        };
                        current = join_patterns(current, extra);
                    }
                    // Flush pending filters.
                    current = apply_filters(current, pending_filters.drain(..));

                    let (
                        return_triples,
                        project_vars,
                        need_distinct,
                        aggregates,
                        extends,
                        post_extends,
                    ) = self.translate_return_clause(r, &mut extra_triples)?;

                    if !return_triples.is_empty() {
                        // Property-access triples in RETURN must be OPTIONAL so that
                        // missing properties project as null rather than filtering rows.
                        // If the triple's subject is a nullable variable (from OPTIONAL MATCH),
                        // wrap in a sub-SELECT with FILTER(BOUND(?subj)) to prevent the
                        // unbound variable from acting as a wildcard when unbound.
                        //
                        // Relationship properties in RDF-star mode produce TWO consecutive
                        // triples with the same fresh subject (reif_var):
                        //   1. ?reif rdf:reifies << src pred dst >>
                        //   2. ?reif <prop> ?result
                        // These must be wrapped in ONE OPTIONAL block together, otherwise the
                        // second triple's ?reif is unbound and matches any subject with that property.
                        // Detect this case by checking if the first triple's predicate is rdf:reifies.
                        const RDF_REIFIES: &str =
                            "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
                        let mut i = 0;
                        while i < return_triples.len() {
                            let tp = return_triples[i].clone();
                            // Group rdf:reifies triple with its following property-access triple.
                            let is_reifies = matches!(
                                &tp.predicate,
                                spargebra::term::NamedNodePattern::NamedNode(nn) if nn.as_str() == RDF_REIFIES
                            );
                            let group_end = if is_reifies && i + 1 < return_triples.len() {
                                i + 2
                            } else {
                                i + 1
                            };
                            let group: Vec<TriplePattern> = return_triples[i..group_end].to_vec();
                            i = group_end;
                            // Determine the subject for nullable check (use first triple's subject).
                            let right = if let TermPattern::Variable(subj_var) =
                                &group[0].subject.clone()
                            {
                                if self.nullable_vars.contains(subj_var.as_str()) {
                                    // Use type-guard approach: OPTIONAL { ?n rdf:type X . ?n <prop> ?val }
                                    // The guard triples capture the type constraints from the OPTIONAL MATCH,
                                    // preventing wildcard matching when no X-typed nodes exist.
                                    let guards = self
                                        .nullable_type_guards
                                        .get(subj_var.as_str())
                                        .cloned()
                                        .unwrap_or_default();
                                    nullable_subject_optional(
                                        group[0].clone(),
                                        subj_var.clone(),
                                        guards,
                                    )
                                } else {
                                    GraphPattern::Bgp { patterns: group }
                                }
                            } else {
                                GraphPattern::Bgp { patterns: group }
                            };
                            current = GraphPattern::LeftJoin {
                                left: Box::new(current),
                                right: Box::new(right),
                                expression: None,
                            };
                        }
                    }
                    // Flush any triples added during return expression translation.
                    // These are property-access triples from expressions like `n.prop`
                    // inside aggregates.  They must be OPTIONAL to avoid filtering
                    // rows where the property doesn't exist (e.g. AVG(n.age) → null).
                    // If the triple's subject is nullable, add FILTER(BOUND) inside the OPTIONAL.
                    // Group rdf:reifies triples with their following property-access triple
                    // into a single OPTIONAL block to avoid unbinding the reification var.
                    if !extra_triples.is_empty() {
                        const RDF_REIFIES: &str =
                            "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
                        let drained: Vec<TriplePattern> = extra_triples.drain(..).collect();
                        let mut i = 0;
                        while i < drained.len() {
                            let tp = drained[i].clone();
                            let is_reifies = matches!(
                                &tp.predicate,
                                spargebra::term::NamedNodePattern::NamedNode(nn) if nn.as_str() == RDF_REIFIES
                            );
                            let group_end = if is_reifies && i + 1 < drained.len() {
                                i + 2
                            } else {
                                i + 1
                            };
                            let group: Vec<TriplePattern> = drained[i..group_end].to_vec();
                            i = group_end;
                            let right = if let TermPattern::Variable(subj_var) =
                                &group[0].subject.clone()
                            {
                                if self.nullable_vars.contains(subj_var.as_str()) {
                                    let guards = self
                                        .nullable_type_guards
                                        .get(subj_var.as_str())
                                        .cloned()
                                        .unwrap_or_default();
                                    nullable_subject_optional(
                                        group[0].clone(),
                                        subj_var.clone(),
                                        guards,
                                    )
                                } else {
                                    GraphPattern::Bgp { patterns: group }
                                }
                            } else {
                                GraphPattern::Bgp { patterns: group }
                            };
                            current = GraphPattern::LeftJoin {
                                left: Box::new(current),
                                right: Box::new(right),
                                expression: None,
                            };
                        }
                    }

                    // Apply pre-group Extend bindings (non-aggregate expression aliases).
                    // First apply any pending BIND extends from IsNull/IsNotNull on complex exprs.
                    current = self.apply_pending_binds(current);
                    // Join in any correlated subqueries from pattern comprehensions.
                    current = self.drain_pending_subqueries(current);
                    for (var, expr) in extends {
                        current = GraphPattern::Extend {
                            inner: Box::new(current),
                            variable: var,
                            expression: expr,
                        };
                    }

                    // Apply aggregation (GROUP BY) if present.
                    if !aggregates.is_empty() {
                        // Work around oxigraph bug: SUM/MIN/MAX/AVG over input that
                        // contains UNDEF values returns null instead of ignoring them.
                        // Fix: add FILTER(BOUND(?v)) for any variable used inside a
                        // SUM/AVG/MIN/MAX aggregate (null-sensitive).
                        for (_, agg_expr) in &aggregates {
                            if let AggregateExpression::FunctionCall {
                                name:
                                    AggregateFunction::Sum
                                    | AggregateFunction::Avg
                                    | AggregateFunction::Min
                                    | AggregateFunction::Max,
                                expr: SparExpr::Variable(inner_var),
                                ..
                            } = agg_expr
                            {
                                // Only add BOUND filter if not already handled via unwind_null_vars.
                                if !self.unwind_null_vars.contains(inner_var.as_str()) {
                                    current = GraphPattern::Filter {
                                        inner: Box::new(current),
                                        expr: SparExpr::Bound(inner_var.clone()),
                                    };
                                }
                            }
                        }
                        // Work around oxigraph bug: MAX/MIN/SUM over VALUES with UNDEF
                        // returns null. Add FILTER(BOUND(?v)) for UNWIND null vars.
                        // Note: non-DISTINCT GROUP_CONCAT naturally ignores UNDEF.
                        // DISTINCT GROUP_CONCAT doesn't handle UNDEF correctly when mixed
                        // with non-null values — add FILTER for mixed-null DISTINCT collects.
                        // All-null DISTINCT collect: no FILTER needed (GROUP_CONCAT(DISTINCT empty)=[] works).
                        let only_nondistinct_groupconcat = aggregates.iter().all(|(_, ae)| {
                            matches!(
                                ae,
                                AggregateExpression::FunctionCall {
                                    name: AggregateFunction::GroupConcat { .. },
                                    distinct: false,
                                    ..
                                }
                            )
                        });
                        if !only_nondistinct_groupconcat && !self.unwind_null_vars.is_empty() {
                            // For DISTINCT GROUP_CONCAT with mixed null/non-null: use FILTER.
                            // For other aggregates (SUM/MIN/MAX): always use FILTER.
                            // For all-null: skip FILTER (FILTER would cause 0-row GROUP → 0 results).
                            let has_distinct_gc = aggregates.iter().any(|(_, ae)| {
                                matches!(
                                    ae,
                                    AggregateExpression::FunctionCall {
                                        name: AggregateFunction::GroupConcat { .. },
                                        distinct: true,
                                        ..
                                    }
                                )
                            });
                            for null_var_name in self.unwind_null_vars.clone() {
                                let is_pure_gc = has_distinct_gc
                                    && !aggregates.iter().any(|(_, ae)| {
                                        !matches!(
                                            ae,
                                            AggregateExpression::FunctionCall {
                                                name: AggregateFunction::GroupConcat { .. },
                                                ..
                                            }
                                        )
                                    });
                                // Only add FILTER for mixed-null vars (not all-null).
                                let should_filter = if is_pure_gc {
                                    self.unwind_mixed_null_vars.contains(&null_var_name)
                                } else {
                                    true
                                };
                                if should_filter {
                                    let bound_var = Variable::new_unchecked(null_var_name);
                                    current = GraphPattern::Filter {
                                        inner: Box::new(current),
                                        expr: SparExpr::Bound(bound_var),
                                    };
                                }
                            }
                        }
                        // Group variables = all projected non-aggregate vars that are
                        // not targets of post-group extends either.
                        let group_vars: Vec<Variable> = project_vars
                            .as_ref()
                            .map(|vs| {
                                vs.iter()
                                    .filter(|v| !aggregates.iter().any(|(av, _)| av == *v))
                                    .filter(|v| !post_extends.iter().any(|(pv, _)| pv == *v))
                                    .cloned()
                                    .collect()
                            })
                            .unwrap_or_default();
                        current = GraphPattern::Group {
                            inner: Box::new(current),
                            variables: group_vars,
                            aggregates,
                        };
                    }

                    // Apply post-group Extend bindings (aggregate-in-expression aliases).
                    for (var, expr) in post_extends {
                        current = GraphPattern::Extend {
                            inner: Box::new(current),
                            variable: var,
                            expression: expr,
                        };
                    }

                    // Track vars for outer Project wrap (needed when ORDER BY uses edge props).
                    let mut needs_outer_project: Option<Vec<Variable>> = None;
                    if let Some(mut vars) = project_vars {
                        // Build property and aggregate substitutions for ORDER BY before projection.
                        if r.order_by.is_some() {
                            if let crate::ast::cypher::ReturnItems::Explicit(ref items) = r.items {
                                for (item, pvar) in items.iter().zip(vars.iter()) {
                                    if let Expression::Property(base, key) = &item.expression {
                                        if let Expression::Variable(base_var) = base.as_ref() {
                                            self.with_prop_subst.insert(
                                                (base_var.clone(), key.clone()),
                                                pvar.clone(),
                                            );
                                        }
                                    }
                                    // Also map aggregate expressions so ORDER BY can reuse them.
                                    if let Expression::Aggregate(agg_expr) = &item.expression {
                                        let key = agg_expr_key(agg_expr);
                                        self.agg_orderby_subst.insert(key, pvar.clone());
                                    }
                                    // Map the RETURN alias → projected variable so ORDER BY
                                    // expressions referencing the alias use the correct var.
                                    // E.g. `RETURN n.num AS n ORDER BY n + 2`: `n` in ORDER BY
                                    // refers to the alias `n` → `?__n_num_0`, not node `?n`.
                                    if let Some(alias) = &item.alias {
                                        if pvar.as_str() != alias.as_str() {
                                            self.return_alias_subst
                                                .insert(alias.clone(), pvar.clone());
                                        }
                                    }
                                }
                            }
                        }
                        // Pre-translate ORDER BY edge-property accesses BEFORE the Project
                        // so that RDF-star reification triples (?reif rdf:reifies << src pred dst >>)
                        // are emitted inside the sub-SELECT where ?src and ?dst are still in scope.
                        // The resulting variable is stored in with_prop_subst so that
                        // apply_order_skip_limit reuses it without re-emitting extra triples.
                        let original_vars = vars.clone();
                        if let Some(ob) = r.order_by.as_ref() {
                            for sort_item in &ob.items {
                                if let Expression::Property(base_expr, key) = &sort_item.expression
                                {
                                    if let Expression::Variable(var_name) = base_expr.as_ref() {
                                        if self.edge_map.contains_key(var_name.as_str())
                                            && !self
                                                .with_prop_subst
                                                .contains_key(&(var_name.clone(), key.clone()))
                                        {
                                            let mut ob_extra = Vec::new();
                                            match self.translate_expr(
                                                &sort_item.expression,
                                                &mut ob_extra,
                                            ) {
                                                Ok(SparExpr::Variable(prop_v)) => {
                                                    for tp in ob_extra {
                                                        current = GraphPattern::LeftJoin {
                                                            left: Box::new(current),
                                                            right: Box::new(GraphPattern::Bgp {
                                                                patterns: vec![tp],
                                                            }),
                                                            expression: None,
                                                        };
                                                    }
                                                    self.with_prop_subst.insert(
                                                        (var_name.clone(), key.clone()),
                                                        prop_v.clone(),
                                                    );
                                                    vars.push(prop_v);
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if vars.len() > original_vars.len() {
                            needs_outer_project = Some(original_vars);
                        }
                        current = GraphPattern::Project {
                            inner: Box::new(current),
                            variables: vars,
                        };
                    }
                    if need_distinct {
                        current = GraphPattern::Distinct {
                            inner: Box::new(current),
                        };
                    }
                    // Apply ORDER BY / SKIP / LIMIT from RETURN clause.
                    current = self.apply_order_skip_limit(
                        current,
                        r.order_by.as_ref(),
                        r.skip.as_ref(),
                        r.limit.as_ref(),
                        &mut extra_triples,
                    )?;
                    // Clear ORDER BY property/aggregate substitutions.
                    self.with_prop_subst.clear();
                    self.agg_orderby_subst.clear();
                    self.return_alias_subst.clear();
                    // If ORDER BY edge-property vars were added to the inner Project,
                    // wrap with an outer Project to hide them from the final result set.
                    if let Some(outer_vars) = needs_outer_project {
                        current = GraphPattern::Project {
                            inner: Box::new(current),
                            variables: outer_vars,
                        };
                    }
                }
                Clause::Unwind(u) => {
                    // UNWIND expr AS var → VALUES ?var { values... }
                    // For list literals we expand inline; for general expressions
                    // we emit a SPARQL VALUES with the computed list.
                    // Phase 4: handle literal list UNWIND directly; complex UNWIND
                    // returns an UnsupportedFeature for non-list expressions.
                    current = self.translate_unwind_clause(u, current, &mut extra_triples)?;
                }
                Clause::Create(c) => {
                    // CREATE is a write clause. We track the pattern into the
                    // BGP so that subsequent clauses can reference created vars.
                    // The actual insertion is engine-level; here we emit the BGP
                    // shape only and flag via PolygraphError::UnsupportedFeature
                    // for now (SPARQL Update requires a separate spargebra::Update).
                    if !self.skip_write_clauses {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: format!(
                                "CREATE clause (SPARQL Update, Phase 4+): {} pattern(s)",
                                c.pattern.0.len()
                            ),
                        });
                    }
                    // Skip: caller handles CREATE as INSERT DATA separately.
                    // Register any named node/relationship variables from the CREATE pattern
                    // so that subsequent RETURN clauses can reference them.
                    for pat in &c.pattern.0 {
                        for elem in &pat.elements {
                            match elem {
                                PatternElement::Node(n) => {
                                    if let Some(v) = &n.variable {
                                        self.node_vars.insert(v.clone());
                                        // Record labels.
                                        let labels: Vec<String> =
                                            n.labels.iter().map(|l| l.clone()).collect();
                                        self.node_labels_from_create.insert(v.clone(), labels);
                                        // Record properties.
                                        if let Some(props) = &n.properties {
                                            let prop_map: std::collections::HashMap<
                                                String,
                                                Expression,
                                            > = props
                                                .iter()
                                                .map(|(k, v)| (k.clone(), v.clone()))
                                                .collect();
                                            // Use entry to avoid overwriting pre-scan folded values
                                            // (e.g. SET a.prop = a.prop + [4,5] already folded).
                                            self.node_props_from_create
                                                .entry(v.clone())
                                                .or_insert(prop_map);
                                        }
                                    }
                                }
                                PatternElement::Relationship(r) => {
                                    if let Some(v) = &r.variable {
                                        // Record relationship properties.
                                        if let Some(props) = &r.properties {
                                            let prop_map: std::collections::HashMap<
                                                String,
                                                Expression,
                                            > = props
                                                .iter()
                                                .map(|(k, vv)| (k.clone(), vv.clone()))
                                                .collect();
                                            self.node_props_from_create.insert(v.clone(), prop_map);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Clause::Merge(m) => {
                    // Simple MERGE (b) with no labels/properties on the pattern node
                    // can be translated as MATCH (b) ONLY in skip_writes mode (where the
                    // write_clauses_to_updates has already inserted the node if needed).
                    // In non-skip-writes mode, always return a write-clause error so the
                    // caller knows to invoke write_clauses_to_updates + skip_writes.
                    let is_simple_node_merge =
                        self.skip_write_clauses && m.pattern.elements.len() == 1 && {
                            if let PatternElement::Node(node) = &m.pattern.elements[0] {
                                node.labels.is_empty() && node.properties.is_none()
                            } else {
                                false
                            }
                        };
                    if is_simple_node_merge {
                        let match_clause = MatchClause {
                            optional: false,
                            pattern: crate::ast::cypher::PatternList(vec![m.pattern.clone()]),
                            where_: None,
                        };
                        let (match_pattern, opt_filter, where_extra) =
                            self.translate_match_clause(&match_clause, &mut extra_triples)?;
                        current = join_patterns(current, match_pattern);
                        for tp in where_extra {
                            current = GraphPattern::LeftJoin {
                                left: Box::new(current),
                                right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                expression: None,
                            };
                        }
                        if let Some(f) = opt_filter {
                            pending_filters.push(f);
                        }
                    } else if self.skip_write_clauses && m.pattern.elements.len() == 1 {
                        // For skip_writes: MERGE with labels/props on a single node →
                        // translate as MATCH so the re-translated SELECT finds the node
                        // that write_clauses_to_updates will have INSERTed if missing.
                        let match_clause = MatchClause {
                            optional: false,
                            pattern: crate::ast::cypher::PatternList(vec![m.pattern.clone()]),
                            where_: None,
                        };
                        let (match_pattern, opt_filter2, where_extra) =
                            self.translate_match_clause(&match_clause, &mut extra_triples)?;
                        // If the MERGE node has no variable (anonymous), it doesn't bind
                        // any result variable — use LEFT JOIN so it doesn't filter rows
                        // when the node hasn't been created yet (or has properties derived
                        // from outer MATCH variables that won't match at SELECT time).
                        let merge_node_has_var = if let PatternElement::Node(node) =
                            &m.pattern.elements[0]
                        {
                            node.variable.is_some()
                        } else {
                            false
                        };
                        if merge_node_has_var {
                            current = join_patterns(current, match_pattern);
                        } else {
                            current = GraphPattern::LeftJoin {
                                left: Box::new(current),
                                right: Box::new(match_pattern),
                                expression: None,
                            };
                        }
                        for tp in where_extra {
                            current = GraphPattern::LeftJoin {
                                left: Box::new(current),
                                right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                expression: None,
                            };
                        }
                        if let Some(f) = opt_filter2 {
                            pending_filters.push(f);
                        }
                        // Also register the path variable if present.
                        if let Some(pv) = &m.pattern.variable {
                            self.node_vars.insert(pv.clone());
                        }
                        // Register node labels and properties from the MERGE pattern so
                        // RETURN labels(a) / a.prop can resolve statically (skip_writes mode).
                        if let PatternElement::Node(node) = &m.pattern.elements[0] {
                            if let Some(v) = &node.variable {
                                let mut all_labels = node.labels.clone();
                                // Also include ON CREATE SET and ON MATCH SET labels.
                                // After write_clauses_to_updates runs, the graph has the
                                // full set of labels (from both MATCH and CREATE paths)
                                // so we include all of them in the static label list.
                                for action in &m.actions {
                                    for item in &action.items {
                                        if let crate::ast::cypher::SetItem::SetLabel {
                                            labels,
                                            ..
                                        } = item
                                        {
                                            for l in labels {
                                                if !all_labels.contains(l) {
                                                    all_labels.push(l.clone());
                                                }
                                            }
                                        }
                                    }
                                }
                                if !all_labels.is_empty() {
                                    self.node_labels_from_create.insert(v.clone(), all_labels);
                                }
                                if let Some(props) = &node.properties {
                                    let mut prop_map = std::collections::HashMap::new();
                                    for (k, val) in props {
                                        if !expr_references_prop(val, v, k) {
                                            prop_map.insert(k.clone(), val.clone());
                                        }
                                    }
                                    if !prop_map.is_empty() {
                                        self.node_props_from_create.insert(v.clone(), prop_map);
                                    }
                                }
                            }
                        }
                        // Note: ON CREATE SET properties are NOT tracked in node_props_from_create
                        // because in skip_writes mode we cannot statically determine at translation
                        // time whether MERGE will CREATE or MATCH. The write handler (write_clauses_to_updates)
                        // inserts ON CREATE SET properties into the graph only when creating, so the
                        // OPTIONAL BGP will correctly find the value (when created) or return null (when matched).
                    } else {
                        if self.skip_write_clauses {
                            // In skip_writes mode: if the MERGE has a relationship variable,
                            // translate as MATCH so the newly-created edge is found by SELECT.
                            // If no rel variable is referenced (anonymous edges), silently skip
                            // to avoid over-matching (finding all edges of that type).
                            let has_rel_var = m.pattern.elements.iter().any(|e| {
                                if let PatternElement::Relationship(r) = e {
                                    r.variable.is_some()
                                } else {
                                    false
                                }
                            });
                            if has_rel_var {
                                // Augment node patterns with known CREATE properties so the
                                // undirected UNION only matches in the direction the edge was
                                // created (not both traversal directions for the same edge).
                                let mut augmented_elements = m.pattern.elements.clone();
                                for elem in &mut augmented_elements {
                                    if let PatternElement::Node(n) = elem {
                                        if let Some(v) = &n.variable {
                                            if let Some(prop_map) = self
                                                .node_props_from_create
                                                .get(v.as_str())
                                                .cloned()
                                            {
                                                let existing =
                                                    n.properties.get_or_insert_with(Vec::new);
                                                for (k, vv) in prop_map {
                                                    if !existing.iter().any(|(ek, _)| ek == &k) {
                                                        existing.push((k, vv));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                let match_clause = MatchClause {
                                    optional: false,
                                    pattern: crate::ast::cypher::PatternList(vec![
                                        crate::ast::cypher::Pattern {
                                            variable: m.pattern.variable.clone(),
                                            elements: augmented_elements,
                                        },
                                    ]),
                                    where_: None,
                                };
                                // Try to translate as MATCH; fall back to silently skip on error
                                // (e.g. complex list properties in the MERGE pattern).
                                match self.translate_match_clause(&match_clause, &mut extra_triples)
                                {
                                    Ok((match_pattern, opt_filter, where_extra)) => {
                                        current = join_patterns(current, match_pattern);
                                        for tp in where_extra {
                                            current = GraphPattern::LeftJoin {
                                                left: Box::new(current),
                                                right: Box::new(GraphPattern::Bgp {
                                                    patterns: vec![tp],
                                                }),
                                                expression: None,
                                            };
                                        }
                                        if let Some(f) = opt_filter {
                                            pending_filters.push(f);
                                        }
                                    }
                                    Err(_) => {
                                        // Complex expression in MERGE pattern — silently skip.
                                    }
                                }
                            }
                            // Register named variables from the pattern.
                            for elem in &m.pattern.elements {
                                if let PatternElement::Node(n) = elem {
                                    if let Some(v) = &n.variable {
                                        self.node_vars.insert(v.clone());
                                    }
                                }
                            }
                            if let Some(pv) = &m.pattern.variable {
                                self.node_vars.insert(pv.clone());
                            }
                        } else {
                            return Err(PolygraphError::UnsupportedFeature {
                                feature: format!(
                                    "MERGE clause (SPARQL Update, Phase 4+): {}",
                                    m.pattern.variable.as_deref().unwrap_or("anon")
                                ),
                            });
                        }
                    }
                }
                Clause::Set(s) => {
                    if !self.skip_write_clauses {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: format!(
                                "SET clause (SPARQL Update, Phase 4+): {} item(s)",
                                s.items.len()
                            ),
                        });
                    }
                    // Skip: record SET n.prop = val for subsequent RETURN use.
                    for item in &s.items {
                        match item {
                            crate::ast::cypher::SetItem::Property {
                                variable,
                                key,
                                value,
                            } => {
                                // Only track if value doesn't reference the same (variable.key)
                                // to avoid infinite recursion in translate_expr.
                                if !expr_references_prop(value, variable, key) {
                                    self.node_props_from_create
                                        .entry(variable.clone())
                                        .or_default()
                                        .insert(key.clone(), value.clone());
                                }
                            }
                            crate::ast::cypher::SetItem::NodeReplace { variable, value } => {
                                // n = {k: v, ...}: record all map keys (only literal-safe)
                                if let Expression::Map(pairs) = value {
                                    for (k, v) in pairs {
                                        if !expr_references_prop(v, variable, k) {
                                            self.node_props_from_create
                                                .entry(variable.clone())
                                                .or_default()
                                                .insert(k.clone(), v.clone());
                                        }
                                    }
                                }
                            }
                            crate::ast::cypher::SetItem::MergeMap { variable, map } => {
                                for (k, v) in map {
                                    if !expr_references_prop(v, variable, k) {
                                        self.node_props_from_create
                                            .entry(variable.clone())
                                            .or_default()
                                            .insert(k.clone(), v.clone());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Clause::Delete(d) => {
                    // Cases:
                    // 1. No RETURN clause (write-only DELETE/DETACH DELETE): silently skip.
                    //    The write is not executed; result is empty. We cannot statically
                    //    detect ConstraintVerificationFailed for connected-node deletion.
                    // 2. RETURN clause that accesses properties of deleted entities:
                    //    raise DeletedEntityAccess so the TCK harness can assert it.
                    // 3. RETURN clause with only safe (metadata) access: skip delete,
                    //    the SELECT will still produce the correct metadata values.
                    let has_return_clause = clauses.iter().any(|c| matches!(c, Clause::Return(_)));
                    if !has_return_clause {
                        // Write-only: silently skip this DELETE/DETACH DELETE clause.
                        if self.skip_write_clauses {
                            // The write path (write_clauses_to_updates) handles execution;
                            // here we just skip the clause for the SELECT translation.
                        } else {
                            return Err(PolygraphError::UnsupportedFeature {
                                feature: format!(
                                    "{} clause (SPARQL Update, Phase 4+): {} expression(s)",
                                    if d.detach { "DETACH DELETE" } else { "DELETE" },
                                    d.expressions.len()
                                ),
                            });
                        }
                        // (continue to next clause)
                    } else {
                        let deleted_vars: std::collections::HashSet<String> = d
                            .expressions
                            .iter()
                            .filter_map(|e| {
                                if let Expression::Variable(v) = e {
                                    Some(v.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        let return_safe = clauses.iter().any(|c| {
                            if let Clause::Return(ret) = c {
                                if let crate::ast::cypher::ReturnItems::Explicit(ref items) =
                                    ret.items
                                {
                                    // Safe only if no item accesses a property of a deleted var.
                                    !items.iter().any(|item| {
                                        expr_accesses_deleted_prop(&item.expression, &deleted_vars)
                                    })
                                } else {
                                    false // RETURN * would project deleted vars' props — unsafe
                                }
                            } else {
                                false
                            }
                        });
                        if !return_safe {
                            if self.skip_write_clauses {
                                // RETURN accesses a property of a deleted entity — this is a
                                // runtime error in Cypher (DeletedEntityAccess). Propagate an
                                // error so the TCK harness recognises it as a runtime failure.
                                return Err(PolygraphError::Translation {
                                    message: "DeletedEntityAccess: accessing property of deleted entity in RETURN".to_string(),
                                });
                            } else {
                                return Err(PolygraphError::UnsupportedFeature {
                                    feature: format!(
                                        "{} clause (SPARQL Update, Phase 4+): {} expression(s)",
                                        if d.detach { "DETACH DELETE" } else { "DELETE" },
                                        d.expressions.len()
                                    ),
                                });
                            }
                        }
                        // DELETE with safe RETURN (e.g. type(r)): skip the deletion,
                        // the SELECT will still produce the correct metadata values.
                    }
                }
                Clause::Remove(r) => {
                    if !self.skip_write_clauses {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: format!(
                                "REMOVE clause (SPARQL Update, Phase 4+): {} item(s)",
                                r.items.len()
                            ),
                        });
                    }
                    // Skip: caller handles the REMOVE as a SPARQL DELETE separately.
                }
                Clause::Call(c) => {
                    // CALL procedure stubs — emit a warning-level error.
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: format!(
                            "CALL procedure stub: {} (Phase 4+, engine-specific)",
                            c.procedure
                        ),
                    });
                }
                Clause::Union { .. } => {
                    // Should never appear here — translate_query routes UNION queries
                    // to translate_union_query which splits on these markers first.
                    unreachable!("Clause::Union in translate_clause_sequence")
                }
            }
        }

        // Final flush (in case no RETURN clause was present).
        if !extra_triples.is_empty() {
            let extra = GraphPattern::Bgp {
                patterns: extra_triples,
            };
            current = join_patterns(current, extra);
        }
        current = self.apply_pending_binds(current);
        current = apply_filters(current, pending_filters.into_iter());

        Ok(current)
    }
}

// ── Helper for BIND-conflict detection ───────────────────────────────────────

/// Collects the variable names that are in scope at the outermost level of
/// `pattern`.  A `Project` node acts as a scope boundary — only its listed
/// variables are visible to the containing pattern.
fn bound_vars_of_pattern(pattern: &GraphPattern) -> std::collections::HashSet<String> {
    match pattern {
        GraphPattern::Bgp { patterns } => {
            let mut vars = std::collections::HashSet::new();
            for tp in patterns {
                extract_tp_vars_for_scope(&tp.subject, &mut vars);
                extract_tp_vars_for_scope(&tp.object, &mut vars);
            }
            vars
        }
        GraphPattern::Join { left, right } => {
            let mut vars = bound_vars_of_pattern(left);
            vars.extend(bound_vars_of_pattern(right));
            vars
        }
        GraphPattern::LeftJoin { left, .. } | GraphPattern::Minus { left, .. } => {
            bound_vars_of_pattern(left)
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner } => bound_vars_of_pattern(inner),
        GraphPattern::Slice { inner, .. } | GraphPattern::OrderBy { inner, .. } => {
            bound_vars_of_pattern(inner)
        }
        GraphPattern::Extend {
            inner, variable, ..
        } => {
            let mut vars = bound_vars_of_pattern(inner);
            vars.insert(variable.as_str().to_string());
            vars
        }
        GraphPattern::Project { variables, .. } => {
            variables.iter().map(|v| v.as_str().to_string()).collect()
        }
        GraphPattern::Values { variables, .. } => {
            variables.iter().map(|v| v.as_str().to_string()).collect()
        }
        GraphPattern::Group {
            variables,
            aggregates,
            ..
        } => {
            let mut vars: std::collections::HashSet<String> =
                variables.iter().map(|v| v.as_str().to_string()).collect();
            for (v, _) in aggregates {
                vars.insert(v.as_str().to_string());
            }
            vars
        }
        GraphPattern::Union { left, right } => {
            let lv = bound_vars_of_pattern(left);
            let rv = bound_vars_of_pattern(right);
            lv.intersection(&rv).cloned().collect()
        }
        _ => Default::default(),
    }
}

fn extract_tp_vars_for_scope(tp: &TermPattern, vars: &mut std::collections::HashSet<String>) {
    match tp {
        TermPattern::Variable(v) => {
            vars.insert(v.as_str().to_string());
        }
        TermPattern::Triple(inner) => {
            extract_tp_vars_for_scope(&inner.subject, vars);
            extract_tp_vars_for_scope(&inner.object, vars);
        }
        _ => {}
    }
}

/// Attempt to statically fold a self-referential SET list expression.
///
/// For `SET var.key = var.key + rhs_list` (or `lhs_list + var.key`), replace
/// the property access with `current` (the known current value) and evaluate
/// the concatenation.  Returns the folded `Expression::List` on success.
fn fold_set_list_concat(
    expr: &Expression,
    var: &str,
    key: &str,
    current: &Expression,
) -> Option<Expression> {
    match expr {
        Expression::Add(a, b) => {
            let a_resolved = resolve_for_fold(a, var, key, current)?;
            let b_resolved = resolve_for_fold(b, var, key, current)?;
            let mut items_a = match a_resolved {
                Expression::List(v) => v,
                other => vec![other],
            };
            let items_b = match b_resolved {
                Expression::List(v) => v,
                other => vec![other],
            };
            items_a.extend(items_b);
            Some(Expression::List(items_a))
        }
        _ => None,
    }
}

/// Resolve a sub-expression for fold: replace `var.key` with `current`,
/// keep literal lists as-is, keep literal scalars as-is.
fn resolve_for_fold(
    expr: &Expression,
    var: &str,
    key: &str,
    current: &Expression,
) -> Option<Expression> {
    match expr {
        Expression::Property(base, prop_key) => {
            if let Expression::Variable(v) = base.as_ref() {
                if v.as_str() == var && prop_key.as_str() == key {
                    return Some(current.clone());
                }
            }
            None
        }
        Expression::List(_) => Some(expr.clone()),
        Expression::Literal(_) => Some(expr.clone()),
        Expression::Add(a, b) => {
            // Recurse to handle chained concatenation.
            fold_set_list_concat(expr, var, key, current).or_else(|| {
                let a_r = resolve_for_fold(a, var, key, current)?;
                let b_r = resolve_for_fold(b, var, key, current)?;
                let mut items_a = match a_r {
                    Expression::List(v) => v,
                    other => vec![other],
                };
                let items_b = match b_r {
                    Expression::List(v) => v,
                    other => vec![other],
                };
                items_a.extend(items_b);
                Some(Expression::List(items_a))
            })
        }
        _ => None,
    }
}
