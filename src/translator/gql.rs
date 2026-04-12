/// ISO GQL → SPARQL algebra translator (Phase 5).
///
/// Since `GqlQuery` stores clauses as equivalent Cypher clause variants
/// (see [`crate::ast::gql::GqlQuery`]), translation is a straight delegation
/// to the Cypher translator: no duplicate mapping logic is needed.
use crate::ast::cypher::CypherQuery;
use crate::ast::gql::GqlQuery;
use crate::error::PolygraphError;

/// Translate an ISO GQL [`GqlQuery`] into a SPARQL 1.1 query string.
///
/// Delegates to [`crate::translator::cypher::translate`] after wrapping the
/// GQL clause list in a [`CypherQuery`], which is valid because the parser
/// has already lowered all GQL-specific constructs (IS labels, FILTER, NEXT)
/// to their Cypher equivalents.
pub fn translate(
    query: &GqlQuery,
    base_iri: Option<&str>,
    rdf_star: bool,
) -> Result<String, PolygraphError> {
    let cypher_query = CypherQuery { clauses: query.clauses.clone() };
    crate::translator::cypher::translate(&cypher_query, base_iri, rdf_star)
}
