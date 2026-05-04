//! Property-graph fixtures used by the differential harness.
//!
//! Fixtures are intentionally small and hand-authored: they describe a
//! property graph that is loadable both into Neo4j (via `CREATE` statements
//! generated from the same struct) and into Oxigraph (via the RDF projection
//! in [`crate::rdf_projection`]).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::value::Value;

/// A property-graph fixture.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PropertyGraph {
    pub nodes: Vec<NodeSpec>,
    pub edges: Vec<EdgeSpec>,
}

/// A single node in the fixture.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSpec {
    /// Stable identifier — used both as the Cypher variable when generating
    /// `CREATE` statements and as the local name in RDF IRIs.
    pub id: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, Value>,
}

/// A single relationship in the fixture.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EdgeSpec {
    pub id: String,
    pub from: String,
    pub to: String,
    pub rel_type: String,
    #[serde(default)]
    pub properties: BTreeMap<String, Value>,
}

impl PropertyGraph {
    /// Render the fixture as a Cypher `CREATE` statement that loads it into
    /// a fresh Neo4j database.
    pub fn to_cypher_create(&self) -> String {
        let mut out = String::new();
        out.push_str("CREATE\n");
        let mut first = true;
        for n in &self.nodes {
            if !first {
                out.push_str(",\n");
            }
            first = false;
            let labels = if n.labels.is_empty() {
                String::new()
            } else {
                let parts: Vec<String> = n.labels.iter().map(|l| format!(":{l}")).collect();
                parts.join("")
            };
            out.push_str(&format!("  ({}{}", n.id, labels));
            if !n.properties.is_empty() {
                out.push_str(" {");
                let mut pfirst = true;
                for (k, v) in &n.properties {
                    if !pfirst {
                        out.push_str(", ");
                    }
                    pfirst = false;
                    out.push_str(&format!("{k}: {}", v.to_cypher_literal()));
                }
                out.push('}');
            }
            out.push(')');
        }
        for e in &self.edges {
            out.push_str(",\n");
            out.push_str(&format!(
                "  ({})-[{}:{}",
                e.from, e.id, e.rel_type
            ));
            if !e.properties.is_empty() {
                out.push_str(" {");
                let mut pfirst = true;
                for (k, v) in &e.properties {
                    if !pfirst {
                        out.push_str(", ");
                    }
                    pfirst = false;
                    out.push_str(&format!("{k}: {}", v.to_cypher_literal()));
                }
                out.push('}');
            }
            out.push_str(&format!("]->({})", e.to));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cypher_create_has_labels_and_props() {
        let g = PropertyGraph {
            nodes: vec![NodeSpec {
                id: "a".into(),
                labels: vec!["Person".into()],
                properties: BTreeMap::from([("name".into(), Value::String("Alice".into()))]),
            }],
            edges: vec![],
        };
        let s = g.to_cypher_create();
        assert!(s.contains("(a:Person {name: 'Alice'})"), "got: {s}");
    }
}
