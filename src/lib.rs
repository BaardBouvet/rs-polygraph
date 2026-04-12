#![forbid(unsafe_code)]

//! `polygraph` — transpile openCypher and ISO GQL queries to SPARQL 1.1.
//!
//! Phases 1–4 are complete:
//! - Phase 1: openCypher parser + AST
//! - Phase 2: SPARQL algebra translator (MATCH/WHERE/RETURN/WITH/OPTIONAL)
//! - Phase 3: RDF-star and reification edge property encoding
//! - Phase 4: ORDER BY/SKIP/LIMIT, aggregation, UNWIND, variable-length paths,
//!   multi-type relationships, IN list literals, write clause stubs
//!
//! Use [`target::RdfStar`] for engines that support SPARQL-star natively, or
//! [`target::GenericSparql11`] for standard SPARQL 1.1.
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
    /// The `engine` is consulted for engine-specific capabilities (RDF-star,
    /// federation). The optional `base_iri` on the engine is used as the
    /// namespace for labels, relationship types and property names.
    ///
    /// # Example
    ///
    /// ```rust
    /// use polygraph::{Transpiler, target::GenericSparql11};
    ///
    /// let engine = GenericSparql11;
    /// let sparql = Transpiler::cypher_to_sparql(
    ///     "MATCH (n:Person) WHERE n.age > 30 RETURN n.name",
    ///     &engine,
    /// ).unwrap();
    /// assert!(sparql.contains("SELECT"));
    /// ```
    pub fn cypher_to_sparql(
        cypher: &str,
        engine: &dyn target::TargetEngine,
    ) -> Result<String, PolygraphError> {
        let ast = parser::parse_cypher(cypher)?;
        let sparql = translator::cypher::translate(&ast, engine.base_iri(), engine.supports_rdf_star())?;
        engine.finalize(sparql)
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
