#![forbid(unsafe_code)]

//! `polygraph` — transpile openCypher and ISO GQL queries to SPARQL 1.1.
//!
//! # Phase 1 — openCypher Parser
//!
//! The current release implements the parser layer. Given an openCypher
//! query string, [`parser::parse_cypher`] returns a typed [`ast::CypherQuery`]
//! AST covering `MATCH`, `OPTIONAL MATCH`, `WHERE`, `RETURN`, and `WITH`.
//!
//! Full transpilation to SPARQL (Phase 2) and ISO GQL support (Phase 5) are
//! not yet available; calling those paths returns
//! [`PolygraphError::UnsupportedFeature`].
//!
//! # Example
//!
//! ```rust
//! use polygraph::parser::parse_cypher;
//!
//! let ast = parse_cypher("MATCH (n:Person) WHERE n.age > 30 RETURN n.name").unwrap();
//! println!("{ast:#?}");
//! ```

pub mod ast;
pub mod error;
pub mod parser;
pub mod rdf_mapping;
pub mod target;
pub mod translator;

pub use error::PolygraphError;

/// The main entry point for transpilation operations.
///
/// Transpilation methods beyond parsing are planned for Phase 2 and later.
pub struct Transpiler;

impl Transpiler {
    /// Parse an openCypher query string and return a typed AST.
    ///
    /// This is the stable Phase 1 API. Transpilation to SPARQL is
    /// implemented in Phase 2 via [`Self::cypher_to_sparql`].
    pub fn parse_cypher(cypher: &str) -> Result<ast::CypherQuery, PolygraphError> {
        parser::parse_cypher(cypher)
    }

    /// Transpile an openCypher query to a SPARQL query string.
    ///
    /// **Phase 2** — not yet implemented.
    pub fn cypher_to_sparql(
        _cypher: &str,
        _engine: &dyn target::TargetEngine,
    ) -> Result<String, PolygraphError> {
        Err(PolygraphError::UnsupportedFeature {
            feature: "Cypher-to-SPARQL transpilation (planned for Phase 2)".to_string(),
        })
    }

    /// Transpile an ISO GQL query to a SPARQL query string.
    ///
    /// **Phase 5** — not yet implemented.
    pub fn gql_to_sparql(
        _gql: &str,
        _engine: &dyn target::TargetEngine,
    ) -> Result<String, PolygraphError> {
        Err(PolygraphError::UnsupportedFeature {
            feature: "GQL-to-SPARQL transpilation (planned for Phase 5)".to_string(),
        })
    }
}
