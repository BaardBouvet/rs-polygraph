//! LQA expression IR — typed expressions with explicit null-propagation.
//!
//! Every operator in [`crate::lqa::op::Op`] uses [`Expr`] for predicate and
//! projection arguments.  The [`Type`] lattice records the static type known
//! at normalisation time; `None` means "not yet inferred" (Phase 4 adds full
//! type inference).
//!
//! # Null-propagation contract
//!
//! openCypher 9 §6.1 ("Handling of null in expressions") specifies that:
//! - Arithmetic, string, and comparison operations with a `null` operand
//!   return `null`.
//! - Boolean expressions are three-valued: `true AND null = null`,
//!   `false AND null = false`, `true OR null = true`, `false OR null = null`.
//! - The `IS NULL` / `IS NOT NULL` tests are the only safe null checks.
//!
//! Individual [`Expr`] variants are annotated with `// NULL-PROPAGATION(spec-ref):`
//! comments identifying the relevant spec section.

// ── Type lattice ─────────────────────────────────────────────────────────────

/// The type lattice for LQA expressions.
///
/// This mirrors the openCypher 9 type system (§2.1 "Types"):
/// primitives < composite < graph < Any, with Null as a special bottom.
///
/// The lattice is used by the normaliser (Phase 4) to infer expression types
/// and by the SPARQL lowerer to decide which coercion / guard patterns to emit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    // ── Primitives ──────────────────────────────────────────────────────────
    /// `INTEGER` — 64-bit signed integer.  openCypher 9 §2.1.1
    Integer,
    /// `FLOAT` — IEEE 754 double.  openCypher 9 §2.1.2
    Float,
    /// `STRING`.  openCypher 9 §2.1.3
    String,
    /// `BOOLEAN`.  openCypher 9 §2.1.4
    Boolean,
    // ── Temporal ────────────────────────────────────────────────────────────
    /// `DATE`.  openCypher 9 §2.1.6
    Date,
    /// `LOCAL TIME`.  openCypher 9 §2.1.7
    LocalTime,
    /// `ZONED TIME`.  openCypher 9 §2.1.8
    ZonedTime,
    /// `LOCAL DATETIME`.  openCypher 9 §2.1.9
    LocalDateTime,
    /// `ZONED DATETIME`.  openCypher 9 §2.1.10
    ZonedDateTime,
    /// `DURATION`.  openCypher 9 §2.1.11
    Duration,
    // ── Composite ───────────────────────────────────────────────────────────
    /// Homogeneous list of elements.  The inner type is `Box<Type>`.
    /// Heterogeneous lists use `List(Box::new(Type::Any))`.
    ///
    /// openCypher 9 §2.1.13
    List(Box<Type>),
    /// Property map (`{key: value, …}`).  openCypher 9 §2.1.14
    Map,
    // ── Graph ────────────────────────────────────────────────────────────────
    /// A graph node.  openCypher 9 §2.1.5 (NODE)
    Node,
    /// A graph relationship.  openCypher 9 §2.1.5 (RELATIONSHIP)
    Relationship,
    /// A path value (alternating nodes and relationships).  openCypher 9 §2.1.15
    Path,
    // ── Meta ─────────────────────────────────────────────────────────────────
    /// Top of the lattice — any type.
    Any,
    /// Explicit `null` type (lower than any concrete type).
    /// An expression of type `Null` always evaluates to null.
    Null,
    /// A numeric type that may be either Integer or Float (used during type
    /// inference before the exact numeric kind is resolved).
    Numeric,
}

impl Type {
    /// Returns `true` if a value of this type can ever be `null`.
    ///
    /// In openCypher 9, all types may be null; this method identifies types
    /// where null is the *only* possible value.
    pub fn is_nullable(&self) -> bool {
        // All Cypher types can be null; Type::Null means provably-null.
        true
    }

    /// Returns `true` if this type is a numeric kind (Integer, Float, or Numeric).
    pub fn is_numeric(&self) -> bool {
        matches!(self, Type::Integer | Type::Float | Type::Numeric)
    }

    /// Returns `true` if this is a graph entity type (Node, Relationship, or Path).
    pub fn is_graph_entity(&self) -> bool {
        matches!(self, Type::Node | Type::Relationship | Type::Path)
    }

