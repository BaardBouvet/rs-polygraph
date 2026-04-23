impl TranslationState {
    // ── Function call translation ─────────────────────────────────────────────

    fn translate_function_call(
        &mut self,
        name: &str,
        args: &[Expression],
        extra: &mut Vec<TriplePattern>,
    ) -> Result<SparExpr, PolygraphError> {
        use spargebra::algebra::Function;
        let name_lower = name.to_ascii_lowercase();
        match name_lower.as_str() {
            "type" => {
                // type(r) → local name of the relationship predicate.
                // type(null) → null
                if let Some(Expression::Literal(Literal::Null)) = args.first() {
                    return Ok(SparExpr::Variable(self.fresh_var("null")));
                }
                // If arg is list[n] that resolves to a known variable, rewrite as type(var).
                let resolved_var: Option<String> =
                    if let Some(Expression::Subscript(coll, idx)) = args.first() {
                        self.resolve_literal_list(coll).and_then(|items| {
                            let n_len = items.len() as i64;
                            get_literal_int(idx).and_then(|iv| {
                                let i = if iv < 0 { n_len + iv } else { iv };
                                if i >= 0 && i < n_len {
                                    if let Expression::Variable(v) = &items[i as usize] {
                                        Some(v.clone())
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                        })
                    } else {
                        None
                    };
                let eff_var_name: Option<String> = resolved_var.or_else(|| {
                    if let Some(Expression::Variable(v)) = args.first() {
                        Some(v.clone())
                    } else {
                        None
                    }
                });
                if let Some(var_name) = &eff_var_name {
                    if let Some(edge) = self.edge_map.get(var_name).cloned() {
                        if let Some(pred_var) = edge.pred_var {
                            // Untyped relationship: extract local name via STRAFTER(STR(?pred), base).
                            let base_lit = SparExpr::Literal(SparLit::new_simple_literal(
                                self.base_iri.clone(),
                            ));
                            let str_pred = SparExpr::FunctionCall(
                                Function::Str,
                                vec![SparExpr::Variable(pred_var)],
                            );
                            return Ok(SparExpr::FunctionCall(
                                Function::StrAfter,
                                vec![str_pred, base_lit],
                            ));
                        } else {
                            // Fixed predicate: extract local name statically.
                            // For OPTIONAL MATCH, the predicate may be null if the
                            // relationship was not matched. Use IF(BOUND(?null_check), ..., null).
                            let iri = edge.pred.as_str().to_string();
                            let local =
                                iri.strip_prefix(&self.base_iri).unwrap_or(&iri).to_string();
                            let type_lit = SparExpr::Literal(SparLit::new_simple_literal(local));
                            // If the edge came from an OPTIONAL MATCH and has a null_check_var,
                            // wrap in IF(BOUND(?check), type_literal, ?null).
                            if let Some(nc_var) = edge.null_check_var {
                                let bound_check = SparExpr::Bound(nc_var);
                                let null_var = SparExpr::Variable(self.fresh_var("null"));
                                return Ok(SparExpr::If(
                                    Box::new(bound_check),
                                    Box::new(type_lit),
                                    Box::new(null_var),
                                ));
                            }
                            return Ok(type_lit);
                        }
                    }
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: "type() requires a relationship variable argument".to_string(),
                })
            }
            "abs" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "abs() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(Function::Abs, vec![arg]))
            }
            "ceil" | "ceiling" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "ceil() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(Function::Ceil, vec![arg]))
            }
            "floor" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "floor() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(Function::Floor, vec![arg]))
            }
            "round" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "round() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(Function::Round, vec![arg]))
            }
            "tostring" | "str" | "tostr" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "toString() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(Function::Str, vec![arg]))
            }
            "tointeger" | "toint" | "todouble" | "tofloat" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "toInteger() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                let cast_iri = if name_lower.starts_with("tod") || name_lower.starts_with("tof") {
                    NamedNode::new_unchecked(XSD_DOUBLE)
                } else {
                    NamedNode::new_unchecked(XSD_INTEGER)
                };
                Ok(SparExpr::FunctionCall(
                    Function::Custom(cast_iri),
                    vec![arg],
                ))
            }
            "toupper" | "touppercase" | "upper" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "toUpper() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(Function::UCase, vec![arg]))
            }
            "tolower" | "tolowercase" | "lower" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "toLower() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(Function::LCase, vec![arg]))
            }
            "strlen" | "length" if args.len() == 1 => {
                // Check if the argument is a named path variable → emit constant hop count.
                if let Expression::Variable(v) = &args[0] {
                    if let Some(&hops) = self.path_hops.get(v.as_str()) {
                        return Ok(SparExpr::Literal(SparLit::new_typed_literal(
                            hops.to_string(),
                            NamedNode::new_unchecked(XSD_INTEGER),
                        )));
                    }
                    // InvalidArgumentType: length() on a node or relationship variable.
                    if self.node_vars.contains(v.as_str()) || self.edge_map.contains_key(v.as_str())
                    {
                        return Err(PolygraphError::Translation {
                            message: format!(
                                "InvalidArgumentType: length() cannot be applied to a node or relationship variable '{v}'"
                            ),
                        });
                    }
                }
                // length(string) maps to STRLEN.
                let arg = self.translate_expr(&args[0], extra)?;
                Ok(SparExpr::FunctionCall(Function::StrLen, vec![arg]))
            }
            "substring" | "substr" => {
                // Cypher substring(str, start[, length]) uses 0-based indexing.
                // SPARQL SUBSTR(str, start[, length]) uses 1-based indexing.
                // Adjust: add 1 to the start position.
                if args.is_empty() {
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: "substring() requires at least 2 arguments".to_string(),
                    });
                }
                let str_arg = self.translate_expr(&args[0], extra)?;
                let start_cypher = self.translate_expr(&args[1], extra)?;
                let one = SparExpr::Literal(SparLit::new_typed_literal(
                    "1",
                    NamedNode::new_unchecked(XSD_INTEGER),
                ));
                let start_sparql = SparExpr::Add(Box::new(start_cypher), Box::new(one));
                if args.len() >= 3 {
                    let len_arg = self.translate_expr(&args[2], extra)?;
                    Ok(SparExpr::FunctionCall(
                        Function::SubStr,
                        vec![str_arg, start_sparql, len_arg],
                    ))
                } else {
                    Ok(SparExpr::FunctionCall(
                        Function::SubStr,
                        vec![str_arg, start_sparql],
                    ))
                }
            }
            "replace" => {
                let mut sargs: Vec<SparExpr> = Vec::new();
                for a in args {
                    sargs.push(self.translate_expr(a, extra)?);
                }
                Ok(SparExpr::FunctionCall(Function::Replace, sargs))
            }
            "startswith" | "starts_with" => {
                let l = self.translate_expr(&args[0], extra)?;
                let r = self.translate_expr(&args[1], extra)?;
                Ok(SparExpr::FunctionCall(Function::StrStarts, vec![l, r]))
            }
            "endswith" | "ends_with" => {
                let l = self.translate_expr(&args[0], extra)?;
                let r = self.translate_expr(&args[1], extra)?;
                Ok(SparExpr::FunctionCall(Function::StrEnds, vec![l, r]))
            }
            "contains" => {
                let l = self.translate_expr(&args[0], extra)?;
                let r = self.translate_expr(&args[1], extra)?;
                Ok(SparExpr::FunctionCall(Function::Contains, vec![l, r]))
            }
            "coalesce" => {
                let sargs: Result<Vec<SparExpr>, _> =
                    args.iter().map(|a| self.translate_expr(a, extra)).collect();
                Ok(SparExpr::Coalesce(sargs?))
            }
            "not" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "not() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::Not(Box::new(arg)))
            }
            "size" => {
                if let Some(arg) = args.first() {
                    // size(collect(x)) → COUNT(x): recognize aggregate arg
                    if let Expression::Aggregate(AggregateExpr::Collect { distinct, expr }) = arg {
                        let inner = self.translate_expr(expr, extra)?;
                        let fresh = self.fresh_var("agg");
                        let agg = AggregateExpression::FunctionCall {
                            name: AggregateFunction::Count,
                            expr: inner,
                            distinct: *distinct,
                        };
                        self.pending_aggs.push((fresh.clone(), agg));
                        return Ok(SparExpr::Variable(fresh));
                    }
                    // size(pattern_comprehension) → count the number of pattern matches.
                    // translate_pattern_comprehension now returns a list string, so we
                    // need to run a separate COUNT subquery instead.
                    if let Expression::PatternComprehension {
                        pattern, predicate, ..
                    } = arg
                    {
                        let mut inner_triples: Vec<TriplePattern> = Vec::new();
                        let mut inner_paths: Vec<GraphPattern> = Vec::new();
                        self.translate_pattern(pattern, &mut inner_triples, &mut inner_paths)?;
                        let anchor_vars: Vec<Variable> = pattern
                            .elements
                            .iter()
                            .filter_map(|e| {
                                if let crate::ast::cypher::PatternElement::Node(n) = e {
                                    n.variable
                                        .as_ref()
                                        .filter(|v| {
                                            self.node_vars.contains(v.as_str())
                                                || self.edge_map.contains_key(v.as_str())
                                        })
                                        .map(|v| Variable::new_unchecked(v.clone()))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        let mut inner_pattern = GraphPattern::Bgp {
                            patterns: inner_triples,
                        };
                        for gp in inner_paths {
                            inner_pattern = join_patterns(inner_pattern, gp);
                        }
                        if let Some(pred) = predicate {
                            let mut pred_extra: Vec<TriplePattern> = Vec::new();
                            let pred_sparql = self.translate_expr(pred, &mut pred_extra)?;
                            for tp in pred_extra {
                                inner_pattern = GraphPattern::LeftJoin {
                                    left: Box::new(inner_pattern),
                                    right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                    expression: None,
                                };
                            }
                            inner_pattern = GraphPattern::Filter {
                                inner: Box::new(inner_pattern),
                                expr: pred_sparql,
                            };
                        }
                        let cnt_var = self.fresh_var("pc_cnt");
                        let count_agg = spargebra::algebra::AggregateExpression::CountSolutions {
                            distinct: false,
                        };
                        let subquery = GraphPattern::Group {
                            inner: Box::new(inner_pattern),
                            variables: anchor_vars,
                            aggregates: vec![(cnt_var.clone(), count_agg)],
                        };
                        self.pending_subqueries.push((cnt_var.clone(), subquery));
                        let zero = SparExpr::Literal(SparLit::new_typed_literal(
                            "0",
                            NamedNode::new_unchecked(XSD_INTEGER),
                        ));
                        return Ok(SparExpr::Coalesce(vec![SparExpr::Variable(cnt_var), zero]));
                    }
                    // size([a, b, c]) or size([a] + [b, c]) → element count as integer
                    if let Some(count) = count_list_elements(arg) {
                        return Ok(SparExpr::Literal(SparLit::new_typed_literal(
                            count.to_string(),
                            NamedNode::new_unchecked(XSD_INTEGER),
                        )));
                    }
                    // Special case: size([x IN literal_list WHERE pred]) with a runtime predicate.
                    // The predicate may reference SPARQL variables (e.g. UNWIND variables).
                    // Generate: IF(pred_1, 1, 0) + IF(pred_2, 1, 0) + ... for each literal element.
                    if let Expression::ListComprehension {
                        variable,
                        list,
                        predicate: Some(pred),
                        ..
                    } = arg
                    {
                        if let Some(items) = self.resolve_literal_list(list) {
                            let zero = SparExpr::Literal(SparLit::new_typed_literal(
                                "0",
                                NamedNode::new_unchecked(XSD_INTEGER),
                            ));
                            let one = SparExpr::Literal(SparLit::new_typed_literal(
                                "1",
                                NamedNode::new_unchecked(XSD_INTEGER),
                            ));
                            let mut sum: SparExpr = zero.clone();
                            let mut all_ok = true;
                            for item in &items {
                                let subst_pred = substitute_var_in_expr(pred, variable, item);
                                // First try static evaluation (avoids redundant SPARQL IF)
                                match try_eval_bool_const(&subst_pred) {
                                    Some(Some(true)) => {
                                        sum = SparExpr::Add(Box::new(sum), Box::new(one.clone()));
                                    }
                                    Some(Some(false)) | Some(None) => {
                                        // Contributes 0 — skip
                                    }
                                    None => {
                                        // Runtime predicate — emit IF(pred, 1, 0)
                                        match self.translate_expr(&subst_pred, extra) {
                                            Ok(sparql_pred) => {
                                                let if_expr = SparExpr::If(
                                                    Box::new(sparql_pred),
                                                    Box::new(one.clone()),
                                                    Box::new(zero.clone()),
                                                );
                                                sum =
                                                    SparExpr::Add(Box::new(sum), Box::new(if_expr));
                                            }
                                            Err(_) => {
                                                all_ok = false;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            if all_ok {
                                return Ok(sum);
                            }
                        }
                    }
                    // size(string) → STRLEN(string), but if the arg resolves to a
                    // compile-time list (e.g. n.prop where SET n.prop = [1,2,3]),
                    // return the element count directly.
                    if let Expression::Property(base_expr, key) = arg {
                        if let Expression::Variable(v) = base_expr.as_ref() {
                            if let Some(list_expr) = self
                                .node_props_from_create
                                .get(v.as_str())
                                .and_then(|m| m.get(key.as_str()))
                                .cloned()
                            {
                                if let Some(count) = count_list_elements(&list_expr) {
                                    return Ok(SparExpr::Literal(SparLit::new_typed_literal(
                                        count.to_string(),
                                        NamedNode::new_unchecked(XSD_INTEGER),
                                    )));
                                }
                            }
                        }
                    }
                    // size(keys(n)) / size(keys(r)) → COUNT of property keys via GROUP subquery
                    if let Expression::FunctionCall { name: kname, args: kargs, .. } = arg {
                        if kname.eq_ignore_ascii_case("keys") {
                            if let Some(Expression::Variable(kv)) = kargs.first() {
                                use spargebra::term::NamedNodePattern as NNP;
                                let pred_v = self.fresh_var("__keys_pred");
                                let val_v = self.fresh_var("__keys_val");
                                let cnt_v = self.fresh_var("__keys_cnt");
                                let base = self.base_iri.clone();
                                let base_lit = SparExpr::Literal(SparLit::new_simple_literal(base.clone()));
                                let str_pred = SparExpr::FunctionCall(
                                    Function::Str,
                                    vec![SparExpr::Variable(pred_v.clone())],
                                );
                                let strstarts_filt = SparExpr::FunctionCall(
                                    Function::StrStarts,
                                    vec![str_pred.clone(), base_lit],
                                );
                                let (inner_pat, group_var) = if let Some(edge) = self.edge_map.get(kv.as_str()).cloned() {
                                    // Relationship: find property triples via reification lookup
                                    let new_reif = self.fresh_var("__ks_reif");
                                    let raw_bgp = if self.rdf_star {
                                        let rdf_reifies = NamedNode::new_unchecked(
                                            "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies",
                                        );
                                        let pred_pat = match edge.pred_var.clone() {
                                            Some(pv) => NNP::Variable(pv),
                                            None => NNP::NamedNode(edge.pred.clone()),
                                        };
                                        let edge_term = TermPattern::Triple(Box::new(
                                            spargebra::term::TriplePattern {
                                                subject: edge.src.clone(),
                                                predicate: pred_pat,
                                                object: edge.dst.clone(),
                                            },
                                        ));
                                        GraphPattern::Bgp {
                                            patterns: vec![
                                                TriplePattern {
                                                    subject: TermPattern::Variable(new_reif.clone()),
                                                    predicate: NNP::NamedNode(rdf_reifies),
                                                    object: edge_term,
                                                },
                                                TriplePattern {
                                                    subject: TermPattern::Variable(new_reif.clone()),
                                                    predicate: NNP::Variable(pred_v.clone()),
                                                    object: TermPattern::Variable(val_v),
                                                },
                                            ],
                                        }
                                    } else {
                                        let rdf_subject = NamedNode::new_unchecked(
                                            "http://www.w3.org/1999/02/22-rdf-syntax-ns#subject",
                                        );
                                        let rdf_predicate = NamedNode::new_unchecked(
                                            "http://www.w3.org/1999/02/22-rdf-syntax-ns#predicate",
                                        );
                                        let rdf_object = NamedNode::new_unchecked(
                                            "http://www.w3.org/1999/02/22-rdf-syntax-ns#object",
                                        );
                                        let pred_obj = match edge.pred_var.clone() {
                                            Some(pv) => TermPattern::Variable(pv),
                                            None => TermPattern::NamedNode(edge.pred.clone()),
                                        };
                                        GraphPattern::Bgp {
                                            patterns: vec![
                                                TriplePattern {
                                                    subject: TermPattern::Variable(new_reif.clone()),
                                                    predicate: NNP::NamedNode(rdf_subject),
                                                    object: edge.src.clone(),
                                                },
                                                TriplePattern {
                                                    subject: TermPattern::Variable(new_reif.clone()),
                                                    predicate: NNP::NamedNode(rdf_predicate),
                                                    object: pred_obj,
                                                },
                                                TriplePattern {
                                                    subject: TermPattern::Variable(new_reif.clone()),
                                                    predicate: NNP::NamedNode(rdf_object),
                                                    object: edge.dst.clone(),
                                                },
                                                TriplePattern {
                                                    subject: TermPattern::Variable(new_reif.clone()),
                                                    predicate: NNP::Variable(pred_v.clone()),
                                                    object: TermPattern::Variable(val_v),
                                                },
                                            ],
                                        }
                                    };
                                    let filtered = GraphPattern::Filter {
                                        expr: strstarts_filt,
                                        inner: Box::new(raw_bgp),
                                    };
                                    let grp = edge.eid_var.clone()
                                        .or_else(|| edge.null_check_var.clone())
                                        .unwrap_or_else(|| Variable::new_unchecked(kv.clone()));
                                    (filtered, grp)
                                } else {
                                    // Node: ?n ?pred ?val with rdf:type / __node excluded
                                    let node_var = Variable::new_unchecked(kv.clone());
                                    let sentinel_iri = format!("{base}__node");
                                    let sentinel_lit = SparExpr::Literal(SparLit::new_simple_literal(sentinel_iri));
                                    let rdf_type_lit = SparExpr::Literal(SparLit::new_simple_literal(RDF_TYPE.to_string()));
                                    let not_sentinel = SparExpr::Not(Box::new(SparExpr::Equal(
                                        Box::new(str_pred.clone()),
                                        Box::new(sentinel_lit),
                                    )));
                                    let not_type = SparExpr::Not(Box::new(SparExpr::Equal(
                                        Box::new(str_pred.clone()),
                                        Box::new(rdf_type_lit),
                                    )));
                                    let bgp = GraphPattern::Bgp {
                                        patterns: vec![TriplePattern {
                                            subject: TermPattern::Variable(node_var.clone()),
                                            predicate: NNP::Variable(pred_v.clone()),
                                            object: TermPattern::Variable(val_v),
                                        }],
                                    };
                                    let full_filter = SparExpr::And(
                                        Box::new(strstarts_filt),
                                        Box::new(SparExpr::And(Box::new(not_sentinel), Box::new(not_type))),
                                    );
                                    let filtered = GraphPattern::Filter {
                                        expr: full_filter,
                                        inner: Box::new(bgp),
                                    };
                                    (filtered, node_var)
                                };
                                let count_agg = AggregateExpression::CountSolutions { distinct: false };
                                let subquery = GraphPattern::Group {
                                    inner: Box::new(inner_pat),
                                    variables: vec![group_var],
                                    aggregates: vec![(cnt_v.clone(), count_agg)],
                                };
                                self.pending_subqueries.push((cnt_v.clone(), subquery));
                                let zero = SparExpr::Literal(SparLit::new_typed_literal(
                                    "0",
                                    NamedNode::new_unchecked(XSD_INTEGER),
                                ));
                                return Ok(SparExpr::Coalesce(vec![SparExpr::Variable(cnt_v), zero]));
                            }
                        }
                    }
                    let translated = self.translate_expr(arg, extra)?;
                    Ok(SparExpr::FunctionCall(Function::StrLen, vec![translated]))
                } else {
                    Err(PolygraphError::UnsupportedFeature {
                        feature: "size() requires an argument".to_string(),
                    })
                }
            }
            "head" => {
                // head(r) on a varlen relationship variable with upper ≤ 1 is the same
                // as last(r): emit forward/backward OPTIONALs and COALESCE.
                // Otherwise fall back to list head: STRBEFORE(str, " ").
                if let Some(Expression::Variable(r_name)) = args.first() {
                    if let Some(&(lower, upper)) = self.varlen_rel_scope.get(r_name.as_str()) {
                        if upper <= 1 {
                            if let Some(edge) = self.edge_map.get(r_name.as_str()).cloned() {
                                use spargebra::term::NamedNodePattern;
                                let fwd_var = self.fresh_var("__last_r_fwd");
                                let bwd_var = self.fresh_var("__last_r_bwd");
                                extra.push(TriplePattern {
                                    subject: edge.src.clone(),
                                    predicate: NamedNodePattern::Variable(fwd_var.clone()),
                                    object: edge.dst.clone(),
                                });
                                extra.push(TriplePattern {
                                    subject: edge.dst.clone(),
                                    predicate: NamedNodePattern::Variable(bwd_var.clone()),
                                    object: edge.src.clone(),
                                });
                                let _ = lower;
                                return Ok(SparExpr::Coalesce(vec![
                                    SparExpr::Variable(fwd_var),
                                    SparExpr::Variable(bwd_var),
                                ]));
                            }
                        }
                    }
                }
                // head(list) → first element. For collected lists (GROUP_CONCAT),
                // extract substring before first separator.
                if let Some(arg) = args.first() {
                    let translated = self.translate_expr(arg, extra)?;
                    let sep = SparExpr::Literal(SparLit::new_simple_literal(" "));
                    Ok(SparExpr::FunctionCall(
                        Function::StrBefore,
                        vec![translated, sep],
                    ))
                } else {
                    Err(PolygraphError::UnsupportedFeature {
                        feature: "head() requires an argument".to_string(),
                    })
                }
            }
            "tail" => {
                // tail(list) → all but first element.
                if let Some(arg) = args.first() {
                    // Try compile-time resolution first (e.g. for skips_writes SET tracking).
                    if let Some(items) = self.resolve_literal_list(arg) {
                        let tail_items: Vec<Expression> =
                            if items.is_empty() { vec![] } else { items[1..].to_vec() };
                        let serialized = serialize_list_literal(&tail_items);
                        return Ok(SparExpr::Literal(SparLit::new_simple_literal(serialized)));
                    }
                    // Runtime fallback: string manipulation on "[a, b, c]" representation.
                    // tail("[a, b, c]") = "[b, c]"
                    // = IF(CONTAINS(s, ", "), CONCAT("[", STRAFTER(s, ", ")), "[]")
                    let translated = self.translate_expr(arg, extra)?;
                    let sep = SparExpr::Literal(SparLit::new_simple_literal(", "));
                    let contains_sep = SparExpr::FunctionCall(
                        Function::Contains,
                        vec![translated.clone(), sep.clone()],
                    );
                    let after_sep = SparExpr::FunctionCall(
                        Function::Concat,
                        vec![
                            SparExpr::Literal(SparLit::new_simple_literal("[")),
                            SparExpr::FunctionCall(
                                Function::StrAfter,
                                vec![translated, sep],
                            ),
                        ],
                    );
                    Ok(SparExpr::If(
                        Box::new(contains_sep),
                        Box::new(after_sep),
                        Box::new(SparExpr::Literal(SparLit::new_simple_literal("[]"))),
                    ))
                } else {
                    Err(PolygraphError::UnsupportedFeature {
                        feature: "tail() requires an argument".to_string(),
                    })
                }
            }
            "nodes" => {
                // nodes(p) → list of node IRIs along named path p.
                // nodes(null) or nodes(nullable) → null
                match args.first() {
                    Some(Expression::Literal(Literal::Null)) => {
                        return Ok(SparExpr::Variable(self.fresh_var("null")));
                    }
                    Some(Expression::Variable(v))
                        if self.null_vars.contains(v.as_str())
                            || self.nullable_vars.contains(v.as_str()) =>
                    {
                        return Ok(SparExpr::Variable(self.fresh_var("null")));
                    }
                    _ => {}
                }
                if let Some(Expression::Variable(path_name)) = args.first() {
                    if let Some(node_vars) = self.path_node_vars.get(path_name.as_str()).cloned() {
                        // Build CONCAT("[", STR(?n0), ", ", STR(?n1), ..., "]")
                        let mut parts: Vec<SparExpr> = Vec::new();
                        parts.push(SparExpr::Literal(SparLit::new_simple_literal("[")));
                        for (idx, v) in node_vars.iter().enumerate() {
                            if idx > 0 {
                                parts.push(SparExpr::Literal(SparLit::new_simple_literal(", ")));
                            }
                            parts.push(SparExpr::FunctionCall(
                                Function::Str,
                                vec![SparExpr::Variable(v.clone())],
                            ));
                        }
                        parts.push(SparExpr::Literal(SparLit::new_simple_literal("]")));
                        return Ok(SparExpr::FunctionCall(Function::Concat, parts));
                    }
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: "nodes() requires a named path argument".to_string(),
                })
            }
            "keys" => {
                let arg = args
                    .first()
                    .ok_or_else(|| PolygraphError::UnsupportedFeature {
                        feature: "keys() requires an argument".to_string(),
                    })?;
                match arg {
                    Expression::Map(pairs) => {
                        // keys({k: v, ...}) → compile-time list of key strings
                        let key_list: Vec<String> =
                            pairs.iter().map(|(k, _)| format!("'{k}'")).collect();
                        let serialized = format!("[{}]", key_list.join(", "));
                        return Ok(SparExpr::Literal(SparLit::new_simple_literal(serialized)));
                    }
                    Expression::Literal(Literal::Null) => {
                        // keys(null) → null
                        return Ok(SparExpr::Variable(self.fresh_var("null")));
                    }
                    Expression::Variable(v) => {
                        let vname = v.clone();
                        // Check if variable is a known map alias from WITH clause
                        if let Some(key_map) = self.map_vars.get(&vname).cloned() {
                            let key_list: Vec<String> =
                                key_map.keys().map(|k| format!("'{k}'")).collect();
                            let serialized = format!("[{}]", key_list.join(", "));
                            return Ok(SparExpr::Literal(SparLit::new_simple_literal(serialized)));
                        }
                        // Unknown variable — return null (unbound)
                        return Ok(SparExpr::Variable(self.fresh_var("null")));
                    }
                    _ => {}
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: "keys() on non-literal map".to_string(),
                })
            }
            "labels" | "relationships" => {
                // For null literal or statically-null/nullable variables, return null.
                // For real node/path variables we can't enumerate labels/relationships at
                // compile time (no graph data).
                let arg = args.first();
                // Resolve subscript access (e.g. labels(list[0])) to the inner variable.
                if let Some(Expression::Subscript(coll, idx)) = arg {
                    if let Some(items) = self.resolve_literal_list(coll) {
                        let n_len = items.len() as i64;
                        if let Some(iv) = get_literal_int(idx) {
                            let i = if iv < 0 { n_len + iv } else { iv };
                            if i >= 0 && i < n_len {
                                let inner_arg = items[i as usize].clone();
                                let rewritten = Expression::FunctionCall {
                                    name: name.to_string(),
                                    distinct: false,
                                    args: vec![inner_arg],
                                };
                                return self.translate_expr(&rewritten, extra);
                            }
                        }
                    }
                }
                match arg {
                    Some(Expression::Literal(Literal::Null)) => {
                        return Ok(SparExpr::Variable(self.fresh_var("null")));
                    }
                    Some(Expression::Variable(v)) => {
                        if self.null_vars.contains(v.as_str())
                            || self.nullable_vars.contains(v.as_str())
                        {
                            return Ok(SparExpr::Variable(self.fresh_var("null")));
                        }
                        // If the variable was created in skip_writes mode, return static labels.
                        if name_lower == "labels" {
                            // Path and relationship variables are invalid for labels().
                            if self.path_hops.contains_key(v.as_str()) {
                                return Err(PolygraphError::Translation {
                                    message: "SyntaxError: InvalidArgumentType: labels() does not apply to path variables".to_string(),
                                });
                            }
                            if self.edge_map.contains_key(v.as_str()) {
                                return Err(PolygraphError::Translation {
                                    message: "SyntaxError: InvalidArgumentType: labels() does not apply to relationship variables".to_string(),
                                });
                            }
                            // Static path: for CREATE/MERGE-bound variables where the
                            // labels are fully known at compile time (including any label
                            // additions from ON MATCH SET, which are pre-scanned below),
                            // return the label list without a graph query. This avoids
                            // spurious extra rows when CREATE is in skip_writes mode and
                            // the variable has no constraining pattern in the SELECT.
                            if let Some(labels) = self.node_labels_from_create.get(v.as_str()).cloned() {
                                let list_str = if labels.is_empty() {
                                    "[]".to_string()
                                } else {
                                    let mut sorted = labels.clone();
                                    sorted.sort();
                                    let items: Vec<String> = sorted.iter().map(|l| format!("'{l}'")).collect();
                                    format!("[{}]", items.join(", "))
                                };
                                return Ok(SparExpr::Literal(SparLit::new_simple_literal(list_str)));
                            }
                            // Dynamic labels query: generate a GROUP BY subquery that
                            // collects all rdf:type values for the node from the graph.
                            // This correctly reflects label additions/removals done via
                            // SET/REMOVE write clauses that ran before this SELECT.
                            let var_name = v.clone();
                            let n_var = Variable::new_unchecked(var_name.clone());
                            let ltype_var = self.fresh_var(&format!("__ltype_{var_name}"));
                            let gc_var = self.fresh_var(&format!("__labels_gc_{var_name}"));
                            let base = self.base_iri.clone();
                            let base_len = base.len();
                            let rdf_type_nn = NamedNode::new_unchecked(RDF_TYPE);
                            use spargebra::term::NamedNodePattern;
                            // Inner: ?n rdf:type ?__ltype . FILTER(STRSTARTS(STR(?__ltype), BASE))
                            let inner_bgp = GraphPattern::Bgp {
                                patterns: vec![TriplePattern {
                                    subject: TermPattern::Variable(n_var.clone()),
                                    predicate: NamedNodePattern::NamedNode(rdf_type_nn),
                                    object: TermPattern::Variable(ltype_var.clone()),
                                }],
                            };
                            let filter_expr = SparExpr::FunctionCall(
                                spargebra::algebra::Function::StrStarts,
                                vec![
                                    SparExpr::FunctionCall(
                                        spargebra::algebra::Function::Str,
                                        vec![SparExpr::Variable(ltype_var.clone())],
                                    ),
                                    SparExpr::Literal(SparLit::new_simple_literal(base)),
                                ],
                            );
                            let inner_filtered = GraphPattern::Filter {
                                expr: filter_expr,
                                inner: Box::new(inner_bgp),
                            };
                            // Label name expression: single-quoted label string
                            //   CONCAT("'", SUBSTR(STR(?ltype), base_len + 1), "'")
                            let label_name = SparExpr::FunctionCall(
                                spargebra::algebra::Function::SubStr,
                                vec![
                                    SparExpr::FunctionCall(
                                        spargebra::algebra::Function::Str,
                                        vec![SparExpr::Variable(ltype_var.clone())],
                                    ),
                                    SparExpr::Literal(SparLit::new_typed_literal(
                                        (base_len + 1).to_string(),
                                        NamedNode::new_unchecked(XSD_INTEGER),
                                    )),
                                ],
                            );
                            let quoted_label = SparExpr::FunctionCall(
                                spargebra::algebra::Function::Concat,
                                vec![
                                    SparExpr::Literal(SparLit::new_simple_literal("'")),
                                    label_name,
                                    SparExpr::Literal(SparLit::new_simple_literal("'")),
                                ],
                            );
                            let gc_agg = AggregateExpression::FunctionCall {
                                name: AggregateFunction::GroupConcat {
                                    separator: Some(", ".to_string()),
                                },
                                expr: quoted_label,
                                distinct: true,
                            };
                            let group_pattern = GraphPattern::Group {
                                inner: Box::new(inner_filtered),
                                variables: vec![n_var],
                                aggregates: vec![(gc_var.clone(), gc_agg)],
                            };
                            self.pending_subqueries.push((gc_var.clone(), group_pattern));
                            // Result: IF(BOUND(?gc), CONCAT("[", ?gc, "]"), "[]")
                            let result_expr = SparExpr::If(
                                Box::new(SparExpr::Bound(gc_var.clone())),
                                Box::new(SparExpr::FunctionCall(
                                    spargebra::algebra::Function::Concat,
                                    vec![
                                        SparExpr::Literal(SparLit::new_simple_literal("[")),
                                        SparExpr::Variable(gc_var),
                                        SparExpr::Literal(SparLit::new_simple_literal("]")),
                                    ],
                                )),
                                Box::new(SparExpr::Literal(SparLit::new_simple_literal("[]"))),
                            );
                            return Ok(result_expr);
                        }
                    }
                    _ => {}
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: format!("function call: {name}()"),
                })
            }
            "properties" => {
                // properties(null) = null
                // properties(map_literal) = map_literal (identity for literal maps)
                // properties(nullable_var) = null (no graph data support; only null case)
                let arg = args
                    .first()
                    .ok_or_else(|| PolygraphError::UnsupportedFeature {
                        feature: "properties() requires an argument".to_string(),
                    })?;
                match arg {
                    Expression::Literal(Literal::Null) => {
                        return Ok(SparExpr::Variable(self.fresh_var("null")));
                    }
                    Expression::Map(pairs) => {
                        // properties({k: v}) = {k: v} itself — serialize as string.
                        let serialized = serialize_list_element(&Expression::Map(pairs.clone()));
                        return Ok(SparExpr::Literal(SparLit::new_simple_literal(serialized)));
                    }
                    Expression::Variable(v) => {
                        let vname = v.clone();
                        // If the variable is statically known to be null (via null_vars)
                        // or is nullable (from OPTIONAL MATCH that didn't match), return null.
                        if self.null_vars.contains(vname.as_str())
                            || self.nullable_vars.contains(vname.as_str())
                        {
                            return Ok(SparExpr::Variable(self.fresh_var("null")));
                        }
                        // If it's a known map alias, serialize it.
                        if let Some(key_map) = self.map_vars.get(&vname).cloned() {
                            let mut entries: Vec<String> = key_map
                                .into_iter()
                                .map(|(k, var)| format!("{k}: ?{}", var.as_str()))
                                .collect();
                            entries.sort();
                            let serialized = format!("{{{}}}", entries.join(", "));
                            return Ok(SparExpr::Literal(SparLit::new_simple_literal(serialized)));
                        }
                    }
                    _ => {}
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: format!("function call: {name}()"),
                })
            }
            "reverse" => {
                // reverse(string) — only supported for constant string literals.
                if let Some(Expression::Literal(Literal::String(s))) = args.first() {
                    let reversed: String = s.chars().rev().collect();
                    return Ok(SparExpr::Literal(SparLit::new_simple_literal(reversed)));
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: "reverse() on non-literal or non-string".to_string(),
                })
            }
            "split" => {
                // split(string, delimiter) — supported for constant string literals.
                // Returns a list string like ['a', 'b'] that can be used in UNWIND.
                if let (
                    Some(Expression::Literal(Literal::String(s))),
                    Some(Expression::Literal(Literal::String(delim))),
                ) = (args.first(), args.get(1))
                {
                    let parts: Vec<String> = if delim.is_empty() {
                        s.chars().map(|c| format!("'{c}'")).collect()
                    } else {
                        s.split(delim.as_str()).map(|p| format!("'{p}'")).collect()
                    };
                    let serialized = format!("[{}]", parts.join(", "));
                    return Ok(SparExpr::Literal(SparLit::new_simple_literal(serialized)));
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: "split() on non-literal arguments".to_string(),
                })
            }
            "sqrt" => {
                // SPARQL has no built-in sqrt; use compile-time constant folding for literals.
                if let Some(arg) = args.first() {
                    if let Some(f) = try_eval_to_float(arg) {
                        let result = f.sqrt();
                        if result.is_nan() || result.is_infinite() {
                            return Err(PolygraphError::UnsupportedFeature {
                                feature: "sqrt() of negative number".to_string(),
                            });
                        }
                        let lit = SparLit::new_typed_literal(
                            cypher_float_str(result),
                            NamedNode::new_unchecked(XSD_DOUBLE),
                        );
                        return Ok(SparExpr::Literal(lit));
                    }
                }
                // For non-constant arguments, use a custom function (like pow).
                if let Some(arg_expr) = args.first() {
                    let arg = self.translate_expr(arg_expr, extra)?;
                    return Ok(SparExpr::FunctionCall(
                        spargebra::algebra::Function::Custom(NamedNode::new_unchecked(
                            "urn:polygraph:sqrt",
                        )),
                        vec![arg],
                    ));
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: "sqrt() requires an argument".to_string(),
                })
            }
            "trim" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "trim() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                // REPLACE(REPLACE(s, leading_spaces, ""), trailing_spaces, "")
                // Use SPARQL REPLACE with regex
                let trimmed = SparExpr::FunctionCall(
                    Function::Replace,
                    vec![
                        arg,
                        SparExpr::Literal(SparLit::new_simple_literal("^\\s+|\\s+$")),
                        SparExpr::Literal(SparLit::new_simple_literal("")),
                    ],
                );
                Ok(trimmed)
            }
            "ltrim" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "ltrim() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(
                    Function::Replace,
                    vec![
                        arg,
                        SparExpr::Literal(SparLit::new_simple_literal("^\\s+")),
                        SparExpr::Literal(SparLit::new_simple_literal("")),
                    ],
                ))
            }
            "rtrim" => {
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "rtrim() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(
                    Function::Replace,
                    vec![
                        arg,
                        SparExpr::Literal(SparLit::new_simple_literal("\\s+$")),
                        SparExpr::Literal(SparLit::new_simple_literal("")),
                    ],
                ))
            }
            "left" => {
                // left(s, n) → SUBSTR(s, 1, n)
                let s_arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "left() requires arguments".to_string(),
                        })?,
                    extra,
                )?;
                let n_arg = self.translate_expr(
                    args.get(1)
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "left() requires 2 arguments".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(
                    Function::SubStr,
                    vec![
                        s_arg,
                        SparExpr::Literal(SparLit::new_typed_literal(
                            "1",
                            NamedNode::new_unchecked(XSD_INTEGER),
                        )),
                        n_arg,
                    ],
                ))
            }
            "right" => {
                // right(s, n) → SUBSTR(s, STRLEN(s) - n + 1, n)
                let s_arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "right() requires arguments".to_string(),
                        })?,
                    extra,
                )?;
                let n_arg = self.translate_expr(
                    args.get(1)
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "right() requires 2 arguments".to_string(),
                        })?,
                    extra,
                )?;
                // start = strlen(s) - n + 1
                let strlen = SparExpr::FunctionCall(Function::StrLen, vec![s_arg.clone()]);
                let offset = SparExpr::Add(
                    Box::new(SparExpr::Subtract(
                        Box::new(strlen),
                        Box::new(n_arg.clone()),
                    )),
                    Box::new(SparExpr::Literal(SparLit::new_typed_literal(
                        "1",
                        NamedNode::new_unchecked(XSD_INTEGER),
                    ))),
                );
                Ok(SparExpr::FunctionCall(
                    Function::SubStr,
                    vec![s_arg, offset, n_arg],
                ))
            }
            "toboolean" => {
                // toBoolean(v): identity for booleans, string-to-bool for strings,
                // null for invalid strings, error for non-string/non-bool.
                // SPARQL: xsd:boolean(STR(v)) — works for "true"/"false" strings and
                // boolean literals; produces error (→ null) for invalid strings.
                let arg = self.translate_expr(
                    args.first()
                        .ok_or_else(|| PolygraphError::UnsupportedFeature {
                            feature: "toBoolean() requires an argument".to_string(),
                        })?,
                    extra,
                )?;
                Ok(SparExpr::FunctionCall(
                    Function::Custom(NamedNode::new_unchecked(XSD_BOOLEAN)),
                    vec![SparExpr::FunctionCall(Function::Str, vec![arg])],
                ))
            }
            "range" => {
                // range(start, end [, step]) → list of integers.
                // Pre-evaluate when arguments can be resolved at compile time
                // (literal integers or variables tracked in const_int_vars).
                let start = args.first().and_then(|a| self.try_eval_to_int(a));
                let end_val = args.get(1).and_then(|a| self.try_eval_to_int(a));
                // Step: if provided and not resolvable to a constant integer, error.
                let step = if let Some(step_arg) = args.get(2) {
                    match self.try_eval_to_int(step_arg) {
                        Some(n) => n,
                        None => {
                            return Err(PolygraphError::Translation {
                                message: "range() step argument must be an integer".to_string(),
                            });
                        }
                    }
                } else {
                    1
                };
                if let (Some(s), Some(e)) = (start, end_val) {
                    if step == 0 {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: "range() with step=0".to_string(),
                        });
                    }
                    let mut items = Vec::new();
                    let mut i = s;
                    while (step > 0 && i <= e) || (step < 0 && i >= e) {
                        items.push(i.to_string());
                        i += step;
                    }
                    let serialized = format!("[{}]", items.join(", "));
                    Ok(SparExpr::Literal(SparLit::new_simple_literal(serialized)))
                } else {
                    Err(PolygraphError::UnsupportedFeature {
                        feature: "range() with non-literal arguments".to_string(),
                    })
                }
            }
            "last" => {
                // last(r) / head(r) on a varlen relationship variable (*lower..upper):
                // Emit two OPTIONAL triple patterns (forward and backward directions)
                // and return COALESCE(?__last_r_fwd, ?__last_r_bwd).
                // For bounded varlen (upper ≤ 1), this correctly returns the predicate
                // IRI of the relationship for 1-hop rows and null for 0-hop rows.
                if let Some(Expression::Variable(r_name)) = args.first() {
                    if let Some(&(lower, upper)) = self.varlen_rel_scope.get(r_name.as_str()) {
                        if upper <= 1 {
                            if let Some(edge) = self.edge_map.get(r_name.as_str()).cloned() {
                                use spargebra::term::NamedNodePattern;
                                let fwd_var = self.fresh_var("__last_r_fwd");
                                let bwd_var = self.fresh_var("__last_r_bwd");
                                // Forward direction: ?src ?fwd_var ?dst
                                extra.push(TriplePattern {
                                    subject: edge.src.clone(),
                                    predicate: NamedNodePattern::Variable(fwd_var.clone()),
                                    object: edge.dst.clone(),
                                });
                                // Backward direction: ?dst ?bwd_var ?src
                                extra.push(TriplePattern {
                                    subject: edge.dst.clone(),
                                    predicate: NamedNodePattern::Variable(bwd_var.clone()),
                                    object: edge.src.clone(),
                                });
                                // Return COALESCE(?fwd, ?bwd) – first non-null wins
                                let _ = lower; // bounds info used for eligibility check only
                                return Ok(SparExpr::Coalesce(vec![
                                    SparExpr::Variable(fwd_var),
                                    SparExpr::Variable(bwd_var),
                                ]));
                            }
                        }
                    }
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: format!("function call: {name}()"),
                })
            }
            // ── Temporal constructors ──────────────────────────────────────────
            // Produce typed (or plain) literals from map or string arguments.
            // All calendar arithmetic is performed at translation time.
            "date" | "localtime" | "localdatetime" | "time" | "datetime" | "duration"
            // .transaction/.statement/.realtime variants return current temporal or null.
            | "date.transaction" | "date.statement" | "date.realtime"
            | "localtime.transaction" | "localtime.statement" | "localtime.realtime"
            | "time.transaction" | "time.statement" | "time.realtime"
            | "localdatetime.transaction" | "localdatetime.statement" | "localdatetime.realtime"
            | "datetime.transaction" | "datetime.statement" | "datetime.realtime" => {
                // Zero-arg form: return a deterministic fixed timestamp.
                // Two calls to the same constructor with no args in the same query
                // will produce the same literal, so duration(v, v) = PT0S correctly.
                if args.is_empty() {
                    let xsd_date_nn = NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#date");
                    let xsd_time_nn = NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#time");
                    let xsd_dt_nn = NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#dateTime");
                    // Strip variant suffix to get the base function name.
                    let base_for_zero = name_lower
                        .strip_suffix(".transaction")
                        .or_else(|| name_lower.strip_suffix(".statement"))
                        .or_else(|| name_lower.strip_suffix(".realtime"))
                        .unwrap_or(name_lower.as_str());
                    let lit = match base_for_zero {
                        "date" => SparLit::new_typed_literal("2000-01-01".to_owned(), xsd_date_nn),
                        "localtime" => SparLit::new_typed_literal("00:00:00".to_owned(), xsd_time_nn.clone()),
                        "time" => SparLit::new_typed_literal("00:00:00Z".to_owned(), xsd_time_nn),
                        "localdatetime" => SparLit::new_typed_literal("2000-01-01T00:00:00".to_owned(), xsd_dt_nn.clone()),
                        "datetime" => SparLit::new_typed_literal("2000-01-01T00:00Z".to_owned(), xsd_dt_nn),
                        "duration" => SparLit::new_simple_literal("PT0S".to_owned()),
                        _ => return Ok(SparExpr::Variable(self.fresh_var("null"))),
                    };
                    return Ok(SparExpr::Literal(lit));
                }
                // Null propagation: func(null) → null.
                if let Some(Expression::Literal(Literal::Null)) = args.first() {
                    return Ok(SparExpr::Variable(self.fresh_var("null")));
                }

                // Fold: temporal_f(v) where v is a known WITH-bound literal.
                // e.g. `WITH date({year:1984,...}) AS d … date(d)` → the same literal.
                if let Some(Expression::Variable(v)) = args.first() {
                    if let Some(s) = self.with_lit_vars.get(v.as_str()).cloned() {
                        // For time(): strip the named timezone bracket since time()
                        // values only carry numeric offsets (no [Region/City]).
                        let effective_s = if name_lower == "time" {
                            temporal_parse_time(&s)
                                .map(|t| strip_named_tz(&t))
                                .unwrap_or(s.clone())
                        } else {
                            s.clone()
                        };
                        let folded = vec![Expression::Literal(Literal::String(effective_s))];
                        return self.translate_function_call(name, &folded, extra);
                    }
                }
                // Fold: temporal_f(toString(v)) where v is a known WITH-bound literal.
                // e.g. `date(toString(d))` → `date(lit_str)` → same literal as d.
                if let Some(Expression::FunctionCall {
                    name: fname,
                    args: fargs,
                    ..
                }) = args.first()
                {
                    if fname.eq_ignore_ascii_case("toString") || fname.eq_ignore_ascii_case("str") {
                        if let Some(Expression::Variable(v)) = fargs.first() {
                            if let Some(s) = self.with_lit_vars.get(v.as_str()).cloned() {
                                let folded = vec![Expression::Literal(Literal::String(s))];
                                return self.translate_function_call(name, &folded, extra);
                            }
                        }
                    }
                }

                // Strip the .transaction/.statement/.realtime suffix for dispatch.
                let base_func = name_lower
                    .strip_suffix(".transaction")
                    .or_else(|| name_lower.strip_suffix(".statement"))
                    .or_else(|| name_lower.strip_suffix(".realtime"))
                    .unwrap_or(name_lower.as_str());

                let xsd_date = NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#date");
                let xsd_time = NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#time");
                let xsd_dt =
                    NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#dateTime");
                let _xsd_dur =
                    NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#duration");

                // Map argument: date({year: …, month: …}) etc.
                // First, expand any Variable refs in map values using with_lit_vars.
                let expanded_map_arg: Option<Expression> = if let Some(Expression::Map(pairs)) = args.first() {
                    let mut changed = false;
                    let expanded: Vec<(String, Expression)> = pairs.iter().map(|(k, v)| {
                        if let Expression::Variable(var) = v {
                            if let Some(s) = self.with_lit_vars.get(var.as_str()) {
                                changed = true;
                                return (k.clone(), Expression::Literal(Literal::String(s.clone())));
                            }
                        }
                        (k.clone(), v.clone())
                    }).collect();
                    if changed { Some(Expression::Map(expanded)) } else { None }
                } else {
                    None
                };
                let effective_args: &[Expression] = if expanded_map_arg.is_some() {
                    // Use the expanded arg (will be picked up below).
                    &[]  // trigger fallthrough; we handle below
                } else {
                    args
                };
                // If we have an expanded map, recurse with it.
                if let Some(ref expanded) = expanded_map_arg {
                    let new_args = vec![expanded.clone()];
                    return self.translate_function_call(name, &new_args, extra);
                }
                let _ = effective_args; // not used after expansion path

                if let Some(Expression::Map(pairs)) = args.first() {
                    let lit_opt: Option<SparLit> = match base_func {
                        "date" => temporal_date_from_map(pairs)
                            .map(|s| SparLit::new_typed_literal(s, xsd_date.clone())),
                        "localtime" => temporal_localtime_from_map(pairs)
                            .map(|s| SparLit::new_typed_literal(s, xsd_time.clone())),
                        "time" => temporal_time_from_map(pairs)
                            .map(|s| SparLit::new_typed_literal(s, xsd_time.clone())),
                        "localdatetime" => temporal_localdatetime_from_map(pairs)
                            .map(|s| SparLit::new_typed_literal(s, xsd_dt.clone())),
                        "datetime" => temporal_datetime_from_map(pairs)
                            .map(|s| SparLit::new_typed_literal(s, xsd_dt.clone())),
                        "duration" => temporal_duration_from_map(pairs)
                            .map(SparLit::new_simple_literal),
                        _ => None,
                    };
                    if let Some(lit) = lit_opt {
                        return Ok(SparExpr::Literal(lit));
                    }
                // String argument: date('2015-07-21') etc.
                } else if let Some(Expression::Literal(Literal::String(s))) = args.first() {
                    let lit_opt: Option<SparLit> = match base_func {
                        "date" => temporal_parse_date(s)
                            .map(|v| SparLit::new_typed_literal(v, xsd_date)),
                        "localtime" => temporal_parse_localtime(s)
                            .map(|v| SparLit::new_typed_literal(v, xsd_time.clone())),
                        "time" => temporal_parse_time(s)
                            .map(|v| SparLit::new_typed_literal(v, xsd_time)),
                        "localdatetime" => temporal_parse_localdatetime(s)
                            .map(|v| SparLit::new_typed_literal(v, xsd_dt.clone())),
                        "datetime" => temporal_parse_datetime(s)
                            .map(|v| SparLit::new_typed_literal(v, xsd_dt)),
                        "duration" => temporal_parse_duration(s)
                            .map(SparLit::new_simple_literal),
                        _ => None,
                    };
                    if let Some(lit) = lit_opt {
                        return Ok(SparExpr::Literal(lit));
                    }
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: format!("function call: {name}() with unsupported arguments"),
                })
            }
            // ── Temporal truncation functions ─────────────────────────────
            "date.truncate"
            | "datetime.truncate"
            | "localdatetime.truncate"
            | "localtime.truncate"
            | "time.truncate" => {
                if args.len() < 3 {
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: format!("{name}() requires 3 arguments"),
                    });
                }
                let unit = match &args[0] {
                    Expression::Literal(Literal::String(s)) => s.clone(),
                    _ => {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: format!("{name}() unit must be a string literal"),
                        })
                    }
                };
                let other_expr = &args[1];
                let overrides: &[(String, Expression)] = match &args[2] {
                    Expression::Map(kvs) => kvs,
                    _ => {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: format!("{name}() map argument must be a literal map"),
                        })
                    }
                };

                let mut comps = tc_from_expr(other_expr).ok_or_else(|| {
                    PolygraphError::UnsupportedFeature {
                        feature: format!("{name}() with non-literal 'other' argument"),
                    }
                })?;

                tc_apply_truncation(&unit, &mut comps);
                tc_apply_overrides(overrides, &mut comps);

                let _is_time_unit = matches!(
                    unit.as_str(),
                    "hour"
                        | "minute"
                        | "second"
                        | "millisecond"
                        | "microsecond"
                        | "nanosecond"
                );

                let xsd_date_t =
                    NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#date");
                let xsd_dt =
                    NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#dateTime");
                let xsd_time =
                    NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#time");

                let lit: SparLit = match name_lower.as_str() {
                    "date.truncate" => {
                        let y = comps.year.unwrap_or(0);
                        let m = comps.month.unwrap_or(1);
                        let d = comps.day.unwrap_or(1);
                        SparLit::new_typed_literal(format!("{y:04}-{m:02}-{d:02}"), xsd_date_t)
                    }
                    "datetime.truncate" => {
                        let y = comps.year.unwrap_or(0);
                        let m = comps.month.unwrap_or(1);
                        let d = comps.day.unwrap_or(1);
                        let h = comps.hour.unwrap_or(0);
                        let min = comps.minute.unwrap_or(0);
                        let sec = comps.second.unwrap_or(0);
                        let ns = comps.ns.unwrap_or(0);
                        let time_part = tc_fmt_time(h, min, sec, ns);
                        let tz = comps.tz.as_deref().unwrap_or("Z");
                        SparLit::new_typed_literal(
                            format!("{y:04}-{m:02}-{d:02}T{time_part}{tz}"),
                            xsd_dt.clone(),
                        )
                    }
                    "localdatetime.truncate" => {
                        let y = comps.year.unwrap_or(0);
                        let m = comps.month.unwrap_or(1);
                        let d = comps.day.unwrap_or(1);
                        let h = comps.hour.unwrap_or(0);
                        let min = comps.minute.unwrap_or(0);
                        let sec = comps.second.unwrap_or(0);
                        let ns = comps.ns.unwrap_or(0);
                        let time_part = tc_fmt_time(h, min, sec, ns);
                        SparLit::new_typed_literal(
                            format!("{y:04}-{m:02}-{d:02}T{time_part}"),
                            xsd_dt,
                        )
                    }
                    "localtime.truncate" => {
                        let h = comps.hour.unwrap_or(0);
                        let min = comps.minute.unwrap_or(0);
                        let sec = comps.second.unwrap_or(0);
                        let ns = comps.ns.unwrap_or(0);
                        SparLit::new_typed_literal(tc_fmt_time(h, min, sec, ns), xsd_time.clone())
                    }
                    "time.truncate" => {
                        let h = comps.hour.unwrap_or(0);
                        let min = comps.minute.unwrap_or(0);
                        let sec = comps.second.unwrap_or(0);
                        let ns = comps.ns.unwrap_or(0);
                        let time_part = tc_fmt_time(h, min, sec, ns);
                        let tz = comps.tz.as_deref().unwrap_or("Z");
                        SparLit::new_typed_literal(
                            format!("{time_part}{tz}"),
                            xsd_time,
                        )
                    }
                    _ => unreachable!(),
                };
                Ok(SparExpr::Literal(lit))
            }

            // ── Duration between two temporal values ──────────────────────────
            "duration.between" | "duration.inmonths" | "duration.indays"
            | "duration.inseconds" => {
                // Null propagation: if either arg is null, return null.
                if args.len() < 2 {
                    return Ok(SparExpr::Variable(self.fresh_var("null")));
                }
                if matches!(&args[0], Expression::Literal(Literal::Null))
                    || matches!(&args[1], Expression::Literal(Literal::Null))
                {
                    return Ok(SparExpr::Variable(self.fresh_var("null")));
                }

                // Evaluate both arguments to SPARQL literals.
                let lhs_expr = self.translate_expr(&args[0], extra);
                let rhs_expr = self.translate_expr(&args[1], extra);
                let (lhs_str, rhs_str) = match (lhs_expr, rhs_expr) {
                    (Ok(SparExpr::Literal(l)), Ok(SparExpr::Literal(r))) => {
                        (l.value().to_string(), r.value().to_string())
                    }
                    _ => {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: format!(
                                "duration function {name}() with non-literal arguments"
                            ),
                        })
                    }
                };
                let result_opt = match name_lower.as_str() {
                    "duration.between" => temporal_duration_between(&lhs_str, &rhs_str),
                    "duration.inmonths" => temporal_duration_in_months(&lhs_str, &rhs_str),
                    "duration.indays" => temporal_duration_in_days(&lhs_str, &rhs_str),
                    "duration.inseconds" => temporal_duration_in_seconds(&lhs_str, &rhs_str),
                    _ => unreachable!(),
                };
                // Return as a plain string literal so Oxigraph doesn't
                // normalize the duration representation (e.g. PT269112H → P11213D).
                match result_opt {
                    Some(s) => Ok(SparExpr::Literal(SparLit::new_simple_literal(s))),
                    None => Ok(SparExpr::Variable(self.fresh_var("null"))),
                }
            }

            // ── datetime.fromepoch / datetime.fromepochmillis ─────────────────
            "datetime.fromepoch" => {
                // datetime.fromepoch(seconds, nanoseconds)
                // Both arguments must be integer literals for compile-time evaluation.
                let (sec_expr, ns_expr) = match (args.first(), args.get(1)) {
                    (Some(a), Some(b)) => (a, b),
                    _ => {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: "datetime.fromepoch() requires two arguments".to_string(),
                        })
                    }
                };
                let epoch_seconds = match get_literal_int(sec_expr) {
                    Some(v) => v,
                    None => {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: "datetime.fromepoch() with non-literal seconds".to_string(),
                        })
                    }
                };
                let nanoseconds = match get_literal_int(ns_expr) {
                    Some(v) if v >= 0 && v <= 999_999_999 => v as u32,
                    _ => {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: "datetime.fromepoch() with non-literal or out-of-range nanoseconds"
                                .to_string(),
                        })
                    }
                };
                let s = temporal_fromepoch_to_str(epoch_seconds, nanoseconds);
                // Use a plain string literal to avoid Oxigraph nanosecond normalization.
                Ok(SparExpr::Literal(SparLit::new_simple_literal(s)))
            }

            "datetime.fromepochmillis" => {
                // datetime.fromepochmillis(milliseconds)
                let millis = match args.first().and_then(get_literal_int) {
                    Some(v) => v,
                    None => {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: "datetime.fromepochmillis() with non-literal argument"
                                .to_string(),
                        })
                    }
                };
                let epoch_seconds = millis / 1000;
                let remaining_ms = millis % 1000;
                let (epoch_seconds, nanoseconds) = if remaining_ms < 0 {
                    (epoch_seconds - 1, ((remaining_ms + 1000) * 1_000_000) as u32)
                } else {
                    (epoch_seconds, (remaining_ms * 1_000_000) as u32)
                };
                let xsd_dt = NamedNode::new_unchecked(
                    "http://www.w3.org/2001/XMLSchema#dateTime",
                );
                let s = temporal_fromepoch_to_str(epoch_seconds, nanoseconds);
                Ok(SparExpr::Literal(SparLit::new_typed_literal(s, xsd_dt)))
            }

            _ => Err(PolygraphError::UnsupportedFeature {
                feature: format!("function call: {name}()"),
            }),
        }
    }

    // ── Aggregate translation ─────────────────────────────────────────────────

}
