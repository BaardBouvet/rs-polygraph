/// openCypher AST node types for Phase 1.
///
/// Covers the core read-query constructs: `MATCH`, `OPTIONAL MATCH`,
/// `WHERE`, `RETURN`, and `WITH`.

// ── Primitive aliases ────────────────────────────────────────────────────────

/// A bare identifier (variable name, label, property key, relationship type).
pub type Ident = String;
/// A node label (the part after `:` in a node pattern).
pub type Label = String;
/// A relationship type (the part after `:` inside `[...]`).
pub type RelType = String;

// ── Top-level query ──────────────────────────────────────────────────────────

/// The root of a parsed openCypher query.
#[derive(Debug, Clone, PartialEq)]
pub struct CypherQuery {
    pub clauses: Vec<Clause>,
}

// ── Clauses ──────────────────────────────────────────────────────────────────

/// A single clause within a Cypher query.
#[derive(Debug, Clone, PartialEq)]
pub enum Clause {
    Match(MatchClause),
    With(WithClause),
    Return(ReturnClause),
}

/// A `MATCH` or `OPTIONAL MATCH` clause, with an optional inline `WHERE`.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchClause {
    pub optional: bool,
    pub pattern: PatternList,
    pub where_: Option<WhereClause>,
}

/// A `WHERE` predicate.
#[derive(Debug, Clone, PartialEq)]
pub struct WhereClause {
    pub expression: Expression,
}

/// A `RETURN` clause.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnClause {
    pub distinct: bool,
    pub items: ReturnItems,
}

/// A `WITH` clause (projection + optional `WHERE`).
#[derive(Debug, Clone, PartialEq)]
pub struct WithClause {
    pub distinct: bool,
    pub items: ReturnItems,
    pub where_: Option<WhereClause>,
}

// ── Return / projection items ────────────────────────────────────────────────

/// Projection list for `RETURN` or `WITH`.
#[derive(Debug, Clone, PartialEq)]
pub enum ReturnItems {
    /// `RETURN *`
    All,
    /// `RETURN expr [AS alias], …`
    Explicit(Vec<ReturnItem>),
}

/// A single projected expression with an optional alias.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnItem {
    pub expression: Expression,
    pub alias: Option<Ident>,
}

// ── Pattern ──────────────────────────────────────────────────────────────────

/// A comma-separated list of patterns.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternList(pub Vec<Pattern>);

/// A single path pattern, optionally bound to a variable.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub variable: Option<Ident>,
    /// Alternating nodes and relationships, always starting and ending with a node.
    /// `[Node, Rel, Node, Rel, Node, …]`
    pub elements: Vec<PatternElement>,
}

/// An element within a path pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum PatternElement {
    Node(NodePattern),
    Relationship(RelationshipPattern),
}

/// A node pattern: `(variable:Label {prop: val})`.
#[derive(Debug, Clone, PartialEq)]
pub struct NodePattern {
    pub variable: Option<Ident>,
    pub labels: Vec<Label>,
    pub properties: Option<MapLiteral>,
}

/// A relationship pattern: `-[:TYPE*range {prop: val}]->`.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationshipPattern {
    pub variable: Option<Ident>,
    pub direction: Direction,
    pub rel_types: Vec<RelType>,
    pub properties: Option<MapLiteral>,
    pub range: Option<RangeQuantifier>,
}

/// The direction of a relationship arrow.
#[derive(Debug, Clone, PartialEq)]
pub enum Direction {
    /// `-->` or `-[…]->`
    Right,
    /// `<--` or `<-[…]-`
    Left,
    /// `--` or `-[…]-` (undirected)
    Both,
}

/// Variable-length range on a relationship pattern (`*`, `*2`, `*1..3`).
#[derive(Debug, Clone, PartialEq)]
pub struct RangeQuantifier {
    pub lower: Option<u64>,
    pub upper: Option<u64>,
}

// ── Expressions ──────────────────────────────────────────────────────────────

/// A Cypher expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Or(Box<Expression>, Box<Expression>),
    Xor(Box<Expression>, Box<Expression>),
    And(Box<Expression>, Box<Expression>),
    Not(Box<Expression>),
    Comparison(Box<Expression>, CompOp, Box<Expression>),
    IsNull(Box<Expression>),
    IsNotNull(Box<Expression>),
    Add(Box<Expression>, Box<Expression>),
    Subtract(Box<Expression>, Box<Expression>),
    Multiply(Box<Expression>, Box<Expression>),
    Divide(Box<Expression>, Box<Expression>),
    Modulo(Box<Expression>, Box<Expression>),
    Negate(Box<Expression>),
    Power(Box<Expression>, Box<Expression>),
    /// Property access: `expr.key`
    Property(Box<Expression>, Ident),
    Variable(Ident),
    Literal(Literal),
    List(Vec<Expression>),
    Map(MapLiteral),
}

/// Binary comparison operators.
#[derive(Debug, Clone, PartialEq)]
pub enum CompOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    StartsWith,
    EndsWith,
    Contains,
}

// ── Literals ─────────────────────────────────────────────────────────────────