    /// Returns `true` for primitive scalar types (not composite or graph).
    pub fn is_primitive(&self) -> bool {
        matches!(
            self,
            Type::Integer
                | Type::Float
                | Type::String
                | Type::Boolean
                | Type::Date
                | Type::LocalTime
                | Type::ZonedTime
                | Type::LocalDateTime
                | Type::ZonedDateTime
                | Type::Duration
                | Type::Null
        )
    }

    /// Lattice meet (greatest lower bound): the most specific type that is a
    /// subtype of both `self` and `other`.
    ///
    /// Used during type inference to narrow a variable's type.
    pub fn meet(&self, other: &Type) -> Type {
        if self == other {
            return self.clone();
        }
        // Numeric coercion: Integer ∧ Float = Numeric
        if self.is_numeric() && other.is_numeric() {
            return Type::Numeric;
        }
        // Null is the bottom; meet with anything stays at self/other unless one is Null
        if matches!(self, Type::Null) {
            return other.clone();
        }
        if matches!(other, Type::Null) {
            return self.clone();
        }
        Type::Any
    }

    /// Lattice join (least upper bound): the least specific type that is a
    /// supertype of both `self` and `other`.
    ///
    /// Used during type inference to widen a variable's type.
    pub fn join(&self, other: &Type) -> Type {
        if self == other {
            return self.clone();
        }
        if matches!(self, Type::Any) || matches!(other, Type::Any) {
            return Type::Any;
        }
        // Numeric widening
        if self.is_numeric() && other.is_numeric() {
            if matches!(self, Type::Float) || matches!(other, Type::Float) {
                return Type::Float;
            }
            return Type::Numeric;
        }
        // Null joins up to the other type
        if matches!(self, Type::Null) {
            return other.clone();
        }
        if matches!(other, Type::Null) {
            return self.clone();
        }
        Type::Any
    }
}

// ── Value literals ────────────────────────────────────────────────────────────

/// A ground (constant) Cypher value used in the LQA expression IR.
///
/// Matches the value variants of `ast::cypher::Literal` but is independent of
/// the AST module so the LQA can be used without the parser.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Integer(i64),
    Float(f64),
    String(std::string::String),
    Boolean(bool),
    Null,
}

impl Literal {
    /// Return the static type of this literal.
    pub fn ty(&self) -> Type {
        match self {
            Literal::Integer(_) => Type::Integer,
            Literal::Float(_) => Type::Float,
            Literal::String(_) => Type::String,
            Literal::Boolean(_) => Type::Boolean,
            Literal::Null => Type::Null,
        }
    }
}

// ── Operator kinds ────────────────────────────────────────────────────────────

/// Binary comparison operators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmpOp {
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
    RegexMatch,
}

/// Unary arithmetic/logical operators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
    Not,
}

/// Sort direction for ORDER BY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

/// Quantifier kind for `all / any / none / single` expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuantKind {
    All,
    Any,
    None,
    Single,
}

/// Aggregate function kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggKind {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    /// `collect()` — serialises to a SPARQL list representation.
    Collect,
    /// `count(*)` — specialized COUNT without an argument.
    CountStar,
}

// ── Expression IR ─────────────────────────────────────────────────────────────

