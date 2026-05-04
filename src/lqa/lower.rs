//! Phase 4.5 — AST → LQA lowering pass.
//!
//! Converts a [`CypherQuery`] AST into an [`Op`] tree that can subsequently
//! be compiled to SPARQL by [`crate::lqa::sparql`].
//!
//! The conversion is structurally straightforward — every AST node maps to
//! one or more LQA nodes — so this module has no SPARQL-specific knowledge.
//!
//! # Entry point
//!
//! ```ignore
//! let mut lowerer = AstLowerer::new();
//! let op = lowerer.lower_query(&cypher_query)?;
//! ```

use std::collections::HashSet;

use crate::ast::cypher::{
    self as ast, AggregateExpr, Clause, CompOp, Direction, PatternElement, QuantifierKind,
};
use crate::error::PolygraphError;
use crate::lqa::expr::{AggKind, CmpOp, Expr, Literal, QuantKind, UnaryOp};
use crate::lqa::op::{
    AggItem, CreateEdge, CreateNode, Direction as LqaDir, MergeClause as LqaMergeClause, Op,
    PathRange, ProjItem, RemoveItem as LqaRemoveItem, SetItem as LqaSetItem, SortKey,
};

// ── Entry point ───────────────────────────────────────────────────────────────

/// Lowers a parsed [`ast::CypherQuery`] into an LQA [`Op`] tree.
pub struct AstLowerer {
    counter: u32,
    /// Variables introduced by earlier MATCH patterns in the same query.
    /// Re-used variables (those seen before) are not re-scanned; they are
    /// already bound in the SPARQL context via shared variable names.
    seen_vars: HashSet<String>,
}

