//! Result mapping: SPARQL bindings → Cypher-shaped rows.
//!
//! This module is the inverse of [`crate::rdf_mapping`]. The translator
//! produces a SPARQL query _and_ a [`ProjectionSchema`] describing the
//! shape of each RETURN column. After the caller executes the SPARQL
//! against their triplestore, [`map_results`] converts the raw bindings
//! back into Cypher values.

pub mod mapper;
pub mod schema;
pub mod types;
pub mod xsd;

pub use mapper::map_results;
pub use schema::{ColumnKind, ProjectedColumn, ProjectionSchema};
pub use types::{CypherNode, CypherRelationship, CypherRow, CypherValue, RdfTerm, SparqlSolution};

use crate::error::PolygraphError;

/// The output of a Cypher/GQL → SPARQL transpilation.
///
/// Contains both the SPARQL query string to execute and a projection schema
/// describing column types and SPARQL variable mappings. Use
/// [`map_results`] (or the convenience method [`TranspileOutput::map_results`])
/// to convert SPARQL bindings back into Cypher rows.
#[derive(Debug, Clone)]
pub struct TranspileOutput {
    /// The SPARQL query string to execute against the triplestore.
    pub sparql: String,

    /// Schema describing column types and SPARQL variable mappings.
    pub schema: ProjectionSchema,
}

impl TranspileOutput {
    /// Map SPARQL query results back into Cypher-shaped rows.
    ///
    /// The caller executes `self.sparql` against their triplestore and
    /// passes the raw bindings here.
    pub fn map_results(
        &self,
        solutions: &[SparqlSolution],
    ) -> Result<Vec<CypherRow>, PolygraphError> {
        mapper::map_results(solutions, &self.schema)
    }
}
