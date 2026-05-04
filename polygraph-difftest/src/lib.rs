//! Differential testing harness for rs-polygraph.
//!
//! See [plans/spec-first-pivot.md](../../plans/spec-first-pivot.md) Phase 1.
//!
//! # Architecture
//!
//! ```text
//!   PropertyGraph fixture ──► RDF projection ──► Oxigraph store
//!         │                                          │
//!         │                                          │ SPARQL
//!         │                                          ▼
//!         │                                     Result rows
//!   Cypher query ──► polygraph::Transpiler ──────────┘
//!         │                                          │
//!         │                                          │
//!         ▼                                          ▼
//!   Expected result bag (curated)  ◄── compare ──► Actual result bag
//!         OR
//!   Live Neo4j (feature live-neo4j) ──────────────────┘
//! ```
//!
//! The curated suite ships with hand-derived expected results citing the
//! openCypher 9 reference semantics. The live-Neo4j path lets nightly CI
//! validate that the same Cypher query produces the same bag on both engines.

#![forbid(unsafe_code)]

pub mod fixture;
pub mod oracle;
pub mod rdf_projection;
pub mod runner;
pub mod suite;
pub mod value;

#[cfg(feature = "live-neo4j")]
pub mod neo4j;

pub use fixture::{PropertyGraph, NodeSpec, EdgeSpec};
pub use oracle::{Comparison, ComparisonOutcome};
pub use runner::{run_curated, run_one, RunReport};
pub use suite::{Expectation, OrderMode, QuerySpec};
pub use value::Value;
