use crate::ast::gql::GqlQuery;
use crate::error::PolygraphError;

/// Parse an ISO GQL query string — stub for Phase 5.
pub fn parse(_input: &str) -> Result<GqlQuery, PolygraphError> {
    Err(PolygraphError::UnsupportedFeature {
        feature: "ISO GQL parsing (planned for Phase 5)".to_string(),
    })
}
