//! Projection schema: describes the shape of each RETURN column.
//!
//! The translator populates a [`ProjectionSchema`] during SPARQL generation.
//! The result mapper uses it to interpret SPARQL bindings back into
//! Cypher-shaped values.

/// Describes how a SPARQL variable maps back to a Cypher value.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnKind {
    /// A scalar value: property access, literal, aggregate, or expression.
    /// The SPARQL variable directly holds the value.
    Scalar {
        /// The SPARQL variable name holding this value.
        var: String,
    },

    /// A node variable. The SPARQL variable holds the node IRI; labels and
    /// properties need hydration from additional variables (Phase R3+).
    Node {
        /// SPARQL variable holding the node IRI.
        iri_var: String,
    },

    /// A relationship variable. Encoding depends on RDF-star vs reification.
    Relationship {
        /// Source node SPARQL variable.
        src_var: String,
        /// Destination node SPARQL variable.
        dst_var: String,
        /// Relationship type SPARQL variable or fixed IRI.
        type_info: String,
    },
}

/// A single projected output column.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedColumn {
    /// The output column name (alias if provided, otherwise variable name).
    pub name: String,
    /// What kind of value this column holds.
    pub kind: ColumnKind,
}

/// Schema describing the projected columns of a transpiled query.
///
/// Built by the translator alongside the SPARQL string. Used by
/// [`super::map_results`] to interpret SPARQL bindings.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionSchema {
    /// Ordered list of output columns matching the Cypher RETURN clause.
    pub columns: Vec<ProjectedColumn>,
    /// Whether the original query used RETURN DISTINCT.
    pub distinct: bool,
    /// The base IRI used during translation (needed to strip prefixes).
    pub base_iri: String,
    /// Whether RDF-star encoding was used.
    pub rdf_star: bool,
}
