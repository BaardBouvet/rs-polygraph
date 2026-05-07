//! Core Cypher value types for result mapping.
//!
//! These types mirror the openCypher type system and are produced by
//! [`super::map_results`] when converting SPARQL bindings back to
//! Cypher-shaped rows.

use std::collections::BTreeMap;

/// A single Cypher result value.
#[derive(Debug, Clone, PartialEq)]
pub enum CypherValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
    List(Vec<CypherValue>),
    Map(BTreeMap<String, CypherValue>),
    Node(CypherNode),
    Relationship(CypherRelationship),
}

/// A property-graph node reconstructed from RDF triples.
#[derive(Debug, Clone, PartialEq)]
pub struct CypherNode {
    /// The IRI identity of this node in the triplestore.
    pub id: String,
    /// Labels (from `rdf:type` triples, with base IRI stripped).
    pub labels: Vec<String>,
    /// Properties (all datatype-valued predicates, base IRI stripped).
    pub properties: BTreeMap<String, CypherValue>,
}

/// A property-graph relationship reconstructed from RDF triples.
#[derive(Debug, Clone, PartialEq)]
pub struct CypherRelationship {
    /// Opaque identity (IRI for reification, or synthesized for RDF-star).
    pub id: String,
    /// Relationship type name (IRI local name).
    pub rel_type: String,
    /// Start node IRI.
    pub start_node: String,
    /// End node IRI.
    pub end_node: String,
    /// Properties (from annotated triples / reification properties).
    pub properties: BTreeMap<String, CypherValue>,
}

/// One result row, with columns named by RETURN aliases.
pub type CypherRow = BTreeMap<String, CypherValue>;

/// A single SPARQL result binding.
///
/// Maps variable names to optional RDF terms. The caller converts from
/// their engine's native type (e.g., `oxigraph::QuerySolution`).
#[derive(Debug, Clone)]
pub struct SparqlSolution {
    pub bindings: BTreeMap<String, Option<RdfTerm>>,
}

/// An RDF term as returned by a SPARQL engine.
#[derive(Debug, Clone, PartialEq)]
pub enum RdfTerm {
    Iri(String),
    Literal {
        value: String,
        datatype: Option<String>,
        language: Option<String>,
    },
    BlankNode(String),
}