/// LQA expression — a typed, null-propagation-aware Cypher expression.
///
/// Each variant that performs arithmetic or comparison carries an implicit
/// null-propagation rule per openCypher 9 §6.1.  Variants that are
/// semantically distinct from their AST counterparts are documented with
/// `// NULL-PROPAGATION(spec):` annotations.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    // ── Terminals ────────────────────────────────────────────────────────────
    /// Reference to a query variable (column in the current scope).
    Variable { name: std::string::String, ty: Option<Type> },
    /// A ground constant value.
    Literal(Literal),
    /// A query parameter: `$name` or `$0`.
    Parameter(std::string::String),

    // ── Arithmetic ───────────────────────────────────────────────────────────
    // NULL-PROPAGATION(openCypher 9 §6.1): all arithmetic operators return
    // null if either operand is null.
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Mod(Box<Expr>, Box<Expr>),
    Pow(Box<Expr>, Box<Expr>),
    Unary(UnaryOp, Box<Expr>),

    // ── Comparison ───────────────────────────────────────────────────────────
    // NULL-PROPAGATION(openCypher 9 §6.1): comparisons return null when either
    // operand is null, except IsNull / IsNotNull which always return a boolean.
    Comparison(CmpOp, Box<Expr>, Box<Expr>),
    IsNull(Box<Expr>),
    IsNotNull(Box<Expr>),

    // ── Boolean ──────────────────────────────────────────────────────────────
    // NULL-PROPAGATION(openCypher 9 §6.1): three-valued logic.
    // `true AND null = null`, `false AND null = false`, etc.
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Xor(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),

    // ── Property / subscript ─────────────────────────────────────────────────
    /// `expr.key` — property lookup.
    ///
    /// NULL-PROPAGATION(openCypher 9 §6.1): returns null if `expr` is null
    /// or if the property does not exist on the node/relationship.
    Property(Box<Expr>, std::string::String),
    /// `expr[index]` — list element or map key access.
    ///
    /// NULL-PROPAGATION: null if `expr` or `index` is null; null if index
    /// is out of range or key is absent.
    Subscript(Box<Expr>, Box<Expr>),
    /// `expr[start..end]` — list slice.
    ListSlice {
        list: Box<Expr>,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
    },

    // ── Collections ──────────────────────────────────────────────────────────
    /// A list literal: `[expr, …]`.
    List(Vec<Expr>),
    /// A map literal: `{key: expr, …}`.
    Map(Vec<(std::string::String, Expr)>),

    // ── Function calls ───────────────────────────────────────────────────────
    /// A general (non-aggregate) function call.
    FunctionCall {
        name: std::string::String,
        distinct: bool,
        args: Vec<Expr>,
    },
    /// An aggregate function call.  Only valid inside a [`crate::lqa::op::Op::GroupBy`].
    Aggregate {
        kind: AggKind,
        distinct: bool,
        /// `None` for `COUNT(*)`.
        arg: Option<Box<Expr>>,
    },

    // ── CASE ─────────────────────────────────────────────────────────────────
    /// **Searched** CASE expression: `CASE WHEN pred THEN result … [ELSE default] END`.
    ///
    /// openCypher 9 §6.2.2 defines this as the canonical form.  Simple CASE
    /// (`CASE expr WHEN value THEN …`) is desugared to searched CASE by
    /// [`crate::lqa::normalize::simple_case_to_searched`].
    ///
    /// NULL-PROPAGATION: `ELSE null` is assumed when no ELSE is present.
    CaseSearched {
        branches: Vec<(Expr, Expr)>,
        else_expr: Option<Box<Expr>>,
    },

    // ── Quantifiers ──────────────────────────────────────────────────────────
    /// `all(x IN list WHERE pred)` etc.  openCypher 9 §6.3.4
    ///
    /// NULL-PROPAGATION: returns null if `list` is null; treats null elements
    /// as neither satisfying nor violating the predicate (three-valued logic).
    Quantifier {
        kind: QuantKind,
        variable: std::string::String,
        list: Box<Expr>,
        predicate: Box<Expr>,
    },

    // ── Comprehensions ───────────────────────────────────────────────────────
    /// `[x IN list WHERE pred | projection]`  openCypher 9 §6.3.3
    ListComprehension {
        variable: std::string::String,
        list: Box<Expr>,
        predicate: Option<Box<Expr>>,
        projection: Option<Box<Expr>>,
    },
    /// `[(n)-[r]->(m) WHERE pred | projection]`  openCypher 9 §6.3.6
    PatternComprehension {
        /// Optional path variable: `[path = (n)-->(m) | …]`
        alias: Option<std::string::String>,
        /// The pattern to match (represented as an inner Op in the LQA).
        /// The contained Op is typically a Scan + Expand chain.
        pattern_op: Box<crate::lqa::op::Op>,
        predicate: Option<Box<Expr>>,
        projection: Box<Expr>,
    },
    /// `REDUCE(acc = init, x IN list | body)`  openCypher 9 §6.3.7
    Reduce {
        acc: std::string::String,
        init: Box<Expr>,
        variable: std::string::String,
        list: Box<Expr>,
        body: Box<Expr>,
    },

    // ── Label check ──────────────────────────────────────────────────────────
    /// `n:Label` or `n:A:B` — label set membership test.
    LabelCheck {
        expr: Box<Expr>,
        labels: Vec<std::string::String>,
    },

    // ── Subquery ─────────────────────────────────────────────────────────────
    /// `EXISTS { … }` — subquery existence test.  openCypher 9 §6.3.8
    Exists(Box<crate::lqa::op::Op>),
}

