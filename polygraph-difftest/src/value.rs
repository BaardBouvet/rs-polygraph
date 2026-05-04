//! Cypher / SPARQL value model used by the harness.
//!
//! The model is intentionally narrower than the full openCypher type system:
//! the differential harness only needs the value shapes that can round-trip
//! through both Neo4j and Oxigraph and be compared under bag-equality with
//! Cypher's null-propagating equality semantics.

use serde::{Deserialize, Serialize};

/// A Cypher value as observed at the result-row level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    /// A node reference — opaque identifier, compared structurally if both sides
    /// are nodes from the same fixture.
    Node(String),
    /// A relationship reference, same treatment as `Node`.
    Rel(String),
    List(Vec<Value>),
}

impl Value {
    /// Render this value as a Cypher literal suitable for inclusion in a
    /// `CREATE` statement. Nodes / rels / nulls are not literal-renderable.
    pub fn to_cypher_literal(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => {
                if f.is_nan() {
                    "0.0/0.0".into()
                } else {
                    format!("{f}")
                }
            }
            Value::String(s) => format!("'{}'", s.replace('\'', "\\'")),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_cypher_literal()).collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Node(_) | Value::Rel(_) => {
                panic!("Node / Rel cannot be rendered as a Cypher literal")
            }
        }
    }

    /// Cypher equality: `null = null` is `null`, not `true`. For the oracle's
    /// purposes we require *structural* equality on non-null values, and treat
    /// `null = null` as match (so a curated expectation containing nulls works).
    pub fn cypher_structural_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Null, _) | (_, Value::Null) => false,
            (Value::Float(a), Value::Float(b)) => {
                if a.is_nan() && b.is_nan() {
                    true
                } else {
                    a == b
                }
            }
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
                (*a as f64) == *b
            }
            (Value::List(a), Value::List(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b.iter()).all(|(x, y)| x.cypher_structural_eq(y))
            }
            (a, b) => a == b,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_only_equals_null() {
        assert!(Value::Null.cypher_structural_eq(&Value::Null));
        assert!(!Value::Null.cypher_structural_eq(&Value::Int(0)));
    }

    #[test]
    fn int_float_cross_compare() {
        assert!(Value::Int(3).cypher_structural_eq(&Value::Float(3.0)));
    }

    #[test]
    fn nan_self_equality_for_oracle() {
        // We treat NaN==NaN as match so curated expectations are deterministic.
        assert!(Value::Float(f64::NAN).cypher_structural_eq(&Value::Float(f64::NAN)));
    }
}
