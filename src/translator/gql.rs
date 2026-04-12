/// ISO GQL → SPARQL algebra translator — stub for Phase 5.
use crate::error::PolygraphError;

/// Placeholder — implementation deferred to Phase 5.
pub struct GqlTranslator;

impl GqlTranslator {
    pub fn new() -> Self {
        Self
    }

    pub fn translate(&self, _query: &crate::ast::gql::GqlQuery) -> Result<(), PolygraphError> {
        Err(PolygraphError::UnsupportedFeature {
            feature: "GQL-to-SPARQL translation (planned for Phase 5)".to_string(),
        })
    }
}