/// A literal value.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Integer(i64),
    Float(f64),
    String(String),
    Boolean(bool),
    Null,
}

/// A map literal: `{key: expr, …}`.
pub type MapLiteral = Vec<(Ident, Expression)>;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cypher_query_holds_clauses() {
        let q = CypherQuery { clauses: vec![] };
        assert!(q.clauses.is_empty());
    }

    #[test]
    fn match_clause_optional_flag() {
        let m = MatchClause {
            optional: true,
            pattern: PatternList(vec![]),
            where_: None,
        };
        assert!(m.optional);
        assert!(m.where_.is_none());
    }

    #[test]
    fn match_clause_non_optional() {
        let m = MatchClause {
            optional: false,
            pattern: PatternList(vec![]),
            where_: None,
        };
        assert!(!m.optional);
    }

    #[test]
    fn node_pattern_fields() {
        let n = NodePattern {
            variable: Some("n".to_string()),
            labels: vec!["Person".to_string()],
            properties: None,
        };
        assert_eq!(n.variable.as_deref(), Some("n"));
        assert_eq!(n.labels, vec!["Person"]);
    }

    #[test]
    fn node_pattern_with_properties() {
        let props = vec![("age".to_string(), Expression::Literal(Literal::Integer(30)))];
        let n = NodePattern {
            variable: Some("n".to_string()),
            labels: vec![],
            properties: Some(props),
        };
        assert!(n.properties.is_some());
    }

    #[test]
    fn relationship_pattern_directions() {
        for dir in [Direction::Right, Direction::Left, Direction::Both] {
            let r = RelationshipPattern {
                variable: None,
                direction: dir.clone(),
                rel_types: vec![],
                properties: None,
                range: None,
            };
            assert_eq!(r.direction, dir);
        }
    }

    #[test]
    fn relationship_pattern_with_type_and_range() {
        let r = RelationshipPattern {
            variable: Some("r".to_string()),
            direction: Direction::Right,
            rel_types: vec!["KNOWS".to_string()],
            properties: None,
            range: Some(RangeQuantifier { lower: Some(1), upper: Some(3) }),
        };
        assert_eq!(r.rel_types, vec!["KNOWS"]);
        assert_eq!(r.range.as_ref().unwrap().lower, Some(1));
        assert_eq!(r.range.as_ref().unwrap().upper, Some(3));
    }

    #[test]
    fn return_items_all_variant() {
        let ri = ReturnItems::All;
        assert!(matches!(ri, ReturnItems::All));
    }

    #[test]
    fn return_item_with_alias() {
        let item = ReturnItem {
            expression: Expression::Variable("n".to_string()),
            alias: Some("node".to_string()),
        };
        assert_eq!(item.alias.as_deref(), Some("node"));
    }

    #[test]
    fn where_clause_holds_expression() {
        let wc = WhereClause {
            expression: Expression::Literal(Literal::Boolean(true)),
        };
        assert_eq!(wc.expression, Expression::Literal(Literal::Boolean(true)));
    }

    #[test]
    fn with_clause_fields() {
        let wc = WithClause {
            distinct: false,
            items: ReturnItems::All,
            where_: None,
        };
        assert!(!wc.distinct);
        assert!(wc.where_.is_none());
    }

    #[test]
    fn expression_literal_variants() {
        let _ = Expression::Literal(Literal::Integer(42));
        let _ = Expression::Literal(Literal::Float(3.14));
        let _ = Expression::Literal(Literal::String("hello".into()));
        let _ = Expression::Literal(Literal::Boolean(false));
        let _ = Expression::Literal(Literal::Null);
    }

    #[test]
    fn expression_comparison() {
        let lhs = Box::new(Expression::Variable("a".to_string()));
        let rhs = Box::new(Expression::Literal(Literal::Integer(5)));
        let expr = Expression::Comparison(lhs, CompOp::Gt, rhs);
        assert!(matches!(expr, Expression::Comparison(_, CompOp::Gt, _)));
    }

    #[test]
    fn expression_property_access() {
        let base = Box::new(Expression::Variable("n".to_string()));
        let expr = Expression::Property(base, "name".to_string());
        assert!(matches!(expr, Expression::Property(_, _)));
    }

    #[test]
    fn range_quantifier_unbounded_upper() {
        let rq = RangeQuantifier { lower: Some(1), upper: None };
        assert_eq!(rq.lower, Some(1));
        assert!(rq.upper.is_none());
    }

    #[test]
    fn pattern_elements_roundtrip() {
        let pattern = Pattern {
            variable: None,
            elements: vec![
                PatternElement::Node(NodePattern {
                    variable: Some("a".to_string()),
                    labels: vec!["Person".to_string()],
                    properties: None,
                }),
                PatternElement::Relationship(RelationshipPattern {
                    variable: None,
                    direction: Direction::Right,
                    rel_types: vec!["KNOWS".to_string()],
                    properties: None,
                    range: None,
                }),
                PatternElement::Node(NodePattern {
                    variable: Some("b".to_string()),
                    labels: vec!["Person".to_string()],
                    properties: None,
                }),
            ],
        };
        assert_eq!(pattern.elements.len(), 3);
    }
}
