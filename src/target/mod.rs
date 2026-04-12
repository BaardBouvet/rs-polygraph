use crate::error::PolygraphError;

/// Describes the SPARQL capabilities of a target engine.
///
/// Implementors are used by the translator to select the correct encoding
/// strategy (e.g., RDF-star vs. reification) and to apply engine-specific
/// post-processing.
pub trait TargetEngine {
    /// Returns `true` if the engine supports SPARQL-star / RDF-star syntax.
    fn supports_rdf_star(&self) -> bool;

    /// Returns `true` if the engine supports SPARQL 1.1 federation (`SERVICE`).
    fn supports_federation(&self) -> bool;

    /// Apply engine-specific finalization to a serialized SPARQL query string.
    ///
    /// The default implementation is a no-op. Override to add engine-specific
    /// optimizations or query rewrites.
    fn finalize(&self, query: String) -> Result<String, PolygraphError> {
        Ok(query)
    }
}

/// A generic SPARQL 1.1 engine with no RDF-star support (reification fallback).
pub struct GenericSparql11;

impl TargetEngine for GenericSparql11 {
    fn supports_rdf_star(&self) -> bool {
        false
    }

    fn supports_federation(&self) -> bool {
        false
    }
}
