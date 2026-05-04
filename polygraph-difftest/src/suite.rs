//! Curated query specification — the on-disk schema for `queries/*.toml`.
//!
//! Each TOML file describes:
//!
//! * `name`, `description`, `spec_ref` — provenance and the openCypher 9 /
//!   GQL spec section the expected result is derived from.
//! * A property graph fixture (inline).
//! * A Cypher query.
//! * Expected result columns and rows, with order semantics.
//!
//! See `queries/match_basic.toml` for a worked example.

use serde::{Deserialize, Serialize};

use crate::fixture::PropertyGraph;
use crate::oracle::OrderMode as RuntimeOrderMode;
use crate::value::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuerySpec {
    pub name: String,
    pub description: String,
    /// Citation: spec section / TCK feature / etc. that justifies the
    /// expected-result encoding.
    pub spec_ref: String,
    pub fixture: PropertyGraph,
    pub cypher: String,
    pub expected: Expectation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Expectation {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    #[serde(default)]
    pub order: OrderMode,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderMode {
    #[default]
    Bag,
    Ordered,
}

impl From<OrderMode> for RuntimeOrderMode {
    fn from(o: OrderMode) -> Self {
        match o {
            OrderMode::Bag => RuntimeOrderMode::Bag,
            OrderMode::Ordered => RuntimeOrderMode::Ordered,
        }
    }
}

impl QuerySpec {
    pub fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}
