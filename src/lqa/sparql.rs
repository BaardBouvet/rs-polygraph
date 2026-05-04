//! Phase 4.5 — LQA → SPARQL lowering pass.
//!
//! Compiles an [`Op`] tree produced by [`crate::lqa::lower`] into a complete
//! [`spargebra::Query`] that can be serialised and executed.
//!
//! # Design
//!
//! The central challenge is **property access**: `Expr::Property(n, "age")`
//! cannot be directly expressed as a SPARQL expression — it must be materialised
//! as a fresh SPARQL variable `?_n_age_0` with an accompanying BGP triple
//! `?n <base:age> ?_n_age_0` injected into the surrounding graph pattern.
//!
//! This module threads a [`Ctx`] carrying `pending_triples` through all
//! expression-lowering calls.  After lowering an expression, the caller is
//! responsible for flushing `pending_triples` into the current graph pattern
//! (see `flush_pending`).
//!
//! # Fallback
//!
//! Complex constructs (variable-length paths, temporal arithmetic, list
//! comprehensions, write operators) return [`PolygraphError::Unsupported`].
//! The calling code in [`crate::lib`] catches this and falls back to the
//! legacy [`crate::translator::cypher`] path, so the TCK floor is maintained.

use std::collections::{HashMap, HashSet};

use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression as SparExpr, Function, GraphPattern,
    OrderExpression,
};
use spargebra::term::{
    Literal as SparLit, NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable,
};
use spargebra::Query;

use crate::error::PolygraphError;
use crate::lqa::expr::{AggKind, CmpOp, Expr, Literal, SortDir, UnaryOp};
use crate::lqa::op::{AggItem, Direction, Op, ProjItem, SortKey};
use crate::result_mapping::schema::{ColumnKind, ProjectedColumn, ProjectionSchema};

// Helper to build a scalar projected column with a single SPARQL variable.
fn scalar_col(name: impl Into<String>) -> ProjectedColumn {
    let n = name.into();
    ProjectedColumn {
        name: n.clone(),
        kind: ColumnKind::Scalar { var: n },
    }
}

// ── RDF / XSD constants ───────────────────────────────────────────────────────

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
#[allow(dead_code)]
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const DEFAULT_BASE: &str = "http://polygraph.example/";

// ── Public result type ────────────────────────────────────────────────────────

pub struct CompiledQuery {
    pub sparql: String,
    pub schema: ProjectionSchema,
}

// ── Compiler state ────────────────────────────────────────────────────────────

/// Predicate info for a named relationship variable.
#[derive(Debug, Clone)]
enum EdgePred {
    /// Single typed relationship: the IRI is statically known.
    Static(NamedNode),
    /// Multi-type or untyped: a SPARQL variable captures the predicate at runtime.
    Dynamic(Variable),
}

/// Tracking info needed to lower property access and `type()` on a named rel-var.
#[derive(Debug, Clone)]
struct EdgeVarInfo {
    /// SPARQL variable name for the canonical RDF triple's subject.
    subj: String,
    /// SPARQL variable name for the canonical RDF triple's object.
    obj: String,
    /// How to refer to the edge predicate in SPARQL.
    pred: EdgePred,
}

struct Compiler {
    base_iri: String,
    counter: u32,
    /// Property-access triple patterns accumulated while lowering an expression.
    pending_triples: Vec<TriplePattern>,
    /// Property-access triple patterns that must be emitted as OPTIONAL { } blocks
    /// (e.g. arguments to coalesce() where the property may be absent).
    pending_optional_triples: Vec<TriplePattern>,
    /// Variables that may be null (produced by OPTIONAL MATCH).
    #[allow(dead_code)]
    nullable: HashSet<String>,
    /// For each edge variable, the set of rel-type IRIs (used in error diagnostics).
    #[allow(dead_code)]
    edge_types: HashMap<String, Vec<String>>,
    /// Projected column schema collected from the topmost Projection op.
    projected_columns: Vec<ProjectedColumn>,
    return_distinct: bool,
    /// Variables bound by BIND/Extend (not by Scan/Expand) that hold scalar RDF values
    /// (literals, dates, etc.) rather than node IRIs.  Property access on these variables
    /// cannot be lowered to a triple pattern and must fall back to the legacy translator.
    scalar_vars: HashSet<String>,
    /// Tracking info for named relationship variables — used to lower `r.prop` and `type(r)`.
    edge_vars: HashMap<String, EdgeVarInfo>,
    /// Groups of optional triples that must be kept together in one OPTIONAL { } block.
    /// Edge property access (RDF-star reification) emits two triples that share a reifier
    /// variable and must not be split across separate OPTIONAL blocks.
    pending_optional_groups: Vec<Vec<TriplePattern>>,
}

impl Compiler {
    fn new(base_iri: String) -> Self {
        Self {
            base_iri,
            counter: 0,
            pending_triples: Vec::new(),
            pending_optional_triples: Vec::new(),
            nullable: HashSet::new(),
            edge_types: HashMap::new(),
            projected_columns: Vec::new(),
            return_distinct: false,
            scalar_vars: HashSet::new(),
            edge_vars: HashMap::new(),
            pending_optional_groups: Vec::new(),
        }
    }

    fn fresh(&mut self, prefix: &str) -> Variable {
        let c = self.counter;
        self.counter += 1;
        Variable::new_unchecked(format!("_{prefix}_{c}"))
    }

    fn var(name: &str) -> Variable {
        Variable::new_unchecked(name)
    }

    // ── IRI helpers ───────────────────────────────────────────────────────────

