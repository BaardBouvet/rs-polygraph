/// openCypher → SPARQL algebra translator.
///
/// Implements the [`AstVisitor`] pattern to walk a [`CypherQuery`] AST and
/// emit a [`spargebra::Query`] (serializable to standard SPARQL 1.1 text).
///
/// # RDF mapping strategy (Phase 2)
///
/// | Cypher construct          | SPARQL mapping                          |
/// |---------------------------|-----------------------------------------|
/// | `(n:Label)`               | `?n rdf:type <base:Label>`              |
/// | `(n {prop: val})`         | `?n <base:prop> val` (literal in BGP)   |
/// | `(a)-[:REL]->(b)`         | `?a <base:REL> ?b`                      |
/// | `WHERE n.prop op val`     | fresh var `?_n_prop_N` + `FILTER`       |
/// | `RETURN n.prop`           | fresh var `?_n_prop_N` projected        |
/// | `RETURN n.prop AS alias`  | `?alias` variable projected             |
/// | `OPTIONAL MATCH`          | `OPTIONAL { }` / `LeftJoin`             |
/// | `WITH … WHERE`            | `FILTER` applied to current pattern     |
/// | `RETURN DISTINCT`         | `DISTINCT` wrapper                      |
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression as SparExpr, GraphPattern, OrderExpression,
};
use spargebra::term::{
    GroundTerm, Literal as SparLit, NamedNode, TermPattern, TriplePattern, Variable,
};
use spargebra::Query;

use crate::rdf_mapping;

use crate::ast::cypher::{
    AggregateExpr, Clause, CompOp, CypherQuery, Expression, Literal, MatchClause, NodePattern,
    Pattern, PatternElement, PatternList, RelationshipPattern, ReturnClause, ReturnItem,
    ReturnItems,
};
use crate::error::PolygraphError;

// ── Well-known IRIs ───────────────────────────────────────────────────────────

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
const DEFAULT_BASE: &str = "http://polygraph.example/";

// ── Public API ────────────────────────────────────────────────────────────────

/// Translates an openCypher [`CypherQuery`] AST into a SPARQL 1.1 query string.
///
/// * `base_iri` — namespace IRI for labels, relationship types and property
///   names. Pass `None` to use `http://polygraph.example/`.
/// * `rdf_star` — when `true`, emit SPARQL-star annotated triple patterns for
///   relationship properties; when `false`, use standard RDF reification.
pub fn translate(
    query: &CypherQuery,
    base_iri: Option<&str>,
    rdf_star: bool,
) -> Result<String, PolygraphError> {
    let base = base_iri.unwrap_or(DEFAULT_BASE).to_string();
    let mut state = TranslationState::new(base, rdf_star);
    let pattern = state.translate_query(query)?;
    let sparql_query = Query::Select {
        dataset: None,
        pattern,
        base_iri: None,
    };
    Ok(sparql_query.to_string())
}

// ── Translation state ─────────────────────────────────────────────────────────

/// Info stored per relationship variable for property access resolution.
#[derive(Clone)]
struct EdgeInfo {
    src: TermPattern,
    pred: NamedNode,
    dst: TermPattern,
    /// In reification mode: the fresh variable used as the reification node.
    reif_var: Option<Variable>,
}

struct TranslationState {
    base_iri: String,
    counter: usize,
    /// Use SPARQL-star annotated triples (true) or RDF reification (false).
    rdf_star: bool,
    /// Tracks relationship variables → edge info for `r.prop` resolution.
    edge_map: std::collections::HashMap<String, EdgeInfo>,
}

impl TranslationState {
    fn new(base_iri: String, rdf_star: bool) -> Self {
        Self {
            base_iri,
            counter: 0,
            rdf_star,
            edge_map: Default::default(),
        }
    }

    /// Allocate a fresh SPARQL variable.
    fn fresh_var(&mut self, hint: &str) -> Variable {
        let n = self.counter;
        self.counter += 1;
        Variable::new_unchecked(format!("__{hint}_{n}"))
    }