impl AstLowerer {
    pub fn new() -> Self {
        Self {
            counter: 0,
            seen_vars: HashSet::new(),
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        let c = self.counter;
        self.counter += 1;
        format!("_lqa_{prefix}_{c}")
    }

    /// Convert the whole query.  Returns the root [`Op`].
    pub fn lower_query(&mut self, query: &ast::CypherQuery) -> Result<Op, PolygraphError> {
        self.lower_clauses(&query.clauses)
    }

    // ── Clause list ───────────────────────────────────────────────────────────

    /// Split on UNION markers, lower each arm, combine.
    fn lower_clauses(&mut self, clauses: &[Clause]) -> Result<Op, PolygraphError> {
        // Find UNION positions
        let mut cut_positions: Vec<(usize, bool)> = Vec::new(); // (index, all)
        for (i, c) in clauses.iter().enumerate() {
            if let Clause::Union { all } = c {
                cut_positions.push((i, *all));
            }
        }

        if cut_positions.is_empty() {
            return self.lower_pipeline(clauses);
        }

        // Split into arms
        let mut arms: Vec<&[Clause]> = Vec::new();
        let mut all_flags: Vec<bool> = Vec::new();
        let mut prev = 0;
        for (pos, all) in &cut_positions {
            arms.push(&clauses[prev..*pos]);
            all_flags.push(*all);
            prev = pos + 1;
        }
        arms.push(&clauses[prev..]);

        let mut result = self.lower_pipeline(arms[0])?;
        for (i, arm) in arms[1..].iter().enumerate() {
            let right = self.lower_pipeline(arm)?;
            result = if all_flags[i] {
                Op::UnionAll {
                    left: Box::new(result),
                    right: Box::new(right),
                }
            } else {
                Op::Union {
                    left: Box::new(result),
                    right: Box::new(right),
                }
            };
        }
        Ok(result)
    }

    /// Lower a single arm (no UNION marker inside).
    fn lower_pipeline(&mut self, clauses: &[Clause]) -> Result<Op, PolygraphError> {
        let mut op = Op::Unit;
        for clause in clauses {
            op = self.lower_clause(op, clause)?;
        }
        Ok(op)
    }

    fn lower_clause(&mut self, current: Op, clause: &Clause) -> Result<Op, PolygraphError> {
        match clause {
            Clause::Match(m) => {
                let match_op = self.lower_match_pattern(m)?;
                if matches!(current, Op::Unit) {
                    Ok(match_op)
                } else if m.optional {
                    Ok(Op::LeftOuterJoin {
                        left: Box::new(current),
                        right: Box::new(match_op),
                        condition: None,
                    })
                } else {
                    // Join: two independent MATCH clauses share variables via natural join.
                    // Use CartesianProduct here; the SPARQL lowerer joins via shared vars.
                    Ok(Op::CartesianProduct {
                        left: Box::new(current),
                        right: Box::new(match_op),
                    })
                }
            }

            Clause::With(w) => self.lower_with(current, w),

            Clause::Return(r) => self.lower_return(current, r),

            Clause::Unwind(u) => {
                let list = self.lower_expr(&u.expression)?;
                Ok(Op::Unwind {
                    inner: Box::new(current),
                    list,
                    variable: u.variable.clone(),
                })
            }

            Clause::Create(c) => {
                let (nodes, edges) = self.lower_create_pattern(&c.pattern)?;
                Ok(Op::Create {
                    inner: Box::new(current),
                    nodes,
                    edges,
                })
            }

            Clause::Set(s) => {
                let items = self.lower_set_items(&s.items)?;
                Ok(Op::Set {
                    inner: Box::new(current),
                    items,
                })
            }

            Clause::Delete(d) => {
                let exprs = d
                    .expressions
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                Ok(Op::Delete {
                    inner: Box::new(current),
                    detach: d.detach,
                    exprs,
                })
            }

            Clause::Remove(r) => {
                let items = self.lower_remove_items(&r.items)?;
                Ok(Op::Remove {
                    inner: Box::new(current),
                    items,
                })
            }

            Clause::Merge(m) => {
                let clause = self.lower_merge_clause(m)?;
                Ok(Op::Merge {
                    inner: Box::new(current),
                    clause,
                })
            }

            Clause::Call(c) => {
                let args = c
                    .args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                Ok(Op::Call {
                    inner: Box::new(current),
                    procedure: c.procedure.clone(),
                    args,
                    yields: c.yields.clone(),
                })
            }

            Clause::Union { .. } => Err(PolygraphError::Translation {
                message: "unexpected UNION inside lower_clause".into(),
            }),
        }
    }

    // ── MATCH ────────────────────────────────────────────────────────────────

    fn lower_match_pattern(&mut self, m: &ast::MatchClause) -> Result<Op, PolygraphError> {
        let mut op = self.lower_pattern_list(&m.pattern)?;
        if let Some(where_) = &m.where_ {
            let pred = self.lower_expr(&where_.expression)?;
            op = Op::Selection {
                inner: Box::new(op),
                predicate: pred,
            };
        }
        Ok(op)
    }

    fn lower_pattern_list(&mut self, pl: &ast::PatternList) -> Result<Op, PolygraphError> {
        let mut iter = pl.0.iter();
        let first =
            self.lower_pattern(iter.next().ok_or_else(|| PolygraphError::Translation {
                message: "empty pattern list".into(),
            })?)?;
        let mut result = first;
        for pat in iter {
            let right = self.lower_pattern(pat)?;
            result = Op::CartesianProduct {
                left: Box::new(result),
                right: Box::new(right),
            };
        }
        Ok(result)
    }

    /// Lower one path pattern: `(n:Label {prop: val})-[r:T]->(m)`.
    ///
    /// The elements are `[Node, Rel, Node, Rel, Node, …]` alternating.
    ///
    /// Rules:
    /// - A node AFTER a relationship is the `to` endpoint — it is already
    ///   bound by the relationship triple.  We add label/property constraints
    ///   via `Selection` wrappers but do NOT create an additional `Scan`.
    /// - A node that was already introduced by a previous MATCH pattern
    ///   (tracked in `self.seen_vars`) is not re-scanned either — SPARQL joins
    ///   on shared variable names automatically.
    /// - A node that is new AND is the first element of a pattern AND has a
    ///   label gets a normal `Scan`.
    fn lower_pattern(&mut self, p: &ast::Pattern) -> Result<Op, PolygraphError> {
        let elements = &p.elements;

        // Pre-assign names to every anonymous node position.
        let mut node_names: Vec<String> = Vec::new();
        for elem in elements {
            if let PatternElement::Node(n) = elem {
                node_names.push(n.variable.clone().unwrap_or_else(|| self.fresh("anon")));
            }
        }

        let mut op: Option<Op> = None;
        let mut node_idx = 0usize;
        let mut last_src: Option<String> = None;
        // Set to true just after we process a Relationship element.
        let mut after_rel = false;

        for elem in elements {
            match elem {
                PatternElement::Node(n) => {
                    let var = node_names[node_idx].clone();
                    node_idx += 1;

                    let is_seen = self.seen_vars.contains(&var);
                    self.seen_vars.insert(var.clone());

                    if after_rel || is_seen {
                        // The variable is already bound — don't create a Scan.
                        // Apply label and property constraints via Selection.
                        let mut acc = op.take().unwrap_or(Op::Unit);
                        for label in &n.labels {
                            acc = Op::Selection {
                                inner: Box::new(acc),
                                predicate: Expr::LabelCheck {
                                    expr: Box::new(Expr::var(&var)),
                                    labels: vec![label.clone()],
                                },
                            };
                        }
                        if let Some(props) = n.properties.as_deref() {
                            for (key, val) in props {
                                acc = Op::Selection {
                                    inner: Box::new(acc),
                                    predicate: Expr::Comparison(
                                        CmpOp::Eq,
                                        Box::new(Expr::Property(
                                            Box::new(Expr::var(&var)),
                                            key.clone(),
                                        )),
                                        Box::new(self.lower_expr(val)?),
                                    ),
                                };
                            }
                        }
                        op = Some(acc);
                    } else {
                        // Fresh variable with no prior binding — emit a Scan.
                        let scan = Op::Scan {
                            variable: var.clone(),
                            label: n.labels.first().cloned(),
                            extra_labels: if n.labels.len() > 1 {
                                n.labels[1..].to_vec()
                            } else {
                                vec![]
                            },
                        };
                        let scan_op =
                            self.apply_prop_predicates(scan, &var, n.properties.as_deref())?;
                        op = Some(match op.take() {
                            None => scan_op,
                            Some(prev) => Op::CartesianProduct {
                                left: Box::new(prev),
                                right: Box::new(scan_op),
                            },
                        });
                    }

                    after_rel = false;
                    last_src = Some(var);
                }

                PatternElement::Relationship(r) => {
                    let from = last_src
                        .clone()
                        .ok_or_else(|| PolygraphError::Translation {
                            message: "relationship pattern without preceding node variable".into(),
                        })?;
                    // The 'to' node is the next node element (not yet incremented).
                    let to = node_names[node_idx].clone();

                    let direction = match r.direction {
                        Direction::Right => LqaDir::Outgoing,
                        Direction::Left => LqaDir::Incoming,
                        Direction::Both => LqaDir::Undirected,
                    };

                    let range = r.range.as_ref().map(|rq| PathRange {
                        lower: rq.lower.unwrap_or(1),
                        upper: rq.upper,
                    });

                    let expand = Op::Expand {
                        inner: Box::new(op.take().unwrap_or(Op::Unit)),
                        from: from.clone(),
                        rel_var: r.variable.clone(),
                        to: to.clone(),
                        rel_types: r.rel_types.clone(),
                        direction,
                        range,
                        path_var: p.variable.clone(),
                    };

                    // Inline relationship property predicates: -[r {w: 1}]->
                    let expand_op = if let (Some(props), Some(rv)) = (&r.properties, &r.variable) {
                        self.apply_prop_predicates(expand, rv, Some(props))?
                    } else {
                        expand
                    };

                    op = Some(expand_op);
                    after_rel = true;
                    // last_src is NOT updated here; the next Node element will update it.
                }
            }
        }

        Ok(op.unwrap_or(Op::Unit))
    }

    /// Wrap `inner_op` with a Selection for each `(key, val)` property predicate.
    fn apply_prop_predicates(
        &mut self,
        inner_op: Op,
        var: &str,
        props: Option<&[(String, ast::Expression)]>,
    ) -> Result<Op, PolygraphError> {
        let Some(props) = props else {
            return Ok(inner_op);
        };
        let mut acc = inner_op;
        for (key, val) in props {
            let pred = Expr::Comparison(
                CmpOp::Eq,
                Box::new(Expr::Property(Box::new(Expr::var(var)), key.clone())),
                Box::new(self.lower_expr(val)?),
            );
            acc = Op::Selection {
                inner: Box::new(acc),
                predicate: pred,
            };
        }
        Ok(acc)
    }

    // ── WITH ─────────────────────────────────────────────────────────────────

    fn lower_with(&mut self, inner: Op, w: &ast::WithClause) -> Result<Op, PolygraphError> {
        let (proj_items, agg_items) = self.lower_return_items(&w.items)?;

        let projected = if !agg_items.is_empty() {
            let agg_aliases: Vec<String> = agg_items.iter().map(|a| a.alias.clone()).collect();
            let group_keys = proj_cols_keys(&proj_items, &agg_aliases);
            let grouped = Op::GroupBy {
                inner: Box::new(inner),
                group_keys,
                agg_items,
            };
            Op::Projection {
                inner: Box::new(grouped),
                items: proj_items,
                distinct: w.distinct,
            }
        } else {
            Op::Projection {
                inner: Box::new(inner),
                items: proj_items,
                distinct: w.distinct,
            }
        };

        // WHERE on WITH applies after projection.
        let filtered = if let Some(wh) = &w.where_ {
            let pred = self.lower_expr(&wh.expression)?;
            Op::Selection {
                inner: Box::new(projected),
                predicate: pred,
            }
        } else {
            projected
        };

        let ordered = self.maybe_order_by(filtered, w.order_by.as_ref())?;
        let skipped = self.maybe_skip(ordered, w.skip.as_ref())?;
        self.maybe_limit(skipped, w.limit.as_ref())
    }

    // ── RETURN ───────────────────────────────────────────────────────────────

    fn lower_return(&mut self, inner: Op, r: &ast::ReturnClause) -> Result<Op, PolygraphError> {
        let (proj_items, agg_items) = self.lower_return_items(&r.items)?;

        let projected = if !agg_items.is_empty() {
            let agg_aliases: Vec<String> = agg_items.iter().map(|a| a.alias.clone()).collect();
            let group_keys = proj_cols_keys(&proj_items, &agg_aliases);
            let grouped = Op::GroupBy {
                inner: Box::new(inner),
                group_keys,
                agg_items,
            };
            Op::Projection {
                inner: Box::new(grouped),
                items: proj_items,
                distinct: r.distinct,
            }
        } else {
            Op::Projection {
                inner: Box::new(inner),
                items: proj_items,
                distinct: r.distinct,
            }
        };

        let ordered = self.maybe_order_by(projected, r.order_by.as_ref())?;
        let skipped = self.maybe_skip(ordered, r.skip.as_ref())?;
        let limited = self.maybe_limit(skipped, r.limit.as_ref())?;

        if r.distinct {
            Ok(Op::Distinct {
                inner: Box::new(limited),
            })
        } else {
            Ok(limited)
        }
    }

    // ── Return / WITH items ───────────────────────────────────────────────────

    /// Split the item list into (projection items, aggregate items).
    fn lower_return_items(
        &mut self,
        items: &ast::ReturnItems,
    ) -> Result<(Vec<ProjItem>, Vec<AggItem>), PolygraphError> {
        let mut proj: Vec<ProjItem> = Vec::new();
        let mut aggs: Vec<AggItem> = Vec::new();
        let mut gen_counter = 0u32;

        match items {
            ast::ReturnItems::All => {
                // RETURN * — represented as a single catch-all; lowered to "project all vars"
                // The SPARQL lowerer handles this by not wrapping in Project.
                proj.push(ProjItem {
                    expr: Expr::var("*"),
                    alias: "*".into(),
                });
            }
            ast::ReturnItems::Explicit(list) => {
                for item in list {
                    let alias = item.alias.clone().unwrap_or_else(|| {
                        let a = format!("_gen_{gen_counter}");
                        gen_counter += 1;
                        a
                    });
                    let expr = self.lower_expr(&item.expression)?;
                    // Check if this expression is/wraps an aggregate.
                    if matches!(expr, Expr::Aggregate { .. }) {
                        // Emit as an aggregate: bind the agg expr to the alias.
                        aggs.push(AggItem {
                            expr: expr.clone(),
                            alias: alias.clone(),
                        });
                        // Project the output alias variable.
                        proj.push(ProjItem {
                            expr: Expr::var(&alias),
                            alias: alias.clone(),
                        });
                    } else {
                        proj.push(ProjItem { expr, alias });
                    }
                }
            }
        }
        Ok((proj, aggs))
    }

    // ── ORDER BY / SKIP / LIMIT helpers ──────────────────────────────────────

    fn maybe_order_by(
        &mut self,
        op: Op,
        order_by: Option<&ast::OrderByClause>,
    ) -> Result<Op, PolygraphError> {
        let Some(ob) = order_by else { return Ok(op) };
        let keys = ob
            .items
            .iter()
            .map(|si| {
                Ok(SortKey {
                    expr: self.lower_expr(&si.expression)?,
                    dir: if si.descending {
                        crate::lqa::expr::SortDir::Desc
                    } else {
                        crate::lqa::expr::SortDir::Asc
                    },
                })
            })
            .collect::<Result<Vec<_>, PolygraphError>>()?;
        Ok(Op::OrderBy {
            inner: Box::new(op),
            keys,
        })
    }

    fn maybe_skip(&mut self, op: Op, skip: Option<&ast::Expression>) -> Result<Op, PolygraphError> {
        let Some(s) = skip else { return Ok(op) };
        Ok(Op::Skip {
            inner: Box::new(op),
            count: self.lower_expr(s)?,
        })
    }

    fn maybe_limit(
        &mut self,
        op: Op,
        limit: Option<&ast::Expression>,
    ) -> Result<Op, PolygraphError> {
        let Some(l) = limit else { return Ok(op) };
        Ok(Op::Limit {
            inner: Box::new(op),
            count: self.lower_expr(l)?,
        })
    }

    // ── Expressions ───────────────────────────────────────────────────────────

    pub fn lower_expr(&mut self, e: &ast::Expression) -> Result<Expr, PolygraphError> {
        use ast::Expression as AE;
        match e {
            AE::Variable(v) => Ok(Expr::var(v)),
            AE::Literal(l) => Ok(Expr::Literal(lower_literal(l))),
            AE::Property(base, key) => Ok(Expr::Property(
                Box::new(self.lower_expr(base)?),
                key.clone(),
            )),
            AE::Add(a, b) => Ok(Expr::Add(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            AE::Subtract(a, b) => Ok(Expr::Sub(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            AE::Multiply(a, b) => Ok(Expr::Mul(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            AE::Divide(a, b) => Ok(Expr::Div(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            AE::Modulo(a, b) => Ok(Expr::Mod(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            AE::Power(a, b) => Ok(Expr::Pow(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            AE::Negate(a) => Ok(Expr::Unary(UnaryOp::Neg, Box::new(self.lower_expr(a)?))),
            AE::Not(a) => Ok(Expr::Not(Box::new(self.lower_expr(a)?))),
            AE::IsNull(a) => Ok(Expr::IsNull(Box::new(self.lower_expr(a)?))),
            AE::IsNotNull(a) => Ok(Expr::IsNotNull(Box::new(self.lower_expr(a)?))),
            AE::Or(a, b) => Ok(Expr::Or(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            AE::And(a, b) => Ok(Expr::And(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            AE::Xor(a, b) => {
                // Xor(a, b) = (a OR b) AND NOT (a AND b)
                let la = self.lower_expr(a)?;
                let lb = self.lower_expr(b)?;
                Ok(Expr::And(
                    Box::new(Expr::Or(Box::new(la.clone()), Box::new(lb.clone()))),
                    Box::new(Expr::Not(Box::new(Expr::And(Box::new(la), Box::new(lb))))),
                ))
            }
            AE::Comparison(a, op, b) => {
                let la = self.lower_expr(a)?;
                let lb = self.lower_expr(b)?;
                match op {
                    CompOp::Eq => Ok(Expr::Comparison(CmpOp::Eq, Box::new(la), Box::new(lb))),
                    CompOp::Ne => Ok(Expr::Comparison(CmpOp::Ne, Box::new(la), Box::new(lb))),
                    CompOp::Lt => Ok(Expr::Comparison(CmpOp::Lt, Box::new(la), Box::new(lb))),
                    CompOp::Le => Ok(Expr::Comparison(CmpOp::Le, Box::new(la), Box::new(lb))),
                    CompOp::Gt => Ok(Expr::Comparison(CmpOp::Gt, Box::new(la), Box::new(lb))),
                    CompOp::Ge => Ok(Expr::Comparison(CmpOp::Ge, Box::new(la), Box::new(lb))),
                    CompOp::In => Ok(Expr::Comparison(CmpOp::In, Box::new(la), Box::new(lb))),
                    CompOp::StartsWith => Ok(Expr::FunctionCall {
                        name: "startsWith".into(),
                        distinct: false,
                        args: vec![la, lb],
                    }),
                    CompOp::EndsWith => Ok(Expr::FunctionCall {
                        name: "endsWith".into(),
                        distinct: false,
                        args: vec![la, lb],
                    }),
                    CompOp::Contains => Ok(Expr::FunctionCall {
                        name: "contains".into(),
                        distinct: false,
                        args: vec![la, lb],
                    }),
                    CompOp::RegexMatch => Ok(Expr::FunctionCall {
                        name: "regex".into(),
                        distinct: false,
                        args: vec![la, lb],
                    }),
                }
            }
            AE::FunctionCall {
                name,
                distinct,
                args,
            } => {
                let largs = args
                    .iter()
                    .map(|a| self.lower_expr(a))
                    .collect::<Result<_, _>>()?;
                Ok(Expr::FunctionCall {
                    name: name.clone(),
                    distinct: *distinct,
                    args: largs,
                })
            }
            AE::Aggregate(agg) => self.lower_agg(agg),
            AE::LabelCheck { variable, labels } => Ok(Expr::LabelCheck {
                expr: Box::new(Expr::var(variable)),
                labels: labels.clone(),
            }),
            AE::List(items) => {
                let litems = items
                    .iter()
                    .map(|i| self.lower_expr(i))
                    .collect::<Result<_, _>>()?;
                Ok(Expr::List(litems))
            }
            AE::Map(pairs) => {
                let lpairs = pairs
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.lower_expr(v)?)))
                    .collect::<Result<Vec<_>, PolygraphError>>()?;
                Ok(Expr::Map(lpairs))
            }
            AE::Subscript(a, b) => Ok(Expr::Subscript(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            AE::ListSlice { list, start, end } => Ok(Expr::ListSlice {
                list: Box::new(self.lower_expr(list)?),
                start: start
                    .as_ref()
                    .map(|e| self.lower_expr(e))
                    .transpose()?
                    .map(Box::new),
                end: end
                    .as_ref()
                    .map(|e| self.lower_expr(e))
                    .transpose()?
                    .map(Box::new),
            }),
            AE::QuantifierExpr {
                kind,
                variable,
                list,
                predicate,
            } => {
                let qkind = match kind {
                    QuantifierKind::All => QuantKind::All,
                    QuantifierKind::Any => QuantKind::Any,
                    QuantifierKind::None => QuantKind::None,
                    QuantifierKind::Single => QuantKind::Single,
                };
                let pred = match predicate {
                    Some(p) => self.lower_expr(p)?,
                    None => Expr::Literal(Literal::Boolean(true)),
                };
                Ok(Expr::Quantifier {
                    kind: qkind,
                    variable: variable.clone(),
                    list: Box::new(self.lower_expr(list)?),
                    predicate: Box::new(pred),
                })
            }
            AE::ListComprehension {
                variable,
                list,
                predicate,
                projection,
            } => Ok(Expr::ListComprehension {
                variable: variable.clone(),
                list: Box::new(self.lower_expr(list)?),
                predicate: predicate
                    .as_ref()
                    .map(|p| self.lower_expr(p))
                    .transpose()?
                    .map(Box::new),
                projection: projection
                    .as_ref()
                    .map(|p| self.lower_expr(p))
                    .transpose()?
                    .map(Box::new),
            }),
            AE::PatternComprehension {
                alias: _alias,
                pattern,
                predicate,
                projection,
            } => {
                let pattern_op = self.lower_pattern(pattern)?;
                let subq_op = if let Some(p) = predicate {
                    let pred = self.lower_expr(p)?;
                    Op::Selection {
                        inner: Box::new(pattern_op),
                        predicate: pred,
                    }
                } else {
                    pattern_op
                };
                Ok(Expr::PatternComprehension {
                    alias: None,
                    pattern_op: Box::new(subq_op),
                    predicate: None,
                    projection: Box::new(self.lower_expr(projection)?),
                })
            }
            AE::CaseExpression {
                operand,
                whens,
                else_expr,
            } => {
                let branches = if let Some(subj) = operand {
                    // Simple CASE → normalise to searched CASE
                    let lsubj = self.lower_expr(subj)?;
                    whens
                        .iter()
                        .map(|(w, t)| {
                            Ok((
                                Expr::Comparison(
                                    CmpOp::Eq,
                                    Box::new(lsubj.clone()),
                                    Box::new(self.lower_expr(w)?),
                                ),
                                self.lower_expr(t)?,
                            ))
                        })
                        .collect::<Result<Vec<_>, PolygraphError>>()?
                } else {
                    whens
                        .iter()
                        .map(|(w, t)| Ok((self.lower_expr(w)?, self.lower_expr(t)?)))
                        .collect::<Result<Vec<_>, PolygraphError>>()?
                };
                Ok(Expr::CaseSearched {
                    branches,
                    else_expr: else_expr
                        .as_ref()
                        .map(|e| self.lower_expr(e))
                        .transpose()?
                        .map(Box::new),
                })
            }
            AE::ExistsSubquery { patterns, where_ } => {
                let pat_op = self.lower_pattern_list(patterns)?;
                let subq = if let Some(w) = where_ {
                    let pred = self.lower_expr(w)?;
                    Op::Selection {
                        inner: Box::new(pat_op),
                        predicate: pred,
                    }
                } else {
                    pat_op
                };
                Ok(Expr::Exists(Box::new(subq)))
            }
            AE::PatternPredicate(pat) => {
                let pat_op = self.lower_pattern(pat)?;
                Ok(Expr::Exists(Box::new(pat_op)))
            }
        }
    }

    fn lower_agg(&mut self, agg: &AggregateExpr) -> Result<Expr, PolygraphError> {
        match agg {
            AggregateExpr::Count { distinct, expr } => {
                let e = expr.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                Ok(Expr::Aggregate {
                    kind: AggKind::Count,
                    distinct: *distinct,
                    arg: e.map(Box::new),
                })
            }
            AggregateExpr::Sum { distinct, expr } => Ok(Expr::Aggregate {
                kind: AggKind::Sum,
                distinct: *distinct,
                arg: Some(Box::new(self.lower_expr(expr)?)),
            }),
            AggregateExpr::Avg { distinct, expr } => Ok(Expr::Aggregate {
                kind: AggKind::Avg,
                distinct: *distinct,
                arg: Some(Box::new(self.lower_expr(expr)?)),
            }),
            AggregateExpr::Min { distinct, expr } => Ok(Expr::Aggregate {
                kind: AggKind::Min,
                distinct: *distinct,
                arg: Some(Box::new(self.lower_expr(expr)?)),
            }),
            AggregateExpr::Max { distinct, expr } => Ok(Expr::Aggregate {
                kind: AggKind::Max,
                distinct: *distinct,
                arg: Some(Box::new(self.lower_expr(expr)?)),
            }),
            AggregateExpr::Collect { distinct, expr } => Ok(Expr::Aggregate {
                kind: AggKind::Collect,
                distinct: *distinct,
                arg: Some(Box::new(self.lower_expr(expr)?)),
            }),
        }
    }

    // ── Write clause helpers ──────────────────────────────────────────────────

    fn lower_create_pattern(
        &mut self,
        pl: &ast::PatternList,
    ) -> Result<(Vec<CreateNode>, Vec<CreateEdge>), PolygraphError> {
        use crate::lqa::op::CreateNode;
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        for pat in &pl.0 {
            let mut node_names: Vec<String> = Vec::new();
            for elem in &pat.elements {
                if let PatternElement::Node(n) = elem {
                    node_names.push(n.variable.clone().unwrap_or_else(|| self.fresh("anon")));
                }
            }

            let mut node_idx = 0usize;
            let mut last_src: Option<String> = None;
            for elem in &pat.elements {
                match elem {
                    PatternElement::Node(n) => {
                        let var = node_names[node_idx].clone();
                        node_idx += 1;
                        let props = n
                            .properties
                            .as_deref()
                            .map(|ps| {
                                ps.iter()
                                    .map(|(k, v)| Ok((k.clone(), self.lower_expr(v)?)))
                                    .collect::<Result<Vec<_>, PolygraphError>>()
                            })
                            .transpose()?
                            .unwrap_or_default();
                        nodes.push(CreateNode {
                            variable: Some(var.clone()),
                            labels: n.labels.clone(),
                            properties: props,
                        });
                        last_src = Some(var);
                    }
                    PatternElement::Relationship(r) => {
                        let from = last_src.clone().unwrap_or_default();
                        let to = node_names[node_idx].clone();
                        let props = r
                            .properties
                            .as_deref()
                            .map(|ps| {
                                ps.iter()
                                    .map(|(k, v)| Ok((k.clone(), self.lower_expr(v)?)))
                                    .collect::<Result<Vec<_>, PolygraphError>>()
                            })
                            .transpose()?
                            .unwrap_or_default();
                        edges.push(CreateEdge {
                            variable: r.variable.clone(),
                            from,
                            to,
                            rel_type: r.rel_types.first().cloned().unwrap_or_default(),
                            direction: match r.direction {
                                Direction::Right => LqaDir::Outgoing,
                                Direction::Left => LqaDir::Incoming,
                                Direction::Both => LqaDir::Undirected,
                            },
                            properties: props,
                        });
                    }
                }
            }
        }
        Ok((nodes, edges))
    }

    fn lower_set_items(
        &mut self,
        items: &[ast::SetItem],
    ) -> Result<Vec<LqaSetItem>, PolygraphError> {
        items
            .iter()
            .map(|item| match item {
                ast::SetItem::Property {
                    variable,
                    key,
                    value,
                } => Ok(LqaSetItem::Property {
                    variable: variable.clone(),
                    key: key.clone(),
                    value: self.lower_expr(value)?,
                }),
                ast::SetItem::MergeMap { variable, map } => {
                    let props = map
                        .iter()
                        .map(|(k, v)| Ok((k.clone(), self.lower_expr(v)?)))
                        .collect::<Result<Vec<_>, PolygraphError>>()?;
                    Ok(LqaSetItem::MergeMap {
                        variable: variable.clone(),
                        map: Expr::Map(props),
                    })
                }
                ast::SetItem::NodeReplace { variable, value } => Ok(LqaSetItem::Replace {
                    variable: variable.clone(),
                    value: self.lower_expr(value)?,
                }),
                ast::SetItem::SetLabel { variable, labels } => Ok(LqaSetItem::Label {
                    variable: variable.clone(),
                    labels: labels.clone(),
                }),
            })
            .collect()
    }

    fn lower_remove_items(
        &mut self,
        items: &[ast::RemoveItem],
    ) -> Result<Vec<LqaRemoveItem>, PolygraphError> {
        items
            .iter()
            .map(|item| match item {
                ast::RemoveItem::Property { variable, key } => Ok(LqaRemoveItem::Property {
                    variable: variable.clone(),
                    key: key.clone(),
                }),
                ast::RemoveItem::Label { variable, labels } => Ok(LqaRemoveItem::Label {
                    variable: variable.clone(),
                    labels: labels.clone(),
                }),
            })
            .collect()
    }

    fn lower_merge_clause(
        &mut self,
        m: &ast::MergeClause,
    ) -> Result<LqaMergeClause, PolygraphError> {
        let pattern_op = self.lower_pattern(&m.pattern)?;
        let mut on_match = Vec::new();
        let mut on_create = Vec::new();
        for action in &m.actions {
            let items = self.lower_set_items(&action.items)?;
            if action.on_create {
                on_create.extend(items);
            } else {
                on_match.extend(items);
            }
        }
        Ok(LqaMergeClause {
            pattern: Box::new(pattern_op),
            on_match,
            on_create,
        })
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

fn lower_literal(l: &ast::Literal) -> Literal {
    match l {
        ast::Literal::Integer(n) => Literal::Integer(*n),
        ast::Literal::Float(f) => Literal::Float(*f),
        ast::Literal::String(s) => Literal::String(s.clone()),
        ast::Literal::Boolean(b) => Literal::Boolean(*b),
        ast::Literal::Null => Literal::Null,
    }
}

/// Extract GROUP BY keys from a projection list: variables that are NOT
/// themselves aggregate-output aliases.
///
/// After `lower_return_items`, every aggregate `AGG(x) AS alias` produces:
///   - an `AggItem { alias: "alias", … }` in the agg list
///   - a `ProjItem { expr: Var("alias"), alias: "alias" }` in the proj list
///
/// Those proj-list entries must NOT become GROUP BY keys — the alias is the
/// aggregate output, not an input column.
fn proj_cols_keys(items: &[ProjItem], agg_aliases: &[String]) -> Vec<String> {
    let agg_set: std::collections::HashSet<&str> =
        agg_aliases.iter().map(|s| s.as_str()).collect();
    items
        .iter()
        .filter_map(|pi| {
            // Every non-aggregate, non-wildcard projection item is a GROUP BY key.
            // This includes both Variable references (already-bound vars) and
            // Property-access expressions (e.g. `n.city AS city`) — the property
            // triple for property-access keys is generated inside the Group inner
            // by the SPARQL lowerer.
            if pi.alias != "*" && !agg_set.contains(pi.alias.as_str()) {
                Some(pi.alias.clone())
            } else {
                None
            }
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_cypher;

    fn lower(src: &str) -> Op {
        let ast = parse_cypher(src).expect("parse");
        let mut l = AstLowerer::new();
        l.lower_query(&ast).expect("lower")
    }

    #[test]
    fn scan_with_label() {
        let op = lower("MATCH (n:Person) RETURN n");
        // Must have a Scan or Projection somewhere above it
        assert!(format!("{op:?}").contains("Scan"));
    }

    #[test]
    fn selection_from_where() {
        let op = lower("MATCH (n:Person) WHERE n.age > 30 RETURN n.name");
        let s = format!("{op:?}");
        assert!(s.contains("Selection"));
        assert!(s.contains("Projection"));
    }

    #[test]
    fn relationship_pattern() {
        let op = lower("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name");
        let s = format!("{op:?}");
        assert!(s.contains("Expand"));
    }

    #[test]
    fn union_all() {
        let op = lower("MATCH (n:A) RETURN n UNION ALL MATCH (n:B) RETURN n");
        assert!(matches!(op, Op::UnionAll { .. }));
    }

    #[test]
    fn with_clause() {
        let op = lower("MATCH (n:Person) WITH n RETURN n");
        let s = format!("{op:?}");
        assert!(s.contains("Projection"));
    }

    #[test]
    fn optional_match() {
        let op = lower("MATCH (n) OPTIONAL MATCH (n)-[r]->(m) RETURN n, m");
        assert!(
            matches!(&op, Op::Projection { inner, .. } if matches!(inner.as_ref(), Op::LeftOuterJoin { .. }))
        );
    }

    #[test]
    fn order_by_limit() {
        let op = lower("MATCH (n:Person) RETURN n.name ORDER BY n.name LIMIT 10");
        let s = format!("{op:?}");
        assert!(s.contains("OrderBy"));
        assert!(s.contains("Limit"));
    }

    #[test]
    fn aggregate() {
        let op = lower("MATCH (n:Person) RETURN count(n) AS cnt");
        let s = format!("{op:?}");
        assert!(s.contains("GroupBy"));
    }
}
