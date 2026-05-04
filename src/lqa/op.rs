//! LQA operator enum — the algebraic operators of the Cypher query language.
//!
//! Every Cypher query compiles to a tree of [`Op`] nodes.  The tree is
//! **referentially transparent**: each `Op` node defines a relation (bag of
//! variable-bindings) entirely in terms of its children and parameters.
//!
//! # Operator conventions
//!
//! - Leaf operators (`Scan`, `Unit`, `Values`) produce a relation.
//! - Unary operators wrap a single `inner: Box<Op>`.
//! - Binary operators have named children (`left`, `right`).
//! - Write operators (`Create`, `Set`, `Delete`, `Remove`, `Merge`) carry the
//!   current read pipeline in `inner` and attach mutation side-effects that are
//!   applied after evaluation.
//!
//! The SPARQL lowerer (Phase 4) traverses this tree bottom-up to produce a
//! [`spargebra::GraphPattern`].

use crate::lqa::expr::{Expr, SortDir};

// ── Relationship direction ───────────────────────────────────────────────────

/// The direction of a relationship traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Direction {
    /// `(a)-->(b)`
    Outgoing,
    /// `(a)<--(b)`
    Incoming,
    /// `(a)--(b)` — undirected
    Undirected,
}

// ── Range quantifier for variable-length paths ───────────────────────────────

/// `[:REL*lower..upper]` range on a relationship expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRange {
    /// Minimum number of hops (default `1`).
    pub lower: u64,
    /// Maximum number of hops (`None` = unbounded).
    pub upper: Option<u64>,
}

impl Default for PathRange {
    fn default() -> Self {
        PathRange { lower: 1, upper: Some(1) }
    }
}

// ── Projection item ──────────────────────────────────────────────────────────

/// A single item in a `Projection` operator: `(expr, alias)`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjItem {
    pub expr: Expr,
    /// The name under which `expr` is available in the parent scope.
    pub alias: String,
}

/// A sort key in an `OrderBy` operator: `(expr, direction)`.
#[derive(Debug, Clone, PartialEq)]
pub struct SortKey {
    pub expr: Expr,
    pub dir: SortDir,
}

/// An aggregate output binding: `(aggregate_expr, alias)`.
#[derive(Debug, Clone, PartialEq)]
pub struct AggItem {
    pub expr: Expr,
    pub alias: String,
}

// ── SET / REMOVE items ───────────────────────────────────────────────────────

/// A single assignment in a `Set` operator.
#[derive(Debug, Clone, PartialEq)]
pub enum SetItem {
    /// `n.prop = expr`
    Property { variable: String, key: String, value: Expr },
    /// `n += {map}` — merge map into node properties
    MergeMap { variable: String, map: Expr },
    /// `n = expr` — replace all node properties
    Replace { variable: String, value: Expr },
    /// `SET n:Label` — add label(s)
    Label { variable: String, labels: Vec<String> },
}

/// A single item in a `Remove` operator.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoveItem {
    /// `REMOVE n.prop`
    Property { variable: String, key: String },
    /// `REMOVE n:Label`
    Label { variable: String, labels: Vec<String> },
}

// ── MERGE patterns ───────────────────────────────────────────────────────────

/// A MERGE pattern with its ON CREATE / ON MATCH handlers.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeClause {
    /// The pattern to match-or-create.
    pub pattern: Box<Op>,
    /// Assignments applied if the pattern was matched (ON MATCH SET).
    pub on_match: Vec<SetItem>,
    /// Assignments applied if the pattern was just created (ON CREATE SET).
    pub on_create: Vec<SetItem>,
}

// ── CREATE patterns ──────────────────────────────────────────────────────────

/// A single node to create.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateNode {
    pub variable: Option<String>,
    pub labels: Vec<String>,
    pub properties: Vec<(String, Expr)>,
}

