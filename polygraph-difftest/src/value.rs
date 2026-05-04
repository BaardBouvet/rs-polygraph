//! Cypher / SPARQL value model used by the harness.
//!
//! The model is intentionally narrower than the full openCypher type system:
//! the differential harness only needs the value shapes that can round-trip
//! through both Neo4j and Oxigraph and be compared under bag-equality with
//! Cypher's null-propagating equality semantics.

use serde::{de, Deserialize, Serialize};

/// Sentinel string in TOML expected-rows that represents a Cypher `null`.
/// TOML has no null literal, so curated test files use `"__null__"` instead.
pub const NULL_SENTINEL: &str = "__null__";

/// A Cypher value as observed at the result-row level.
#[derive(Clone, Debug, PartialEq, Serialize)]
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

/// Custom deserializer: identical to a `#[serde(untagged)]` derive, except
/// that the string `"__null__"` is decoded as `Value::Null` so that TOML
/// expected-row arrays can express null results without a native null literal.
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ValueVisitor;
        impl<'de> de::Visitor<'de> for ValueVisitor {
            type Value = Value;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(
                    f,
                    "a Cypher value (bool, int, float, string, list, or \"__null__\")"
                )
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
                Ok(Value::Int(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
                Ok(Value::Int(v as i64))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
                Ok(Value::Float(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
                if v == NULL_SENTINEL {
                    Ok(Value::Null)
                } else {
                    Ok(Value::String(v.to_owned()))
                }
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
                self.visit_str(&v)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
                let mut items = Vec::new();
                while let Some(v) = seq.next_element::<Value>()? {
                    items.push(v);
                }
                Ok(Value::List(items))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_none<E: de::Error>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_some<D2: de::Deserializer<'de>>(self, d: D2) -> Result<Value, D2::Error> {
                Deserialize::deserialize(d)
            }
        }
        d.deserialize_any(ValueVisitor)
    }
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
                    && a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| x.cypher_structural_eq(y))
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
