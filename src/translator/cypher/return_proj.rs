impl TranslationState {

    // ── RETURN clause ─────────────────────────────────────────────────────────

    /// Returns `(extra_bgp_triples, Some(projected_vars) | None for *, distinct_flag, aggregates, pre_extends, post_extends)`.
    #[allow(clippy::type_complexity)]
    fn translate_return_clause(
        &mut self,
        ret: &ReturnClause,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<
        (
            Vec<TriplePattern>,
            Option<Vec<Variable>>,
            bool,
            Vec<(Variable, AggregateExpression)>,
            Vec<(Variable, SparExpr)>, // pre-group extend bindings (expr AS alias)
            Vec<(Variable, SparExpr)>, // post-group extend bindings (agg-in-expr AS alias)
        ),
        PolygraphError,
    > {
        match &ret.items {
            ReturnItems::All => Ok((vec![], None, ret.distinct, vec![], vec![], vec![])),
            ReturnItems::Explicit(items) => {
                // Check for duplicate column aliases.
                {
                    let mut seen: std::collections::HashSet<String> = Default::default();
                    for item in items {
                        let name = item.alias.clone().unwrap_or_else(|| {
                            if let Expression::Variable(v) = &item.expression {
                                v.clone()
                            } else {
                                String::new()
                            }
                        });
                        if !name.is_empty() && !seen.insert(name.clone()) {
                            return Err(PolygraphError::UnsupportedFeature {
                                feature: format!("duplicate RETURN column name: {name}"),
                            });
                        }
                    }
                }
                let mut triples = Vec::new();
                let mut vars = Vec::new();
                let mut aggregates: Vec<(Variable, AggregateExpression)> = Vec::new();
                let mut extends: Vec<(Variable, SparExpr)> = Vec::new();
                let mut post_extends: Vec<(Variable, SparExpr)> = Vec::new();
                self.projected_columns.clear();
                self.return_distinct = ret.distinct;
                for item in items {
                    let (var, agg_pair_opt, ext_opt) =
                        self.translate_return_item(item, &mut triples, extra)?;

                    // Record projected column for schema.
                    let col_name = item.alias.clone().unwrap_or_else(|| {
                        if let Expression::Variable(v) = &item.expression {
                            v.clone()
                        } else {
                            var.as_str().to_string()
                        }
                    });
                    let col_kind = self.classify_return_item(item, &var);
                    self.projected_columns.push(ProjectedColumn {
                        name: col_name,
                        kind: col_kind,
                    });

                    vars.push(var.clone());
                    // Drain any pre-extends generated during temporal prop translation.
                    // These must come BEFORE the main extend so intermediate variables
                    // are bound first (avoids SPARQL operator precedence serialization issues).
                    if !self.pending_pre_extends.is_empty() {
                        extends.append(&mut self.pending_pre_extends);
                    }
                    if let Some((agg_var, agg)) = agg_pair_opt {
                        aggregates.push((agg_var, agg));
                        if let Some(ext_expr) = ext_opt {
                            // post-group extend: result_var = expr(agg_var)
                            post_extends.push((var, ext_expr));
                        }
                    } else if let Some(ext_expr) = ext_opt {
                        extends.push((var, ext_expr));
                    }
                    // Drain any extra aggregates pushed during item translation
                    // (e.g. the COUNT auxiliary for AVG null-on-empty semantics).
                    if !self.pending_aggs.is_empty() {
                        aggregates.append(&mut self.pending_aggs);
                    }
                }
                Ok((
                    triples,
                    Some(vars),
                    ret.distinct,
                    aggregates,
                    extends,
                    post_extends,
                ))
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn translate_return_item(
        &mut self,
        item: &ReturnItem,
        triples: &mut Vec<TriplePattern>,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<
        (
            Variable,
            Option<(Variable, AggregateExpression)>,
            Option<SparExpr>,
        ),
        PolygraphError,
    > {
        match &item.expression {
            Expression::Variable(name) => {
                // Special case: for untyped edge variables, project the predicate variable
                // rather than the plain edge name (which is unbound in SPARQL).  This allows
                // subsequent MATCH clauses to join on the same predicate variable and thus
                // correctly constrain the edge type to the originally-matched relationship.
                if let Some(edge) = self.edge_map.get(name.as_str()).cloned() {
                    if let Some(pred_var) = edge.pred_var.clone() {
                        let alias_str = item.alias.as_deref().unwrap_or(name.as_str());
                        let expected_pred = format!("{alias_str}_pred");
                        // Register the alias in edge_map so that property accesses
                        // like `rel.id` in ORDER BY resolve via the aliased pred_var.
                        let aliased_pred = Variable::new_unchecked(expected_pred.clone());
                        let new_edge = EdgeInfo {
                            src: edge.src.clone(),
                            pred: edge.pred.clone(),
                            pred_var: Some(aliased_pred),
                            dst: edge.dst.clone(),
                            reif_var: edge.reif_var.clone(),
                            null_check_var: edge.null_check_var.clone(),
                            eid_var: edge.eid_var.clone(),
                            binding_generation: self.with_generation,
                        };
                        self.edge_map.insert(alias_str.to_string(), new_edge);
                        if pred_var.as_str() == expected_pred {
                            // Alias matches the generated pred_var name — project it directly.
                            return Ok((pred_var, None, None));
                        } else {
                            // Different alias: BIND(?old_pred AS ?new_alias_pred).
                            let new_pred = Variable::new_unchecked(expected_pred);
                            return Ok((
                                new_pred.clone(),
                                None,
                                Some(SparExpr::Variable(pred_var)),
                            ));
                        }
                    }
                    // Typed edge (pred is a NamedNode): fall through to default handling.
                }
                let var = Variable::new_unchecked(name.clone());
                if let Some(alias) = &item.alias {
                    if alias == name {
                        Ok((var, None, None))
                    } else {
                        // Different alias: emit BIND(?name AS ?alias).
                        let alias_var = Variable::new_unchecked(alias.clone());
                        Ok((alias_var, None, Some(SparExpr::Variable(var))))
                    }
                } else {
                    Ok((var, None, None))
                }
            }
            Expression::Property(base_expr, key) => {
                // n.prop or r.prop [AS alias] → add BGP triple + projected var.
                // First check compile-time map resolution for cases like (list[n]).key
                if let Some(map_pairs) = self.try_resolve_to_literal_map(base_expr) {
                    let result_var = match &item.alias {
                        Some(alias) => Variable::new_unchecked(alias.clone()),
                        None => self.fresh_var(&format!("__map_{key}")),
                    };
                    if let Some((_, val_expr)) = map_pairs.iter().find(|(k, _)| k == key) {
                        let val_expr2 = val_expr.clone();
                        if matches!(val_expr2, Expression::Literal(Literal::Null)) {
                            // null value → unbound (null) result
                            return Ok((result_var, None, None));
                        }
                        let sparql_expr = self.translate_expr(&val_expr2, &mut Vec::new())?;
                        return Ok((result_var, None, Some(sparql_expr)));
                    } else {
                        // Key not found → null (unbound result)
                        return Ok((result_var, None, None));
                    }
                }
                // If base is a list subscript that resolves to a known variable
                // (e.g. `(list[1]).prop` where `list = [123, n]` → `n.prop`), rewrite.
                if let Expression::Subscript(coll, idx) = base_expr.as_ref() {
                    if let Some(items) = self.resolve_literal_list(coll) {
                        let n_len = items.len() as i64;
                        if let Some(iv) = get_literal_int(idx) {
                            let i = if iv < 0 { n_len + iv } else { iv };
                            if i >= 0 && i < n_len {
                                if let Expression::Variable(v_inner) = &items[i as usize] {
                                    let rewritten_item = ReturnItem {
                                        expression: Expression::Property(
                                            Box::new(Expression::Variable(v_inner.clone())),
                                            key.clone(),
                                        ),
                                        alias: item.alias.clone(),
                                    };
                                    return self.translate_return_item(
                                        &rewritten_item,
                                        triples,
                                        extra,
                                    );
                                }
                            }
                        }
                    }
                }
                // Handle startNode(r).prop and endNode(r).prop by rewriting to the
                // underlying node variable's property access.
                if let Expression::FunctionCall { name: fn_name, args: fn_args, .. } =
                    base_expr.as_ref()
                {
                    let fn_lc = fn_name.to_ascii_lowercase();
                    if (fn_lc == "startnode" || fn_lc == "endnode") && fn_args.len() == 1 {
                        if let Some(Expression::Variable(rel_var)) = fn_args.first() {
                            if let Some(edge) = self.edge_map.get(rel_var.as_str()).cloned() {
                                let node_term =
                                    if fn_lc == "startnode" { &edge.src } else { &edge.dst };
                                if let TermPattern::Variable(node_var) = node_term {
                                    let rewritten_item = ReturnItem {
                                        expression: Expression::Property(
                                            Box::new(Expression::Variable(
                                                node_var.as_str().to_string(),
                                            )),
                                            key.clone(),
                                        ),
                                        alias: item.alias.clone(),
                                    };
                                    return self.translate_return_item(
                                        &rewritten_item,
                                        triples,
                                        extra,
                                    );
                                }
                            }
                        }
                    }
                }
                let base_var = self.extract_variable(base_expr)?;
                let var_name = base_var.as_str().to_string();
                // Check if base is a virtual map alias from head(collect({...})).
                if let Some(key_map) = self.map_vars.get(&var_name) {
                    if let Some(v) = key_map.get(key.as_str()).cloned() {
                        return Ok((v, None, None));
                    }
                }
                // Use alias when it doesn't conflict with the base variable name.
                // When alias == base_var (e.g. `a.name AS a`), always use a fresh var
                // to avoid self-referential triple `?a <name> ?a`.  The WITH handler
                // is responsible for renaming the fresh var to the alias in a fresh scope.
                let result_var = match &item.alias {
                    Some(alias) if alias != &var_name => Variable::new_unchecked(alias.clone()),
                    _ => self.fresh_var(&format!("{}_{}", var_name, key)),
                };
                // Check if this property is known from CREATE/SET tracking (skip_writes mode).
                // Return a BIND expression instead of a BGP triple so that the value
                // comes from the known expression rather than a missing RDF triple.
                if let Some(prop_val) = self
                    .node_props_from_create
                    .get(&var_name)
                    .and_then(|m| m.get(key.as_str()))
                    .cloned()
                {
                    let sparql_expr = self.translate_expr(&prop_val, extra)?;
                    return Ok((result_var, None, Some(sparql_expr)));
                }
                // Check if base var is a compile-time duration literal → extract component.
                if let Some(dur_str) = self.with_lit_vars.get(var_name.as_str()).cloned() {
                    if dur_str.starts_with('P') || dur_str.starts_with("-P") {
                        if let Some(val_str) = duration_get_component(&dur_str, key.as_str()) {
                            let int_lit = SparLit::new_typed_literal(
                                val_str,
                                NamedNode::new_unchecked(XSD_INTEGER),
                            );
                            return Ok((result_var, None, Some(SparExpr::Literal(int_lit))));
                        }
                    }
                }
                // Check if the property is a known temporal component accessor
                // (year, month, day, week, weekYear, weekDay, ordinalDay, quarter, dayOfQuarter,
                //  hour, minute, second, millisecond, microsecond, nanosecond,
                //  timezone, offset, offsetMinutes, offsetSeconds, epochSeconds, epochMillis,
                //  years, months, days, hours, minutes, seconds, milliseconds, microseconds,
                //  nanoseconds, quartersOfYear, monthsOfQuarter, monthsOfYear, daysOfWeek,
                //  minutesOfHour, secondsOfMinute, millisecondsOfSecond, microsecondsOfSecond,
                //  nanosecondsOfSecond).
                // Generate a SPARQL expression that extracts the component from the string value.
                const TEMPORAL_PROPS: &[&str] = &[
                    "year",
                    "month",
                    "day",
                    "quarter",
                    "ordinalDay",
                    "week",
                    "weekYear",
                    "weekDay",
                    "dayOfQuarter",
                    "hour",
                    "minute",
                    "second",
                    "millisecond",
                    "microsecond",
                    "nanosecond",
                    "timezone",
                    "offset",
                    "offsetMinutes",
                    "offsetSeconds",
                    "epochSeconds",
                    "epochMillis",
                    "years",
                    "months",
                    "quarters",
                    "weeks",
                    "days",
                    "hours",
                    "minutes",
                    "seconds",
                    "milliseconds",
                    "microseconds",
                    "nanoseconds",
                    "quartersOfYear",
                    "monthsOfQuarter",
                    "monthsOfYear",
                    "daysOfWeek",
                    "minutesOfHour",
                    "secondsOfMinute",
                    "millisecondsOfSecond",
                    "microsecondsOfSecond",
                    "nanosecondsOfSecond",
                ];
                if TEMPORAL_PROPS.contains(&key.as_str())
                    && !self.node_vars.contains(&var_name)
                    && !self.edge_map.contains_key(&var_name)
                {
                    if let Some(te) =
                        self.temporal_prop_binds(SparExpr::Variable(base_var.clone()), key.as_str())
                    {
                        return Ok((result_var, None, Some(te)));
                    }
                }
                // Check whether base_var is a relationship variable.
                if let Some(edge) = self.edge_map.get(&var_name).cloned() {
                    let prop_iri = self.iri(key);
                    if self.rdf_star {
                        use spargebra::term::NamedNodePattern;
                        let pred_pattern: NamedNodePattern = match edge.pred_var.clone() {
                            Some(pv) => NamedNodePattern::Variable(pv),
                            None => NamedNodePattern::NamedNode(edge.pred.clone()),
                        };
                        // RDF 1.2 reification: ?reif rdf:reifies <<(src pred dst)>>, ?reif <prop> ?result
                        // Both triples are pushed together; caller must group them in one OPTIONAL.
                        let reif_var = self.fresh_var(&format!("__rdf12_reif_{key}"));
                        let rdf_reifies = NamedNode::new_unchecked(
                            "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies",
                        );
                        let edge_term =
                            TermPattern::Triple(Box::new(spargebra::term::TriplePattern {
                                subject: edge.src.clone(),
                                predicate: pred_pattern,
                                object: edge.dst.clone(),
                            }));
                        triples.push(spargebra::term::TriplePattern {
                            subject: reif_var.clone().into(),
                            predicate: rdf_reifies.into(),
                            object: edge_term,
                        });
                        triples.push(spargebra::term::TriplePattern {
                            subject: reif_var.into(),
                            predicate: prop_iri.into(),
                            object: result_var.clone().into(),
                        });
                    } else {
                        let reif_var = edge
                            .reif_var
                            .clone()
                            .unwrap_or_else(|| self.fresh_var(&format!("reif_{var_name}")));
                        triples.push(TriplePattern {
                            subject: reif_var.into(),
                            predicate: prop_iri.into(),
                            object: result_var.clone().into(),
                        });
                    }
                } else {
                    triples.push(TriplePattern {
                        subject: base_var.into(),
                        predicate: self.iri(key).into(),
                        object: result_var.clone().into(),
                    });
                }
                Ok((result_var, None, None))
            }
            Expression::Aggregate(agg_expr) => {
                // Aggregate in RETURN → create result var + AggregateExpression.
                let result_var = match &item.alias {
                    Some(alias) => Variable::new_unchecked(alias.clone()),
                    None => self.fresh_var("agg"),
                };
                // AVG special case: Cypher returns null when no values,
                // but SPARQL returns 0. Add COUNT to check and wrap in IF.
                if let AggregateExpr::Avg { distinct, expr } = agg_expr {
                    let inner = self.translate_expr(expr, extra)?;
                    let avg_var = self.fresh_var("avg");
                    let cnt_var = self.fresh_var("cnt");
                    let null_var = self.fresh_var("null");

                    let avg_agg = AggregateExpression::FunctionCall {
                        name: AggregateFunction::Avg,
                        expr: inner.clone(),
                        distinct: *distinct,
                    };
                    let cnt_agg = AggregateExpression::FunctionCall {
                        name: AggregateFunction::Count,
                        expr: inner,
                        distinct: false,
                    };
                    // Store extra COUNT aggregate in pending_aggs for the caller.
                    self.pending_aggs.push((cnt_var.clone(), cnt_agg));

                    // IF(?cnt = 0, ?null_var, ?avg_var)
                    let zero = SparExpr::Literal(SparLit::new_typed_literal(
                        "0",
                        NamedNode::new_unchecked(XSD_INTEGER),
                    ));
                    let if_expr = SparExpr::If(
                        Box::new(SparExpr::Equal(
                            Box::new(SparExpr::Variable(cnt_var)),
                            Box::new(zero),
                        )),
                        Box::new(SparExpr::Variable(null_var)),
                        Box::new(SparExpr::Variable(avg_var.clone())),
                    );
                    return Ok((result_var, Some((avg_var, avg_agg)), Some(if_expr)));
                }
                // Collect special case: wrap GROUP_CONCAT output in ['...', '...'] format.
                if let AggregateExpr::Collect { distinct, expr } = agg_expr {
                    let inner = self.translate_expr(expr, extra)?;
                    let gc_var = self.fresh_var("gc");

                    // Numeric/boolean values get bare STR(); strings get single-quoted.
                    // IF(isLiteral(v) && datatype(v) IN {integer,double,boolean}, STR(v), CONCAT("'",STR(v),"'"))
                    let is_numeric = {
                        let dt_expr = SparExpr::FunctionCall(
                            spargebra::algebra::Function::Datatype,
                            vec![inner.clone()],
                        );
                        let is_int = SparExpr::Equal(
                            Box::new(dt_expr.clone()),
                            Box::new(SparExpr::NamedNode(NamedNode::new_unchecked(XSD_INTEGER))),
                        );
                        let is_dbl = SparExpr::Equal(
                            Box::new(dt_expr.clone()),
                            Box::new(SparExpr::NamedNode(NamedNode::new_unchecked(XSD_DOUBLE))),
                        );
                        let is_bool = SparExpr::Equal(
                            Box::new(dt_expr),
                            Box::new(SparExpr::NamedNode(NamedNode::new_unchecked(XSD_BOOLEAN))),
                        );
                        SparExpr::And(
                            Box::new(SparExpr::FunctionCall(
                                spargebra::algebra::Function::IsLiteral,
                                vec![inner.clone()],
                            )),
                            Box::new(SparExpr::Or(
                                Box::new(SparExpr::Or(Box::new(is_int), Box::new(is_dbl))),
                                Box::new(is_bool),
                            )),
                        )
                    };
                    let str_inner =
                        SparExpr::FunctionCall(spargebra::algebra::Function::Str, vec![inner]);
                    let str_quoted = SparExpr::FunctionCall(
                        spargebra::algebra::Function::Concat,
                        vec![
                            SparExpr::Literal(SparLit::new_simple_literal("'")),
                            str_inner.clone(),
                            SparExpr::Literal(SparLit::new_simple_literal("'")),
                        ],
                    );
                    let quoted = SparExpr::If(
                        Box::new(is_numeric),
                        Box::new(str_inner),
                        Box::new(str_quoted),
                    );
                    let gc_agg = AggregateExpression::FunctionCall {
                        name: AggregateFunction::GroupConcat {
                            separator: Some(", ".to_string()),
                        },
                        expr: quoted,
                        distinct: *distinct,
                    };
                    // Wrap: CONCAT("[", COALESCE(gc_var, ""), "]")
                    // COALESCE handles the case where GROUP_CONCAT returns UNDEF (empty input).
                    let wrap_expr = SparExpr::FunctionCall(
                        spargebra::algebra::Function::Concat,
                        vec![
                            SparExpr::Literal(SparLit::new_simple_literal("[")),
                            SparExpr::Coalesce(vec![
                                SparExpr::Variable(gc_var.clone()),
                                SparExpr::Literal(SparLit::new_simple_literal("")),
                            ]),
                            SparExpr::Literal(SparLit::new_simple_literal("]")),
                        ],
                    );
                    return Ok((result_var, Some((gc_var, gc_agg)), Some(wrap_expr)));
                }
                let sparql_agg = self.translate_aggregate_expr(agg_expr, extra)?;
                Ok((result_var.clone(), Some((result_var, sparql_agg)), None))
            }
            other => {
                // Peephole: `head(collect({k1: v1, k2: v2, ...})) AS alias`
                // → MIN-aggregate each key value, register alias→{key→var} in map_vars.
                // This allows `alias.k` property accesses in downstream clauses.
                if let Expression::FunctionCall { name, args, .. } = other {
                    if (name.eq_ignore_ascii_case("head") || name.eq_ignore_ascii_case("last"))
                        && args.len() == 1
                    {
                        if let Expression::Aggregate(AggregateExpr::Collect {
                            expr: collect_expr,
                            ..
                        }) = &args[0]
                        {
                            if let Expression::Map(pairs) = collect_expr.as_ref() {
                                if let Some(alias) = &item.alias {
                                    if !pairs.is_empty() {
                                        let mut key_vars: std::collections::HashMap<
                                            String,
                                            Variable,
                                        > = Default::default();
                                        let mut first_result: Option<(
                                            Variable,
                                            Option<(Variable, AggregateExpression)>,
                                            Option<SparExpr>,
                                        )> = None;
                                        for (key, val_expr) in pairs {
                                            let v_key = self.fresh_var(&format!("{alias}__{key}"));
                                            let inner = self.translate_expr(val_expr, extra)?;
                                            let min_agg = AggregateExpression::FunctionCall {
                                                name: AggregateFunction::Min,
                                                expr: inner,
                                                distinct: false,
                                            };
                                            key_vars.insert(key.clone(), v_key.clone());
                                            if first_result.is_none() {
                                                first_result = Some((
                                                    v_key.clone(),
                                                    Some((v_key, min_agg)),
                                                    None,
                                                ));
                                            } else {
                                                // Additional keys go into pending_aggs so
                                                // translate_return_clause wires them up.
                                                self.pending_aggs.push((v_key, min_agg));
                                            }
                                        }
                                        self.map_vars.insert(alias.clone(), key_vars);
                                        if let Some(res) = first_result {
                                            return Ok(res);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // General expression: try to translate as SPARQL expression,
                // and bind it via Extend. If the expression contains aggregates
                // (e.g. count(*) * 10), record them via pending_aggs so that
                // the caller can wire them up to a GROUP pattern.
                //
                // Use the alias name directly as the result var so that UNION arms
                // with the same alias produce compatible SPARQL projection variables.
                // Fall back to fresh_var when there is no alias or the alias
                // conflicts with a node/edge pattern variable.
                let alias_name = item.alias.as_deref().unwrap_or("");
                let result_var = if !alias_name.is_empty()
                    && !self.node_vars.contains(alias_name)
                    && !self.edge_map.contains_key(alias_name)
                    && !expr_references_var(other, alias_name)
                {
                    Variable::new_unchecked(alias_name.to_string())
                } else {
                    self.fresh_var(if alias_name.is_empty() {
                        "ret"
                    } else {
                        alias_name
                    })
                };
                self.pending_aggs.clear();
                match self.translate_expr(other, extra) {
                    Ok(sparql_expr) => {
                        if !self.pending_aggs.is_empty() {
                            // Expression wraps one or more aggregates (e.g. count(*) * 10
                            // or count(a) * 10 + count(b) * 5).
                            // Take the first aggregate as the "primary" for the return tuple;
                            // any additional aggregates remain in pending_aggs so the caller's
                            // drain loop (line ~4255) picks them up and adds them to GROUP.
                            let (agg_var, agg) = self.pending_aggs.remove(0);
                            // pending_aggs may still contain additional aggregates — leave them
                            // for the caller to drain rather than clearing here.
                            Ok((result_var, Some((agg_var, agg)), Some(sparql_expr)))
                        } else {
                            Ok((result_var, None, Some(sparql_expr)))
                        }
                    }
                    Err(_) => Err(PolygraphError::UnsupportedFeature {
                        feature: format!(
                            "complex return expression (Phase 4+): {}",
                            result_var.as_str()
                        ),
                    }),
                }
            }
        }
    }

}