/// A single directed edge to create between two nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateEdge {
    pub variable: Option<String>,
    pub from: String,
    pub to: String,
    pub rel_type: String,
    pub direction: Direction,
    pub properties: Vec<(String, Expr)>,
}

// ── The main operator enum ────────────────────────────────────────────────────

/// Logical Query Algebra operator.
///
/// Each variant represents a relational algebra operation specialised for
/// openCypher bag semantics.  The tree is constructed by the AST lowering
/// pass (Phase 4) and consumed by the SPARQL lowering pass (Phase 4+).
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    // ── Leaf operators ───────────────────────────────────────────────────────

    /// Produces a single empty row (the identity element for joins).
    /// Used for `RETURN` clauses with no preceding `MATCH`.
    Unit,

    /// Scans all nodes optionally filtered to a label.
    ///
    /// openCypher 9 §4.1 MATCH: each pattern element with a node binds a
    /// variable ranging over nodes in the graph.
    Scan {
        /// The variable bound to each node.
        variable: String,
        /// If set, only nodes carrying this label are produced.
        label: Option<String>,
        /// Additional label predicates (all must hold — conjunction).
        extra_labels: Vec<String>,
    },

    /// Extends each row by traversing a relationship from `from` to `to`.
    ///
    /// openCypher 9 §4.1 MATCH: relationship patterns.
    Expand {
        inner: Box<Op>,
        from: String,
        rel_var: Option<String>,
        to: String,
        rel_types: Vec<String>,
        direction: Direction,
        /// `None` = single hop; `Some(range)` = variable-length.
        range: Option<PathRange>,
        /// Optional path variable bound to the entire traversal.
        path_var: Option<String>,
    },

    /// Produces rows from a literal VALUES clause.
    ///
    /// Used to generate constant bindings (e.g. `WITH 1 AS n`).
    Values {
        /// Each item is `(column_name, literal_value)`.
        bindings: Vec<(String, Expr)>,
    },

    // ── Unary pipeline operators ─────────────────────────────────────────────

    /// Filters rows by a predicate.
    ///
    /// openCypher 9 §4.3 WHERE; openCypher 9 §4.4 WITH … WHERE.
    ///
    /// NULL-PROPAGATION: a row is kept only when `predicate` evaluates to
    /// `true`; `null` and `false` both remove the row.
    Selection {
        inner: Box<Op>,
        predicate: Expr,
    },

    /// Projects each row to a new set of columns.
    ///
    /// Corresponds to `RETURN` and `WITH` projections.
    /// openCypher 9 §4.4 WITH, §4.5 RETURN.
    Projection {
        inner: Box<Op>,
        items: Vec<ProjItem>,
        distinct: bool,
    },

    /// Groups rows and applies aggregate functions.
    ///
    /// openCypher 9 §5.3 Aggregation: any `RETURN` / `WITH` that contains
    /// an aggregate expression introduces an implicit `GroupBy`.
    ///
    /// `group_keys` are the non-aggregate items from the projection;
    /// `agg_items` are the aggregate expressions with their output aliases.
    GroupBy {
        inner: Box<Op>,
        group_keys: Vec<String>,
        agg_items: Vec<AggItem>,
    },

    /// Sorts rows.  `RETURN … ORDER BY` / `WITH … ORDER BY`.
    ///
    /// openCypher 9 §4.5.3 ORDER BY.
    OrderBy {
        inner: Box<Op>,
        keys: Vec<SortKey>,
    },

    /// Skips a fixed number of rows.  `SKIP n`.
    ///
    /// openCypher 9 §4.5.4 SKIP.
    Skip {
        inner: Box<Op>,
        count: Expr,
    },

    /// Limits the output to at most `count` rows.  `LIMIT n`.
    ///
    /// openCypher 9 §4.5.5 LIMIT.
    Limit {
        inner: Box<Op>,
        count: Expr,
    },

    /// Deduplicates rows.  Introduced by `RETURN DISTINCT` / `WITH DISTINCT`.
    ///
    /// openCypher 9 §4.5.2 DISTINCT: two rows are equal iff all column values
    /// compare equal under Cypher equality (null ≠ null; types must match).
    Distinct {
        inner: Box<Op>,
    },

    /// Iterates over a list expression and emits one row per element.
    ///
    /// openCypher 9 §4.6 UNWIND.
    Unwind {
        inner: Box<Op>,
        list: Expr,
        variable: String,
    },

    // ── Binary operators ─────────────────────────────────────────────────────

    /// UNION ALL — append two relations preserving duplicates.
    ///
    /// openCypher 9 §4.7 UNION ALL.
    UnionAll {
        left: Box<Op>,
        right: Box<Op>,
    },

    /// UNION — append two relations and deduplicate.
    ///
    /// openCypher 9 §4.7 UNION.
    Union {
        left: Box<Op>,
        right: Box<Op>,
    },

    /// Cartesian product of two independent patterns (no shared variables).
    ///
    /// Arises from `MATCH (a), (b)` where `a` and `b` don't overlap.
    CartesianProduct {
        left: Box<Op>,
        right: Box<Op>,
    },

    /// Left outer join — implements `OPTIONAL MATCH`.
    ///
    /// openCypher 9 §4.2 OPTIONAL MATCH: if the right branch does not produce
    /// any rows for a given left row, a single row with the right-side
    /// variables bound to `null` is produced.
    ///
    /// **Phase 4 fix target:** property lookups on nullable variables from the
    /// right branch must be scoped *inside* this operator's right branch.
    /// Currently the translator emits property-access OPTIONAL blocks *after*
    /// the LeftOuterJoin, which causes re-binding when the optional match
    /// fails.  The LQA lowerer will scope them correctly.
    LeftOuterJoin {
        left: Box<Op>,
        right: Box<Op>,
        /// An optional join condition (from inline WHERE).
        condition: Option<Expr>,
    },

    // ── Subquery / control flow ───────────────────────────────────────────────

    /// `CALL { … }` subquery.
    ///
    /// The inner query is evaluated independently; its result is joined with
    /// the outer query's current row as a lateral join.
    ///
    /// openCypher 9 / Neo4j 5.x CALL subquery.
    Subquery {
        outer: Box<Op>,
        inner: Box<Op>,
    },

    /// `FOREACH (var IN list | clause+)`.
    ///
    /// openCypher 9 §4.9 FOREACH: evaluates `body` once for each element of
    /// `list`, binding it to `variable`.  Side-effects only; does not change
    /// the outer row cardinality.
    Foreach {
        inner: Box<Op>,
        variable: String,
        list: Expr,
        body: Box<Op>,
    },

    // ── Write operators ──────────────────────────────────────────────────────

    /// `CREATE (n:Label {…}), (a)-[:T]->(b), …`
    ///
    /// openCypher 9 §6.1 CREATE.
    Create {
        inner: Box<Op>,
        nodes: Vec<CreateNode>,
        edges: Vec<CreateEdge>,
    },

    /// `MERGE pattern [ON MATCH SET …] [ON CREATE SET …]`
    ///
    /// openCypher 9 §6.3 MERGE.
    Merge {
        inner: Box<Op>,
        clause: MergeClause,
    },

    /// `SET n.prop = expr, …`
    ///
    /// openCypher 9 §6.4 SET.
    Set {
        inner: Box<Op>,
        items: Vec<SetItem>,
    },

    /// `[DETACH] DELETE expr, …`
    ///
    /// openCypher 9 §6.5 DELETE.
    Delete {
        inner: Box<Op>,
        detach: bool,
        exprs: Vec<Expr>,
    },

    /// `REMOVE n.prop, n:Label, …`
    ///
    /// openCypher 9 §6.6 REMOVE.
    Remove {
        inner: Box<Op>,
        items: Vec<RemoveItem>,
    },

    /// `CALL proc.name(args) YIELD …` — stored procedure invocation.
    Call {
        inner: Box<Op>,
        procedure: String,
        args: Vec<Expr>,
        yields: Vec<String>,
    },
}

