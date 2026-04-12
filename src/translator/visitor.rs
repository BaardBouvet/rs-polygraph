use crate::ast::cypher::{
    MatchClause, NodePattern, RelationshipPattern, ReturnClause, WhereClause,
};
use crate::error::PolygraphError;

/// Visitor trait for walking an openCypher AST.
///
/// Implementors map each AST node type to a `Result<Output, Error>`. The
/// concrete translator in Phase 2 will implement this trait to emit
/// `spargebra` algebra. Only the nodes relevant to Phase 1's covered subset
/// are required; additional visitor methods will be added in later phases.
pub trait AstVisitor {
    type Output;
    type Error: From<PolygraphError>;

    fn visit_match(&mut self, clause: &MatchClause) -> Result<Self::Output, Self::Error>;
    fn visit_return(&mut self, clause: &ReturnClause) -> Result<Self::Output, Self::Error>;
    fn visit_where(&mut self, clause: &WhereClause) -> Result<Self::Output, Self::Error>;
    fn visit_node_pattern(&mut self, node: &NodePattern) -> Result<Self::Output, Self::Error>;
    fn visit_relationship_pattern(
        &mut self,
        rel: &RelationshipPattern,
    ) -> Result<Self::Output, Self::Error>;
}
