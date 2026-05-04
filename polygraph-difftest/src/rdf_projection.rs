//! Project a [`PropertyGraph`] into RDF triples loadable by Oxigraph.
//!
//! The encoding mirrors what the TCK harness uses (see `tests/tck/main.rs`):
//!
//! * Each node becomes an IRI under the harness base IRI.
//! * Each node gets a self-sentinel: `<node> <base:__node> <base:__node>` so
//!   that `MATCH (n)` / `OPTIONAL MATCH` patterns (which require this triple
//!   in the generated SPARQL) can bind `?n`.
//! * Each label produces `?n rdf:type <base:Label>`.
//! * Each property produces `?n <base:propname> "value"^^xsd:typed`.
//! * Each edge produces `?from <base:RELTYPE> ?to`.
//! * Each edge property produces an RDF-star annotated triple:
//!   `<< ?from <base:RELTYPE> ?to >> <base:key> "value"^^xsd:typed`
//!   matching the TCK harness RDF-star encoding.

use crate::fixture::PropertyGraph;
use crate::value::Value;

pub const DEFAULT_BASE: &str = "http://difftest.example.org/";

/// Render the fixture as a SPARQL `INSERT DATA { … }` string against `base`.
pub fn to_insert_data(graph: &PropertyGraph, base: &str) -> String {
    let mut triples: Vec<String> = Vec::new();
    for n in &graph.nodes {
        let s = format!("<{base}{}>", n.id);
        // Universal node-existence sentinel — mirrors TCK harness emit_create_pattern_with_bindings.
        // The translator emits `?n <base:__node> <base:__node>` in every MATCH pattern; without
        // this triple in the store, MATCH (n) returns no rows at all.
        triples.push(format!("{s} <{base}__node> <{base}__node> ."));
        for l in &n.labels {
            triples.push(format!(
                "{s} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{base}{l}> ."
            ));
        }
        for (k, v) in &n.properties {
            triples.push(format!("{s} <{base}{k}> {} .", value_to_rdf_object(v)));
        }
    }
    for e in &graph.edges {
        let s = format!("<{base}{}>", e.from);
        let o = format!("<{base}{}>", e.to);
        triples.push(format!("{s} <{base}{}> {o} .", e.rel_type));
        // RDF-star reification for edge properties — mirrors TCK harness encoding.
        // The translator emits `<< ?s <base:REL> ?o >> <base:key> ?val` for edge properties.
        for (k, v) in &e.properties {
            triples.push(format!(
                "<< {s} <{base}{}> {o} >> <{base}{k}> {} .",
                e.rel_type,
                value_to_rdf_object(v)
            ));
        }
    }
    if triples.is_empty() {
        "INSERT DATA {}".to_owned()
    } else {
        format!("INSERT DATA {{\n  {}\n}}", triples.join("\n  "))
    }
}

fn value_to_rdf_object(v: &Value) -> String {
    match v {
        Value::Null => "\"\"".into(), // shouldn't be a property value, but be permissive
        Value::Bool(b) => format!("\"{b}\"^^<http://www.w3.org/2001/XMLSchema#boolean>"),
        Value::Int(i) => format!("\"{i}\"^^<http://www.w3.org/2001/XMLSchema#integer>"),
        Value::Float(f) => format!("\"{f}\"^^<http://www.w3.org/2001/XMLSchema#double>"),
        Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Value::List(_) | Value::Node(_) | Value::Rel(_) => {
            panic!("RDF projection does not support {v:?} as a property value")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::NodeSpec;
    use std::collections::BTreeMap;

    #[test]
    fn empty_graph_yields_empty_insert() {
        let g = PropertyGraph::default();
        assert_eq!(to_insert_data(&g, DEFAULT_BASE), "INSERT DATA {}");
    }

    #[test]
    fn node_with_label_and_int_property() {
        let g = PropertyGraph {
            nodes: vec![NodeSpec {
                id: "n1".into(),
                labels: vec!["Person".into()],
                properties: BTreeMap::from([("age".into(), Value::Int(42))]),
            }],
            edges: vec![],
        };
        let s = to_insert_data(&g, DEFAULT_BASE);
        assert!(s.contains("rdf-syntax-ns#type> <http://difftest.example.org/Person>"));
        assert!(s.contains("difftest.example.org/age> \"42\"^^"));
        // Sentinel triple must be present so MATCH (n) patterns bind the node.
        assert!(s.contains("difftest.example.org/__node> <http://difftest.example.org/__node>"));
    }
}