impl Op {
    /// Wraps `self` in a `Selection` with the given predicate.
    pub fn filter(self, predicate: Expr) -> Op {
        Op::Selection { inner: Box::new(self), predicate }
    }

    /// Wraps `self` in a `Projection`.
    pub fn project(self, items: Vec<ProjItem>, distinct: bool) -> Op {
        Op::Projection { inner: Box::new(self), items, distinct }
    }

    /// Wraps `self` in an `OrderBy`.
    pub fn order_by(self, keys: Vec<SortKey>) -> Op {
        Op::OrderBy { inner: Box::new(self), keys }
    }

    /// Wraps `self` in a `Limit`.
    pub fn limit(self, count: Expr) -> Op {
        Op::Limit { inner: Box::new(self), count }
    }

    /// Wraps `self` in a `Skip`.
    pub fn skip(self, count: Expr) -> Op {
        Op::Skip { inner: Box::new(self), count }
    }

    /// Wraps `self` in a `Distinct`.
    pub fn distinct(self) -> Op {
        Op::Distinct { inner: Box::new(self) }
    }

    /// Wraps `self` in an `Unwind`.
    pub fn unwind(self, list: Expr, variable: impl Into<String>) -> Op {
        Op::Unwind { inner: Box::new(self), list, variable: variable.into() }
    }

