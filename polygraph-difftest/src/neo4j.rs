//! Live Neo4j HTTP driver (gated behind the `live-neo4j` Cargo feature).
//!
//! Configuration via env vars:
//!
//! * `NEO4J_URL` — base URL, e.g. `http://localhost:7474`.
//! * `NEO4J_USER`, `NEO4J_PASSWORD` — basic-auth credentials.
//! * `NEO4J_DATABASE` — optional, defaults to `neo4j`.
//!
//! The driver is intentionally minimal: it speaks the HTTP-API transactional
//! endpoint via JSON, no Bolt. This keeps the dependency footprint to `ureq`
//! and leaves higher-throughput integration to a later phase.

use serde::Deserialize;

use crate::fixture::PropertyGraph;
use crate::value::Value;

#[derive(Clone, Debug)]
pub struct Neo4jConfig {
    pub url: String,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl Neo4jConfig {
    /// Build from `NEO4J_*` environment variables. Returns `None` if any
    /// required variable is missing.
    pub fn from_env() -> Option<Self> {
        Some(Self {
            url: std::env::var("NEO4J_URL").ok()?,
            user: std::env::var("NEO4J_USER").ok()?,
            password: std::env::var("NEO4J_PASSWORD").ok()?,
            database: std::env::var("NEO4J_DATABASE").unwrap_or_else(|_| "neo4j".into()),
        })
    }
}

/// Resulting bag from a Neo4j query.
#[derive(Clone, Debug)]
pub struct Neo4jResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

#[derive(Deserialize)]
struct Tx {
    results: Vec<TxResult>,
    errors: Vec<TxError>,
}

#[derive(Deserialize)]
struct TxResult {
    columns: Vec<String>,
    data: Vec<TxRow>,
}

#[derive(Deserialize)]
struct TxRow {
    row: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct TxError {
    code: String,
    message: String,
}

/// Reset the database, load `fixture`, run `cypher`, return the result bag.
///
/// "Reset" means `MATCH (n) DETACH DELETE n` — destructive. **Never point this
/// at a database whose contents you need to keep.**
pub fn run_against_neo4j(
    cfg: &Neo4jConfig,
    fixture: &PropertyGraph,
    cypher: &str,
) -> Result<Neo4jResult, String> {
    let endpoint = format!("{}/db/{}/tx/commit", cfg.url.trim_end_matches('/'), cfg.database);
    let auth = format!(
        "Basic {}",
        base64_encode(&format!("{}:{}", cfg.user, cfg.password))
    );

    // 1. Wipe.
    post_cypher(&endpoint, &auth, "MATCH (n) DETACH DELETE n")?;

    // 2. Load fixture (only if non-empty).
    let create = fixture.to_cypher_create();
    if !create.trim().eq_ignore_ascii_case("CREATE") {
        post_cypher(&endpoint, &auth, &create)?;
    }

    // 3. Run query.
    let tx = post_cypher(&endpoint, &auth, cypher)?;
    if !tx.errors.is_empty() {
        let msgs: Vec<String> = tx
            .errors
            .iter()
            .map(|e| format!("{}: {}", e.code, e.message))
            .collect();
        return Err(msgs.join("; "));
    }
    let result = tx
        .results
        .into_iter()
        .next()
        .ok_or_else(|| "neo4j: no results returned".to_owned())?;
    let rows: Vec<Vec<Value>> = result
        .data
        .into_iter()
        .map(|r| r.row.iter().map(json_to_value).collect())
        .collect();
    Ok(Neo4jResult {
        columns: result.columns,
        rows,
    })
}

fn post_cypher(endpoint: &str, auth: &str, cypher: &str) -> Result<Tx, String> {
    let body = serde_json::json!({
        "statements": [{ "statement": cypher }]
    });
    let resp = ureq::post(endpoint)
        .set("Authorization", auth)
        .set("Accept", "application/json;charset=UTF-8")
        .send_json(body)
        .map_err(|e| format!("neo4j HTTP: {e}"))?;
    let tx: Tx = resp.into_json().map_err(|e| format!("neo4j JSON: {e}"))?;
    Ok(tx)
}

fn json_to_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(items) => Value::List(items.iter().map(json_to_value).collect()),
        serde_json::Value::Object(_) => {
            // Node / relationship maps; we collapse to a stable string for now.
            Value::String(v.to_string())
        }
    }
}

// Tiny base64 encoder so we don't pull in another crate just for HTTP basic auth.
fn base64_encode(s: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = s.as_bytes();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_basic_auth_round_trip() {
        // RFC 4648 vector: "user:pass" -> "dXNlcjpwYXNz"
        assert_eq!(base64_encode("user:pass"), "dXNlcjpwYXNz");
    }
}
