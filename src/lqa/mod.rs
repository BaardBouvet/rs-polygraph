//! Logical Query Algebra (LQA) — openCypher semantic IR.
//!
//! The LQA is the load-bearing addition introduced in Phase 3 of the
//! spec-first pivot.  Its purpose is to separate openCypher semantics from
//! SPARQL mechanics: every semantic rule is encoded here; everything below
//! is mechanical lowering.
//!
//! # Overview
//!
//! ```text
//! Cypher AST  (src/ast/cypher.rs)
//!      │
//! [AST → LQA lowering]          ←── Phase 4
//!      │
//! LQA Op tree  (src/lqa/)       ←── Phase 3 (this module)
//!      │
//! [LQA → SPARQL lowering]       ←── Phase 4
//!      │
//! spargebra::GraphPattern
//! ```
//!
//! # Module layout
//!
//! | Module | Purpose |
//! |--------|---------|
//! [`expr`] | Typed expression IR (`Expr`) and type lattice (`Type`) |
//! [`op`]   | Operator enum (`Op`) covering all Cypher algebra operators |
//! [`bag`]  | Generic bag (multiset) combinators for bag-semantics reasoning |
//! [`normalize`] | Desugaring rules with spec citations; Phase 3 implements CASE normalisation and alias lifting |
//!
//! # Spec references
//!
//! - openCypher 9 §2 — Types
//! - openCypher 9 §4 — Clauses (MATCH, WITH, RETURN, UNWIND, …)
//! - openCypher 9 §5 — Aggregation
//! - openCypher 9 §6 — Expressions (CASE, quantifiers, comprehensions, …)
//!
//! # Phase 4 road map
//!
//! - AST → LQA lowering (clause-by-clause, with unit tests per clause)
//! - LQA → SPARQL lowering (replaces the direct AST → SPARQL path)
//! - Type inference pass (annotates every `Expr` with a `Type`)
//! - Full normalisation: list/pattern comprehensions, variable scoping,
//!   alias resolution, quantifier tautology folding

pub mod bag;
pub mod expr;
pub mod lower;
pub mod normalize;
pub mod op;
pub mod sparql;

// Re-export the most commonly used types for ergonomic use at the crate root.
pub use expr::{AggKind, CmpOp, Expr, Literal, QuantKind, SortDir, Type, UnaryOp};
pub use op::{Direction, Op, PathRange, ProjItem, SortKey};