    /// Build a `<base:local>` IRI.
    fn iri(&self, local: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("{}{}", self.base_iri, local))
    }

    /// Build the `rdf:type` predicate IRI.
    fn rdf_type(&self) -> NamedNode {
        NamedNode::new_unchecked(RDF_TYPE)
    }

    // ── Top-level query translation ──────────────────────────────────────────

    fn translate_query(&mut self, query: &CypherQuery) -> Result<GraphPattern, PolygraphError> {
        // Accumulate extra BGP triples emitted during expression translation.
        let mut extra_triples: Vec<TriplePattern> = Vec::new();
        // The pattern is built left-to-right over clauses.
        let mut current = empty_bgp();
        // Collects filters to apply at the end of each scope.
        let mut pending_filters: Vec<SparExpr> = Vec::new();

        for clause in &query.clauses {
            match clause {
                Clause::Match(m) => {
                    let (match_pattern, opt_filter) =
                        self.translate_match_clause(m, &mut extra_triples)?;
                    if m.optional {
                        current = GraphPattern::LeftJoin {
                            left: Box::new(current),
                            right: Box::new(match_pattern),
                            expression: opt_filter,
                        };
                    } else {
                        current = join_patterns(current, match_pattern);
                        if let Some(f) = opt_filter {
                            pending_filters.push(f);
                        }
                    }
                }
                Clause::With(w) => {
                    // Flush any pending extra triples.
                    if !extra_triples.is_empty() {
                        let extra = GraphPattern::Bgp {
                            patterns: extra_triples.drain(..).collect(),
                        };
                        current = join_patterns(current, extra);
                    }
                    // Flush pending filters.
                    current = apply_filters(current, pending_filters.drain(..));
                    // Apply ORDER BY / SKIP / LIMIT from WITH clause.
                    current = self.apply_order_skip_limit(
                        current,
                        w.order_by.as_ref(),
                        w.skip.as_ref(),
                        w.limit.as_ref(),
                        &mut extra_triples,
                    )?;
                    // Translate WITH's WHERE if present.
                    if let Some(wc) = &w.where_ {
                        let filter_expr =
                            self.translate_expr(&wc.expression, &mut extra_triples)?;
                        pending_filters.push(filter_expr);
                    }
                    // WITH items: narrowing — let variables flow through.
                }
                Clause::Return(r) => {
                    // Flush pending extra triples before projection.
                    if !extra_triples.is_empty() {
                        let extra = GraphPattern::Bgp {
                            patterns: extra_triples.drain(..).collect(),
                        };
                        current = join_patterns(current, extra);
                    }
                    // Flush pending filters.
                    current = apply_filters(current, pending_filters.drain(..));

                    let (return_triples, project_vars, need_distinct, aggregates) =
                        self.translate_return_clause(r, &mut extra_triples)?;

                    if !return_triples.is_empty() {
                        let extra = GraphPattern::Bgp {
                            patterns: return_triples,
                        };
                        current = join_patterns(current, extra);
                    }
                    // Flush any triples added during return expression translation.
                    if !extra_triples.is_empty() {
                        let extra = GraphPattern::Bgp {
                            patterns: extra_triples.drain(..).collect(),
                        };
                        current = join_patterns(current, extra);
                    }

                    // Apply aggregation (GROUP BY) if present.
                    if !aggregates.is_empty() {
                        // Group variables = all projected non-aggregate vars.
                        let group_vars: Vec<Variable> = project_vars
                            .as_ref()
                            .map(|vs| {
                                vs.iter()
                                    .filter(|v| !aggregates.iter().any(|(av, _)| av == *v))
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

                    if let Some(vars) = project_vars {
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
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: format!(
                            "CREATE clause (SPARQL Update, Phase 4+): {} pattern(s)",
                            c.pattern.0.len()
                        ),
                    });
                }
                Clause::Merge(m) => {
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: format!(
                            "MERGE clause (SPARQL Update, Phase 4+): {}",
                            m.pattern.variable.as_deref().unwrap_or("anon")
                        ),
                    });
                }
                Clause::Set(s) => {
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: format!(
                            "SET clause (SPARQL Update, Phase 4+): {} item(s)",
                            s.items.len()
                        ),
                    });
                }
                Clause::Delete(d) => {
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: format!(
                            "{} clause (SPARQL Update, Phase 4+): {} expression(s)",
                            if d.detach { "DETACH DELETE" } else { "DELETE" },
                            d.expressions.len()
                        ),
                    });
                }
                Clause::Remove(r) => {
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: format!(
                            "REMOVE clause (SPARQL Update, Phase 4+): {} item(s)",
                            r.items.len()
                        ),
                    });
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
            }
        }

        // Final flush (in case no RETURN clause was present).
        if !extra_triples.is_empty() {
            let extra = GraphPattern::Bgp {
                patterns: extra_triples,
            };
            current = join_patterns(current, extra);
        }
        current = apply_filters(current, pending_filters.into_iter());

        Ok(current)
    }

    // ── MATCH clause ─────────────────────────────────────────────────────────

    /// Translate a `MATCH` or `OPTIONAL MATCH` clause into a graph pattern plus
    /// an optional filter expression (from the inline `WHERE`).
    fn translate_match_clause(
        &mut self,
        m: &MatchClause,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<(GraphPattern, Option<SparExpr>), PolygraphError> {
        let mut triples: Vec<TriplePattern> = Vec::new();
        let mut path_patterns: Vec<GraphPattern> = Vec::new();
        self.translate_pattern_list(&m.pattern, &mut triples, &mut path_patterns)?;

        // Combine BGP triples + path patterns into a single graph pattern.
        let bgp = GraphPattern::Bgp { patterns: triples };
        let combined = path_patterns.into_iter().fold(bgp, join_patterns);

        let filter = if let Some(wc) = &m.where_ {
            Some(self.translate_expr(&wc.expression, extra)?)
        } else {
            None
        };

        Ok((combined, filter))
    }

    // ── Pattern translation ───────────────────────────────────────────────────

    fn translate_pattern_list(
        &mut self,
        list: &PatternList,
        triples: &mut Vec<TriplePattern>,
        path_patterns: &mut Vec<GraphPattern>,
    ) -> Result<(), PolygraphError> {
        for pattern in &list.0 {
            self.translate_pattern(pattern, triples, path_patterns)?;
        }
        Ok(())
    }

    fn translate_pattern(
        &mut self,
        pattern: &Pattern,
        triples: &mut Vec<TriplePattern>,
        path_patterns: &mut Vec<GraphPattern>,
    ) -> Result<(), PolygraphError> {
        // Walk the element list: [Node, Rel, Node, Rel, Node, …]
        // We need to pair each Relationship with its surrounding nodes.
        let elements = &pattern.elements;
        let mut i = 0;
        while i < elements.len() {
            match &elements[i] {
                PatternElement::Node(n) => {
                    self.translate_node_pattern(n, triples)?;
                    i += 1;
                }
                PatternElement::Relationship(r) => {
                    let src = node_var_at(elements, i.wrapping_sub(1));
                    let dst = node_var_at(elements, i + 1);
                    self.translate_relationship_pattern(r, &src, &dst, triples, path_patterns)?;
                    i += 1;
                }
            }
        }
        Ok(())
    }

    fn translate_node_pattern(
        &mut self,
        node: &NodePattern,
        triples: &mut Vec<TriplePattern>,
    ) -> Result<(), PolygraphError> {
        let node_var = node_term(node);

        // One triple per label: `?n rdf:type <base:Label>`
        for label in &node.labels {
            triples.push(TriplePattern {
                subject: node_var.clone(),
                predicate: self.rdf_type().into(),
                object: self.iri(label).into(),
            });
        }

        // Inline properties: `?n <base:prop> <literal>`
        if let Some(props) = &node.properties {
            for (key, val_expr) in props {
                let obj = self.expr_to_ground_term(val_expr)?;
                triples.push(TriplePattern {
                    subject: node_var.clone(),
                    predicate: self.iri(key).into(),
                    object: obj,
                });
            }
        }

        Ok(())
    }

    fn translate_relationship_pattern(
        &mut self,
        rel: &RelationshipPattern,
        src: &TermPattern,
        dst: &TermPattern,
        triples: &mut Vec<TriplePattern>,
        path_patterns: &mut Vec<GraphPattern>,
    ) -> Result<(), PolygraphError> {
        use crate::ast::cypher::Direction;
        use spargebra::algebra::PropertyPathExpression as PPE;

        // Build predicate: NamedNode or PropertyPath with |/*/+/?
        let has_range = rel.range.is_some();
        let multi_type = rel.rel_types.len() > 1;

        if rel.rel_types.is_empty() {
            // No type constraint: emit an anonymous predicate variable (no path).
            let pred_var = match &rel.variable {
                Some(v) => Variable::new_unchecked(format!("{}_pred", v)),
                None => self.fresh_var("rel"),
            };
            let pred_term: spargebra::term::NamedNodePattern = pred_var.into();
            match rel.direction {
                Direction::Left => triples.push(TriplePattern {
                    subject: dst.clone(),
                    predicate: pred_term,
                    object: src.clone(),
                }),
                _ => triples.push(TriplePattern {
                    subject: src.clone(),
                    predicate: pred_term,
                    object: dst.clone(),
                }),
            }
            return Ok(());
        }

        // Build base path expression (single type or Alternative for multiple).
        let base_ppe: PPE = if multi_type {
            let types: Vec<PPE> = rel
                .rel_types
                .iter()
                .map(|t| PPE::from(self.iri(t)))
                .collect();
            types
                .into_iter()
                .reduce(|a, b| PPE::Alternative(Box::new(a), Box::new(b)))
                .expect("at least 2 types")
        } else {
            PPE::from(self.iri(&rel.rel_types[0]))
        };

        // Apply range quantifier if present.
        let ppe: Option<PPE> = if has_range {
            let range = rel.range.as_ref().unwrap();
            let q = match (range.lower, range.upper) {
                // * or *0..
                (None, None) | (Some(0), None) => PPE::ZeroOrMore(Box::new(base_ppe)),
                // *1..
                (Some(1), None) => PPE::OneOrMore(Box::new(base_ppe)),
                // *0..1 or *..1
                (None, Some(1)) | (Some(0), Some(1)) => PPE::ZeroOrOne(Box::new(base_ppe)),
                // *1..1 = exact 1 hop, treat as simple triple
                (Some(1), Some(1)) => {
                    // Emit as regular triple (no range modifier).
                    let pred = self.iri(&rel.rel_types[0]);
                    self.emit_edge_triple(rel, src, dst, pred, triples, path_patterns)?;
                    return Ok(());
                }
                // Bounded ranges like *2..5 — not supported by SPARQL 1.1 property paths.
                (lo, hi) => {
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: format!(
                            "bounded variable-length path *{lo:?}..{hi:?}: SPARQL 1.1 property paths do not support bounded ranges"
                        ),
                    });
                }
            };
            Some(q)
        } else {
            None
        };

        if let Some(path) = ppe {
            // Emit a GraphPattern::Path
            let (subj, obj) = match rel.direction {
                Direction::Left => (dst.clone(), src.clone()),
                _ => (src.clone(), dst.clone()),
            };
            let path = if rel.direction == Direction::Left {
                PPE::Reverse(Box::new(path))
            } else {
                path
            };
            path_patterns.push(GraphPattern::Path {
                subject: subj,
                path,
                object: obj,
            });
            // Register edge variable in edge_map (no inline properties on path patterns).
            if let Some(ref var_name) = rel.variable {
                let pred = self.iri(&rel.rel_types[0]);
                self.edge_map.insert(
                    var_name.clone(),
                    EdgeInfo {
                        src: src.clone(),
                        pred,
                        dst: dst.clone(),
                        reif_var: None,
                    },
                );
            }
            Ok(())
        } else {
            // No range quantifier: single or multi-type, treat as plain triple.
            let pred = self.iri(&rel.rel_types[0]);
            if multi_type {
                // Multi-type without range: use property path Alternative.
                let types: Vec<PPE> = rel
                    .rel_types
                    .iter()
                    .map(|t| PPE::from(self.iri(t)))
                    .collect();
                let path = types
                    .into_iter()
                    .reduce(|a, b| PPE::Alternative(Box::new(a), Box::new(b)))
                    .expect("multi_type has ≥2 types");
                let (subj, obj) = match rel.direction {
                    Direction::Left => (dst.clone(), src.clone()),
                    _ => (src.clone(), dst.clone()),
                };
                path_patterns.push(GraphPattern::Path {
                    subject: subj,
                    path,
                    object: obj,
                });
                if let Some(ref var_name) = rel.variable {
                    self.edge_map.insert(
                        var_name.clone(),
                        EdgeInfo {
                            src: src.clone(),
                            pred,
                            dst: dst.clone(),
                            reif_var: None,
                        },
                    );
                }
                Ok(())
            } else {
                self.emit_edge_triple(rel, src, dst, pred, triples, path_patterns)
            }
        }
    }

    /// Emit a single-hop edge triple (no path expression) and handle inline properties.
    fn emit_edge_triple(
        &mut self,
        rel: &RelationshipPattern,
        src: &TermPattern,
        dst: &TermPattern,
        pred: NamedNode,
        triples: &mut Vec<TriplePattern>,
        _path_patterns: &mut Vec<GraphPattern>,
    ) -> Result<(), PolygraphError> {
        use crate::ast::cypher::Direction;

        match rel.direction {
            Direction::Left => triples.push(TriplePattern {
                subject: dst.clone(),
                predicate: pred.clone().into(),
                object: src.clone(),
            }),
            _ => triples.push(TriplePattern {
                subject: src.clone(),
                predicate: pred.clone().into(),
                object: dst.clone(),
            }),
        }

        // Register edge info for later `r.prop` resolution.
        if let Some(ref var_name) = rel.variable {
            let reif_var = if self.rdf_star {
                None
            } else {
                Some(self.fresh_var(&format!("reif_{var_name}")))
            };
            self.edge_map.insert(
                var_name.clone(),
                EdgeInfo {
                    src: src.clone(),
                    pred: pred.clone(),
                    dst: dst.clone(),
                    reif_var,
                },
            );
        }

        // Inline relationship properties.
        if let Some(ref props) = rel.properties {
            if !props.is_empty() {
                let mut prop_pairs: Vec<(NamedNode, TermPattern)> = Vec::new();
                for (key, val_expr) in props {
                    let obj = self.expr_to_ground_term(val_expr)?;
                    prop_pairs.push((self.iri(key), obj));
                }

                if self.rdf_star {
                    let extra = rdf_mapping::rdf_star::all_property_triples(
                        src.clone(),
                        pred.clone(),
                        dst.clone(),
                        &prop_pairs,
                    );
                    triples.extend(extra);
                } else {
                    let reif_var = rel
                        .variable
                        .as_ref()
                        .and_then(|v| self.edge_map.get(v))
                        .and_then(|ei| ei.reif_var.clone())
                        .unwrap_or_else(|| self.fresh_var("reif"));
                    let extra = rdf_mapping::reification::all_triples(
                        &reif_var,
                        src.clone(),
                        pred.clone(),
                        dst.clone(),
                        &prop_pairs,
                    );
                    triples.extend(extra);
                }
            }
        }

        Ok(())
    }

    // ── RETURN clause ─────────────────────────────────────────────────────────

    /// Returns `(extra_bgp_triples, Some(projected_vars) | None for *, distinct_flag, aggregates)`.
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
        ),
        PolygraphError,
    > {
        match &ret.items {
            ReturnItems::All => Ok((vec![], None, ret.distinct, vec![])),
            ReturnItems::Explicit(items) => {
                let mut triples = Vec::new();
                let mut vars = Vec::new();
                let mut aggregates: Vec<(Variable, AggregateExpression)> = Vec::new();
                for item in items {
                    let (var, agg_opt) = self.translate_return_item(item, &mut triples, extra)?;
                    vars.push(var.clone());
                    if let Some(agg) = agg_opt {
                        aggregates.push((var, agg));
                    }
                }
                Ok((triples, Some(vars), ret.distinct, aggregates))
            }
        }
    }

    fn translate_return_item(
        &mut self,
        item: &ReturnItem,
        triples: &mut Vec<TriplePattern>,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<(Variable, Option<AggregateExpression>), PolygraphError> {
        match &item.expression {
            Expression::Variable(name) => {
                let var = Variable::new_unchecked(name.clone());
                if let Some(alias) = &item.alias {
                    let _ = var;
                    Ok((Variable::new_unchecked(alias.clone()), None))
                } else {
                    Ok((var, None))
                }
            }
            Expression::Property(base_expr, key) => {
                // n.prop or r.prop [AS alias] → add BGP triple + projected var.
                let base_var = self.extract_variable(base_expr)?;
                let var_name = base_var.as_str().to_string();
                let result_var = match &item.alias {
                    Some(alias) => Variable::new_unchecked(alias.clone()),
                    None => self.fresh_var(&format!("{}_{}", var_name, key)),
                };
                // Check whether base_var is a relationship variable.
                if let Some(edge) = self.edge_map.get(&var_name).cloned() {
                    let prop_iri = self.iri(key);
                    if self.rdf_star {
                        triples.push(rdf_mapping::rdf_star::annotated_triple(
                            edge.src.clone(),
                            edge.pred.clone(),
                            edge.dst.clone(),
                            prop_iri,
                            result_var.clone().into(),
                        ));
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
                Ok((result_var, None))
            }
            Expression::Aggregate(agg_expr) => {
                // Aggregate in RETURN → create result var + AggregateExpression.
                let result_var = match &item.alias {
                    Some(alias) => Variable::new_unchecked(alias.clone()),
                    None => self.fresh_var("agg"),
                };
                let sparql_agg = self.translate_aggregate_expr(agg_expr, extra)?;
                Ok((result_var, Some(sparql_agg)))
            }
            other => {
                // General expression: try to translate as SPARQL expression,
                // emit an Extend binding.
                let result_var = match &item.alias {
                    Some(alias) => Variable::new_unchecked(alias.clone()),
                    None => self.fresh_var("ret"),
                };
                let _sparql_expr = self.translate_expr(other, extra)?;
                Err(PolygraphError::UnsupportedFeature {
                    feature: format!(
                        "complex return expression (Phase 4+): {}",
                        result_var.as_str()
                    ),
                })
            }
        }
    }

    // ── Expression translation ────────────────────────────────────────────────

    /// Translate a Cypher [`Expression`] to a spargebra [`SparExpr`].
    ///
    /// Property accesses `n.key` are rewritten to fresh SPARQL variables, and
    /// the corresponding BGP triple is pushed into `extra_triples`.
    fn translate_expr(
        &mut self,
        expr: &Expression,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<SparExpr, PolygraphError> {
        match expr {
            Expression::Variable(name) => {
                Ok(SparExpr::Variable(Variable::new_unchecked(name.clone())))
            }
            Expression::Literal(lit) => Ok(SparExpr::Literal(self.translate_literal(lit)?)),
            Expression::Property(base_expr, key) => {
                let base_var = self.extract_variable(base_expr)?;
                let var_name = base_var.as_str().to_string();
                let fresh = self.fresh_var(&format!("{}_{}", var_name, key));
                // Check if `base_var` is a relationship variable (edge_map hit).
                if let Some(edge) = self.edge_map.get(&var_name).cloned() {
                    let prop_iri = self.iri(key);
                    if self.rdf_star {
                        extra.push(rdf_mapping::rdf_star::annotated_triple(
                            edge.src.clone(),
                            edge.pred.clone(),
                            edge.dst.clone(),
                            prop_iri,
                            fresh.clone().into(),
                        ));
                    } else {
                        let reif_var = edge
                            .reif_var
                            .clone()
                            .unwrap_or_else(|| self.fresh_var(&format!("reif_{var_name}")));
                        extra.push(TriplePattern {
                            subject: reif_var.into(),
                            predicate: prop_iri.into(),
                            object: fresh.clone().into(),
                        });
                    }
                } else {
                    extra.push(TriplePattern {
                        subject: base_var.into(),
                        predicate: self.iri(key).into(),
                        object: fresh.clone().into(),
                    });
                }
                Ok(SparExpr::Variable(fresh))
            }
            Expression::Or(a, b) => {
                let la = self.translate_expr(a, extra)?;
                let rb = self.translate_expr(b, extra)?;
                Ok(SparExpr::Or(Box::new(la), Box::new(rb)))
            }
            Expression::Xor(a, b) => {
                // XOR = (A OR B) AND NOT (A AND B)
                let la1 = self.translate_expr(a, extra)?;
                let rb1 = self.translate_expr(b, extra)?;
                let la2 = self.translate_expr(a, extra)?;
                let rb2 = self.translate_expr(b, extra)?;
                let or_ab = SparExpr::Or(Box::new(la1), Box::new(rb1));
                let and_ab = SparExpr::And(Box::new(la2), Box::new(rb2));
                Ok(SparExpr::And(
                    Box::new(or_ab),
                    Box::new(SparExpr::Not(Box::new(and_ab))),
                ))
            }
            Expression::And(a, b) => {
                let la = self.translate_expr(a, extra)?;
                let rb = self.translate_expr(b, extra)?;
                Ok(SparExpr::And(Box::new(la), Box::new(rb)))
            }
            Expression::Not(inner) => {
                let e = self.translate_expr(inner, extra)?;
                Ok(SparExpr::Not(Box::new(e)))
            }
            Expression::IsNull(inner) => {
                // IS NULL → !BOUND(?var)
                let e = self.translate_expr(inner, extra)?;
                let var = match e {
                    SparExpr::Variable(v) => v,
                    _ => {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: "IS NULL on non-variable expression".to_string(),
                        })
                    }
                };
                Ok(SparExpr::Not(Box::new(SparExpr::Bound(var))))
            }
            Expression::IsNotNull(inner) => {
                // IS NOT NULL → BOUND(?var)
                let e = self.translate_expr(inner, extra)?;
                let var = match e {
                    SparExpr::Variable(v) => v,
                    _ => {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: "IS NOT NULL on non-variable expression".to_string(),
                        })
                    }
                };
                Ok(SparExpr::Bound(var))
            }
            Expression::Comparison(lhs, op, rhs) => {
                // Special case: IN with a list literal rhs → SparExpr::In(lhs, [items...])
                if matches!(op, CompOp::In) {
                    if let Expression::List(items) = rhs.as_ref() {
                        let l = self.translate_expr(lhs, extra)?;
                        let members: Result<Vec<_>, _> = items
                            .iter()
                            .map(|e| self.translate_expr(e, extra))
                            .collect();
                        return Ok(SparExpr::In(Box::new(l), members?));
                    }
                }
                let l = self.translate_expr(lhs, extra)?;
                let r = self.translate_expr(rhs, extra)?;
                let result = match op {
                    CompOp::Eq => SparExpr::Equal(Box::new(l), Box::new(r)),
                    CompOp::Ne => {
                        SparExpr::Not(Box::new(SparExpr::Equal(Box::new(l), Box::new(r))))
                    }
                    CompOp::Lt => SparExpr::Less(Box::new(l), Box::new(r)),
                    CompOp::Le => SparExpr::LessOrEqual(Box::new(l), Box::new(r)),
                    CompOp::Gt => SparExpr::Greater(Box::new(l), Box::new(r)),
                    CompOp::Ge => SparExpr::GreaterOrEqual(Box::new(l), Box::new(r)),
                    CompOp::In => {
                        // `n.foo IN [a, b, c]` — the rhs was already translated.
                        // For a proper list literal we should have a vec of expressions.
                        // Since rhs came from translate_expr, which returns a single SparExpr,
                        // we wrap it in a vec. For inline list literals,
                        // we intercept before translation (see Expression::List arm above).
                        // Real list IN is handled: build SparExpr::In(lhs, vec![each item]).
                        SparExpr::In(Box::new(l), vec![r])
                    }
                    CompOp::StartsWith => {
                        SparExpr::FunctionCall(spargebra::algebra::Function::StrStarts, vec![l, r])
                    }
                    CompOp::EndsWith => {
                        SparExpr::FunctionCall(spargebra::algebra::Function::StrEnds, vec![l, r])
                    }
                    CompOp::Contains => {
                        SparExpr::FunctionCall(spargebra::algebra::Function::Contains, vec![l, r])
                    }
                };
                Ok(result)
            }
            Expression::Add(a, b) => Ok(SparExpr::Add(
                Box::new(self.translate_expr(a, extra)?),
                Box::new(self.translate_expr(b, extra)?),
            )),
            Expression::Subtract(a, b) => Ok(SparExpr::Subtract(
                Box::new(self.translate_expr(a, extra)?),
                Box::new(self.translate_expr(b, extra)?),
            )),
            Expression::Multiply(a, b) => Ok(SparExpr::Multiply(
                Box::new(self.translate_expr(a, extra)?),
                Box::new(self.translate_expr(b, extra)?),
            )),
            Expression::Divide(a, b) => Ok(SparExpr::Divide(
                Box::new(self.translate_expr(a, extra)?),
                Box::new(self.translate_expr(b, extra)?),
            )),
            Expression::Modulo(_, _) => Err(PolygraphError::UnsupportedFeature {
                feature: "modulo operator (Phase 4)".to_string(),
            }),
            Expression::Negate(inner) => Ok(SparExpr::UnaryMinus(Box::new(
                self.translate_expr(inner, extra)?,
            ))),
            Expression::Power(_, _) => Err(PolygraphError::UnsupportedFeature {
                feature: "power operator (Phase 4)".to_string(),
            }),
            Expression::List(_) => {
                // Lists are handled inline for IN expressions (see Comparison arm above).
                // A standalone list literal in filter context is not yet supported.
                Err(PolygraphError::UnsupportedFeature {
                    feature: "standalone list literal in filter expression (use IN [a,b,c])"
                        .to_string(),
                })
            }
            Expression::Map(_) => Err(PolygraphError::UnsupportedFeature {
                feature: "map literal in filter expression context".to_string(),
            }),
            Expression::Aggregate(agg) => {
                // Aggregates in expressions (e.g. HAVING) are not yet handled; they
                // are handled at the RETURN level via translate_aggregate_expr.
                let fresh = self.fresh_var("agg");
                let agg_expr = self.translate_aggregate_expr(agg, extra)?;
                // Store as GROUP aggregate; we can only signal that this is pending.
                // For now return the variable reference that will be bound via Group.
                let _ = agg_expr;
                Ok(SparExpr::Variable(fresh))
            }
        }
    }

    // ── Aggregate translation ─────────────────────────────────────────────────

    fn translate_aggregate_expr(
        &mut self,
        agg: &AggregateExpr,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<AggregateExpression, PolygraphError> {
        match agg {
            AggregateExpr::Count { distinct, expr } => {
                if expr.is_none() {
                    Ok(AggregateExpression::CountSolutions {
                        distinct: *distinct,
                    })
                } else {
                    let e = self.translate_expr(expr.as_ref().unwrap(), extra)?;
                    Ok(AggregateExpression::FunctionCall {
                        name: AggregateFunction::Count,
                        expr: e,
                        distinct: *distinct,
                    })
                }
            }
            AggregateExpr::Sum { distinct, expr } => Ok(AggregateExpression::FunctionCall {
                name: AggregateFunction::Sum,
                expr: self.translate_expr(expr, extra)?,
                distinct: *distinct,
            }),
            AggregateExpr::Avg { distinct, expr } => Ok(AggregateExpression::FunctionCall {
                name: AggregateFunction::Avg,
                expr: self.translate_expr(expr, extra)?,
                distinct: *distinct,
            }),
            AggregateExpr::Min { distinct, expr } => Ok(AggregateExpression::FunctionCall {
                name: AggregateFunction::Min,
                expr: self.translate_expr(expr, extra)?,
                distinct: *distinct,
            }),
            AggregateExpr::Max { distinct, expr } => Ok(AggregateExpression::FunctionCall {
                name: AggregateFunction::Max,
                expr: self.translate_expr(expr, extra)?,
                distinct: *distinct,
            }),
            AggregateExpr::Collect { distinct, expr } => Ok(AggregateExpression::FunctionCall {
                name: AggregateFunction::GroupConcat { separator: None },
                expr: self.translate_expr(expr, extra)?,
                distinct: *distinct,
            }),
        }
    }

    // ── UNWIND clause ─────────────────────────────────────────────────────────

    fn translate_unwind_clause(
        &mut self,
        u: &crate::ast::cypher::UnwindClause,
        current: GraphPattern,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<GraphPattern, PolygraphError> {
        let var = Variable::new_unchecked(u.variable.clone());
        match &u.expression {
            Expression::List(items) => {
                // Literal list: expand to VALUES ?var { val1 val2 ... }
                let bindings: Result<Vec<Vec<Option<GroundTerm>>>, _> = items
                    .iter()
                    .map(|e| {
                        let ground = self.expr_to_ground_term(e)?;
                        let gt = term_pattern_to_ground(ground)?;
                        Ok(vec![Some(gt)])
                    })
                    .collect();
                let values = GraphPattern::Values {
                    variables: vec![var],
                    bindings: bindings?,
                };
                Ok(join_patterns(current, values))
            }
            Expression::Variable(list_var) => {
                // UNWIND variable — the variable must already be bound to a list.
                // SPARQL 1.1 has no native list iteration; we emit a placeholder
                // that signals the engine must expand it.
                let _ = extra;
                Err(PolygraphError::UnsupportedFeature {
                    feature: format!(
                        "UNWIND of variable ?{list_var} (non-literal list): requires engine extension"
                    ),
                })
            }
            _ => Err(PolygraphError::UnsupportedFeature {
                feature: "UNWIND of non-literal expression".to_string(),
            }),
        }
    }

    // ── ORDER BY / SKIP / LIMIT ───────────────────────────────────────────────

    fn apply_order_skip_limit(
        &mut self,
        mut pattern: GraphPattern,
        order_by: Option<&crate::ast::cypher::OrderByClause>,
        skip: Option<&Expression>,
        limit: Option<&Expression>,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<GraphPattern, PolygraphError> {
        if let Some(ob) = order_by {
            let mut sort_exprs = Vec::new();
            for sort_item in &ob.items {
                let e = self.translate_expr(&sort_item.expression, extra)?;
                sort_exprs.push(if sort_item.descending {
                    OrderExpression::Desc(e)
                } else {
                    OrderExpression::Asc(e)
                });
            }
            pattern = GraphPattern::OrderBy {
                inner: Box::new(pattern),
                expression: sort_exprs,
            };
        }

        let start = if let Some(skip_expr) = skip {
            match skip_expr {
                Expression::Literal(Literal::Integer(n)) => *n as usize,
                _ => {
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: "non-integer SKIP expression".to_string(),
                    })
                }
            }
        } else {
            0
        };

        let length = if let Some(lim_expr) = limit {
            match lim_expr {
                Expression::Literal(Literal::Integer(n)) => Some(*n as usize),
                _ => {
                    return Err(PolygraphError::UnsupportedFeature {
                        feature: "non-integer LIMIT expression".to_string(),
                    })
                }
            }
        } else {
            None
        };

        if start > 0 || length.is_some() {
            pattern = GraphPattern::Slice {
                inner: Box::new(pattern),
                start,
                length,
            };
        }

        Ok(pattern)
    }

    // ── Literal translation ───────────────────────────────────────────────────

    fn translate_literal(&self, lit: &Literal) -> Result<SparLit, PolygraphError> {
        match lit {
            Literal::Integer(n) => Ok(SparLit::new_typed_literal(
                n.to_string(),
                NamedNode::new_unchecked(XSD_INTEGER),
            )),
            Literal::Float(f) => Ok(SparLit::new_typed_literal(
                format!("{f:e}"),
                NamedNode::new_unchecked(XSD_DOUBLE),
            )),
            Literal::String(s) => Ok(SparLit::new_simple_literal(s.clone())),
            Literal::Boolean(b) => Ok(SparLit::new_typed_literal(
                b.to_string(),
                NamedNode::new_unchecked(XSD_BOOLEAN),
            )),
            Literal::Null => Err(PolygraphError::UnsupportedFeature {
                feature: "null literal in expression context".to_string(),
            }),
        }
    }

    /// Translate a literal-valued expression into an RDF term for use as a
    /// BGP object (inline property map values).
    fn expr_to_ground_term(&self, expr: &Expression) -> Result<TermPattern, PolygraphError> {
        match expr {
            Expression::Literal(lit) => {
                let spar_lit = self.translate_literal(lit)?;
                Ok(spar_lit.into())
            }
            Expression::Variable(name) => Ok(Variable::new_unchecked(name.clone()).into()),
            _ => Err(PolygraphError::UnsupportedFeature {
                feature: "complex expression in inline property map (Phase 4)".to_string(),
            }),
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Extract the SPARQL [`Variable`] from a variable expression.
    fn extract_variable(&self, expr: &Expression) -> Result<Variable, PolygraphError> {
        match expr {
            Expression::Variable(name) => Ok(Variable::new_unchecked(name.clone())),
            _ => Err(PolygraphError::UnsupportedFeature {
                feature: "property access on non-variable base expression (Phase 4)".to_string(),
            }),
        }
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

/// Build an empty BGP for use as the identity element in joins.
fn empty_bgp() -> GraphPattern {
    GraphPattern::Bgp { patterns: vec![] }
}

/// Join two `GraphPattern`s, merging adjacent BGPs where possible.
fn join_patterns(left: GraphPattern, right: GraphPattern) -> GraphPattern {
    // Identity: join with empty BGP is a no-op.
    if let GraphPattern::Bgp { patterns } = &left {
        if patterns.is_empty() {
            return right;
        }
    }
    if let GraphPattern::Bgp { patterns } = &right {
        if patterns.is_empty() {
            return left;
        }
    }
    // Merge two BGPs into one (flattening is valid in SPARQL).
    match (left, right) {
        (GraphPattern::Bgp { patterns: mut lp }, GraphPattern::Bgp { patterns: rp }) => {
            lp.extend(rp);
            GraphPattern::Bgp { patterns: lp }
        }
        (l, r) => GraphPattern::Join {
            left: Box::new(l),
            right: Box::new(r),
        },
    }
}

/// Apply an iterator of filter expressions to a pattern, innermost first.
fn apply_filters(
    mut pattern: GraphPattern,
    filters: impl Iterator<Item = SparExpr>,
) -> GraphPattern {
    for expr in filters {
        pattern = GraphPattern::Filter {
            expr,
            inner: Box::new(pattern),
        };
    }
    pattern
}

/// Get the SPARQL `TermPattern` for the n-th element in a pattern chain,
/// assuming it is a node.  Returns a fresh anonymous variable if the index
/// is out of bounds (shouldn't happen for a well-formed pattern).
fn node_var_at(elements: &[PatternElement], i: usize) -> TermPattern {
    elements
        .get(i)
        .and_then(|e| match e {
            PatternElement::Node(n) => Some(node_term(n)),
            _ => None,
        })
        .unwrap_or_else(|| Variable::new_unchecked("__anon").into())
}

/// Build the SPARQL `TermPattern` for a node: `?var` if the node has a
/// variable, otherwise a blank node using the node's label as a hint, or a
/// truly anonymous blank node.
fn node_term(node: &NodePattern) -> TermPattern {
    match &node.variable {
        Some(v) => Variable::new_unchecked(v.clone()).into(),
        None => {
            use spargebra::term::BlankNode;
            BlankNode::default().into()
        }
    }
}

/// Convert a `TermPattern` (literal or named node) to a `GroundTerm` for use
/// in `GraphPattern::Values` bindings. Returns an error for variable terms.
fn term_pattern_to_ground(tp: TermPattern) -> Result<GroundTerm, PolygraphError> {
    match tp {
        TermPattern::NamedNode(nn) => Ok(GroundTerm::NamedNode(nn)),
        TermPattern::Literal(lit) => Ok(GroundTerm::Literal(lit)),
        TermPattern::BlankNode(_) => Err(PolygraphError::UnsupportedFeature {
            feature: "blank node in UNWIND list literal — only ground terms supported".to_string(),
        }),
        TermPattern::Variable(_) => Err(PolygraphError::UnsupportedFeature {
            feature: "variable in UNWIND list literal — only literal values supported".to_string(),
        }),
        TermPattern::Triple(_) => Err(PolygraphError::UnsupportedFeature {
            feature: "triple pattern in UNWIND list — not a ground term".to_string(),
        }),
    }
}