impl Expr {
    /// Returns a reference to the static type annotation, or `None` if not yet inferred.
    pub fn ty(&self) -> Option<&Type> {
        match self {
            Expr::Variable { ty, .. } => ty.as_ref(),
            Expr::Literal(lit) => {
                // We can't return a reference to a temporary; callers use lit.ty() directly.
                let _ = lit;
                None
            }
            _ => None,
        }
    }

    /// Wrap this expression in a `NOT(…)`.
    pub fn not(self) -> Expr {
        Expr::Not(Box::new(self))
    }

    /// Wrap this expression in `IS NULL`.
    pub fn is_null(self) -> Expr {
        Expr::IsNull(Box::new(self))
    }

    /// Wrap two expressions in `AND`.
    pub fn and(self, other: Expr) -> Expr {
        Expr::And(Box::new(self), Box::new(other))
    }

    /// Wrap two expressions in `OR`.
    pub fn or(self, other: Expr) -> Expr {
        Expr::Or(Box::new(self), Box::new(other))
    }

    /// Convenience constructor for a simple equality comparison.
    pub fn eq(self, rhs: Expr) -> Expr {
        Expr::Comparison(CmpOp::Eq, Box::new(self), Box::new(rhs))
    }

    /// Convenience constructor for a variable reference with no type annotation.
    pub fn var(name: impl Into<std::string::String>) -> Expr {
        Expr::Variable { name: name.into(), ty: None }
    }

    /// Convenience constructor for an integer literal.
    pub fn int(v: i64) -> Expr {
        Expr::Literal(Literal::Integer(v))
    }

    /// Convenience constructor for a string literal.
    pub fn str(s: impl Into<std::string::String>) -> Expr {
        Expr::Literal(Literal::String(s.into()))
    }

