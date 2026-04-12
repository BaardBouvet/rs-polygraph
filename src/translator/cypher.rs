/// openCypher → SPARQL algebra translator — stub for Phase 2.
///
/// Will implement [`crate::translator::AstVisitor`] emitting `spargebra`
/// `GraphPattern` algebra.
use crate::error::PolygraphError;

/// Placeholder translator struct — implementation deferred to Phase 2.
pub struct CypherTranslator;

impl CypherTranslator {
    pub fn new() -> Self {
        Self
    }

    pub fn translate(&self, _query: &crate::ast::cypher::CypherQuery) -> Result<(), PolygraphError> {
        Err(PolygraphError::UnsupportedFeature {
            feature: "Cypher-to-SPARQL translation (planned for Phase 2)".to_string(),
        })
    }
}
