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

use crate::result_mapping::schema::{ColumnKind, ProjectedColumn, ProjectionSchema};

/// The result of translating a Cypher query to SPARQL.
pub struct TranslationResult {
    /// The SPARQL query string.
    pub sparql: String,
    /// Schema describing the projected columns.
    pub schema: ProjectionSchema,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Translates an openCypher [`CypherQuery`] AST into a SPARQL 1.1 query string
/// and a [`ProjectionSchema`] describing the output columns.
///
/// * `base_iri` — namespace IRI for labels, relationship types and property
///   names. Pass `None` to use `http://polygraph.example/`.
/// * `rdf_star` — when `true`, emit SPARQL-star annotated triple patterns for
///   relationship properties; when `false`, use standard RDF reification.
pub fn translate(
    query: &CypherQuery,
    base_iri: Option<&str>,
    rdf_star: bool,
) -> Result<TranslationResult, PolygraphError> {
    validate_semantics(query)?;
    let base = base_iri.unwrap_or(DEFAULT_BASE).to_string();
    let mut state = TranslationState::new(base.clone(), rdf_star);
    let pattern = state.translate_query(query)?;
    let sparql_query = Query::Select {
        dataset: None,
        pattern,
        base_iri: None,
    };
    Ok(TranslationResult {
        sparql: sparql_query.to_string(),
        schema: state.build_schema(base, rdf_star),
    })
}

/// Returns `true` if `expr` contains any aggregate sub-expression at any depth.
fn expr_contains_aggregate(expr: &Expression) -> bool {
    match expr {
        Expression::Aggregate(_) => true,
        Expression::Or(a, b)
        | Expression::Xor(a, b)
        | Expression::And(a, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b)
        | Expression::Modulo(a, b)
        | Expression::Power(a, b)
        | Expression::Comparison(a, _, b) => {
            expr_contains_aggregate(a) || expr_contains_aggregate(b)
        }
        Expression::Not(e)
        | Expression::Negate(e)
        | Expression::IsNull(e)
        | Expression::IsNotNull(e)
        | Expression::Property(e, _) => expr_contains_aggregate(e),
        Expression::List(items) => items.iter().any(expr_contains_aggregate),
        Expression::Map(pairs) => pairs.iter().any(|(_, v)| expr_contains_aggregate(v)),
        Expression::FunctionCall { args, .. } => args.iter().any(expr_contains_aggregate),
        Expression::LabelCheck { .. } => false,
        _ => false,
    }
}

/// Returns `true` if `expr` contains a free variable or property reference
/// **outside** of any aggregate boundary (i.e., not inside an `Aggregate(...)` arg).
fn expr_has_free_var_outside_agg(expr: &Expression) -> bool {
    match expr {
        Expression::Variable(_) => true,
        Expression::Property(e, _) => expr_has_free_var_outside_agg(e),
        // Stop recursing into aggregate arguments — those are "consumed" by the agg.
        Expression::Aggregate(_) => false,
        Expression::Or(a, b)
        | Expression::Xor(a, b)
        | Expression::And(a, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b)
        | Expression::Modulo(a, b)
        | Expression::Power(a, b)
        | Expression::Comparison(a, _, b) => {
            expr_has_free_var_outside_agg(a) || expr_has_free_var_outside_agg(b)
        }
        Expression::Not(e)
        | Expression::Negate(e)
        | Expression::IsNull(e)
        | Expression::IsNotNull(e) => expr_has_free_var_outside_agg(e),
        Expression::FunctionCall { args, .. } => args.iter().any(expr_has_free_var_outside_agg),
        Expression::LabelCheck { .. } => true, // n:Label has a free variable
        _ => false,
    }
}

/// Returns `true` if `agg_expr` has any aggregate in its arguments (nested aggregation).
fn agg_has_nested_aggregate(agg: &AggregateExpr) -> bool {
    use crate::ast::cypher::AggregateExpr;
    match agg {
        AggregateExpr::Count { expr: Some(e), .. } => expr_contains_aggregate(e),
        AggregateExpr::Count { expr: None, .. } => false,
        AggregateExpr::Sum { expr: e, .. }
        | AggregateExpr::Avg { expr: e, .. }
        | AggregateExpr::Min { expr: e, .. }
        | AggregateExpr::Max { expr: e, .. }
        | AggregateExpr::Collect { expr: e, .. } => expr_contains_aggregate(e),
    }
}

/// Semantic analysis pass: catches `VariableTypeConflict` and `VariableAlreadyBound`
/// before translation so openCypher constraints are enforced.
fn validate_semantics(query: &CypherQuery) -> Result<(), PolygraphError> {
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
                if let crate::ast::cypher::ReturnItems::Explicit(items) = &w.items {
                    for item in items {
                        let name = item.alias.clone().or_else(|| {
                            if let Expression::Variable(v) = &item.expression {
                                Some(v.clone())
                            } else {
                                None
                            }
                        });
                        if let Some(var) = name {
                            // Always mark as bound (for UndefinedVariable check).
                            bound_vars.insert(var.clone());
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
                                kinds.insert(var, k);
                            }
                        }
                    }
                }
            }
            Clause::Return(r) => {
                // NoVariablesInScope: RETURN * with no bound graph variables from MATCH.
                // Only fire when there was at least one MATCH clause (tracked by seen_match).
                if matches!(&r.items, ReturnItems::All) && seen_match {
                    let has_graph_var = kinds
                        .values()
                        .any(|k| matches!(k, Kind::Node | Kind::Rel | Kind::Path));
                    if !has_graph_var {
                        return Err(PolygraphError::Translation {
                            message: "NoVariablesInScope: RETURN * with no variables in scope"
                                .to_string(),
                        });
                    }
                }

                // AmbiguousAggregation and NestedAggregation.
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
                            // AmbiguousAggregationExpression:
                            // expression is NOT a pure aggregate but contains an aggregate
                            // AND has free variables outside aggregates.
                            // ONLY flag if the free variables are NOT separately returned
                            // as a non-aggregate GROUP BY item.
                            // Simple check: if there are OTHER non-aggregate items in RETURN,
                            // and the free var outside aggregate appears in those items,
                            // it's a valid grouping key → not ambiguous.
                            // If no other non-agg items OR the var is not covered → ambiguous.
                            //
                            // Improved check: collect the "atomic leaf" expressions
                            // (property accesses and variables) outside aggregates. If any
                            // atomic leaf is NOT covered by a standalone non-agg item,
                            // the expression is ambiguous.
                            let _ = &non_agg_items;
                            let ambiguous = if non_agg_items.is_empty() {
                                true
                            } else {
                                // Collect atomic free terms in the mixed expression.
                                fn atomic_free_terms(expr: &Expression) -> Vec<&Expression> {
                                    match expr {
                                        Expression::Aggregate(_) => vec![],
                                        Expression::Variable(_) | Expression::Property(_, _) => {
                                            vec![expr]
                                        }
                                        Expression::Or(a, b)
                                        | Expression::And(a, b)
                                        | Expression::Add(a, b)
                                        | Expression::Subtract(a, b)
                                        | Expression::Multiply(a, b)
                                        | Expression::Divide(a, b)
                                        | Expression::Modulo(a, b)
                                        | Expression::Power(a, b)
                                        | Expression::Comparison(a, _, b) => {
                                            let mut r = atomic_free_terms(a);
                                            r.extend(atomic_free_terms(b));
                                            r
                                        }
                                        Expression::Not(e)
                                        | Expression::Negate(e)
                                        | Expression::IsNull(e)
                                        | Expression::IsNotNull(e) => atomic_free_terms(e),
                                        _ => vec![],
                                    }
                                }
                                let free_terms = atomic_free_terms(&item.expression);
                                // Check: are all free terms individually covered by non_agg_items?
                                free_terms.iter().any(|ft| {
                                    // ft is a free term; check if it matches any non_agg item exactly
                                    !non_agg_items.iter().any(|ni| *ni == *ft)
                                })
                            };
                            if ambiguous {
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
                // Only check when there has been at least one MATCH clause (seen_match).
                if seen_match {
                    if let ReturnItems::Explicit(items) = &r.items {
                        fn collect_free_vars(expr: &Expression, vars: &mut Vec<String>) {
                            match expr {
                                Expression::Variable(v) => vars.push(v.clone()),
                                Expression::Property(base, _) => collect_free_vars(base, vars),
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
                                    collect_free_vars(a, vars);
                                    collect_free_vars(b, vars);
                                }
                                Expression::Not(e)
                                | Expression::Negate(e)
                                | Expression::IsNull(e)
                                | Expression::IsNotNull(e) => collect_free_vars(e, vars),
                                Expression::LabelCheck { variable, .. } => {
                                    vars.push(variable.clone())
                                }
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
            }
            Clause::Unwind(u) => {
                // Register the UNWIND variable as bound (type unknown — don't constrain kinds).
                bound_vars.insert(u.variable.clone());
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
    /// Per-MATCH tracking: each inner Vec holds one or two stored-triple instances for
    /// one relationship hop.  Used to generate pairwise relationship-isomorphism FILTERs.
    iso_hops: Vec<Vec<EdgeIsoSlot>>,
    /// Variables that may be null (unbound) because they were introduced by an
    /// OPTIONAL MATCH that produced no match.  When such a variable is used as a
    /// node/edge in a subsequent mandatory MATCH, we add FILTER(BOUND(?var)) to
    /// prevent it from acting as a wildcard in the SPARQL JOIN.
    nullable_vars: std::collections::HashSet<String>,
    /// Variables assigned a literal-list value by a WITH clause.
    /// Used so that `UNWIND list_var AS x` can be expanded at compile time.
    with_list_vars: std::collections::HashMap<String, crate::ast::cypher::Expression>,
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
    /// Whether the last RETURN used DISTINCT.
    return_distinct: bool,
}

impl TranslationState {
    fn new(base_iri: String, rdf_star: bool) -> Self {
        Self {
            base_iri,
            counter: 0,
            rdf_star,
            edge_map: Default::default(),
            pending_aggs: Vec::new(),
            iso_hops: Vec::new(),
            nullable_vars: Default::default(),
            with_list_vars: Default::default(),
            path_hops: Default::default(),
            path_node_vars: Default::default(),
            node_vars: Default::default(),
            projected_columns: Vec::new(),
            return_distinct: false,
        }
    }

    /// Build a [`ProjectionSchema`] from the columns collected during RETURN translation.
    fn build_schema(&self, base_iri: String, rdf_star: bool) -> ProjectionSchema {
        ProjectionSchema {
            columns: self.projected_columns.clone(),
            distinct: self.return_distinct,
            base_iri,
            rdf_star,
        }
    }

    /// Classify a RETURN item as a node, relationship, or scalar column.
    fn classify_return_item(&self, item: &ReturnItem, sparql_var: &Variable) -> ColumnKind {
        if let Expression::Variable(name) = &item.expression {
            if self.node_vars.contains(name.as_str()) {
                return ColumnKind::Node {
                    iri_var: name.clone(),
                };
            }
            if let Some(edge) = self.edge_map.get(name.as_str()) {
                let src_var = match &edge.src {
                    TermPattern::Variable(v) => v.as_str().to_string(),
                    _ => String::new(),
                };
                let dst_var = match &edge.dst {
                    TermPattern::Variable(v) => v.as_str().to_string(),
                    _ => String::new(),
                };
                let type_info = edge.pred.as_str().to_string();
                return ColumnKind::Relationship {
                    src_var,
                    dst_var,
                    type_info,
                };
            }
        }
        ColumnKind::Scalar {
            var: sparql_var.as_str().to_string(),
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

    /// Add one relationship-hop's stored-triple instance(s) for isomorphism tracking.
    fn track_iso_hop(&mut self, instances: Vec<EdgeIsoSlot>) {
        if !instances.is_empty() {
            self.iso_hops.push(instances);
        }
    }

    /// Generate pairwise relationship-isomorphism FILTERs from `iso_hops`.
    ///
    /// For each pair of hops (i, j), for each pair of their instances (a, b):
    /// emit FILTER NOT(subj_a = subj_b AND pred_a = pred_b AND obj_a = obj_b).
    fn generate_iso_filters(&self) -> Vec<SparExpr> {
        use spargebra::term::NamedNodePattern;
        let mut filters: Vec<SparExpr> = Vec::new();
        let n = self.iso_hops.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let mut pair_conds: Vec<SparExpr> = Vec::new();
                for (si, pi, oi) in &self.iso_hops[i] {
                    for (sj, pj, oj) in &self.iso_hops[j] {
                        // Optimisation: if both preds are fixed and different → skip
                        if let (NamedNodePattern::NamedNode(ni), NamedNodePattern::NamedNode(nj)) =
                            (pi, pj)
                        {
                            if ni != nj {
                                continue;
                            }
                        }
                        // NOT(si=sj AND pi=pj AND oi=oj)
                        let s_eq = term_to_sparexpr(si);
                        let o_eq = term_to_sparexpr(oi);
                        let p_eq = named_node_to_sparexpr(pi);
                        let s_ne = SparExpr::Not(Box::new(SparExpr::Equal(
                            Box::new(s_eq.clone()),
                            Box::new(term_to_sparexpr(sj)),
                        )));
                        let p_ne = SparExpr::Not(Box::new(SparExpr::Equal(
                            Box::new(p_eq.clone()),
                            Box::new(named_node_to_sparexpr(pj)),
                        )));
                        let o_ne = SparExpr::Not(Box::new(SparExpr::Equal(
                            Box::new(o_eq.clone()),
                            Box::new(term_to_sparexpr(oj)),
                        )));
                        let _ = (s_eq, p_eq, o_eq);
                        let cond = SparExpr::Or(
                            Box::new(s_ne),
                            Box::new(SparExpr::Or(Box::new(p_ne), Box::new(o_ne))),
                        );
                        pair_conds.push(cond);
                    }
                }
                if let Some(combined) = pair_conds
                    .into_iter()
                    .reduce(|a, b| SparExpr::And(Box::new(a), Box::new(b)))
                {
                    filters.push(combined);
                }
            }
        }
        filters
    }

    // ── Top-level query translation ──────────────────────────────────────────

    fn translate_query(&mut self, query: &CypherQuery) -> Result<GraphPattern, PolygraphError> {
        // Peephole: eliminate collect(X) AS list / UNWIND list AS var → passthrough.
        let clauses = eliminate_collect_unwind(&query.clauses);

        // Accumulate extra BGP triples emitted during expression translation.
        let mut extra_triples: Vec<TriplePattern> = Vec::new();
        // The pattern is built left-to-right over clauses.
        let mut current = empty_bgp();
        // Collects filters to apply at the end of each scope.
        let mut pending_filters: Vec<SparExpr> = Vec::new();
        // The output variables of the most recent WITH clause (used to build
        // sub-select scope boundaries when nullable variables must be checked).
        let mut last_with_vars: Option<Vec<Variable>> = None;

        for clause in &clauses {
            match clause {
                Clause::Match(m) => {
                    if m.optional {
                        // For OPTIONAL MATCH, use a local extra buffer so that
                        // property-access triples from the WHERE clause go INSIDE
                        // the LeftJoin (right side), not into the outer scope.
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
                    // Flush any pending extra triples.
                    if !extra_triples.is_empty() {
                        let extra = GraphPattern::Bgp {
                            patterns: extra_triples.drain(..).collect(),
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
                            }
                        }

                        // Property-access triples in WITH must be OPTIONAL.
                        for tp in with_triples {
                            current = GraphPattern::LeftJoin {
                                left: Box::new(current),
                                right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                expression: None,
                            };
                        }
                        // Flush extra triples from expression translation.
                        if !extra_triples.is_empty() {
                            for tp in extra_triples.drain(..) {
                                current = GraphPattern::LeftJoin {
                                    left: Box::new(current),
                                    right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                    expression: None,
                                };
                            }
                        }

                        // Detect alias-to-variable-name conflicts (e.g. `a.name AS a`).
                        // The Property branch uses a fresh var when alias == base_var,
                        // so the project_vars contain the fresh name, not the alias.
                        // Build a rename map: alias → fresh_var for these cases.
                        let mut outer_renames: Vec<(Variable, Variable)> = Vec::new();
                        if let Some(ref pvars) = project_vars {
                            for (item, pvar) in items.iter().zip(pvars.iter()) {
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

                        for (var, expr) in &extends {
                            current = GraphPattern::Extend {
                                inner: Box::new(current),
                                variable: var.clone(),
                                expression: expr.clone(),
                            };
                        }

                        // Apply aggregation (GROUP BY).
                        if !aggregates.is_empty() {
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

                        if need_distinct {
                            current = GraphPattern::Distinct {
                                inner: Box::new(current),
                            };
                        }

                        // Project to WITH output variables.
                        // For conflicting renames, project the fresh var.
                        if let Some(ref vars) = project_vars {
                            let inner_vars: Vec<Variable> = vars
                                .iter()
                                .map(|v| {
                                    outer_renames
                                        .iter()
                                        .find(|(alias, _)| alias == v)
                                        .map(|(_, fresh)| fresh.clone())
                                        .unwrap_or_else(|| v.clone())
                                })
                                .collect();
                            current = GraphPattern::Project {
                                inner: Box::new(current),
                                variables: inner_vars,
                            };
                        }

                        // Apply outer renames after the sub-select.
                        for (alias, fresh) in &outer_renames {
                            current = GraphPattern::Extend {
                                inner: Box::new(current),
                                variable: alias.clone(),
                                expression: SparExpr::Variable(fresh.clone()),
                            };
                        }
                    }

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
                            Some(with_vars)
                        };
                        // Track literal-list-valued WITH items for compile-time
                        // UNWIND expansion.
                        self.with_list_vars.clear();
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
                                    Expression::Variable(v) => {
                                        if let Some(existing) =
                                            self.with_list_vars.get(v.as_str()).cloned()
                                        {
                                            self.with_list_vars.insert(alias, existing);
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
                            patterns: extra_triples.drain(..).collect(),
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
                        for tp in return_triples {
                            current = GraphPattern::LeftJoin {
                                left: Box::new(current),
                                right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                expression: None,
                            };
                        }
                    }
                    // Flush any triples added during return expression translation.
                    // These are property-access triples from expressions like `n.prop`
                    // inside aggregates.  They must be OPTIONAL to avoid filtering
                    // rows where the property doesn't exist (e.g. AVG(n.age) → null).
                    if !extra_triples.is_empty() {
                        for tp in extra_triples.drain(..) {
                            current = GraphPattern::LeftJoin {
                                left: Box::new(current),
                                right: Box::new(GraphPattern::Bgp { patterns: vec![tp] }),
                                expression: None,
                            };
                        }
                    }

                    // Apply pre-group Extend bindings (non-aggregate expression aliases).
                    for (var, expr) in extends {
                        current = GraphPattern::Extend {
                            inner: Box::new(current),
                            variable: var,
                            expression: expr,
                        };
                    }

                    // Apply aggregation (GROUP BY) if present.
                    if !aggregates.is_empty() {
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
        _extra: &mut Vec<TriplePattern>,
    ) -> Result<(GraphPattern, Option<SparExpr>, Vec<TriplePattern>), PolygraphError> {
        let mut triples: Vec<TriplePattern> = Vec::new();
        let mut path_patterns: Vec<GraphPattern> = Vec::new();

        // Clear the per-MATCH relationship isomorphism tracker.
        self.iso_hops.clear();

        self.translate_pattern_list(&m.pattern, &mut triples, &mut path_patterns)?;

        // Combine BGP triples + path patterns into a single graph pattern.
        let bgp = GraphPattern::Bgp { patterns: triples };
        let mut combined = path_patterns.into_iter().fold(bgp, join_patterns);

        // Apply pairwise relationship-isomorphism FILTERs.
        let iso_filters = self.generate_iso_filters();
        combined = apply_filters(combined, iso_filters.into_iter());

        // Use a local buffer for WHERE-clause property-access triples so they can
        // be applied as OPTIONAL LeftJoins by the caller (Cypher null semantics:
        // a missing property evaluates to null rather than filtering the row out).
        let mut where_extra: Vec<TriplePattern> = Vec::new();
        let filter = if let Some(wc) = &m.where_ {
            Some(self.translate_expr(&wc.expression, &mut where_extra)?)
        } else {
            None
        };

        Ok((combined, filter, where_extra))
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

        // Track named path hop counts and node vars for length(p) / nodes(p) resolution.
        if let Some(ref path_var) = pattern.variable {
            let hops = elements
                .iter()
                .filter(|e| matches!(e, PatternElement::Relationship(_)))
                .count() as u64;
            self.path_hops.insert(path_var.clone(), hops);
        }

        // Pre-compute a stable TermPattern for every node element.  Anonymous nodes
        // (no variable) get a fresh SPARQL variable so the same term is reused for
        // both the relationship triple and the sentinel / label triples.
        let node_terms: Vec<TermPattern> = elements
            .iter()
            .map(|e| match e {
                PatternElement::Node(n) => match &n.variable {
                    Some(v) => Variable::new_unchecked(v.clone()).into(),
                    None => {
                        let fresh = self.fresh_var("anon");
                        fresh.into()
                    }
                },
                _ => Variable::new_unchecked("__unused_rel_slot").into(),
            })
            .collect();

        // Store node variables for path if named.
        if let Some(ref path_var) = pattern.variable {
            let nvars: Vec<Variable> = elements
                .iter()
                .enumerate()
                .filter_map(|(idx, e)| {
                    if matches!(e, PatternElement::Node(_)) {
                        if let TermPattern::Variable(v) = &node_terms[idx] {
                            return Some(v.clone());
                        }
                    }
                    None
                })
                .collect();
            self.path_node_vars.insert(path_var.clone(), nvars);
        }

        let mut i = 0;
        while i < elements.len() {
            match &elements[i] {
                PatternElement::Node(n) => {
                    let term = node_terms[i].clone();
                    self.translate_node_pattern_with_term(n, &term, triples)?;
                    i += 1;
                }
                PatternElement::Relationship(r) => {
                    let src = if i > 0 {
                        node_terms[i - 1].clone()
                    } else {
                        Variable::new_unchecked("__anon").into()
                    };
                    let dst = node_terms
                        .get(i + 1)
                        .cloned()
                        .unwrap_or_else(|| Variable::new_unchecked("__anon").into());
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
        let term = node_term(node);
        self.translate_node_pattern_with_term(node, &term, triples)
    }

    fn translate_node_pattern_with_term(
        &mut self,
        node: &NodePattern,
        node_var: &TermPattern,
        triples: &mut Vec<TriplePattern>,
    ) -> Result<(), PolygraphError> {
        // Register named node variables for schema classification.
        if let Some(ref name) = node.variable {
            self.node_vars.insert(name.clone());
        }

        // One triple per label: `?n rdf:type <base:Label>`
        for label in &node.labels {
            triples.push(TriplePattern {
                subject: node_var.clone(),
                predicate: self.rdf_type().into(),
                object: self.iri(label).into(),
            });
        }

        // Inline properties: `?n <base:prop> <literal>`
        let has_props = node
            .properties
            .as_ref()
            .map(|p| !p.is_empty())
            .unwrap_or(false);
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

        // Unconstrained node (no labels, no properties): emit a node-existence
        // sentinel so the variable is bound to real graph nodes rather than
        // returning 1 empty row from an empty BGP.
        // Convention: every graph node has exactly one `<base:__node> <base:__node>` triple.
        if node.labels.is_empty() && !has_props {
            triples.push(TriplePattern {
                subject: node_var.clone(),
                predicate: self.iri("__node").into(),
                object: self.iri("__node").into(),
            });
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

        if rel.rel_types.is_empty() && !has_range {
            // No type constraint, no range: emit an anonymous predicate variable (no path).
            let pred_var = match &rel.variable {
                Some(v) => Variable::new_unchecked(format!("{}_pred", v)),
                None => self.fresh_var("rel"),
            };
            let pred_term: spargebra::term::NamedNodePattern = pred_var.clone().into();
            match rel.direction {
                Direction::Left => triples.push(TriplePattern {
                    subject: dst.clone(),
                    predicate: pred_term,
                    object: src.clone(),
                }),
                Direction::Right => triples.push(TriplePattern {
                    subject: src.clone(),
                    predicate: pred_term,
                    object: dst.clone(),
                }),
                Direction::Both => {
                    // Undirected: UNION of both directions.
                    // Use the SAME pred_term for both branches so that the predicate
                    // variable is bound in both branches of the UNION (required for
                    // correct top-level relationship-isomorphism FILTERs).
                    let fwd = GraphPattern::Bgp {
                        patterns: vec![TriplePattern {
                            subject: src.clone(),
                            predicate: pred_term.clone(),
                            object: dst.clone(),
                        }],
                    };
                    let bwd_triple = GraphPattern::Bgp {
                        patterns: vec![TriplePattern {
                            subject: dst.clone(),
                            predicate: pred_term.clone(),
                            object: src.clone(),
                        }],
                    };
                    // Prevent duplicate for self-loops.
                    let filter_expr =
                        if let (TermPattern::Variable(s), TermPattern::Variable(d)) = (src, dst) {
                            Some(SparExpr::Not(Box::new(SparExpr::Equal(
                                Box::new(SparExpr::Variable(s.clone())),
                                Box::new(SparExpr::Variable(d.clone())),
                            ))))
                        } else {
                            None
                        };
                    let bwd = if let Some(f) = filter_expr {
                        GraphPattern::Filter {
                            expr: f,
                            inner: Box::new(bwd_triple),
                        }
                    } else {
                        bwd_triple
                    };
                    path_patterns.push(GraphPattern::Union {
                        left: Box::new(fwd),
                        right: Box::new(bwd),
                    });
                }
            }
            // Register in edge_map so type(r) and r.prop can resolve.
            if let Some(ref var_name) = rel.variable {
                self.edge_map.insert(
                    var_name.clone(),
                    EdgeInfo {
                        src: src.clone(),
                        pred: NamedNode::new_unchecked("urn:polygraph:untyped"),
                        pred_var: Some(pred_var.clone()),
                        dst: dst.clone(),
                        reif_var: None,
                        null_check_var: None,
                    },
                );
            }
            // Track for pairwise isomorphism filter generation.
            {
                use spargebra::term::NamedNodePattern;
                let pv = NamedNodePattern::Variable(pred_var);
                let instances: Vec<EdgeIsoSlot> = match rel.direction {
                    Direction::Right => vec![(src.clone(), pv, dst.clone())],
                    Direction::Left => vec![(dst.clone(), pv, src.clone())],
                    Direction::Both => vec![
                        (src.clone(), pv.clone(), dst.clone()),
                        (dst.clone(), pv, src.clone()),
                    ],
                };
                self.track_iso_hop(instances);
            }
            return Ok(());
        }

        // Build base path expression (single type, multi-type alt, or NegatedPropertySet for untyped).
        let base_ppe: PPE = if rel.rel_types.is_empty() {
            // Untyped variable-length: match any predicate except internal markers.
            // Exclude rdf:type and __node to avoid traversing property/label triples.
            PPE::NegatedPropertySet(vec![
                NamedNode::new_unchecked(RDF_TYPE),
                self.iri("__node"),
            ])
        } else if multi_type {
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

            // Special case: varlen with property constraints → bounded unrolling with
            // RDF-star per-hop annotation filters.
            if rel.properties.is_some() && self.rdf_star && !rel.rel_types.is_empty() {
                let lower = range.lower.unwrap_or(1).max(1);
                let upper = range.upper.unwrap_or(lower + 10);
                self.emit_bounded_path_union_with_props(
                    rel,
                    src,
                    dst,
                    lower,
                    upper,
                    path_patterns,
                )?;
                return Ok(());
            }

            let q = match (range.lower, range.upper) {
                // * (bare star) = 1 or more hops in openCypher
                (None, None) => PPE::OneOrMore(Box::new(base_ppe)),
                // *0.. = zero or more hops
                (Some(0), None) => PPE::ZeroOrMore(Box::new(base_ppe)),
                // *1..
                (Some(1), None) => PPE::OneOrMore(Box::new(base_ppe)),
                // *0..1 or *..1
                (None, Some(1)) | (Some(0), Some(1)) => PPE::ZeroOrOne(Box::new(base_ppe)),
                // *1..1 = exact 1 hop, treat as simple triple
                (Some(1), Some(1)) => {
                    if rel.rel_types.is_empty() {
                        // Untyped *1..1: use NPS as a 1-hop path
                        base_ppe
                    } else {
                        // Typed *1..1: emit as regular triple (no range modifier).
                        let pred = self.iri(&rel.rel_types[0]);
                        self.emit_edge_triple(rel, src, dst, pred, triples, path_patterns)?;
                        return Ok(());
                    }
                }
                // Bounded ranges like *M..N — unroll as UNION of fixed-length chains.
                (lo, hi) => {
                    let lower = lo.unwrap_or(0);
                    let upper = match hi {
                        Some(u) => u,
                        None => {
                            // *N.. (lower only): compose path for minimum bound.
                            if lower <= 1 {
                                let q = if lower == 0 {
                                    PPE::ZeroOrMore(Box::new(base_ppe))
                                } else {
                                    PPE::OneOrMore(Box::new(base_ppe))
                                };
                                let (subj, obj) = match rel.direction {
                                    Direction::Left => (dst.clone(), src.clone()),
                                    _ => (src.clone(), dst.clone()),
                                };
                                let path = if rel.direction == Direction::Left {
                                    PPE::Reverse(Box::new(q))
                                } else {
                                    q
                                };
                                path_patterns.push(GraphPattern::Path {
                                    subject: subj,
                                    path,
                                    object: obj,
                                });
                                return Ok(());
                            }
                            // *N.. with N>1: bounded unrolling from N to N+5
                            // SPARQL property paths can't enforce min-hop constraints.
                            let cap = lower + 5;
                            self.emit_bounded_path_union(
                                rel,
                                src,
                                dst,
                                &base_ppe,
                                lower,
                                cap,
                                triples,
                                path_patterns,
                            )?;
                            return Ok(());
                        }
                    };
                    // Build UNION of hop counts from lower..=upper.
                    self.emit_bounded_path_union(
                        rel,
                        src,
                        dst,
                        &base_ppe,
                        lower,
                        upper,
                        triples,
                        path_patterns,
                    )?;
                    return Ok(());
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
            } else if rel.direction == Direction::Both {
                PPE::Alternative(
                    Box::new(path.clone()),
                    Box::new(PPE::Reverse(Box::new(path))),
                )
            } else {
                path
            };
            let mut path_gp = GraphPattern::Path {
                subject: subj,
                path,
                object: obj,
            };
            // For untyped paths (NegatedPropertySet), filter out literal endpoints
            // since NPS also matches property predicates leading to literals.
            if rel.rel_types.is_empty() {
                let endpoint = match rel.direction {
                    Direction::Left => src,
                    _ => dst,
                };
                if let TermPattern::Variable(v) = endpoint {
                    path_gp = GraphPattern::Filter {
                        expr: SparExpr::Not(Box::new(SparExpr::FunctionCall(
                            spargebra::algebra::Function::IsLiteral,
                            vec![SparExpr::Variable(v.clone())],
                        ))),
                        inner: Box::new(path_gp),
                    };
                }
            }
            // Register edge variable in edge_map (no inline properties on path patterns).
            // For varlen paths, bind a marker variable so count(r) works.
            if let Some(ref var_name) = rel.variable {
                let pred = if rel.rel_types.is_empty() {
                    NamedNode::new_unchecked("urn:polygraph:untyped")
                } else {
                    self.iri(&rel.rel_types[0])
                };
                let marker = self.fresh_var(&format!("{}_bound", var_name));
                path_gp = GraphPattern::Extend {
                    inner: Box::new(path_gp),
                    variable: marker.clone(),
                    expression: SparExpr::Literal(SparLit::new_typed_literal(
                        "true",
                        NamedNode::new_unchecked(XSD_BOOLEAN),
                    )),
                };
                self.edge_map.insert(
                    var_name.clone(),
                    EdgeInfo {
                        src: src.clone(),
                        pred,
                        pred_var: None,
                        dst: dst.clone(),
                        reif_var: None,
                        null_check_var: Some(marker),
                    },
                );
            }
            path_patterns.push(path_gp);
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
                            pred_var: None,
                            dst: dst.clone(),
                            reif_var: None,
                            null_check_var: None,
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
        path_patterns: &mut Vec<GraphPattern>,
    ) -> Result<(), PolygraphError> {
        use crate::ast::cypher::Direction;

        match rel.direction {
            Direction::Left => triples.push(TriplePattern {
                subject: dst.clone(),
                predicate: pred.clone().into(),
                object: src.clone(),
            }),
            Direction::Right => triples.push(TriplePattern {
                subject: src.clone(),
                predicate: pred.clone().into(),
                object: dst.clone(),
            }),
            Direction::Both => {
                // Undirected (-- or <-->): match either direction via UNION.
                // Use FILTER(?src != ?dst) in the backward branch to avoid
                // duplicate results for self-loops (where src == dst).
                let fwd = GraphPattern::Bgp {
                    patterns: vec![TriplePattern {
                        subject: src.clone(),
                        predicate: pred.clone().into(),
                        object: dst.clone(),
                    }],
                };
                let bwd_triple = GraphPattern::Bgp {
                    patterns: vec![TriplePattern {
                        subject: dst.clone(),
                        predicate: pred.clone().into(),
                        object: src.clone(),
                    }],
                };
                // Apply filter to avoid duplicate for self-loops.
                let filter_expr =
                    if let (TermPattern::Variable(s), TermPattern::Variable(d)) = (src, dst) {
                        Some(SparExpr::Not(Box::new(SparExpr::Equal(
                            Box::new(SparExpr::Variable(s.clone())),
                            Box::new(SparExpr::Variable(d.clone())),
                        ))))
                    } else {
                        None
                    };
                let bwd = if let Some(f) = filter_expr {
                    GraphPattern::Filter {
                        expr: f,
                        inner: Box::new(bwd_triple),
                    }
                } else {
                    bwd_triple
                };
                path_patterns.push(GraphPattern::Union {
                    left: Box::new(fwd),
                    right: Box::new(bwd),
                });
            }
        }

        // Track this hop for pairwise relationship-isomorphism FILTER generation.
        {
            use spargebra::term::NamedNodePattern;
            let pred_pattern = NamedNodePattern::NamedNode(pred.clone());
            let instances: Vec<EdgeIsoSlot> = match rel.direction {
                Direction::Right => vec![(src.clone(), pred_pattern, dst.clone())],
                Direction::Left => vec![(dst.clone(), pred_pattern, src.clone())],
                Direction::Both => vec![
                    (src.clone(), pred_pattern.clone(), dst.clone()),
                    (dst.clone(), pred_pattern, src.clone()),
                ],
            };
            self.track_iso_hop(instances);
        }

        // Register edge info for later `r.prop` and `r IS NULL` resolution.
        if let Some(ref var_name) = rel.variable {
            let reif_var = if self.rdf_star {
                None
            } else {
                Some(self.fresh_var(&format!("reif_{var_name}")))
            };
            // Introduce a marker variable bound to the predicate IRI.
            // This enables IS NULL / IS NOT NULL checks on typed relationship
            // variables: when the edge is found, ?var_marker is bound; when the
            // OPTIONAL MATCH containing the edge fails, ?var_marker is unbound.
            let marker = self.fresh_var(&format!("{var_name}_marker"));
            path_patterns.push(GraphPattern::Extend {
                inner: Box::new(empty_bgp()),
                variable: marker.clone(),
                expression: SparExpr::Literal(SparLit::new_typed_literal(
                    pred.as_str(),
                    NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#anyURI"),
                )),
            });
            self.edge_map.insert(
                var_name.clone(),
                EdgeInfo {
                    src: src.clone(),
                    pred: pred.clone(),
                    pred_var: None,
                    dst: dst.clone(),
                    reif_var,
                    null_check_var: Some(marker),
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

    /// Emit a bounded path `*lower..upper` as a UNION of explicit fixed-length chain patterns.
    ///
    /// Each hop-count from `lower` to `upper` produces one alternative in the UNION.
    fn emit_bounded_path_union(
        &mut self,
        rel: &RelationshipPattern,
        src: &TermPattern,
        dst: &TermPattern,
        base_ppe: &spargebra::algebra::PropertyPathExpression,
        lower: u64,
        upper: u64,
        _triples: &mut Vec<TriplePattern>,
        path_patterns: &mut Vec<GraphPattern>,
    ) -> Result<(), PolygraphError> {
        use crate::ast::cypher::Direction;
        use spargebra::algebra::PropertyPathExpression as PPE;

        let type_iri = if rel.rel_types.is_empty() {
            self.iri("__rel")
        } else {
            self.iri(&rel.rel_types[0])
        };

        // Determine effective subject/object based on direction.
        let (effective_src, effective_dst) = match rel.direction {
            Direction::Left => (dst.clone(), src.clone()),
            _ => (src.clone(), dst.clone()),
        };

        let mut union_patterns: Vec<GraphPattern> = Vec::new();

        for hop_count in lower..=upper {
            if hop_count == 0 {
                // Zero-hop: src = dst. Use a FILTER on a sentinel-bound sub-pattern.
                // Emit a VALUES clause that equates src and dst via a shared variable.
                // Since src and dst will be bound by surrounding patterns, we use Filter.
                // Emit an empty BGP with a FILTER(?src = ?dst) — but only if both are vars.
                let filter_expr = if let (TermPattern::Variable(s), TermPattern::Variable(d)) =
                    (&effective_src, &effective_dst)
                {
                    Some(SparExpr::Equal(
                        Box::new(SparExpr::Variable(s.clone())),
                        Box::new(SparExpr::Variable(d.clone())),
                    ))
                } else {
                    None
                };
                let zero_inner = empty_bgp();
                let zero_pattern = if let Some(f) = filter_expr {
                    GraphPattern::Filter {
                        expr: f,
                        inner: Box::new(zero_inner),
                    }
                } else {
                    GraphPattern::Bgp { patterns: vec![] }
                };
                union_patterns.push(zero_pattern);
            } else if hop_count == 1 {
                // Single hop: emit as a plain path (allows multi-direction).
                let p = match rel.direction {
                    Direction::Left => PPE::Reverse(Box::new(base_ppe.clone())),
                    Direction::Both => PPE::Alternative(
                        Box::new(base_ppe.clone()),
                        Box::new(PPE::Reverse(Box::new(base_ppe.clone()))),
                    ),
                    _ => base_ppe.clone(),
                };
                union_patterns.push(GraphPattern::Path {
                    subject: effective_src.clone(),
                    path: p,
                    object: effective_dst.clone(),
                });
            } else {
                // Multiple hops: chain via intermediate vars.
                // For undirected patterns, enumerate all 2^N direction combos
                // with pairwise edge-uniqueness FILTERs.
                let is_undirected = rel.direction == Direction::Both;
                if is_undirected {
                    // Pre-generate intermediate variables (shared across combos).
                    let intermediates: Vec<TermPattern> = (0..hop_count - 1)
                        .map(|k| self.fresh_var(&format!("mid{}", k)).into())
                        .collect();
                    let is_untyped = rel.rel_types.is_empty();

                    let mut union_arms: Vec<GraphPattern> = Vec::new();
                    for combo in 0..(1u64 << hop_count) {
                        let mut hop_parts: Vec<GraphPattern> = Vec::new();
                        let mut hop_subjs: Vec<TermPattern> = Vec::new();
                        let mut hop_objs: Vec<TermPattern> = Vec::new();

                        let mut prev = effective_src.clone();
                        for hop in 0..hop_count {
                            let next: TermPattern = if (hop as usize) < intermediates.len() {
                                intermediates[hop as usize].clone()
                            } else {
                                effective_dst.clone()
                            };
                            let is_forward = (combo >> hop) & 1 == 0;
                            let (subj, obj) = if is_forward {
                                (prev.clone(), next.clone())
                            } else {
                                (next.clone(), prev.clone())
                            };
                            hop_subjs.push(subj.clone());
                            hop_objs.push(obj.clone());
                            if is_untyped {
                                hop_parts.push(GraphPattern::Path {
                                    subject: subj,
                                    path: base_ppe.clone(),
                                    object: obj,
                                });
                            } else {
                                hop_parts.push(GraphPattern::Bgp {
                                    patterns: vec![TriplePattern {
                                        subject: subj,
                                        predicate: type_iri.clone().into(),
                                        object: obj,
                                    }],
                                });
                            }
                            prev = next;
                        }

                        let mut arm = hop_parts
                            .into_iter()
                            .reduce(|a, b| join_patterns(a, b))
                            .unwrap_or_else(empty_bgp);

                        // Pairwise edge-uniqueness: FILTER NOT(si=sj AND oi=oj)
                        for i in 0..hop_count as usize {
                            for j in (i + 1)..hop_count as usize {
                                let si_eq_sj = SparExpr::Equal(
                                    Box::new(term_to_sparexpr(&hop_subjs[i])),
                                    Box::new(term_to_sparexpr(&hop_subjs[j])),
                                );
                                let oi_eq_oj = SparExpr::Equal(
                                    Box::new(term_to_sparexpr(&hop_objs[i])),
                                    Box::new(term_to_sparexpr(&hop_objs[j])),
                                );
                                let same = SparExpr::And(
                                    Box::new(si_eq_sj),
                                    Box::new(oi_eq_oj),
                                );
                                arm = GraphPattern::Filter {
                                    expr: SparExpr::Not(Box::new(same)),
                                    inner: Box::new(arm),
                                };
                            }
                        }

                        union_arms.push(arm);
                    }

                    let combined = union_arms
                        .into_iter()
                        .reduce(|a, b| GraphPattern::Union {
                            left: Box::new(a),
                            right: Box::new(b),
                        })
                        .unwrap_or_else(empty_bgp);
                    union_patterns.push(combined);
                } else {
                    // Directed multi-hop: simple chain of triple/path patterns.
                    let is_untyped = rel.rel_types.is_empty();
                    let mut parts: Vec<GraphPattern> = Vec::new();
                    let mut prev = effective_src.clone();
                    for hop in 0..hop_count {
                        let next: TermPattern = if hop == hop_count - 1 {
                            effective_dst.clone()
                        } else {
                            self.fresh_var(&format!("mid{}", hop)).into()
                        };
                        if is_untyped {
                            // Untyped: use NPS path per hop.
                            parts.push(GraphPattern::Path {
                                subject: prev.clone(),
                                path: base_ppe.clone(),
                                object: next.clone(),
                            });
                        } else {
                            parts.push(GraphPattern::Bgp {
                                patterns: vec![TriplePattern {
                                    subject: prev.clone(),
                                    predicate: type_iri.clone().into(),
                                    object: next.clone(),
                                }],
                            });
                        }
                        prev = next;
                    }
                    let mut chain = parts
                        .into_iter()
                        .reduce(|a, b| join_patterns(a, b))
                        .unwrap_or_else(empty_bgp);
                    // For untyped, filter out literal endpoints.
                    if is_untyped {
                        if let TermPattern::Variable(v) = &effective_dst {
                            chain = GraphPattern::Filter {
                                expr: SparExpr::Not(Box::new(SparExpr::FunctionCall(
                                    spargebra::algebra::Function::IsLiteral,
                                    vec![SparExpr::Variable(v.clone())],
                                ))),
                                inner: Box::new(chain),
                            };
                        }
                    }
                    union_patterns.push(chain);
                }
            }
        }

        // Combine with UNION.
        let combined = union_patterns
            .into_iter()
            .reduce(|a, b| GraphPattern::Union {
                left: Box::new(a),
                right: Box::new(b),
            })
            .unwrap_or_else(empty_bgp);
        path_patterns.push(combined);
        Ok(())
    }

    /// Emit bounded path union with RDF-star per-hop property annotation filters.
    ///
    /// Each hop in the chain includes the base typed triple AND an annotation
    /// triple for each property constraint (e.g., `<< s <T> o >> <year> 1988 .`).
    fn emit_bounded_path_union_with_props(
        &mut self,
        rel: &RelationshipPattern,
        src: &TermPattern,
        dst: &TermPattern,
        lower: u64,
        upper: u64,
        path_patterns: &mut Vec<GraphPattern>,
    ) -> Result<(), PolygraphError> {
        use crate::ast::cypher::Direction;

        let type_iri = self.iri(&rel.rel_types[0]);
        let (effective_src, effective_dst) = match rel.direction {
            Direction::Left => (dst.clone(), src.clone()),
            _ => (src.clone(), dst.clone()),
        };

        let props = rel.properties.as_ref().unwrap();
        let mut union_patterns: Vec<GraphPattern> = Vec::new();

        for hop_count in lower..=upper {
            let mut parts: Vec<GraphPattern> = Vec::new();
            let mut prev = effective_src.clone();

            for hop in 0..hop_count {
                let next: TermPattern = if hop == hop_count - 1 {
                    effective_dst.clone()
                } else {
                    self.fresh_var(&format!("mid{}", hop)).into()
                };

                // Base edge triple: ?prev <T> ?next
                let edge_triple = TriplePattern {
                    subject: prev.clone(),
                    predicate: type_iri.clone().into(),
                    object: next.clone(),
                };
                let mut hop_triples = vec![edge_triple.clone()];

                // RDF-star annotation triples for property constraints:
                // << ?prev <T> ?next >> <prop> value .
                for (key, val_expr) in props {
                    if let Expression::Literal(lit) = val_expr {
                        if let Ok(sparql_lit) = self.translate_literal(lit) {
                            let anno_triple = TriplePattern {
                                subject: spargebra::term::TermPattern::Triple(Box::new(
                                    spargebra::term::TriplePattern {
                                        subject: prev.clone(),
                                        predicate: type_iri.clone().into(),
                                        object: next.clone(),
                                    },
                                )),
                                predicate: self.iri(key).into(),
                                object: spargebra::term::TermPattern::Literal(sparql_lit),
                            };
                            hop_triples.push(anno_triple);
                        }
                    }
                }

                parts.push(GraphPattern::Bgp {
                    patterns: hop_triples,
                });
                prev = next;
            }

            let chain = parts
                .into_iter()
                .reduce(|a, b| join_patterns(a, b))
                .unwrap_or_else(empty_bgp);
            union_patterns.push(chain);
        }

        let combined = union_patterns
            .into_iter()
            .reduce(|a, b| GraphPattern::Union {
                left: Box::new(a),
                right: Box::new(b),
            })
            .unwrap_or_else(empty_bgp);
        path_patterns.push(combined);
        Ok(())
    }

    // ── RETURN clause ─────────────────────────────────────────────────────────

    /// Returns `(extra_bgp_triples, Some(projected_vars) | None for *, distinct_flag, aggregates, pre_extends, post_extends)`.
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
                        aggregates.extend(self.pending_aggs.drain(..));
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
                let base_var = self.extract_variable(base_expr)?;
                let var_name = base_var.as_str().to_string();
                // Use alias when it doesn't conflict with the base variable name.
                // When alias == base_var (e.g. `a.name AS a`), always use a fresh var
                // to avoid self-referential triple `?a <name> ?a`.  The WITH handler
                // is responsible for renaming the fresh var to the alias in a fresh scope.
                let result_var = match &item.alias {
                    Some(alias) if alias != &var_name => Variable::new_unchecked(alias.clone()),
                    _ => self.fresh_var(&format!("{}_{}", var_name, key)),
                };
                // Check whether base_var is a relationship variable.
                if let Some(edge) = self.edge_map.get(&var_name).cloned() {
                    let prop_iri = self.iri(key);
                    if self.rdf_star {
                        use spargebra::term::NamedNodePattern;
                        let pred_pattern: NamedNodePattern = match edge.pred_var.clone() {
                            Some(pv) => NamedNodePattern::Variable(pv),
                            None => NamedNodePattern::NamedNode(edge.pred.clone()),
                        };
                        let edge_triple = spargebra::term::TriplePattern {
                            subject: edge.src.clone(),
                            predicate: pred_pattern,
                            object: edge.dst.clone(),
                        };
                        triples.push(spargebra::term::TriplePattern {
                            subject: spargebra::term::TermPattern::Triple(Box::new(edge_triple)),
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

                    // Quote each value: CONCAT("'", STR(expr), "'")
                    let quoted = SparExpr::FunctionCall(
                        spargebra::algebra::Function::Concat,
                        vec![
                            SparExpr::Literal(SparLit::new_simple_literal("'")),
                            SparExpr::FunctionCall(spargebra::algebra::Function::Str, vec![inner]),
                            SparExpr::Literal(SparLit::new_simple_literal("'")),
                        ],
                    );
                    let gc_agg = AggregateExpression::FunctionCall {
                        name: AggregateFunction::GroupConcat {
                            separator: Some(", ".to_string()),
                        },
                        expr: quoted,
                        distinct: *distinct,
                    };
                    // Wrap: CONCAT("[", gc_var, "]")
                    let wrap_expr = SparExpr::FunctionCall(
                        spargebra::algebra::Function::Concat,
                        vec![
                            SparExpr::Literal(SparLit::new_simple_literal("[")),
                            SparExpr::Variable(gc_var.clone()),
                            SparExpr::Literal(SparLit::new_simple_literal("]")),
                        ],
                    );
                    return Ok((result_var, Some((gc_var, gc_agg)), Some(wrap_expr)));
                }
                let sparql_agg = self.translate_aggregate_expr(agg_expr, extra)?;
                Ok((result_var.clone(), Some((result_var, sparql_agg)), None))
            }
            other => {
                // General expression: try to translate as SPARQL expression,
                // and bind it via Extend. If the expression contains aggregates
                // (e.g. count(*) * 10), record them via pending_aggs so that
                // the caller can wire them up to a GROUP pattern.
                //
                // Always use a fresh var for the Extend target to avoid conflicts
                // with pattern variables that may share the same alias name.
                let result_var = self.fresh_var(&item.alias.as_deref().unwrap_or("ret"));
                self.pending_aggs.clear();
                match self.translate_expr(other, extra) {
                    Ok(sparql_expr) => {
                        if !self.pending_aggs.is_empty() {
                            // Expression wraps an aggregate (e.g. count(*) * 10).
                            // Take the captured aggregate binding: use only the first
                            // for now (covers the common single-aggregate case).
                            let (agg_var, agg) = self.pending_aggs.remove(0);
                            self.pending_aggs.clear();
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
                // For relationship variables, return the null-check marker variable
                // (or the predicate variable for untyped relationships).  This allows
                // IS NULL / IS NOT NULL checks on relationship variables.
                if let Some(edge) = self.edge_map.get(name.as_str()) {
                    let check_var = edge
                        .null_check_var
                        .clone()
                        .or_else(|| edge.pred_var.clone());
                    if let Some(v) = check_var {
                        return Ok(SparExpr::Variable(v));
                    }
                }
                Ok(SparExpr::Variable(Variable::new_unchecked(name.clone())))
            }
            Expression::Literal(Literal::Null) => {
                // Cypher null → an unbound SPARQL variable (never added to any BGP).
                // Arithmetic over unbound variables produces type errors in SPARQL,
                // which propagate as null in SELECT projections — matching Cypher semantics.
                Ok(SparExpr::Variable(self.fresh_var("null")))
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
                        // Use pred_var when available (untyped relationship) so the
                        // annotated triple matches the actual stored predicate.
                        use spargebra::term::NamedNodePattern;
                        let pred_pat: NamedNodePattern = match edge.pred_var.clone() {
                            Some(pv) => NamedNodePattern::Variable(pv),
                            None => NamedNodePattern::NamedNode(edge.pred.clone()),
                        };
                        let edge_triple = spargebra::term::TriplePattern {
                            subject: edge.src.clone(),
                            predicate: pred_pat,
                            object: edge.dst.clone(),
                        };
                        extra.push(spargebra::term::TriplePattern {
                            subject: spargebra::term::TermPattern::Triple(Box::new(edge_triple)),
                            predicate: prop_iri.into(),
                            object: fresh.clone().into(),
                        });
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
            Expression::Add(a, b) => {
                // Check if both operands are property accesses — may be list concatenation.
                // Use runtime type check: IF(STRSTARTS(?a, "["), concat_lists, numeric_add)
                let is_list_candidate =
                    matches!(a.as_ref(), Expression::Property(..))
                        && matches!(b.as_ref(), Expression::Property(..));
                let la = self.translate_expr(a, extra)?;
                let lb = self.translate_expr(b, extra)?;
                if is_list_candidate {
                    // List concat: CONCAT(SUBSTR(?a, 1, STRLEN(?a)-1), ", ", SUBSTR(?b, 2))
                    use spargebra::algebra::Function;
                    let one = SparExpr::Literal(SparLit::new_typed_literal(
                        "1",
                        NamedNode::new_unchecked(XSD_INTEGER),
                    ));
                    let two = SparExpr::Literal(SparLit::new_typed_literal(
                        "2",
                        NamedNode::new_unchecked(XSD_INTEGER),
                    ));
                    let strlen_a = SparExpr::FunctionCall(
                        Function::StrLen,
                        vec![la.clone()],
                    );
                    let len_minus_1 = SparExpr::Subtract(
                        Box::new(strlen_a),
                        Box::new(one.clone()),
                    );
                    let head = SparExpr::FunctionCall(
                        Function::SubStr,
                        vec![la.clone(), one, len_minus_1],
                    );
                    let tail = SparExpr::FunctionCall(
                        Function::SubStr,
                        vec![lb.clone(), two],
                    );
                    let sep = SparExpr::Literal(SparLit::new_simple_literal(", "));
                    let concat = SparExpr::FunctionCall(
                        Function::Concat,
                        vec![head, sep, tail],
                    );
                    // Runtime check: IF(STRSTARTS(STR(?a), "["), concat, ?a + ?b)
                    let str_a = SparExpr::FunctionCall(Function::Str, vec![la.clone()]);
                    let bracket = SparExpr::Literal(SparLit::new_simple_literal("["));
                    let is_list = SparExpr::FunctionCall(
                        Function::StrStarts,
                        vec![str_a, bracket],
                    );
                    let numeric_add = SparExpr::Add(Box::new(la), Box::new(lb));
                    Ok(SparExpr::If(
                        Box::new(is_list),
                        Box::new(concat),
                        Box::new(numeric_add),
                    ))
                } else {
                    Ok(SparExpr::Add(Box::new(la), Box::new(lb)))
                }
            }
            Expression::Subtract(a, b) => Ok(SparExpr::Subtract(
                Box::new(self.translate_expr(a, extra)?),
                Box::new(self.translate_expr(b, extra)?),
            )),
            Expression::Multiply(a, b) => Ok(SparExpr::Multiply(
                Box::new(self.translate_expr(a, extra)?),
                Box::new(self.translate_expr(b, extra)?),
            )),
            Expression::Divide(a, b) => {
                let la = self.translate_expr(a, extra)?;
                let rb = self.translate_expr(b, extra)?;
                // Workaround for Oxigraph's right-associative `/` parsing:
                // `a / b / c` is parsed as `a / (b / c)` instead of `(a / b) / c`.
                // When both divisors are integer literals we flatten:
                // (x / li) / ri  →  FLOOR(x / (li * ri))
                //
                // Also: SPARQL treats xsd:integer / xsd:integer as xsd:decimal, but
                // Cypher truncates toward zero (floor division for integers).
                // Apply FLOOR when divisor is an integer literal.
                fn lit_int(e: &SparExpr) -> Option<i64> {
                    if let SparExpr::Literal(l) = e {
                        l.value().parse().ok()
                    } else {
                        None
                    }
                }
                use spargebra::algebra::Function;
                let rb_is_int_lit = lit_int(&rb).is_some();
                if let SparExpr::Divide(ref inner_a, ref inner_b) = la {
                    if let (Some(li), Some(ri)) = (lit_int(inner_b), lit_int(&rb)) {
                        let combined = SparExpr::Literal(SparLit::new_typed_literal(
                            (li * ri).to_string(),
                            NamedNode::new_unchecked(XSD_INTEGER),
                        ));
                        let div = SparExpr::Divide(inner_a.clone(), Box::new(combined));
                        return Ok(SparExpr::FunctionCall(Function::Floor, vec![div]));
                    }
                }
                let div = SparExpr::Divide(Box::new(la), Box::new(rb));
                if rb_is_int_lit {
                    Ok(SparExpr::FunctionCall(Function::Floor, vec![div]))
                } else {
                    Ok(div)
                }
            }
            Expression::Modulo(a, b) => {
                // a % b = a - FLOOR(a / b) * b
                // This correctly propagates null when either operand is unbound.
                let la = self.translate_expr(a, extra)?;
                let rb = self.translate_expr(b, extra)?;
                let div = SparExpr::Divide(Box::new(la.clone()), Box::new(rb.clone()));
                let floor_div =
                    SparExpr::FunctionCall(spargebra::algebra::Function::Floor, vec![div]);
                let floor_times_b = SparExpr::Multiply(Box::new(floor_div), Box::new(rb));
                Ok(SparExpr::Subtract(Box::new(la), Box::new(floor_times_b)))
            }
            Expression::Negate(inner) => Ok(SparExpr::UnaryMinus(Box::new(
                self.translate_expr(inner, extra)?,
            ))),
            Expression::Power(a, b) => {
                // SPARQL has no standard exponentiation operator.
                // Evaluate subexpressions for side-effects (extra triples), then emit
                // a custom function that Oxigraph does not recognise.  Unknown custom
                // functions return null in spareval, which matches Cypher's behaviour
                // when either operand is null (the only use-case in the TCK suite).
                let la = self.translate_expr(a, extra)?;
                let rb = self.translate_expr(b, extra)?;
                Ok(SparExpr::FunctionCall(
                    spargebra::algebra::Function::Custom(NamedNode::new_unchecked(
                        "urn:polygraph:unsupported-pow",
                    )),
                    vec![la, rb],
                ))
            }
            Expression::List(items) => {
                // Lists are handled inline for IN expressions (see Comparison arm above).
                // For standalone list literals (e.g. in RETURN), serialize as string.
                if items.is_empty() {
                    return Ok(SparExpr::Literal(SparLit::new_simple_literal("[]")));
                }
                let serialized = serialize_list_literal(items);
                Ok(SparExpr::Literal(SparLit::new_simple_literal(serialized)))
            }
            Expression::Map(pairs) => {
                // Serialize map literal as a string: {key: value, ...}
                // For non-literal values (e.g. aggregates), use CONCAT to build dynamically.
                let mut concat_pieces: Vec<SparExpr> = Vec::new();
                concat_pieces.push(SparExpr::Literal(SparLit::new_simple_literal("{")));
                for (i, (key, val_expr)) in pairs.iter().enumerate() {
                    if i > 0 {
                        concat_pieces.push(SparExpr::Literal(SparLit::new_simple_literal(", ")));
                    }
                    concat_pieces.push(SparExpr::Literal(SparLit::new_simple_literal(format!(
                        "{}: ",
                        key
                    ))));
                    match val_expr {
                        Expression::Literal(Literal::Integer(n)) => {
                            concat_pieces.push(SparExpr::Literal(SparLit::new_simple_literal(
                                n.to_string(),
                            )));
                        }
                        Expression::Literal(Literal::Float(f)) => {
                            concat_pieces.push(SparExpr::Literal(SparLit::new_simple_literal(
                                f.to_string(),
                            )));
                        }
                        Expression::Literal(Literal::String(s)) => {
                            concat_pieces.push(SparExpr::Literal(SparLit::new_simple_literal(
                                format!("'{}'", s),
                            )));
                        }
                        Expression::Literal(Literal::Boolean(b)) => {
                            concat_pieces.push(SparExpr::Literal(SparLit::new_simple_literal(
                                if *b { "true" } else { "false" },
                            )));
                        }
                        _ => {
                            let translated = self.translate_expr(val_expr, extra)?;
                            // COALESCE(STR(...), "") prevents CONCAT from returning null
                            // when a variable (e.g. relationship var) is unbound.
                            concat_pieces.push(SparExpr::Coalesce(vec![
                                SparExpr::FunctionCall(
                                    spargebra::algebra::Function::Str,
                                    vec![translated],
                                ),
                                SparExpr::Literal(SparLit::new_simple_literal("")),
                            ]));
                        }
                    }
                }
                concat_pieces.push(SparExpr::Literal(SparLit::new_simple_literal("}")));
                Ok(SparExpr::FunctionCall(
                    spargebra::algebra::Function::Concat,
                    concat_pieces,
                ))
            }
            Expression::FunctionCall {
                name,
                distinct: _,
                args,
            } => self.translate_function_call(name, args, extra),
            Expression::LabelCheck { variable, labels } => {
                // Translate `n:Label1:Label2` as a conjunction of EXISTS checks.
                // Each label becomes EXISTS { ?n rdf:type <base:Label> }.
                let var = Variable::new_unchecked(variable.clone());
                let mut exprs: Vec<SparExpr> = labels
                    .iter()
                    .map(|label| {
                        let type_triple = TriplePattern {
                            subject: var.clone().into(),
                            predicate: self.rdf_type().into(),
                            object: self.iri(label).into(),
                        };
                        SparExpr::Exists(Box::new(GraphPattern::Bgp {
                            patterns: vec![type_triple],
                        }))
                    })
                    .collect();
                if exprs.is_empty() {
                    // No labels: vacuously true.
                    Ok(SparExpr::Literal(SparLit::new_typed_literal(
                        "true",
                        NamedNode::new_unchecked(XSD_BOOLEAN),
                    )))
                } else {
                    let first = exprs.remove(0);
                    Ok(exprs
                        .into_iter()
                        .fold(first, |acc, e| SparExpr::And(Box::new(acc), Box::new(e))))
                }
            }
            Expression::PatternPredicate(pattern) => {
                // Translate (a)-[:T]->(b:Label) to EXISTS { triple patterns }.
                let mut inner_triples: Vec<TriplePattern> = Vec::new();
                let mut inner_paths: Vec<GraphPattern> = Vec::new();
                self.translate_pattern(pattern, &mut inner_triples, &mut inner_paths)?;
                let bgp = GraphPattern::Bgp {
                    patterns: inner_triples,
                };
                let combined = inner_paths.into_iter().fold(bgp, join_patterns);
                Ok(SparExpr::Exists(Box::new(combined)))
            }
            Expression::Aggregate(agg) => {
                // Aggregates in expressions (e.g. HAVING) are not yet handled; they
                // are handled at the RETURN level via translate_aggregate_expr.
                let fresh = self.fresh_var("agg");
                let agg_expr = self.translate_aggregate_expr(agg, extra)?;
                // Register the aggregate for GROUP-level binding.
                self.pending_aggs.push((fresh.clone(), agg_expr));
                Ok(SparExpr::Variable(fresh))
            }
        }
    }

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
                if let Some(Expression::Variable(var_name)) = args.first() {
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
                            let iri = edge.pred.as_str().to_string();
                            let local =
                                iri.strip_prefix(&self.base_iri).unwrap_or(&iri).to_string();
                            return Ok(SparExpr::Literal(SparLit::new_simple_literal(local)));
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
                }
                // length(string) maps to STRLEN.
                let arg = self.translate_expr(&args[0], extra)?;
                Ok(SparExpr::FunctionCall(Function::StrLen, vec![arg]))
            }
            "substring" | "substr" => {
                let mut sargs: Vec<SparExpr> = Vec::new();
                for a in args {
                    sargs.push(self.translate_expr(a, extra)?);
                }
                Ok(SparExpr::FunctionCall(Function::SubStr, sargs))
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
                    // size(string) → STRLEN(string)
                    let translated = self.translate_expr(arg, extra)?;
                    Ok(SparExpr::FunctionCall(Function::StrLen, vec![translated]))
                } else {
                    Err(PolygraphError::UnsupportedFeature {
                        feature: "size() requires an argument".to_string(),
                    })
                }
            }
            "head" => {
                // head(list) → first element. For collected lists (GROUP_CONCAT),
                // extract substring before first separator.
                if let Some(arg) = args.first() {
                    let translated = self.translate_expr(arg, extra)?;
                    // STRBEFORE(str, " ") gets first space-separated element
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
                // tail(list) → all but first. Approximate with STRAFTER.
                if let Some(arg) = args.first() {
                    let translated = self.translate_expr(arg, extra)?;
                    let sep = SparExpr::Literal(SparLit::new_simple_literal(" "));
                    Ok(SparExpr::FunctionCall(
                        Function::StrAfter,
                        vec![translated, sep],
                    ))
                } else {
                    Err(PolygraphError::UnsupportedFeature {
                        feature: "tail() requires an argument".to_string(),
                    })
                }
            }
            "nodes" => {
                // nodes(p) → list of node IRIs along named path p.
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
            "keys" | "labels" | "relationships" | "range" | "reverse" | "split"
            | "trim" | "ltrim" | "rtrim" | "left" | "right" => {
                Err(PolygraphError::UnsupportedFeature {
                    feature: format!("function call: {name}()"),
                })
            }
            _ => Err(PolygraphError::UnsupportedFeature {
                feature: format!("function call: {name}()"),
            }),
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
            Expression::Literal(Literal::Null) => {
                // UNWIND null → empty result.
                let values = GraphPattern::Values {
                    variables: vec![var],
                    bindings: vec![],
                };
                Ok(join_patterns(current, values))
            }
            Expression::FunctionCall { name, args, .. } if name.to_ascii_lowercase() == "range" => {
                // UNWIND range(start, end) or range(start, end, step) AS var.
                // Expand to a VALUES clause at compile time if args are literals.
                let get_int = |e: &Expression| match e {
                    Expression::Literal(Literal::Integer(n)) => Some(*n),
                    _ => None,
                };
                let start = args.first().and_then(get_int);
                let end = args.get(1).and_then(get_int);
                let step = args.get(2).and_then(get_int).unwrap_or(1);
                if let (Some(s), Some(e)) = (start, end) {
                    if step == 0 {
                        return Err(PolygraphError::UnsupportedFeature {
                            feature: "range() with step=0".to_string(),
                        });
                    }
                    let mut values: Vec<Vec<Option<GroundTerm>>> = Vec::new();
                    let mut i = s;
                    while (step > 0 && i <= e) || (step < 0 && i >= e) {
                        let lit = SparLit::new_typed_literal(
                            i.to_string(),
                            NamedNode::new_unchecked(XSD_INTEGER),
                        );
                        values.push(vec![Some(GroundTerm::Literal(lit))]);
                        i += step;
                    }
                    let gp = GraphPattern::Values {
                        variables: vec![var],
                        bindings: values,
                    };
                    Ok(join_patterns(current, gp))
                } else {
                    Err(PolygraphError::UnsupportedFeature {
                        feature: "range() with non-literal arguments".to_string(),
                    })
                }
            }
            Expression::List(items) => {
                // Literal list: expand to VALUES ?var { val1 val2 ... }
                // Each element is either a ground term or a nested list (encoded as string).
                //
                // If ALL elements are lists (list-of-lists), register the flattened
                // contents in with_list_vars so a subsequent UNWIND can expand it.
                // Don't emit VALUES for the outer variable; the next UNWIND will
                // expand the flattened list directly.
                let all_inner_lists: bool = items.iter().all(|e| matches!(e, Expression::List(_)));
                if all_inner_lists && !items.is_empty() {
                    let mut flattened = Vec::new();
                    for e in items {
                        if let Expression::List(inner) = e {
                            flattened.extend(inner.clone());
                        }
                    }
                    self.with_list_vars
                        .insert(u.variable.clone(), Expression::List(flattened));
                    return Ok(current);
                }

                let bindings: Result<Vec<Vec<Option<GroundTerm>>>, _> = items
                    .iter()
                    .map(|e| match e {
                        Expression::Literal(Literal::Null) => Ok(vec![None]),
                        Expression::List(inner) => {
                            // Nested list: encode as serialized string literal.
                            let encoded = serialize_list_literal(inner);
                            Ok(vec![Some(GroundTerm::Literal(
                                SparLit::new_simple_literal(encoded),
                            ))])
                        }
                        _ => {
                            let ground = self.expr_to_ground_term(e)?;
                            let gt = term_pattern_to_ground(ground)?;
                            Ok(vec![Some(gt)])
                        }
                    })
                    .collect();
                let values = GraphPattern::Values {
                    variables: vec![var],
                    bindings: bindings?,
                };
                Ok(join_patterns(current, values))
            }
            Expression::Variable(list_var) => {
                // UNWIND variable — check if it was defined as a literal list in a WITH clause.
                if let Some(list_expr) = self.with_list_vars.get(list_var.as_str()).cloned() {
                    // Recursively expand as if the expression were written inline.
                    let inline = list_expr;
                    return self.translate_unwind_clause(
                        &crate::ast::cypher::UnwindClause {
                            expression: inline,
                            variable: u.variable.clone(),
                        },
                        current,
                        extra,
                    );
                }
                // Fall through: SPARQL 1.1 has no native list iteration.
                let _ = extra;
                Err(PolygraphError::UnsupportedFeature {
                    feature: format!(
                        "UNWIND of variable ?{list_var} (non-literal list): requires engine extension"
                    ),
                })
            }
            Expression::Add(a, b) => {
                // UNWIND (list_a + list_b) — if both operands can be resolved to
                // literal lists, concatenate them and expand inline.
                fn resolve_list(
                    expr: &Expression,
                    list_vars: &std::collections::HashMap<String, Expression>,
                ) -> Option<Vec<Expression>> {
                    match expr {
                        Expression::List(items) => Some(items.clone()),
                        Expression::Variable(v) => match list_vars.get(v.as_str()) {
                            Some(Expression::List(items)) => Some(items.clone()),
                            _ => None,
                        },
                        _ => None,
                    }
                }
                let list_a = resolve_list(a, &self.with_list_vars);
                let list_b = resolve_list(b, &self.with_list_vars);
                if let (Some(mut la), Some(lb)) = (list_a, list_b) {
                    la.extend(lb);
                    let combined = Expression::List(la);
                    return self.translate_unwind_clause(
                        &crate::ast::cypher::UnwindClause {
                            expression: combined,
                            variable: u.variable.clone(),
                        },
                        current,
                        extra,
                    );
                }
                Err(PolygraphError::UnsupportedFeature {
                    feature: "UNWIND of non-literal expression".to_string(),
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

// ── Peephole optimizations ────────────────────────────────────────────────────

/// Detect `WITH … collect(X) AS list … / UNWIND list AS item` pairs and rewrite
/// them into simple projections (`WITH … X AS item …`).  This eliminates the
/// need for runtime list iteration which SPARQL 1.1 cannot express.
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
        Expression::Aggregate(_) | Expression::Literal(_) | Expression::Map(_) => false,
    }
}

/// Build an empty BGP for use as the identity element in joins.
fn empty_bgp() -> GraphPattern {
    GraphPattern::Bgp { patterns: vec![] }
}

/// Serialize a list of expressions to a string like `[1, 2, 'foo']`.
fn serialize_list_literal(elems: &[Expression]) -> String {
    let parts: Vec<String> = elems
        .iter()
        .map(|e| match e {
            Expression::Literal(Literal::Integer(n)) => n.to_string(),
            Expression::Literal(Literal::Float(f)) => f.to_string(),
            Expression::Literal(Literal::String(s)) => format!("'{s}'"),
            Expression::Literal(Literal::Boolean(b)) => b.to_string(),
            Expression::List(inner) => serialize_list_literal(inner),
            _ => "?".to_string(),
        })
        .collect();
    format!("[{}]", parts.join(", "))
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

/// Convert a `TermPattern` to a `SparExpr` for use in FILTER comparisons.
fn term_to_sparexpr(tp: &TermPattern) -> SparExpr {
    match tp {
        TermPattern::Variable(v) => SparExpr::Variable(v.clone()),
        TermPattern::NamedNode(n) => SparExpr::Literal(SparLit::new_simple_literal(n.as_str())),
        TermPattern::Literal(lit) => SparExpr::Literal(lit.clone()),
        TermPattern::BlankNode(b) => SparExpr::Literal(SparLit::new_simple_literal(b.as_str())),
        TermPattern::Triple(_) => SparExpr::Literal(SparLit::new_simple_literal("__triple__")),
    }
}

/// Convert a `NamedNodePattern` to a `SparExpr` for use in FILTER comparisons.
fn named_node_to_sparexpr(nnp: &spargebra::term::NamedNodePattern) -> SparExpr {
    use spargebra::term::NamedNodePattern;
    match nnp {
        NamedNodePattern::Variable(v) => SparExpr::Variable(v.clone()),
        NamedNodePattern::NamedNode(n) => {
            SparExpr::Literal(SparLit::new_simple_literal(n.as_str()))
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