    /// Convenience constructor for a boolean literal.
    pub fn bool(b: bool) -> Expr {
        Expr::Literal(Literal::Boolean(b))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_meet_same() {
        assert_eq!(Type::Integer.meet(&Type::Integer), Type::Integer);
        assert_eq!(Type::String.meet(&Type::String), Type::String);
    }

    #[test]
    fn type_meet_numeric() {
        assert_eq!(Type::Integer.meet(&Type::Float), Type::Numeric);
        assert_eq!(Type::Float.meet(&Type::Integer), Type::Numeric);
    }

    #[test]
    fn type_meet_null_identity() {
        assert_eq!(Type::Null.meet(&Type::String), Type::String);
        assert_eq!(Type::String.meet(&Type::Null), Type::String);
    }

    #[test]
    fn type_join_same() {
        assert_eq!(Type::Boolean.join(&Type::Boolean), Type::Boolean);
    }

    #[test]
    fn type_join_widens_to_any() {
        assert_eq!(Type::String.join(&Type::Integer), Type::Any);
    }

    #[test]
    fn type_join_null_identity() {
        assert_eq!(Type::Null.join(&Type::Float), Type::Float);
    }

    #[test]
    fn type_join_any_absorbs() {
        assert_eq!(Type::Any.join(&Type::Integer), Type::Any);
    }

    #[test]
    fn type_numeric_predicate() {
        assert!(Type::Integer.is_numeric());
        assert!(Type::Float.is_numeric());
        assert!(Type::Numeric.is_numeric());
        assert!(!Type::String.is_numeric());
        assert!(!Type::Node.is_numeric());
    }

    #[test]
    fn type_is_graph_entity() {
        assert!(Type::Node.is_graph_entity());
        assert!(Type::Relationship.is_graph_entity());
        assert!(Type::Path.is_graph_entity());
        assert!(!Type::Integer.is_graph_entity());
    }

    #[test]
    fn type_list_inner() {
        let t = Type::List(Box::new(Type::String));
        assert!(!t.is_numeric());
        assert!(!t.is_graph_entity());
    }

    #[test]
    fn literal_types() {
        assert_eq!(Literal::Integer(42).ty(), Type::Integer);
        assert_eq!(Literal::Float(1.5).ty(), Type::Float);
        assert_eq!(Literal::String("hi".into()).ty(), Type::String);
        assert_eq!(Literal::Boolean(true).ty(), Type::Boolean);
        assert_eq!(Literal::Null.ty(), Type::Null);
    }

    #[test]
    fn expr_var_constructor() {
        let e = Expr::var("n");
        assert!(matches!(&e, Expr::Variable { name, ty: None } if name == "n"));
    }

    #[test]
    fn expr_int_constructor() {
        let e = Expr::int(7);
        assert_eq!(e, Expr::Literal(Literal::Integer(7)));
    }

    #[test]
    fn expr_not_wrapping() {
        let e = Expr::var("x").not();
        assert!(matches!(e, Expr::Not(_)));
    }

    #[test]
    fn expr_and_combinator() {
        let a = Expr::var("a");
        let b = Expr::var("b");
        let c = a.and(b);
        assert!(matches!(c, Expr::And(_, _)));
    }

    #[test]
    fn expr_or_combinator() {
        let a = Expr::var("a");
        let b = Expr::var("b");
        let c = a.or(b);
        assert!(matches!(c, Expr::Or(_, _)));
    }

    #[test]
    fn expr_eq_combinator() {
        let a = Expr::var("n");
        let b = Expr::int(5);
        let cmp = a.eq(b);
        assert!(matches!(cmp, Expr::Comparison(CmpOp::Eq, _, _)));
    }

    #[test]
    fn expr_is_null() {
        let e = Expr::var("n").is_null();
        assert!(matches!(e, Expr::IsNull(_)));
    }

    #[test]
    fn expr_case_searched_no_else() {
        let e = Expr::CaseSearched {
            branches: vec![(Expr::bool(true), Expr::int(1))],
            else_expr: None,
        };
        assert!(matches!(e, Expr::CaseSearched { else_expr: None, .. }));
    }

    #[test]
    fn expr_quantifier_any() {
        let e = Expr::Quantifier {
            kind: QuantKind::Any,
            variable: "x".into(),
            list: Box::new(Expr::var("list")),
            predicate: Box::new(Expr::bool(true)),
        };
        assert!(matches!(e, Expr::Quantifier { kind: QuantKind::Any, .. }));
    }

    #[test]
    fn expr_list_comprehension() {
        let e = Expr::ListComprehension {
            variable: "x".into(),
            list: Box::new(Expr::var("items")),
            predicate: None,
            projection: Some(Box::new(Expr::Property(
                Box::new(Expr::var("x")),
                "name".into(),
            ))),
        };
        assert!(matches!(e, Expr::ListComprehension { projection: Some(_), .. }));
    }

    #[test]
    fn expr_aggregate_count_star() {
        let e = Expr::Aggregate {
            kind: AggKind::CountStar,
            distinct: false,
            arg: None,
        };
        assert!(matches!(e, Expr::Aggregate { kind: AggKind::CountStar, arg: None, .. }));
    }

    #[test]
    fn expr_aggregate_collect_distinct() {
        let e = Expr::Aggregate {
            kind: AggKind::Collect,
            distinct: true,
            arg: Some(Box::new(Expr::var("x"))),
        };
        assert!(matches!(e, Expr::Aggregate { kind: AggKind::Collect, distinct: true, .. }));
    }

    #[test]
    fn sort_dir_variants() {
        assert_eq!(SortDir::Asc, SortDir::Asc);
        assert_ne!(SortDir::Asc, SortDir::Desc);
    }

    #[test]
    fn cmp_op_variants() {
        let ops = [CmpOp::Eq, CmpOp::Ne, CmpOp::Lt, CmpOp::Le, CmpOp::Gt, CmpOp::Ge];
        assert_eq!(ops.len(), 6);
    }
}
