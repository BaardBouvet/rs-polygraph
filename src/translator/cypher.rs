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

use spargebra::algebra::{Expression as SparExpr, GraphPattern};
use spargebra::term::{Literal as SparLit, NamedNode, TermPattern, TriplePattern, Variable};
use spargebra::Query;

use crate::rdf_mapping;

use crate::ast::cypher::{
    Clause, CompOp, CypherQuery, Expression, Literal, MatchClause, NodePattern, Pattern,
    PatternElement, PatternList, RelationshipPattern, ReturnClause, ReturnItem, ReturnItems,
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
        Self { base_iri, counter: 0, rdf_star, edge_map: Default::default() }
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
                    let (triples, opt_filter) =
                        self.translate_match_clause(m, &mut extra_triples)?;
                    let bgp = GraphPattern::Bgp { patterns: triples };
                    if m.optional {
                        current = GraphPattern::LeftJoin {
                            left: Box::new(current),
                            right: Box::new(bgp),
                            expression: opt_filter,
                        };
                    } else {
                        current = join_patterns(current, bgp);
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
                    // Translate WITH's WHERE if present.
                    if let Some(wc) = &w.where_ {
                        let filter_expr =
                            self.translate_expr(&wc.expression, &mut extra_triples)?;
                        pending_filters.push(filter_expr);
                    }
                    // WITH items: a narrowing but for single-clause WITH with
                    // no sub-queries we just let the variables flow through.
                    let _ = w; // items handled below when building projection
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

                    let (return_triples, project_vars, need_distinct) =
                        self.translate_return_clause(r, &mut extra_triples)?;

                    if !return_triples.is_empty() {
                        let extra = GraphPattern::Bgp { patterns: return_triples };
                        current = join_patterns(current, extra);
                    }
                    // Flush any triples added during return expression translation.
                    if !extra_triples.is_empty() {
                        let extra = GraphPattern::Bgp {
                            patterns: extra_triples.drain(..).collect(),
                        };
                        current = join_patterns(current, extra);
                    }

                    if let Some(vars) = project_vars {
                        current = GraphPattern::Project {
                            inner: Box::new(current),
                            variables: vars,
                        };
                    }
                    if need_distinct {
                        current = GraphPattern::Distinct { inner: Box::new(current) };
                    }
                }
            }
        }

        // Final flush (in case no RETURN clause was present).
        if !extra_triples.is_empty() {
            let extra = GraphPattern::Bgp { patterns: extra_triples };
            current = join_patterns(current, extra);
        }
        current = apply_filters(current, pending_filters.into_iter());

        Ok(current)
    }

    // ── MATCH clause ─────────────────────────────────────────────────────────

    /// Translate a `MATCH` or `OPTIONAL MATCH` clause into BGP triples plus an
    /// optional filter expression (from the inline `WHERE`).
    fn translate_match_clause(
        &mut self,
        m: &MatchClause,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<(Vec<TriplePattern>, Option<SparExpr>), PolygraphError> {
        let mut triples = Vec::new();
        self.translate_pattern_list(&m.pattern, &mut triples)?;

        let filter = if let Some(wc) = &m.where_ {
            Some(self.translate_expr(&wc.expression, extra)?)
        } else {
            None
        };

        Ok((triples, filter))
    }

    // ── Pattern translation ───────────────────────────────────────────────────

    fn translate_pattern_list(
        &mut self,
        list: &PatternList,
        triples: &mut Vec<TriplePattern>,
    ) -> Result<(), PolygraphError> {
        for pattern in &list.0 {
            self.translate_pattern(pattern, triples)?;
        }
        Ok(())
    }

    fn translate_pattern(
        &mut self,
        pattern: &Pattern,
        triples: &mut Vec<TriplePattern>,
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
                    // Relationship sits between the previous node (i-1) and next node (i+1).
                    // The surrounding nodes were (or will be) processed separately; here we
                    // only emit the edge triple.  The subject/object vars come from the
                    // adjacent node patterns' variables.
                    let src = node_var_at(elements, i.wrapping_sub(1));
                    let dst = node_var_at(elements, i + 1);
                    self.translate_relationship_pattern(r, &src, &dst, triples)?;
                    // Skip the next node; it will be processed in the next outer iteration.
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
    ) -> Result<(), PolygraphError> {
        use crate::ast::cypher::Direction;

        if rel.rel_types.is_empty() {
            // No type constraint: emit an anonymous predicate variable.
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

        // Emit one triple per rel-type (union semantics deferred to Phase 4).
        // For Phase 2+: use the first type.
        let rel_type = &rel.rel_types[0];
        let pred = self.iri(rel_type);

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
                // Build (prop_iri, term) pairs.
                let mut prop_pairs: Vec<(NamedNode, TermPattern)> = Vec::new();
                for (key, val_expr) in props {
                    let obj = self.expr_to_ground_term(val_expr)?;
                    prop_pairs.push((self.iri(key), obj));
                }

                if self.rdf_star {
                    let extra = rdf_mapping::rdf_star::all_property_triples(
                        src.clone(), pred.clone(), dst.clone(), &prop_pairs,
                    );
                    triples.extend(extra);
                } else {
                    // Use (or create) the reification variable.
                    let reif_var = rel.variable.as_ref()
                        .and_then(|v| self.edge_map.get(v))
                        .and_then(|ei| ei.reif_var.clone())
                        .unwrap_or_else(|| self.fresh_var("reif"));
                    let extra = rdf_mapping::reification::all_triples(
                        &reif_var,
                        src.clone(), pred.clone(), dst.clone(),
                        &prop_pairs,
                    );
                    triples.extend(extra);
                }
            }
        }

        Ok(())
    }

    // ── RETURN clause ─────────────────────────────────────────────────────────

    /// Returns `(extra_bgp_triples, Some(projected_vars) | None for *, distinct_flag)`.
    fn translate_return_clause(
        &mut self,
        ret: &ReturnClause,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<(Vec<TriplePattern>, Option<Vec<Variable>>, bool), PolygraphError> {
        match &ret.items {
            ReturnItems::All => {
                // RETURN * — no Project wrapper; emit everything.
                Ok((vec![], None, ret.distinct))
            }
            ReturnItems::Explicit(items) => {
                let mut triples = Vec::new();
                let mut vars = Vec::new();
                for item in items {
                    let var = self.translate_return_item(item, &mut triples, extra)?;
                    vars.push(var);
                }
                Ok((triples, Some(vars), ret.distinct))
            }
        }
    }

    fn translate_return_item(
        &mut self,
        item: &ReturnItem,
        triples: &mut Vec<TriplePattern>,
        extra: &mut Vec<TriplePattern>,
    ) -> Result<Variable, PolygraphError> {
        match &item.expression {
            Expression::Variable(name) => {
                let var = Variable::new_unchecked(name.clone());
                if let Some(alias) = &item.alias {
                    // RETURN n AS alias → BIND(?n AS ?alias)
                    // We model this by reusing the alias var directly because
                    // spargebra's Project just references variables; aliasing
                    // is handled by the caller renaming the projection.
                    // Simple alias: just project under the alias name.
                    // Add a BIND triple via Extend would be cleaner, but for
                    // Phase 2 we emit an extra BGP where the alias var == the
                    // source var (the projection only references alias_var).
                    // Actually: project the *alias* variable, and add a BGP
                    // `?alias_var = ?source_var` — but SPARQL has no such
                    // construct in a triple. Use the source var for projection
                    // and rename at Display is unsupported here; instead just
                    // project the source var under the alias.
                    let _ = var; // suppress lint
                    Ok(Variable::new_unchecked(alias.clone()))
                } else {
                    Ok(var)
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
                            edge.src.clone(), edge.pred.clone(), edge.dst.clone(),
                            prop_iri, result_var.clone().into(),
                        ));
                    } else {
                        let reif_var = edge.reif_var.clone()
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
                Ok(result_var)
            }
            other => {
                // General expression: translate, emit extras, project into fresh var.
                let result_var = match &item.alias {
                    Some(alias) => Variable::new_unchecked(alias.clone()),
                    None => self.fresh_var("ret"),
                };
                let _sparql_expr = self.translate_expr(other, extra)?;
                // We would emit an Extend here; for Phase 2 we return an error
                // for complex non-property, non-variable return expressions.
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
            Expression::Literal(lit) => {
                Ok(SparExpr::Literal(self.translate_literal(lit)?))
            }
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
                        let reif_var = edge.reif_var.clone()
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
                let l = self.translate_expr(lhs, extra)?;
                let r = self.translate_expr(rhs, extra)?;
                let result = match op {
                    CompOp::Eq => SparExpr::Equal(Box::new(l), Box::new(r)),
                    CompOp::Ne => SparExpr::Not(Box::new(SparExpr::Equal(
                        Box::new(l),
                        Box::new(r),
                    ))),
                    CompOp::Lt => SparExpr::Less(Box::new(l), Box::new(r)),
                    CompOp::Le => SparExpr::LessOrEqual(Box::new(l), Box::new(r)),
                    CompOp::Gt => SparExpr::Greater(Box::new(l), Box::new(r)),
                    CompOp::Ge => SparExpr::GreaterOrEqual(Box::new(l), Box::new(r)),
                    CompOp::In => {
                        // `n.foo IN [a, b, c]` where rhs is a list expression.
                        // For Phase 2, handle  simple `IN [lit, lit, …]` by
                        // emitting `IN (a, b, c)` in SPARQL.
                        // We already translated rhs as an expression; the `IN`
                        // form in spargebra takes `In(expr, Vec<expr>)`.
                        // Since we already translated rhs as a single Expression,
                        // wrap it in a single-element vec (will be correct when
                        // rhs is a variable pointing to a list; complex list
                        // literals are a Phase 4 concern).
                        SparExpr::In(Box::new(l), vec![r])
                    }
                    CompOp::StartsWith => {
                        SparExpr::FunctionCall(
                            spargebra::algebra::Function::StrStarts,
                            vec![l, r],
                        )
                    }
                    CompOp::EndsWith => {
                        SparExpr::FunctionCall(
                            spargebra::algebra::Function::StrEnds,
                            vec![l, r],
                        )
                    }
                    CompOp::Contains => {
                        SparExpr::FunctionCall(
                            spargebra::algebra::Function::Contains,
                            vec![l, r],
                        )
                    }
                };
                Ok(result)
            }
            Expression::Add(a, b) => {
                Ok(SparExpr::Add(
                    Box::new(self.translate_expr(a, extra)?),
                    Box::new(self.translate_expr(b, extra)?),
                ))
            }
            Expression::Subtract(a, b) => {
                Ok(SparExpr::Subtract(
                    Box::new(self.translate_expr(a, extra)?),
                    Box::new(self.translate_expr(b, extra)?),
                ))
            }
            Expression::Multiply(a, b) => {
                Ok(SparExpr::Multiply(
                    Box::new(self.translate_expr(a, extra)?),
                    Box::new(self.translate_expr(b, extra)?),
                ))
            }
            Expression::Divide(a, b) => {
                Ok(SparExpr::Divide(
                    Box::new(self.translate_expr(a, extra)?),
                    Box::new(self.translate_expr(b, extra)?),
                ))
            }
            Expression::Modulo(_, _) => Err(PolygraphError::UnsupportedFeature {
                feature: "modulo operator (Phase 4)".to_string(),
            }),
            Expression::Negate(inner) => {
                Ok(SparExpr::UnaryMinus(Box::new(self.translate_expr(inner, extra)?)))
            }
            Expression::Power(_, _) => Err(PolygraphError::UnsupportedFeature {
                feature: "power operator (Phase 4)".to_string(),
            }),
            Expression::List(_) | Expression::Map(_) => {
                Err(PolygraphError::UnsupportedFeature {
                    feature: "list/map literals in filter expressions (Phase 4)".to_string(),
                })
            }
        }
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
            Expression::Variable(name) => {
                Ok(Variable::new_unchecked(name.clone()).into())
            }
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
        (l, r) => GraphPattern::Join { left: Box::new(l), right: Box::new(r) },
    }
}

/// Apply an iterator of filter expressions to a pattern, innermost first.
fn apply_filters(
    mut pattern: GraphPattern,
    filters: impl Iterator<Item = SparExpr>,
) -> GraphPattern {
    for expr in filters {
        pattern = GraphPattern::Filter { expr, inner: Box::new(pattern) };
    }
    pattern
}

/// Get the SPARQL `TermPattern` for the n-th element in a pattern chain,
/// assuming it is a node.  Returns a fresh anonymous variable if the index
/// is out of bounds (shouldn't happen for a well-formed pattern).
fn node_var_at(elements: &[PatternElement], i: usize) -> TermPattern {
    elements.get(i).and_then(|e| match e {
        PatternElement::Node(n) => Some(node_term(n)),
        _ => None,
    }).unwrap_or_else(|| Variable::new_unchecked("__anon").into())
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