    fn prop_iri(&self, key: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("{}{}", self.base_iri, key))
    }

    fn label_iri(&self, label: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("{}{}", self.base_iri, label))
    }

    fn lit_integer(n: i64) -> SparExpr {
        SparExpr::Literal(SparLit::new_typed_literal(
            n.to_string(),
            NamedNode::new_unchecked(XSD_INTEGER),
        ))
    }

    fn lit_double(f: f64) -> SparExpr {
        SparExpr::Literal(SparLit::new_typed_literal(
            format!("{f:?}"),
            NamedNode::new_unchecked(XSD_DOUBLE),
        ))
    }

    fn lit_bool(b: bool) -> SparExpr {
        SparExpr::Literal(SparLit::new_typed_literal(
            b.to_string(),
            NamedNode::new_unchecked(XSD_BOOLEAN),
        ))
    }

    fn lit_str(s: &str) -> SparExpr {
        SparExpr::Literal(SparLit::new_simple_literal(s))
    }

    // ── Pending triple flush ──────────────────────────────────────────────────

    /// Take all pending BGP triples and join them into `pat` as a BGP.
    /// Any pending *optional* triples (from coalesce args, etc.) are appended
    /// as `OPTIONAL { }` blocks via LEFT JOIN.
    fn flush_pending(&mut self, pat: GraphPattern) -> GraphPattern {
        let triples = std::mem::take(&mut self.pending_triples);
        let opt_triples = std::mem::take(&mut self.pending_optional_triples);
        let opt_groups = std::mem::take(&mut self.pending_optional_groups);
        let mut result = if triples.is_empty() {
            pat
        } else {
            join(pat, GraphPattern::Bgp { patterns: triples })
        };
        for ot in opt_triples {
            result = GraphPattern::LeftJoin {
                left: Box::new(result),
                right: Box::new(GraphPattern::Bgp { patterns: vec![ot] }),
                expression: None,
            };
        }
        // Grouped optional triples (e.g. RDF-star reification pairs) must stay
        // together in one OPTIONAL block so the reifier variable links them.
        for group in opt_groups {
            if !group.is_empty() {
                result = GraphPattern::LeftJoin {
                    left: Box::new(result),
                    right: Box::new(GraphPattern::Bgp { patterns: group }),
                    expression: None,
                };
            }
        }
        result
    }

    // ── Op lowering ───────────────────────────────────────────────────────────

    /// Lower the Op tree and produce a full SELECT query.
    fn compile_inner(&mut self, op: &Op, base_iri: &str) -> Result<CompiledQuery, PolygraphError> {
        let pattern = self.lower_op_as_query(op)?;
        let schema = ProjectionSchema {
            columns: self.projected_columns.clone(),
            distinct: self.return_distinct,
            base_iri: base_iri.to_string(),
            rdf_star: false,
        };
        let query = Query::Select {
            dataset: None,
            pattern,
            base_iri: None,
        };
        Ok(CompiledQuery {
            sparql: query.to_string(),
            schema,
        })
    }

    /// Walk the top of the Op tree, peeling off query-level wrappers.
    fn lower_op_as_query(&mut self, op: &Op) -> Result<GraphPattern, PolygraphError> {
        match op {
            Op::Limit { inner, count } => {
                let length = expr_to_usize(count)?;
                let inner_pat = self.lower_op_as_query(inner)?;
                // If the direct inner is a SKIP-only Slice (from Op::Skip), merge it
                // with this LIMIT into a single Slice rather than creating nested
                // Slices that spargebra cannot always flatten into one OFFSET+LIMIT.
                let (start, unwrapped) = match inner_pat {
                    GraphPattern::Slice {
                        inner: skip_inner,
                        start: skip_start,
                        length: None,
                    } => (skip_start, *skip_inner),
                    other => (0, other),
                };
                Ok(GraphPattern::Slice {
                    inner: Box::new(unwrapped),
                    start,
                    length: Some(length),
                })
            }
            Op::Skip { inner, count } => {
                let start = expr_to_usize(count)?;
                let inner_pat = self.lower_op_as_query(inner)?;
                Ok(GraphPattern::Slice {
                    inner: Box::new(inner_pat),
                    start,
                    length: None,
                })
            }
            Op::OrderBy { inner, keys } => {
                // When ORDER BY wraps a Projection (RETURN clause), flatten the
                // projected body so that sort-key property triples live in the
                // same WHERE scope as the MATCH patterns.  Creating a nested
                // sub-SELECT here would hide ?node variables from sort triples
                // added after the sub-SELECT boundary.
                if let Op::Projection {
                    inner: proj_inner,
                    items,
                    distinct,
                } = inner.as_ref()
                {
                    // 1. Lower sort-key expressions first; capture any property
                    //    triples they generate.
                    //
                    //    If the sort key is a variable alias from the RETURN clause:
                    //    - If it's a GROUP BY key or aggregate output, use the
                    //      variable directly (it's already bound by the Group).
                    //    - If it's a computed expression alias (e.g. n.name + '!'),
                    //      inline the underlying expression so ORDER BY doesn't
                    //      reference a SELECT-clause alias that may be unbound at
                    //      sort time in some SPARQL engines.
                    let agg_alias_set: std::collections::HashSet<&str> =
                        if let Op::GroupBy { agg_items, .. } = proj_inner.as_ref() {
                            agg_items.iter().map(|a| a.alias.as_str()).collect()
                        } else {
                            std::collections::HashSet::new()
                        };
                    // GROUP BY key aliases are also "already bound" after evaluation
                    // of the Group pattern — no need to expand them to property exprs.
                    let group_key_aliases: std::collections::HashSet<&str> =
                        if let Op::GroupBy { group_keys, .. } = proj_inner.as_ref() {
                            group_keys.iter().map(|k| k.as_str()).collect()
                        } else {
                            std::collections::HashSet::new()
                        };
                    let sort_exprs = keys
                        .iter()
                        .map(|sk| {
                            // Expand alias reference to underlying expression when
                            // the alias refers to a computed (non-variable) RETURN
                            // expression and is not a GROUP BY key or aggregate alias.
                            let effective = if let Expr::Variable { name, .. } = &sk.expr {
                                let is_agg = agg_alias_set.contains(name.as_str());
                                let is_gk = group_key_aliases.contains(name.as_str());
                                if !is_agg && !is_gk {
                                    items
                                        .iter()
                                        .find(|pi| {
                                            pi.alias == *name
                                                && !matches!(pi.expr, Expr::Variable { .. })
                                        })
                                        .map(|pi| &pi.expr)
                                        .unwrap_or(&sk.expr)
                                } else {
                                    &sk.expr
                                }
                            } else {
                                &sk.expr
                            };
                            let sparql_expr = self.lower_expr(effective)?;
                            Ok(match sk.dir {
                                SortDir::Asc => OrderExpression::Asc(sparql_expr),
                                SortDir::Desc => OrderExpression::Desc(sparql_expr),
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let sort_req = std::mem::take(&mut self.pending_triples);
                    let sort_opt = std::mem::take(&mut self.pending_optional_triples);

                    // 2. Flatten the Projection body (handles GroupBy etc.).
                    let (proj_gp, _agg_vars) = self.lower_projection_inner(proj_inner, items)?;
                    let project_vars = self.build_project_vars(items)?;

                    // 3. Flush projection's own pending triples.
                    let mut flat = self.flush_pending(proj_gp);

                    // 4. Inject sort-key triples into the same flat scope.
                    if !sort_req.is_empty() {
                        flat = join(flat, GraphPattern::Bgp { patterns: sort_req });
                    }
                    for ot in sort_opt {
                        flat = GraphPattern::LeftJoin {
                            left: Box::new(flat),
                            right: Box::new(GraphPattern::Bgp { patterns: vec![ot] }),
                            expression: None,
                        };
                    }

                    // 5. Wrap: OrderBy → Project → (Distinct if needed).
                    let ordered = GraphPattern::OrderBy {
                        inner: Box::new(flat),
                        expression: sort_exprs,
                    };
                    let projected = if project_vars.is_empty() {
                        ordered
                    } else {
                        GraphPattern::Project {
                            inner: Box::new(ordered),
                            variables: project_vars,
                        }
                    };
                    return Ok(if *distinct {
                        GraphPattern::Distinct {
                            inner: Box::new(projected),
                        }
                    } else {
                        projected
                    });
                }

                // Default path: inner is not a Projection (e.g. mid-pipeline
                // OrderBy from a WITH clause).
                let inner_pat = self.lower_op_as_query(inner)?;
                let expressions = keys
                    .iter()
                    .map(|sk| self.lower_order_key(sk))
                    .collect::<Result<Vec<_>, _>>()?;
                let flushed = self.flush_pending(inner_pat);
                Ok(GraphPattern::OrderBy {
                    inner: Box::new(flushed),
                    expression: expressions,
                })
            }
            Op::Distinct { inner } => {
                self.return_distinct = true;
                let inner_pat = self.lower_op_as_query(inner)?;
                Ok(GraphPattern::Distinct {
                    inner: Box::new(inner_pat),
                })
            }
            Op::Projection {
                inner,
                items,
                distinct,
            } => {
                self.return_distinct = *distinct;
                let (inner_gp, _agg_vars) = self.lower_projection_inner(inner, items)?;
                let project_vars = self.build_project_vars(items)?;
                let flushed = self.flush_pending(inner_gp);
                let mut projected =
                    if project_vars.is_empty() || items.iter().any(|pi| pi.alias == "*") {
                        flushed
                    } else {
                        GraphPattern::Project {
                            inner: Box::new(flushed),
                            variables: project_vars,
                        }
                    };
                if *distinct {
                    projected = GraphPattern::Distinct {
                        inner: Box::new(projected),
                    };
                }
                Ok(projected)
            }
            other => self.lower_op(other),
        }
    }

    fn lower_projection_inner(
        &mut self,
        inner: &Op,
        proj_items: &[ProjItem],
    ) -> Result<(GraphPattern, Vec<Variable>), PolygraphError> {
        if let Op::GroupBy {
            inner: gb_inner,
            group_keys,
            agg_items,
        } = inner
        {
            let inner_gp = self.lower_op(gb_inner)?;
            let flushed = self.flush_pending(inner_gp);

            let group_key_set: std::collections::HashSet<&str> =
                group_keys.iter().map(|s| s.as_str()).collect();

            // Group variables from keys; start collecting them.
            let mut group_vars: Vec<Variable> = Vec::new();
            // Complex group-key expressions (not simple Variable or Property) that need
            // a BIND inside the group body before the Group is formed.
            let mut complex_group_binds: Vec<(Variable, SparExpr)> = Vec::new();

            // For GROUP BY key expressions that are Property accesses
            // (e.g. `n.city AS city`), generate the property triple
            // inside the Group inner using the alias variable directly —
            // no fresh intermediate variable.  That way the GROUP BY
            // variable is the same as the output alias.
            for pi in proj_items {
                if !group_key_set.contains(pi.alias.as_str()) {
                    continue; // aggregate output or wildcard, skip
                }
                let alias_var = Self::var(&pi.alias);
                match &pi.expr {
                    Expr::Variable { name, .. } => {
                        // Pre-bound variable — just track it as a group var.
                        group_vars.push(Self::var(name));
                    }
                    Expr::Property(node_expr, prop_key) => {
                        // Property access: produce ?node :prop ?alias triple inside
                        // the Group inner, using the alias variable directly.
                        let node_var = match node_expr.as_ref() {
                            Expr::Variable { name, .. } => Self::var(name),
                            other => {
                                return Err(PolygraphError::Unsupported {
                                    construct: format!("complex GROUP BY key expr {:?}", other),
                                    spec_ref: "openCypher 9 §3.4".into(),
                                    reason: "non-variable base in property GROUP BY key".into(),
                                })
                            }
                        };
                        let pred = NamedNodePattern::NamedNode(self.prop_iri(prop_key));
                        self.pending_triples.push(TriplePattern {
                            subject: TermPattern::Variable(node_var),
                            predicate: pred,
                            object: TermPattern::Variable(alias_var.clone()),
                        });
                        group_vars.push(alias_var.clone());
                        self.projected_columns.push(scalar_col(&pi.alias));
                    }
                    _ => {
                        // Complex expression (e.g. `x IS NULL`, function calls) —
                        // evaluate and bind to the alias variable inside the group body
                        // so that GROUP BY can reference it.
                        let e = self.lower_expr(&pi.expr)?;
                        // Flush any required pending triples produced by lower_expr.
                        // We'll apply them to the group inner below.
                        let pending_req = std::mem::take(&mut self.pending_triples);
                        // Re-add them so flush_pending picks them up later.
                        self.pending_triples.extend(pending_req);
                        complex_group_binds.push((alias_var.clone(), e));
                        group_vars.push(alias_var.clone());
                        self.projected_columns.push(scalar_col(&pi.alias));
                    }
                }
            }

            // Lower aggregates — this may add property-access triples to
            // `pending_triples` (e.g. AVG(n.age) → fresh ?_age_0 + pending triple).
            // Those triples must live INSIDE the Group inner, not outside it.
            let aggregates = agg_items
                .iter()
                .map(|ai| self.lower_agg_item(ai))
                .collect::<Result<Vec<_>, _>>()?;

            // Flush all pending triples (group-key property triples +
            // agg-arg property triples) into the inner pattern.
            let mut group_inner = self.flush_pending(flushed);
            // Apply complex group-key BIND expressions inside the group body.
            for (var, expr) in complex_group_binds {
                group_inner = GraphPattern::Extend {
                    inner: Box::new(group_inner),
                    variable: var,
                    expression: expr,
                };
            }

            let group_pattern = GraphPattern::Group {
                inner: Box::new(group_inner),
                variables: group_vars.clone(),
                aggregates,
            };

            // Emit any remaining non-group, non-agg proj items as Extends
            // (aggregate output aliases need no Extend; they're bound by the Group).
            let agg_alias_set: std::collections::HashSet<&str> =
                agg_items.iter().map(|a| a.alias.as_str()).collect();
            let mut extended = group_pattern;
            for pi in proj_items {
                if agg_alias_set.contains(pi.alias.as_str()) {
                    // Aggregate output: variable bound by the Group pattern.
                    self.projected_columns.push(scalar_col(&pi.alias));
                    continue;
                }
                if group_key_set.contains(pi.alias.as_str()) {
                    // Already handled above (property triple or variable passthrough).
                    continue;
                }
                let sparql_expr = self.lower_expr(&pi.expr)?;
                let flush = std::mem::take(&mut self.pending_triples);
                let target = Self::var(&pi.alias);
                extended = GraphPattern::Extend {
                    inner: Box::new(extended),
                    variable: target,
                    expression: sparql_expr,
                };
                if !flush.is_empty() {
                    extended = join(GraphPattern::Bgp { patterns: flush }, extended);
                }
                self.projected_columns.push(scalar_col(&pi.alias));
            }

            Ok((extended, group_vars))
        } else {
            let inner_gp = self.lower_op(inner)?;
            let mut extended = inner_gp;
            for pi in proj_items {
                if pi.alias == "*" {
                    continue;
                }
                if let Expr::Variable { name, .. } = &pi.expr {
                    if *name == pi.alias {
                        self.projected_columns.push(scalar_col(name.clone()));
                        continue;
                    }
                }
                let sparql_expr = self.lower_expr(&pi.expr)?;
                // Flush required and optional pending triples BEFORE wrapping in
                // Extend, so that OPTIONAL { } blocks appear BEFORE the BIND and
                // the bound variables are in scope when BIND executes.
                extended = self.flush_pending(extended);
                let alias_var = Self::var(&pi.alias);
                extended = GraphPattern::Extend {
                    inner: Box::new(extended),
                    variable: alias_var,
                    expression: sparql_expr,
                };
                self.projected_columns.push(scalar_col(&pi.alias));
            }
            Ok((extended, vec![]))
        }
    }

    fn lower_agg_item(
        &mut self,
        ai: &AggItem,
    ) -> Result<(Variable, AggregateExpression), PolygraphError> {
        let out_var = Self::var(&ai.alias);
        if let Expr::Aggregate {
            kind,
            distinct,
            arg,
        } = &ai.expr
        {
            let agg_expr = match kind {
                AggKind::Count => {
                    if let Some(arg_expr) = arg {
                        AggregateExpression::FunctionCall {
                            name: AggregateFunction::Count,
                            expr: self.lower_expr(arg_expr)?,
                            distinct: *distinct,
                        }
                    } else {
                        AggregateExpression::CountSolutions {
                            distinct: *distinct,
                        }
                    }
                }
                AggKind::Sum => AggregateExpression::FunctionCall {
                    name: AggregateFunction::Sum,
                    expr: self.lower_expr(arg.as_deref().unwrap())?,
                    distinct: *distinct,
                },
                AggKind::Avg => AggregateExpression::FunctionCall {
                    name: AggregateFunction::Avg,
                    expr: self.lower_expr(arg.as_deref().unwrap())?,
                    distinct: *distinct,
                },
                AggKind::Min => AggregateExpression::FunctionCall {
                    name: AggregateFunction::Min,
                    expr: self.lower_expr(arg.as_deref().unwrap())?,
                    distinct: *distinct,
                },
                AggKind::Max => AggregateExpression::FunctionCall {
                    name: AggregateFunction::Max,
                    expr: self.lower_expr(arg.as_deref().unwrap())?,
                    distinct: *distinct,
                },
                AggKind::Collect => {
                    // collect() maps to SPARQL GROUP_CONCAT which serialises to a
                    // string, not a list.  Cypher `collect()` semantics require a
                    // true list type which the LQA path doesn't yet encode; fall
                    // back to the legacy translator for any query using collect().
                    return Err(PolygraphError::Unsupported {
                        construct: "collect() aggregate".into(),
                        spec_ref: "openCypher 9 §3.4.6".into(),
                        reason: "collect() requires list encoding not yet in LQA path; legacy fallback applies".into(),
                    });
                }
                AggKind::CountStar => AggregateExpression::CountSolutions {
                    distinct: *distinct,
                },
            };
            Ok((out_var, agg_expr))
        } else {
            Err(PolygraphError::Translation {
                message: format!("AggItem.expr is not Aggregate: {:?}", ai.expr),
            })
        }
    }

    fn build_project_vars(&self, items: &[ProjItem]) -> Result<Vec<Variable>, PolygraphError> {
        if items.iter().any(|pi| pi.alias == "*") {
            return Ok(vec![]);
        }
        Ok(items.iter().map(|pi| Self::var(&pi.alias)).collect())
    }

    fn lower_op(&mut self, op: &Op) -> Result<GraphPattern, PolygraphError> {
        match op {
            Op::Unit => Ok(GraphPattern::Bgp { patterns: vec![] }),

            Op::Scan {
                variable,
                label,
                extra_labels,
            } => {
                let subj = TermPattern::Variable(Self::var(variable));

                let label = match label {
                    Some(l) => l,
                    None => {
                        // Unlabeled node scan: use the __node existence sentinel.
                        // Every graph node carries exactly one `<base:__node> <base:__node>`
                        // triple inserted by the TCK data loader.
                        let sentinel_iri =
                            NamedNode::new_unchecked(format!("{}{}", self.base_iri, "__node"));
                        return Ok(GraphPattern::Bgp {
                            patterns: vec![TriplePattern {
                                subject: subj,
                                predicate: NamedNodePattern::NamedNode(sentinel_iri.clone()),
                                object: TermPattern::NamedNode(sentinel_iri),
                            }],
                        });
                    }
                };

                let mut patterns = vec![TriplePattern {
                    subject: subj.clone(),
                    predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(RDF_TYPE)),
                    object: TermPattern::NamedNode(self.label_iri(label)),
                }];

                for lbl in extra_labels {
                    patterns.push(TriplePattern {
                        subject: subj.clone(),
                        predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(RDF_TYPE)),
                        object: TermPattern::NamedNode(self.label_iri(lbl)),
                    });
                }

                Ok(GraphPattern::Bgp { patterns })
            }

            Op::Expand {
                inner,
                from,
                rel_var,
                to,
                rel_types,
                direction,
                range,
                ..
            } => {
                let inner_pat = self.lower_op(inner)?;
                let from_tp = TermPattern::Variable(Self::var(from));
                let to_tp = TermPattern::Variable(Self::var(to));

                // ── Variable-length paths ───────────────────────────────────
                if let Some(path_range) = range {
                    let edge_bgp =
                        self.lower_varlen(from_tp, to_tp, rel_types, direction, path_range)?;
                    return Ok(join(inner_pat, edge_bgp));
                }

                // ── Named relationship variable ──────────────────────────────
                if let Some(rv) = rel_var {
                    // Register static rel types for type(r) fast path.
                    self.edge_types.insert(rv.clone(), rel_types.clone());

                    let edge_bgp = self.lower_expand_rel_var(from, to, rel_types, direction, rv)?;
                    return Ok(join(inner_pat, edge_bgp));
                }

                // ── Anonymous expansion (no rel-var, no path range) ──────────
                let rel_bgp = if rel_types.is_empty() {
                    let pred_var = self.fresh("rtype");
                    self.lower_expand_any_type(from_tp, pred_var, to_tp, direction)
                } else if rel_types.len() == 1 {
                    let pred = NamedNodePattern::NamedNode(NamedNode::new_unchecked(format!(
                        "{}{}",
                        self.base_iri, &rel_types[0]
                    )));
                    self.lower_expand_typed(from_tp, pred, to_tp, direction)
                } else {
                    let mut union_pats: Vec<GraphPattern> = rel_types
                        .iter()
                        .map(|rt| {
                            let pred = NamedNodePattern::NamedNode(NamedNode::new_unchecked(
                                format!("{}{}", self.base_iri, rt),
                            ));
                            self.lower_expand_typed(from_tp.clone(), pred, to_tp.clone(), direction)
                        })
                        .collect();
                    let first = union_pats.remove(0);
                    union_pats
                        .into_iter()
                        .fold(first, |acc, pat| GraphPattern::Union {
                            left: Box::new(acc),
                            right: Box::new(pat),
                        })
                };
                Ok(join(inner_pat, rel_bgp))
            }

            Op::Values { bindings } => {
                if bindings.is_empty() {
                    return Ok(GraphPattern::Bgp { patterns: vec![] });
                }
                let vars: Vec<Variable> =
                    bindings.iter().map(|(name, _)| Self::var(name)).collect();
                let row = bindings
                    .iter()
                    .map(|(_, expr)| literal_to_ground(expr))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(GraphPattern::Values {
                    variables: vars,
                    bindings: vec![row],
                })
            }

            Op::Selection { inner, predicate } => {
                let inner_pat = self.lower_op(inner)?;
                let expr = self.lower_expr(predicate)?;
                let flushed = self.flush_pending(inner_pat);
                Ok(GraphPattern::Filter {
                    expr,
                    inner: Box::new(flushed),
                })
            }

            Op::Projection { inner, items, .. } => {
                // Mid-pipeline Projection (from WITH clause): flatten as Extend + Filter
                // rather than creating a nested SELECT subquery. A nested SELECT in SPARQL
                // hides internal variables from the outer scope, breaking WHERE clauses and
                // RETURN expressions that reference those variables.
                let mut gp = self.lower_op(inner)?;
                for pi in items {
                    if pi.alias == "*" {
                        continue;
                    }
                    match &pi.expr {
                        Expr::Variable { name, .. } if *name == pi.alias => {
                            // Pure passthrough — no Extend needed.
                        }
                        _ => {
                            // Emit Extend to bind the alias variable.
                            let e = self.lower_expr(&pi.expr)?;
                            // Flush both required and optional pending triples BEFORE the BIND
                            // so the OPTIONAL { } blocks that define helper variables appear
                            // in SPARQL order before the BIND that uses them.
                            gp = self.flush_pending(gp);
                            gp = GraphPattern::Extend {
                                inner: Box::new(gp),
                                variable: Self::var(&pi.alias),
                                expression: e,
                            };
                            self.scalar_vars.insert(pi.alias.clone());
                        }
                    }
                }
                Ok(gp)
            }

            Op::GroupBy {
                inner,
                group_keys: _,
                agg_items: _,
            } => {
                // GroupBy mid-pipeline should not happen without a surrounding Projection;
                // lower the inner and propagate (the GroupBy is handled by lower_projection_inner).
                self.lower_op(inner)
            }

            Op::OrderBy { inner, keys } => {
                let inner_pat = self.lower_op(inner)?;
                let expressions = keys
                    .iter()
                    .map(|sk| self.lower_order_key(sk))
                    .collect::<Result<Vec<_>, _>>()?;
                let flushed = self.flush_pending(inner_pat);
                Ok(GraphPattern::OrderBy {
                    inner: Box::new(flushed),
                    expression: expressions,
                })
            }

            Op::Skip { inner, count } => {
                let start = expr_to_usize(count)?;
                let inner_pat = self.lower_op_as_query(inner)?;
                Ok(GraphPattern::Slice {
                    inner: Box::new(inner_pat),
                    start,
                    length: None,
                })
            }

            Op::Limit { inner, count } => {
                let length = expr_to_usize(count)?;
                let inner_pat = self.lower_op_as_query(inner)?;
                let start = query_slice_start(&inner_pat);
                Ok(GraphPattern::Slice {
                    inner: Box::new(inner_pat),
                    start,
                    length: Some(length),
                })
            }

            Op::Distinct { inner } => {
                let inner_pat = self.lower_op(inner)?;
                Ok(GraphPattern::Distinct {
                    inner: Box::new(inner_pat),
                })
            }

            Op::Unwind {
                inner,
                list,
                variable,
            } => {
                if let Expr::List(items) = list {
                    let inner_pat = self.lower_op(inner)?;
                    let var = Self::var(variable);
                    let bindings = items
                        .iter()
                        .map(|item| literal_to_ground(item).map(|g| vec![g]))
                        .collect::<Result<Vec<_>, _>>()?;
                    let values = GraphPattern::Values {
                        variables: vec![var],
                        bindings,
                    };
                    Ok(join(inner_pat, values))
                } else {
                    Err(PolygraphError::Unsupported {
                        construct: "UNWIND with variable/expression list in LQA path".into(),
                        spec_ref: "openCypher 9 §4.5".into(),
                        reason: "runtime list UNWIND requires legacy path".into(),
                    })
                }
            }

            Op::UnionAll { left, right } => {
                let lp = self.lower_op(left)?;
                let rp = self.lower_op(right)?;
                Ok(GraphPattern::Union {
                    left: Box::new(lp),
                    right: Box::new(rp),
                })
            }

            Op::Union { left, right } => {
                let lp = self.lower_op(left)?;
                let rp = self.lower_op(right)?;
                Ok(GraphPattern::Distinct {
                    inner: Box::new(GraphPattern::Union {
                        left: Box::new(lp),
                        right: Box::new(rp),
                    }),
                })
            }

            Op::CartesianProduct { left, right } => {
                let lp = self.lower_op(left)?;
                let rp = self.lower_op(right)?;
                // If the right pattern is a Filter (i.e. the right side came from a
                // MATCH…WHERE clause), lift the FILTER above the join so that variables
                // bound by BIND in the left side remain visible.  Without this, spargebra
                // wraps the right side in a nested `{ }` group that hides outer BIND
                // variables from the FILTER condition.
                match rp {
                    GraphPattern::Filter { expr, inner } => Ok(GraphPattern::Filter {
                        expr,
                        inner: Box::new(join(lp, *inner)),
                    }),
                    other => Ok(join(lp, other)),
                }
            }

            Op::LeftOuterJoin {
                left,
                right,
                condition,
            } => {
                let lp = self.lower_op(left)?;
                let rp = self.lower_op(right)?;
                let cond = condition.as_ref().map(|c| self.lower_expr(c)).transpose()?;
                let flushed_l = self.flush_pending(lp);
                let flushed_r = self.flush_pending(rp);
                Ok(GraphPattern::LeftJoin {
                    left: Box::new(flushed_l),
                    right: Box::new(flushed_r),
                    expression: cond,
                })
            }

            Op::Subquery { outer, inner } => {
                let outer_pat = self.lower_op(outer)?;
                let inner_pat = self.lower_op(inner)?;
                Ok(join(outer_pat, inner_pat))
            }

            Op::Create { .. }
            | Op::Merge { .. }
            | Op::Set { .. }
            | Op::Delete { .. }
            | Op::Remove { .. } => Err(PolygraphError::Unsupported {
                construct: "write clause".into(),
                spec_ref: "openCypher 9 §6".into(),
                reason: "write operators are not handled in the LQA SPARQL path".into(),
            }),

            Op::Foreach { .. } => Err(PolygraphError::Unsupported {
                construct: "FOREACH".into(),
                spec_ref: "openCypher 9 §4.8".into(),
                reason: "FOREACH not yet in LQA path".into(),
            }),

            Op::Call { .. } => Err(PolygraphError::Unsupported {
                construct: "CALL subquery".into(),
                spec_ref: "openCypher 9 §7".into(),
                reason: "CALL subquery not yet in LQA path".into(),
            }),
        }
    }

    // ── Relationship expansion helpers ────────────────────────────────────────

    /// Lower a named relationship-variable expand into SPARQL.
    ///
    /// Registers the edge in `self.edge_vars` so that downstream property-access
    /// and `type(r)` expressions can resolve it.  Returns the BGP/UNION pattern.
    fn lower_expand_rel_var(
        &mut self,
        from: &str,
        to: &str,
        rel_types: &[String],
        direction: &Direction,
        rv: &str,
    ) -> Result<GraphPattern, PolygraphError> {
        use spargebra::term::GroundTerm;
        let from_tp = TermPattern::Variable(Self::var(from));
        let to_tp = TermPattern::Variable(Self::var(to));

        // Canonical RDF triple subject/object (used for property-access reification).
        let (rdf_subj, rdf_obj) = match direction {
            Direction::Outgoing | Direction::Undirected => (from.to_owned(), to.to_owned()),
            Direction::Incoming => (to.to_owned(), from.to_owned()),
        };

        if rel_types.is_empty() {
            // Untyped: use a variable predicate with a negated-property-set filter
            // to exclude internal triples (rdf:type, __node).
            let pred_var = self.fresh(&format!("{rv}_type"));
            let bgp = self.lower_expand_any_type(from_tp, pred_var.clone(), to_tp, direction);
            // Bind the rel-var to the dynamic predicate variable so IS NULL checks
            // (`r IS NULL` → `!BOUND(?r)`) work correctly when in OPTIONAL MATCH.
            // `?pred_var` is bound by the triple pattern when a match is found.
            let bgp_with_marker = GraphPattern::Extend {
                inner: Box::new(bgp),
                variable: Self::var(rv),
                expression: SparExpr::Variable(pred_var.clone()),
            };
            self.edge_vars.insert(
                rv.to_owned(),
                EdgeVarInfo {
                    subj: rdf_subj,
                    obj: rdf_obj,
                    pred: EdgePred::Dynamic(pred_var),
                },
            );
            return Ok(bgp_with_marker);
        }

        if rel_types.len() == 1 {
            // Typed single-hop: static predicate.
            let iri = NamedNode::new_unchecked(format!("{}{}", self.base_iri, &rel_types[0]));
            let pred = NamedNodePattern::NamedNode(iri.clone());
            let bgp = self.lower_expand_typed(from_tp, pred, to_tp, direction);
            // Bind the rel-var to the relationship type IRI so that IS NULL checks
            // (`r IS NULL` → `!BOUND(?r)`) work correctly in OPTIONAL MATCH contexts.
            // When the OPTIONAL triple pattern matches, `?rv` is bound; when the
            // OPTIONAL has no match, `?rv` remains unbound (null).
            let xsd_any_uri =
                NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#anyURI");
            let marker_lit = SparLit::new_typed_literal(
                format!("{}{}", self.base_iri, &rel_types[0]),
                xsd_any_uri,
            );
            let bgp_with_marker = GraphPattern::Extend {
                inner: Box::new(bgp),
                variable: Self::var(rv),
                expression: SparExpr::Literal(marker_lit),
            };
            self.edge_vars.insert(
                rv.to_owned(),
                EdgeVarInfo {
                    subj: rdf_subj,
                    obj: rdf_obj,
                    pred: EdgePred::Static(iri),
                },
            );
            return Ok(bgp_with_marker);
        }

        // Multi-type: introduce a pred variable bound via VALUES so reification can
        // use it, then UNION branches per type each with that VALUES constraint.
        let pred_var = self.fresh(&format!("{rv}_type"));
        let bindings: Vec<Vec<Option<GroundTerm>>> = rel_types
            .iter()
            .map(|rt| {
                vec![Some(GroundTerm::NamedNode(NamedNode::new_unchecked(
                    format!("{}{}", self.base_iri, rt),
                )))]
            })
            .collect();
        let values_pat = GraphPattern::Values {
            variables: vec![pred_var.clone()],
            bindings,
        };
        let triple_tp = TermPattern::Variable(Self::var(from));
        let triple_obj = TermPattern::Variable(Self::var(to));
        let bgp = match direction {
            Direction::Outgoing => GraphPattern::Bgp {
                patterns: vec![TriplePattern {
                    subject: triple_tp,
                    predicate: NamedNodePattern::Variable(pred_var.clone()),
                    object: triple_obj,
                }],
            },
            Direction::Incoming => GraphPattern::Bgp {
                patterns: vec![TriplePattern {
                    subject: triple_obj,
                    predicate: NamedNodePattern::Variable(pred_var.clone()),
                    object: triple_tp,
                }],
            },
            Direction::Undirected => {
                let fwd = GraphPattern::Bgp {
                    patterns: vec![TriplePattern {
                        subject: triple_tp.clone(),
                        predicate: NamedNodePattern::Variable(pred_var.clone()),
                        object: triple_obj.clone(),
                    }],
                };
                let bwd = GraphPattern::Bgp {
                    patterns: vec![TriplePattern {
                        subject: triple_obj,
                        predicate: NamedNodePattern::Variable(pred_var.clone()),
                        object: triple_tp,
                    }],
                };
                GraphPattern::Union {
                    left: Box::new(fwd),
                    right: Box::new(bwd),
                }
            }
        };
        // Join VALUES before the triple pattern so pred_var is bound first.
        let edge_bgp = join(values_pat, bgp);
        self.edge_vars.insert(
            rv.to_owned(),
            EdgeVarInfo {
                subj: rdf_subj,
                obj: rdf_obj,
                pred: EdgePred::Dynamic(pred_var),
            },
        );
        Ok(edge_bgp)
    }

    /// Lower a variable-length expansion into a SPARQL property path pattern.
    fn lower_varlen(
        &mut self,
        from_tp: TermPattern,
        to_tp: TermPattern,
        rel_types: &[String],
        direction: &Direction,
        range: &crate::lqa::op::PathRange,
    ) -> Result<GraphPattern, PolygraphError> {
        use spargebra::algebra::PropertyPathExpression as PPE;

        // Build the base PPE from the rel types.
        let base_ppe: PPE = if rel_types.is_empty() {
            // Untyped: exclude internal predicates.
            PPE::NegatedPropertySet(vec![
                NamedNode::new_unchecked(RDF_TYPE),
                NamedNode::new_unchecked(format!("{}{}", self.base_iri, "__node")),
            ])
        } else if rel_types.len() == 1 {
            PPE::NamedNode(NamedNode::new_unchecked(format!(
                "{}{}",
                self.base_iri, &rel_types[0]
            )))
        } else {
            let ppes: Vec<PPE> = rel_types
                .iter()
                .map(|rt| {
                    PPE::NamedNode(NamedNode::new_unchecked(format!("{}{}", self.base_iri, rt)))
                })
                .collect();
            ppes.into_iter()
                .reduce(|a, b| PPE::Alternative(Box::new(a), Box::new(b)))
                .expect("non-empty")
        };

        // Build quantified PPE based on range.
        let lower = range.lower;
        let upper = range.upper;

        let quantified_ppe: PPE = match (lower, upper) {
            // Exact single hop — treat as simple triple (use Expand without range).
            // This shouldn't occur since is_lqa_safe no longer guards this, but
            // handle it anyway by using a 1-hop path.
            (1, Some(1)) => base_ppe,
            // *1.. or bare * (one or more hops)
            (1, None) => PPE::OneOrMore(Box::new(base_ppe)),
            // *0.. (zero or more hops)
            (0, None) => PPE::ZeroOrMore(Box::new(base_ppe)),
            // *0..1 (zero or one hop)
            (0, Some(1)) => PPE::ZeroOrOne(Box::new(base_ppe)),
            // *M.. (M or more hops, M > 1): Sequence of M fixed + OneOrMore
            (m, None) if m > 1 => {
                let mut ppe = PPE::OneOrMore(Box::new(base_ppe.clone()));
                for _ in 0..m.saturating_sub(1) {
                    ppe = PPE::Sequence(Box::new(base_ppe.clone()), Box::new(ppe));
                }
                ppe
            }
            // *M..N bounded: unroll as UNION of path lengths M..=N (max 10 hops)
            (m, Some(n)) if n > 1 => {
                let max_n = n.min(m + 10);
                // Build a chain PPE for a given number of hops.
                let chain = |count: u64| -> PPE {
                    let mut p = base_ppe.clone();
                    for _ in 1..count {
                        p = PPE::Sequence(Box::new(base_ppe.clone()), Box::new(p));
                    }
                    p
                };
                let ranges: Vec<PPE> = (m.max(1)..=max_n).map(chain).collect();
                if ranges.is_empty() {
                    base_ppe
                } else {
                    ranges
                        .into_iter()
                        .reduce(|a, b| PPE::Alternative(Box::new(a), Box::new(b)))
                        .expect("non-empty")
                }
            }
            _ => base_ppe,
        };

        // Apply direction.
        let path_ppe = match direction {
            Direction::Outgoing => quantified_ppe,
            Direction::Incoming => PPE::Reverse(Box::new(quantified_ppe)),
            Direction::Undirected => PPE::Alternative(
                Box::new(quantified_ppe.clone()),
                Box::new(PPE::Reverse(Box::new(quantified_ppe))),
            ),
        };

        Ok(GraphPattern::Path {
            subject: from_tp,
            path: path_ppe,
            object: to_tp,
        })
    }

    fn lower_expand_typed(
        &self,
        from: TermPattern,
        pred: NamedNodePattern,
        to: TermPattern,
        direction: &Direction,
    ) -> GraphPattern {
        match direction {
            Direction::Outgoing => GraphPattern::Bgp {
                patterns: vec![TriplePattern {
                    subject: from,
                    predicate: pred,
                    object: to,
                }],
            },
            Direction::Incoming => GraphPattern::Bgp {
                patterns: vec![TriplePattern {
                    subject: to,
                    predicate: pred,
                    object: from,
                }],
            },
            Direction::Undirected => {
                let fwd = GraphPattern::Bgp {
                    patterns: vec![TriplePattern {
                        subject: from.clone(),
                        predicate: pred.clone(),
                        object: to.clone(),
                    }],
                };
                let bwd = GraphPattern::Bgp {
                    patterns: vec![TriplePattern {
                        subject: to,
                        predicate: pred,
                        object: from,
                    }],
                };
                GraphPattern::Union {
                    left: Box::new(fwd),
                    right: Box::new(bwd),
                }
            }
        }
    }

    fn lower_expand_any_type(
        &self,
        from: TermPattern,
        pred_var: Variable,
        to: TermPattern,
        direction: &Direction,
    ) -> GraphPattern {
        let edge_pat = match direction {
            Direction::Outgoing => GraphPattern::Bgp {
                patterns: vec![TriplePattern {
                    subject: from,
                    predicate: NamedNodePattern::Variable(pred_var),
                    object: to.clone(),
                }],
            },
            Direction::Incoming => GraphPattern::Bgp {
                patterns: vec![TriplePattern {
                    subject: to.clone(),
                    predicate: NamedNodePattern::Variable(pred_var),
                    object: from,
                }],
            },
            Direction::Undirected => {
                let fwd = GraphPattern::Bgp {
                    patterns: vec![TriplePattern {
                        subject: from.clone(),
                        predicate: NamedNodePattern::Variable(pred_var.clone()),
                        object: to.clone(),
                    }],
                };
                // Backward branch: subject=to, object=from.
                // Add FILTER(?to != ?from) to prevent self-loop duplication:
                // when `from == to` (same SPARQL variable), both branches would match
                // identically. The FILTER suppresses the backward branch for self-loops
                // so they are counted exactly once (from the forward branch only).
                let bwd_bgp = GraphPattern::Bgp {
                    patterns: vec![TriplePattern {
                        subject: to.clone(),
                        predicate: NamedNodePattern::Variable(pred_var),
                        object: from.clone(),
                    }],
                };
                let bwd = if from == to {
                    // Self-loop case: suppress backward branch entirely.
                    GraphPattern::Filter {
                        expr: SparExpr::Literal(SparLit::new_typed_literal(
                            "false",
                            NamedNode::new_unchecked(XSD_BOOLEAN),
                        )),
                        inner: Box::new(bwd_bgp),
                    }
                } else if let (TermPattern::Variable(to_v), TermPattern::Variable(from_v)) =
                    (&to, &from)
                {
                    // Distinct variables: add FILTER(?to_v != ?from_v) to avoid
                    // duplicate matches when the two endpoints happen to bind to the
                    // same node in a concrete graph.
                    let filter_expr = SparExpr::Not(Box::new(SparExpr::Equal(
                        Box::new(SparExpr::Variable(to_v.clone())),
                        Box::new(SparExpr::Variable(from_v.clone())),
                    )));
                    GraphPattern::Filter {
                        expr: filter_expr,
                        inner: Box::new(bwd_bgp),
                    }
                } else {
                    bwd_bgp
                };
                GraphPattern::Union {
                    left: Box::new(fwd),
                    right: Box::new(bwd),
                }
            }
        };

        // Add endpoint sentinel: ensure `to` is an actual PG node.
        // Without this, untyped expand matches rdf:type and property triples as well
        // (since those share the same predicate namespace in our RDF encoding).
        if let TermPattern::Variable(to_var) = &to {
            let sentinel_iri = NamedNode::new_unchecked(format!("{}{}", self.base_iri, "__node"));
            let sentinel_bgp = GraphPattern::Bgp {
                patterns: vec![TriplePattern {
                    subject: TermPattern::Variable(to_var.clone()),
                    predicate: NamedNodePattern::NamedNode(sentinel_iri.clone()),
                    object: TermPattern::NamedNode(sentinel_iri),
                }],
            };
            join(edge_pat, sentinel_bgp)
        } else {
            edge_pat
        }
    }

    // ── Expression lowering ───────────────────────────────────────────────────

    fn lower_expr(&mut self, expr: &Expr) -> Result<SparExpr, PolygraphError> {
        match expr {
            Expr::Variable { name, .. } => Ok(SparExpr::Variable(Self::var(name))),

            Expr::Literal(lit) => match lit {
                Literal::Integer(n) => Ok(Self::lit_integer(*n)),
                Literal::Float(f) => Ok(Self::lit_double(*f)),
                Literal::String(s) => Ok(Self::lit_str(s)),
                Literal::Boolean(b) => Ok(Self::lit_bool(*b)),
                Literal::Null => {
                    let null_var = self.fresh("null");
                    Ok(SparExpr::Variable(null_var))
                }
            },

            Expr::Property(base, key) => {
                // If the base is an edge (relationship) variable, use RDF-star reification
                // to access the edge property.  Two triples must stay together in one
                // OPTIONAL block: the rdf:reifies triple and the property triple.
                if let Expr::Variable { name, .. } = base.as_ref() {
                    if let Some(edge_info) = self.edge_vars.get(name.as_str()).cloned() {
                        let subj_var = Self::var(&edge_info.subj);
                        let obj_var = Self::var(&edge_info.obj);
                        let pred_pat = match &edge_info.pred {
                            EdgePred::Static(iri) => NamedNodePattern::NamedNode(iri.clone()),
                            EdgePred::Dynamic(v) => NamedNodePattern::Variable(v.clone()),
                        };
                        let prop_var = self.fresh(key);
                        let reif_var = self.fresh(&format!("reif_{}", key));
                        let rdf_reifies = NamedNode::new_unchecked(
                            "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies",
                        );
                        let edge_triple_term = TermPattern::Triple(Box::new(TriplePattern {
                            subject: TermPattern::Variable(subj_var),
                            predicate: pred_pat,
                            object: TermPattern::Variable(obj_var),
                        }));
                        // Both reification triples must be in the same OPTIONAL block.
                        self.pending_optional_groups.push(vec![
                            TriplePattern {
                                subject: TermPattern::Variable(reif_var.clone()),
                                predicate: NamedNodePattern::NamedNode(rdf_reifies),
                                object: edge_triple_term,
                            },
                            TriplePattern {
                                subject: TermPattern::Variable(reif_var),
                                predicate: NamedNodePattern::NamedNode(self.prop_iri(key)),
                                object: TermPattern::Variable(prop_var.clone()),
                            },
                        ]);
                        return Ok(SparExpr::Variable(prop_var));
                    }
                }

                // If the base is a scalar variable (bound via BIND/Extend, not Scan),
                // it holds an RDF literal and cannot be the subject of a triple.
                // Fall back to the legacy translator for these cases.
                if let Expr::Variable { name, .. } = base.as_ref() {
                    if self.scalar_vars.contains(name) {
                        return Err(PolygraphError::Unsupported {
                            construct: "property access on scalar variable".into(),
                            spec_ref: "openCypher 9 §6.1".into(),
                            reason: format!(
                                "Variable `{name}` is bound to a scalar value (not a node); \
                                 triple-based property access is not applicable"
                            ),
                        });
                    }
                }
                let base_expr = self.lower_expr(base)?;
                let base_var = match &base_expr {
                    SparExpr::Variable(v) => v.clone(),
                    _ => {
                        return Err(PolygraphError::Unsupported {
                            construct: "property access on non-variable expression".into(),
                            spec_ref: "openCypher 9 §6.1".into(),
                            reason: "LQA path only supports property access on variables".into(),
                        })
                    }
                };
                let prop_var = self.fresh(key);
                // In openCypher, accessing an absent property returns null rather
                // than excluding the row.  Use OPTIONAL so a missing property
                // leaves the variable unbound (≡ null) rather than dropping the
                // solution — matching openCypher null-propagation semantics.
                self.pending_optional_triples.push(TriplePattern {
                    subject: TermPattern::Variable(base_var),
                    predicate: NamedNodePattern::NamedNode(self.prop_iri(key)),
                    object: TermPattern::Variable(prop_var.clone()),
                });
                Ok(SparExpr::Variable(prop_var))
            }

            Expr::Add(a, b) => {
                // In openCypher, `+` is overloaded: arithmetic for numbers,
                // string concatenation when either operand is a string.
                // SPARQL `+` is arithmetic-only; strings must use CONCAT().
                let la = self.lower_expr(a)?;
                let lb = self.lower_expr(b)?;
                if lqa_expr_is_string(a) || lqa_expr_is_string(b) {
                    Ok(SparExpr::FunctionCall(Function::Concat, vec![la, lb]))
                } else {
                    Ok(SparExpr::Add(Box::new(la), Box::new(lb)))
                }
            }
            Expr::Sub(a, b) => Ok(SparExpr::Subtract(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            Expr::Mul(a, b) => Ok(SparExpr::Multiply(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            Expr::Div(a, b) => Ok(SparExpr::Divide(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            Expr::Mod(_, _) => Err(PolygraphError::Unsupported {
                construct: "modulo operator".into(),
                spec_ref: "openCypher 9 §6.3.1".into(),
                reason: "SPARQL has no modulo; legacy path handles this".into(),
            }),
            Expr::Pow(base, exp) => {
                if let (Expr::Literal(Literal::Integer(b)), Expr::Literal(Literal::Integer(e))) =
                    (base.as_ref(), exp.as_ref())
                {
                    let result = (*b as f64).powi(*e as i32);
                    if result.is_finite() {
                        return Ok(Self::lit_double(result));
                    }
                }
                Err(PolygraphError::Unsupported {
                    construct: "^ exponentiation with runtime operands".into(),
                    spec_ref: "openCypher 9 §6.3.1".into(),
                    reason: "SPARQL has no POW; legacy path handles this".into(),
                })
            }
            Expr::Unary(UnaryOp::Neg, e) => Ok(SparExpr::UnaryMinus(Box::new(self.lower_expr(e)?))),
            Expr::Unary(UnaryOp::Not, e) => Ok(SparExpr::Not(Box::new(self.lower_expr(e)?))),
            Expr::Unary(UnaryOp::Pos, e) => self.lower_expr(e),

            Expr::Comparison(op, a, b) => {
                let la = self.lower_expr(a)?;
                let lb = self.lower_expr(b)?;
                Ok(match op {
                    CmpOp::Eq => SparExpr::Equal(Box::new(la), Box::new(lb)),
                    CmpOp::Ne => {
                        SparExpr::Not(Box::new(SparExpr::Equal(Box::new(la), Box::new(lb))))
                    }
                    CmpOp::Lt => SparExpr::Less(Box::new(la), Box::new(lb)),
                    CmpOp::Le => SparExpr::LessOrEqual(Box::new(la), Box::new(lb)),
                    CmpOp::Gt => SparExpr::Greater(Box::new(la), Box::new(lb)),
                    CmpOp::Ge => SparExpr::GreaterOrEqual(Box::new(la), Box::new(lb)),
                    CmpOp::In => SparExpr::In(Box::new(la), vec![lb]),
                    CmpOp::StartsWith | CmpOp::EndsWith | CmpOp::Contains | CmpOp::RegexMatch => {
                        return Err(PolygraphError::Unsupported {
                            construct: format!("string comparison op {op:?}"),
                            spec_ref: "openCypher 9 §6.2".into(),
                            reason: "use FunctionCall form".into(),
                        })
                    }
                })
            }

            Expr::IsNull(e) => {
                // For property access: `n.prop IS NULL` → NOT EXISTS { ?n <prop> ?_val }
                // This avoids adding a required BGP triple that would filter out
                // rows where the property is absent.
                if let Expr::Property(base, key) = e.as_ref() {
                    let base_expr = self.lower_expr(base)?;
                    if let SparExpr::Variable(base_var) = base_expr {
                        let val_var = self.fresh(key);
                        let exists_pat = GraphPattern::Bgp {
                            patterns: vec![TriplePattern {
                                subject: TermPattern::Variable(base_var),
                                predicate: NamedNodePattern::NamedNode(self.prop_iri(key)),
                                object: TermPattern::Variable(val_var),
                            }],
                        };
                        return Ok(SparExpr::Not(Box::new(SparExpr::Exists(Box::new(
                            exists_pat,
                        )))));
                    }
                }
                let inner = self.lower_expr(e)?;
                if let SparExpr::Variable(v) = &inner {
                    Ok(SparExpr::Not(Box::new(SparExpr::Bound(v.clone()))))
                } else {
                    Ok(SparExpr::Not(Box::new(SparExpr::Bound(
                        self.fresh("isnull_probe"),
                    ))))
                }
            }
            Expr::IsNotNull(e) => {
                // For property access: `n.prop IS NOT NULL` → EXISTS { ?n <prop> ?_val }
                if let Expr::Property(base, key) = e.as_ref() {
                    let base_expr = self.lower_expr(base)?;
                    if let SparExpr::Variable(base_var) = base_expr {
                        let val_var = self.fresh(key);
                        let exists_pat = GraphPattern::Bgp {
                            patterns: vec![TriplePattern {
                                subject: TermPattern::Variable(base_var),
                                predicate: NamedNodePattern::NamedNode(self.prop_iri(key)),
                                object: TermPattern::Variable(val_var),
                            }],
                        };
                        return Ok(SparExpr::Exists(Box::new(exists_pat)));
                    }
                }
                let inner = self.lower_expr(e)?;
                if let SparExpr::Variable(v) = &inner {
                    Ok(SparExpr::Bound(v.clone()))
                } else {
                    Ok(SparExpr::Bound(self.fresh("isnotnull_probe")))
                }
            }

            Expr::And(a, b) => Ok(SparExpr::And(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            Expr::Or(a, b) => Ok(SparExpr::Or(
                Box::new(self.lower_expr(a)?),
                Box::new(self.lower_expr(b)?),
            )),
            // XOR(a,b) = (a OR b) AND NOT (a AND b)
            Expr::Xor(a, b) => {
                let la = self.lower_expr(a)?;
                let lb = self.lower_expr(b)?;
                Ok(SparExpr::And(
                    Box::new(SparExpr::Or(Box::new(la.clone()), Box::new(lb.clone()))),
                    Box::new(SparExpr::Not(Box::new(SparExpr::And(
                        Box::new(la),
                        Box::new(lb),
                    )))),
                ))
            }
            Expr::Not(e) => Ok(SparExpr::Not(Box::new(self.lower_expr(e)?))),

            Expr::LabelCheck { expr, labels } => {
                let base_inner = self.lower_expr(expr)?;
                let base_var = match base_inner {
                    SparExpr::Variable(v) => v,
                    _ => {
                        return Err(PolygraphError::Unsupported {
                            construct: "label check on non-variable".into(),
                            spec_ref: "openCypher 9 §6.3".into(),
                            reason: "LQA path only supports label check on variables".into(),
                        })
                    }
                };

                let mut result: Option<SparExpr> = None;
                for label in labels {
                    let label_tp = GraphPattern::Bgp {
                        patterns: vec![TriplePattern {
                            subject: TermPattern::Variable(base_var.clone()),
                            predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(
                                RDF_TYPE,
                            )),
                            object: TermPattern::NamedNode(self.label_iri(label)),
                        }],
                    };
                    let check = SparExpr::Exists(Box::new(label_tp));
                    result = Some(match result {
                        None => check,
                        Some(acc) => SparExpr::And(Box::new(acc), Box::new(check)),
                    });
                }
                Ok(result.unwrap_or(Self::lit_bool(true)))
            }

            Expr::FunctionCall { name, args, .. } => self.lower_function_call(name, args),

            Expr::CaseSearched {
                branches,
                else_expr,
            } => {
                let else_sparql = match else_expr {
                    Some(e) => self.lower_expr(e)?,
                    None => {
                        let null_v = self.fresh("case_null");
                        SparExpr::Variable(null_v)
                    }
                };

                branches
                    .iter()
                    .rev()
                    .try_fold(else_sparql, |acc, (cond, then_)| {
                        let c = self.lower_expr(cond)?;
                        let t = self.lower_expr(then_)?;
                        Ok::<_, PolygraphError>(SparExpr::If(
                            Box::new(c),
                            Box::new(t),
                            Box::new(acc),
                        ))
                    })
            }

            Expr::List(_)
            | Expr::Map(_)
            | Expr::Subscript(_, _)
            | Expr::ListSlice { .. }
            | Expr::Quantifier { .. }
            | Expr::ListComprehension { .. }
            | Expr::PatternComprehension { .. }
            | Expr::Reduce { .. }
            | Expr::Exists(_)
            | Expr::Aggregate { .. } => Err(PolygraphError::Unsupported {
                construct: format!(
                    "expression type {} in LQA SPARQL lowering",
                    expr_type_name(expr)
                ),
                spec_ref: "openCypher 9 §6".into(),
                reason:
                    "complex expression not yet fully handled in LQA path; legacy fallback applies"
                        .into(),
            }),

            Expr::Parameter(name) => Err(PolygraphError::Unsupported {
                construct: format!("parameter ${name}"),
                spec_ref: "openCypher 9 §4.1".into(),
                reason: "parameterized queries not yet supported in LQA path".into(),
            }),
        }
    }

    fn lower_function_call(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<SparExpr, PolygraphError> {
        use spargebra::algebra::Function;

        let name_lower = name.to_lowercase();
        match name_lower.as_str() {
            "abs" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::Abs, vec![a]))
            }
            "ceil" | "ceiling" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::Ceil, vec![a]))
            }
            "floor" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::Floor, vec![a]))
            }
            "round" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::Round, vec![a]))
            }
            "sign" => {
                let arg = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                let zero = Self::lit_integer(0);
                let one = Self::lit_integer(1);
                let m1 = Self::lit_integer(-1);
                Ok(SparExpr::If(
                    Box::new(SparExpr::Greater(
                        Box::new(arg.clone()),
                        Box::new(zero.clone()),
                    )),
                    Box::new(one),
                    Box::new(SparExpr::If(
                        Box::new(SparExpr::Less(Box::new(arg), Box::new(zero.clone()))),
                        Box::new(m1),
                        Box::new(zero),
                    )),
                ))
            }
            "tostring" | "string" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::Str, vec![a]))
            }
            "tointeger" | "int" | "integer" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(
                    Function::Custom(NamedNode::new_unchecked(XSD_INTEGER)),
                    vec![a],
                ))
            }
            "todouble" | "tofloat" | "float" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(
                    Function::Custom(NamedNode::new_unchecked(XSD_DOUBLE)),
                    vec![a],
                ))
            }
            "toupper" | "touppercase" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::UCase, vec![a]))
            }
            "tolower" | "tolowercase" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::LCase, vec![a]))
            }
            "ltrim" | "rtrim" | "trim" => Err(PolygraphError::Unsupported {
                construct: format!("{name}()"),
                spec_ref: "openCypher 9 §6.3.2".into(),
                reason: "no direct SPARQL built-in; legacy path applies".into(),
            }),
            "strlen" | "length" | "size" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::StrLen, vec![a]))
            }
            "substring" | "substr" => {
                let a0 = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                let raw_start = self.lower_expr(args.get(1).ok_or_else(|| arg_err(name))?)?;
                // Cypher substring() uses 0-based start index; SPARQL SUBSTR()
                // uses 1-based → add 1 to the start argument.
                let a1 = SparExpr::Add(Box::new(raw_start), Box::new(Self::lit_integer(1)));
                let mut sargs = vec![a0, a1];
                if let Some(a2) = args.get(2) {
                    sargs.push(self.lower_expr(a2)?);
                }
                Ok(SparExpr::FunctionCall(Function::SubStr, sargs))
            }
            "startswith" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                let b = self.lower_expr(args.get(1).ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::StrStarts, vec![a, b]))
            }
            "endswith" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                let b = self.lower_expr(args.get(1).ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::StrEnds, vec![a, b]))
            }
            "contains" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                let b = self.lower_expr(args.get(1).ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::Contains, vec![a, b]))
            }
            "regex" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                let b = self.lower_expr(args.get(1).ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::Regex, vec![a, b]))
            }
            "type" => {
                if let Some(Expr::Variable { name: rv, .. }) = args.first() {
                    let rv = rv.as_str();
                    // Fast path: single static type known at compile time.
                    if let Some(types) = self.edge_types.get(rv).cloned() {
                        if types.len() == 1 {
                            return Ok(Self::lit_str(&types[0]));
                        }
                    }
                    // Dynamic path: extract local name from the predicate variable.
                    if let Some(edge_info) = self.edge_vars.get(rv).cloned() {
                        if let EdgePred::Dynamic(pred_var) = &edge_info.pred {
                            // STRAFTER(STR(?pred_var), base_iri) extracts the local name.
                            let base_lit = SparExpr::Literal(SparLit::new_simple_literal(
                                self.base_iri.clone(),
                            ));
                            return Ok(SparExpr::FunctionCall(
                                Function::StrAfter,
                                vec![
                                    SparExpr::FunctionCall(
                                        Function::Str,
                                        vec![SparExpr::Variable(pred_var.clone())],
                                    ),
                                    base_lit,
                                ],
                            ));
                        }
                    }
                }
                Err(PolygraphError::Unsupported {
                    construct: "type(r) with unknown/multiple edge types".into(),
                    spec_ref: "openCypher 9 §6.3.2".into(),
                    reason: "multi-type or unbound relationship type requires legacy path".into(),
                })
            }
            "startnode" | "endnode" => {
                if let Some(Expr::Variable { name: rv, .. }) = args.first() {
                    if let Some(edge_info) = self.edge_vars.get(rv.as_str()).cloned() {
                        let node_var = if name_lower == "startnode" {
                            &edge_info.subj
                        } else {
                            &edge_info.obj
                        };
                        return Ok(SparExpr::Variable(Self::var(node_var)));
                    }
                }
                Err(PolygraphError::Unsupported {
                    construct: format!("{name}()"),
                    spec_ref: "openCypher 9 §6.3.2".into(),
                    reason: "startNode/endNode requires a known relationship variable".into(),
                })
            }
            "id" | "elementid" => {
                let a = self.lower_expr(args.first().ok_or_else(|| arg_err(name))?)?;
                Ok(SparExpr::FunctionCall(Function::Str, vec![a]))
            }
            "coalesce" => {
                // Property-access triples generated inside coalesce() arguments
                // must be OPTIONAL in SPARQL — the whole point of coalesce is
                // to handle absent/null properties gracefully.
                let largs = args
                    .iter()
                    .map(|a| {
                        let before = self.pending_triples.len();
                        let expr = self.lower_expr(a)?;
                        // Promote any new required triples to optional triples.
                        let new_triples: Vec<_> = self.pending_triples.drain(before..).collect();
                        self.pending_optional_triples.extend(new_triples);
                        Ok(expr)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(SparExpr::Coalesce(largs))
            }
            _ => Err(PolygraphError::Unsupported {
                construct: format!("{name}()"),
                spec_ref: "openCypher 9 §6.3".into(),
                reason: format!("function '{name}' not yet in LQA path; legacy fallback applies"),
            }),
        }
    }

    fn lower_order_key(&mut self, sk: &SortKey) -> Result<OrderExpression, PolygraphError> {
        let expr = self.lower_expr(&sk.expr)?;
        Ok(match sk.dir {
            SortDir::Asc => OrderExpression::Asc(expr),
            SortDir::Desc => OrderExpression::Desc(expr),
        })
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

fn join(left: GraphPattern, right: GraphPattern) -> GraphPattern {
    match (&left, &right) {
        (GraphPattern::Bgp { patterns: lp }, _) if lp.is_empty() => right,
        (_, GraphPattern::Bgp { patterns: rp }) if rp.is_empty() => left,
        _ => GraphPattern::Join {
            left: Box::new(left),
            right: Box::new(right),
        },
    }
}

fn expr_to_usize(expr: &Expr) -> Result<usize, PolygraphError> {
    match expr {
        Expr::Literal(Literal::Integer(n)) if *n >= 0 => Ok(*n as usize),
        _ => Err(PolygraphError::Translation {
            message: format!("SKIP/LIMIT requires a non-negative integer literal, got {expr:?}"),
        }),
    }
}

fn query_slice_start(pat: &GraphPattern) -> usize {
    if let GraphPattern::Slice { start, .. } = pat {
        *start
    } else {
        0
    }
}

fn literal_to_ground(expr: &Expr) -> Result<Option<spargebra::term::GroundTerm>, PolygraphError> {
    match expr {
        Expr::Literal(Literal::Integer(n)) => Ok(Some(spargebra::term::GroundTerm::Literal(
            SparLit::new_typed_literal(n.to_string(), NamedNode::new_unchecked(XSD_INTEGER)),
        ))),
        Expr::Literal(Literal::Float(f)) => Ok(Some(spargebra::term::GroundTerm::Literal(
            SparLit::new_typed_literal(format!("{f:?}"), NamedNode::new_unchecked(XSD_DOUBLE)),
        ))),
        Expr::Literal(Literal::String(s)) => Ok(Some(spargebra::term::GroundTerm::Literal(
            SparLit::new_simple_literal(s.as_str()),
        ))),
        Expr::Literal(Literal::Boolean(b)) => Ok(Some(spargebra::term::GroundTerm::Literal(
            SparLit::new_typed_literal(b.to_string(), NamedNode::new_unchecked(XSD_BOOLEAN)),
        ))),
        Expr::Literal(Literal::Null) => Ok(None),
        _ => Err(PolygraphError::Translation {
            message: format!("expected literal in VALUES/UNWIND context, got {expr:?}"),
        }),
    }
}

fn expr_type_name(expr: &Expr) -> &'static str {
    match expr {
        Expr::Variable { .. } => "Variable",
        Expr::Literal(_) => "Literal",
        Expr::Property(_, _) => "Property",
        Expr::Add(_, _) => "Add",
        Expr::Sub(_, _) => "Sub",
        Expr::Mul(_, _) => "Mul",
        Expr::Div(_, _) => "Div",
        Expr::Mod(_, _) => "Mod",
        Expr::Pow(_, _) => "Pow",
        Expr::Unary(_, _) => "Unary",
        Expr::Comparison(_, _, _) => "Comparison",
        Expr::IsNull(_) => "IsNull",
        Expr::IsNotNull(_) => "IsNotNull",
        Expr::And(_, _) => "And",
        Expr::Or(_, _) => "Or",
        Expr::Not(_) => "Not",
        Expr::LabelCheck { .. } => "LabelCheck",
        Expr::FunctionCall { .. } => "FunctionCall",
        Expr::Aggregate { .. } => "Aggregate",
        Expr::CaseSearched { .. } => "CaseSearched",
        Expr::List(_) => "List",
        Expr::Map(_) => "Map",
        Expr::Subscript(_, _) => "Subscript",
        Expr::ListSlice { .. } => "ListSlice",
        Expr::Quantifier { .. } => "Quantifier",
        Expr::ListComprehension { .. } => "ListComprehension",
        Expr::PatternComprehension { .. } => "PatternComprehension",
        Expr::Reduce { .. } => "Reduce",
        Expr::Exists(_) => "Exists",
        Expr::Parameter(_) => "Parameter",
        Expr::Xor(_, _) => "Xor",
    }
}

