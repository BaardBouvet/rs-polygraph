/// Integration tests for the result mapping pipeline.
///
/// Tests the full flow: Cypher → SPARQL + ProjectionSchema → map SPARQL
/// results back to CypherValues.
use polygraph::{
    result_mapping::{ColumnKind, CypherValue, ProjectedColumn, RdfTerm, SparqlSolution},
    target::GenericSparql11,
    Transpiler,
};
use std::collections::BTreeMap;

const ENGINE: GenericSparql11 = GenericSparql11;

fn solution(bindings: Vec<(&str, Option<RdfTerm>)>) -> SparqlSolution {
    SparqlSolution {
        bindings: bindings
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

fn lit_int(val: &str) -> Option<RdfTerm> {
    Some(RdfTerm::Literal {
        value: val.into(),
        datatype: Some("http://www.w3.org/2001/XMLSchema#integer".into()),
        language: None,
    })
}

fn lit_str(val: &str) -> Option<RdfTerm> {
    Some(RdfTerm::Literal {
        value: val.into(),
        datatype: None,
        language: None,
    })
}

// ── Schema generation tests ──────────────────────────────────────────────────

#[test]
fn schema_scalar_property_access() {
    let output = Transpiler::cypher_to_sparql("MATCH (n:Person) RETURN n.name", &ENGINE).unwrap();

    assert_eq!(output.schema.columns.len(), 1);
    let col = &output.schema.columns[0];
    // n.name should be classified as Scalar (it's a property access, not a node).
    assert!(matches!(col.kind, ColumnKind::Scalar { .. }));
}

#[test]
fn schema_node_variable() {
    let output = Transpiler::cypher_to_sparql("MATCH (n:Person) RETURN n", &ENGINE).unwrap();

    assert_eq!(output.schema.columns.len(), 1);
    let col = &output.schema.columns[0];
    assert_eq!(col.name, "n");
    assert!(matches!(col.kind, ColumnKind::Node { .. }));
    if let ColumnKind::Node { iri_var } = &col.kind {
        assert_eq!(iri_var, "n");
    }
}

#[test]
fn schema_aliased_property() {
    let output =
        Transpiler::cypher_to_sparql("MATCH (n:Person) RETURN n.name AS fullName", &ENGINE)
            .unwrap();

    assert_eq!(output.schema.columns.len(), 1);
    let col = &output.schema.columns[0];
    assert_eq!(col.name, "fullName");
    assert!(matches!(col.kind, ColumnKind::Scalar { .. }));
}

#[test]
fn schema_distinct_flag() {
    let output = Transpiler::cypher_to_sparql("MATCH (n) RETURN DISTINCT n.name", &ENGINE).unwrap();
    assert!(output.schema.distinct);
}

#[test]
fn schema_multiple_columns() {
    let output = Transpiler::cypher_to_sparql(
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b, a.name AS name",
        &ENGINE,
    )
    .unwrap();

    assert_eq!(output.schema.columns.len(), 3);
    assert_eq!(output.schema.columns[0].name, "a");
    assert!(matches!(
        output.schema.columns[0].kind,
        ColumnKind::Node { .. }
    ));
    assert_eq!(output.schema.columns[1].name, "b");
    assert!(matches!(
        output.schema.columns[1].kind,
        ColumnKind::Node { .. }
    ));
    assert_eq!(output.schema.columns[2].name, "name");
    assert!(matches!(
        output.schema.columns[2].kind,
        ColumnKind::Scalar { .. }
    ));
}

#[test]
fn schema_aggregate_is_scalar() {
    let output = Transpiler::cypher_to_sparql("MATCH (n) RETURN count(n) AS cnt", &ENGINE).unwrap();

    assert_eq!(output.schema.columns.len(), 1);
    assert_eq!(output.schema.columns[0].name, "cnt");
    assert!(matches!(
        output.schema.columns[0].kind,
        ColumnKind::Scalar { .. }
    ));
}

// ── End-to-end scalar result mapping tests ───────────────────────────────────

#[test]
fn map_scalar_integer_results() {
    let output = Transpiler::cypher_to_sparql("MATCH (n) RETURN count(n) AS cnt", &ENGINE).unwrap();

    // Simulate SPARQL engine returning one row with count=5.
    // We need to know the SPARQL variable name for the count column.
    if let ColumnKind::Scalar { var } = &output.schema.columns[0].kind {
        let solutions = vec![solution(vec![(var.as_str(), lit_int("5"))])];
        let rows = output.map_results(&solutions).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["cnt"], CypherValue::Integer(5));
    } else {
        panic!("expected scalar column");
    }
}

#[test]
fn map_null_for_unbound() {
    let output = Transpiler::cypher_to_sparql("MATCH (n) RETURN n.name AS name", &ENGINE).unwrap();

    if let ColumnKind::Scalar { var } = &output.schema.columns[0].kind {
        let solutions = vec![solution(vec![(var.as_str(), None)])];
        let rows = output.map_results(&solutions).unwrap();
        assert_eq!(rows[0]["name"], CypherValue::Null);
    } else {
        panic!("expected scalar column");
    }
}

#[test]
fn map_empty_results() {
    let output =
        Transpiler::cypher_to_sparql("MATCH (n:Person) RETURN n.name AS name", &ENGINE).unwrap();

    let rows = output.map_results(&[]).unwrap();
    assert!(rows.is_empty());
}

#[test]
fn output_contains_sparql_and_schema() {
    let output =
        Transpiler::cypher_to_sparql("MATCH (n:Person) WHERE n.age > 30 RETURN n.name", &ENGINE)
            .unwrap();
    assert!(output.sparql.contains("SELECT"));
    assert_eq!(output.schema.columns.len(), 1);
}