    /// Returns `true` if this operator is a write operator (CREATE/MERGE/SET/DELETE/REMOVE).
    pub fn is_write(&self) -> bool {
        matches!(self, Op::Create { .. } | Op::Merge { .. } | Op::Set { .. } | Op::Delete { .. } | Op::Remove { .. })
    }

    /// Returns `true` if this operator has no side effects (pure read-only).
    pub fn is_read_only(&self) -> bool {
        !self.is_write()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lqa::expr::{Expr, Literal, SortDir};

    #[test]
    fn unit_is_read_only() {
        assert!(Op::Unit.is_read_only());
        assert!(!Op::Unit.is_write());
    }

    #[test]
    fn scan_with_label() {
        let op = Op::Scan {
            variable: "n".into(),
            label: Some("Person".into()),
            extra_labels: vec![],
        };
        assert!(op.is_read_only());
    }

    #[test]
    fn filter_builder() {
        let op = Op::Unit.filter(Expr::bool(true));
        assert!(matches!(op, Op::Selection { .. }));
    }

    #[test]
    fn project_builder() {
        let items = vec![ProjItem { expr: Expr::var("n"), alias: "node".into() }];
        let op = Op::Unit.project(items, false);
        assert!(matches!(op, Op::Projection { distinct: false, .. }));
    }

    #[test]
    fn project_distinct_builder() {
        let items = vec![ProjItem { expr: Expr::var("n"), alias: "node".into() }];
        let op = Op::Unit.project(items, true);
        assert!(matches!(op, Op::Projection { distinct: true, .. }));
    }

    #[test]
    fn order_by_builder() {
        let op = Op::Unit.order_by(vec![SortKey {
            expr: Expr::var("name"),
            dir: SortDir::Asc,
        }]);
        assert!(matches!(op, Op::OrderBy { .. }));
    }

    #[test]
    fn limit_builder() {
        let op = Op::Unit.limit(Expr::int(10));
        assert!(matches!(op, Op::Limit { .. }));
    }

    #[test]
    fn skip_builder() {
        let op = Op::Unit.skip(Expr::int(5));
        assert!(matches!(op, Op::Skip { .. }));
    }

    #[test]
    fn distinct_builder() {
        let op = Op::Unit.distinct();
        assert!(matches!(op, Op::Distinct { .. }));
    }

    #[test]
    fn unwind_builder() {
        let op = Op::Unit.unwind(Expr::var("list"), "x");
        assert!(matches!(op, Op::Unwind { variable, .. } if variable == "x"));
    }

    #[test]
    fn create_is_write() {
        let op = Op::Create {
            inner: Box::new(Op::Unit),
            nodes: vec![],
            edges: vec![],
        };
        assert!(op.is_write());
        assert!(!op.is_read_only());
    }

    #[test]
    fn merge_is_write() {
        let op = Op::Merge {
            inner: Box::new(Op::Unit),
            clause: MergeClause {
                pattern: Box::new(Op::Unit),
                on_match: vec![],
                on_create: vec![],
            },
        };
        assert!(op.is_write());
    }

    #[test]
    fn delete_is_write() {
        let op = Op::Delete {
            inner: Box::new(Op::Unit),
            detach: false,
            exprs: vec![Expr::var("n")],
        };
        assert!(op.is_write());
    }

    #[test]
    fn union_all_is_read_only() {
        let op = Op::UnionAll {
            left: Box::new(Op::Unit),
            right: Box::new(Op::Unit),
        };
        assert!(op.is_read_only());
    }

    #[test]
    fn left_outer_join_no_condition() {
        let op = Op::LeftOuterJoin {
            left: Box::new(Op::Unit),
            right: Box::new(Op::Unit),
            condition: None,
        };
        assert!(matches!(op, Op::LeftOuterJoin { condition: None, .. }));
    }

    #[test]
    fn expand_single_hop() {
        let scan = Op::Scan {
            variable: "a".into(),
            label: None,
            extra_labels: vec![],
        };
        let expand = Op::Expand {
            inner: Box::new(scan),
            from: "a".into(),
            rel_var: Some("r".into()),
            to: "b".into(),
            rel_types: vec!["KNOWS".into()],
            direction: Direction::Outgoing,
            range: None,
            path_var: None,
        };
        assert!(matches!(expand, Op::Expand { range: None, .. }));
    }

    #[test]
    fn expand_variable_length() {
        let expand = Op::Expand {
            inner: Box::new(Op::Unit),
            from: "a".into(),
            rel_var: None,
            to: "b".into(),
            rel_types: vec![],
            direction: Direction::Undirected,
            range: Some(PathRange { lower: 1, upper: None }),
            path_var: None,
        };
        assert!(matches!(expand, Op::Expand { range: Some(_), .. }));
    }

    #[test]
    fn values_literal_binding() {
        let op = Op::Values {
            bindings: vec![
                ("a".into(), Expr::int(1)),
                ("b".into(), Expr::str("hello")),
            ],
        };
        assert!(matches!(op, Op::Values { .. }));
    }

    #[test]
    fn path_range_default() {
        let r = PathRange::default();
        assert_eq!(r.lower, 1);
        assert_eq!(r.upper, Some(1));
    }

    #[test]
    fn set_item_property_variant() {
        let s = SetItem::Property {
            variable: "n".into(),
            key: "age".into(),
            value: Expr::int(30),
        };
        assert!(matches!(s, SetItem::Property { .. }));
    }

    #[test]
    fn remove_item_label() {
        let r = RemoveItem::Label {
            variable: "n".into(),
            labels: vec!["Temp".into()],
        };
        assert!(matches!(r, RemoveItem::Label { .. }));
    }

    #[test]
    fn groupby_structure() {
        let op = Op::GroupBy {
            inner: Box::new(Op::Unit),
            group_keys: vec!["dept".into()],
            agg_items: vec![AggItem {
                expr: Expr::Aggregate {
                    kind: crate::lqa::expr::AggKind::Count,
                    distinct: false,
                    arg: None,
                },
                alias: "cnt".into(),
            }],
        };
        assert!(matches!(op, Op::GroupBy { .. }));
    }
}