/// Returns `true` if `e` is guaranteed to produce a string value.
///
/// Used by the `+` operator handler to decide between SPARQL arithmetic `+`
/// and `CONCAT()`.  A conservative check: only literal strings and
/// string-producing function calls / Add-chains are detected; property
/// accesses are treated as unknown (numeric `+` will be attempted, and callers
/// relying on string concat must include at least one literal string argument).
fn lqa_expr_is_string(e: &Expr) -> bool {
    match e {
        Expr::Literal(lit) => matches!(lit, Literal::String(_)),
        Expr::Add(a, b) => lqa_expr_is_string(a) || lqa_expr_is_string(b),
        Expr::FunctionCall { name, .. } => matches!(
            name.as_str(),
            "toString"
                | "toLower"
                | "toUpper"
                | "trim"
                | "ltrim"
                | "rtrim"
                | "replace"
                | "substring"
                | "left"
                | "right"
                | "reverse"
                | "split"
                | "tostring"
        ),
        _ => false,
    }
}

fn arg_err(name: &str) -> PolygraphError {
    PolygraphError::UnsupportedFeature {
        feature: format!("{name}() requires an argument"),
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Compile an [`Op`] tree to a SPARQL SELECT query string + projection schema.
///
/// Returns [`PolygraphError::Unsupported`] for constructs not yet handled in
/// the LQA path.  The caller should fall back to the legacy translator.
pub fn compile(op: &Op, base_iri: Option<&str>) -> Result<CompiledQuery, PolygraphError> {
    let base = base_iri.unwrap_or(DEFAULT_BASE).to_string();
    let mut c = Compiler::new(base.clone());
    c.compile_inner(op, &base)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lqa::lower::AstLowerer;
    use crate::parser::parse_cypher;

    fn compile_query(src: &str) -> String {
        let ast = parse_cypher(src).expect("parse");
        let mut l = AstLowerer::new();
        let op = l.lower_query(&ast).expect("lower");
        let result = compile(&op, None).expect("compile");
        result.sparql
    }

    #[test]
    fn simple_match_return() {
        let sparql = compile_query("MATCH (n:Person) RETURN n");
        assert!(sparql.contains("SELECT"), "expected SELECT, got: {sparql}");
    }

    #[test]
    fn where_clause() {
        let sparql = compile_query("MATCH (n:Person) WHERE n.age > 30 RETURN n.name");
        let upper = sparql.to_uppercase();
        assert!(upper.contains("FILTER"), "expected FILTER, got: {sparql}");
    }

    #[test]
    fn relationship_match() {
        let sparql = compile_query("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name");
        assert!(sparql.contains("SELECT"));
    }

    #[test]
    fn order_limit() {
        let sparql = compile_query("MATCH (n:Person) RETURN n LIMIT 10");
        assert!(sparql.contains("10"), "expected limit 10, got: {sparql}");
    }
}
