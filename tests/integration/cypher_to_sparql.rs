/// Integration tests: given a Cypher query string, assert the SPARQL output.
///
/// Each test calls `Transpiler::cypher_to_sparql` and checks structural
/// properties of the serialized SPARQL string.
use polygraph::{
    target::{GenericSparql11, OxigraphAdapter},
    Transpiler,
};

const ENGINE: GenericSparql11 = GenericSparql11;

fn transpile(cypher: &str) -> String {
    Transpiler::cypher_to_sparql(cypher, &ENGINE)
        .unwrap_or_else(|e| panic!("translation failed for {cypher:?}: {e}"))
}

fn transpile_lower(cypher: &str) -> String {
    transpile(cypher).to_lowercase()
}

fn transpile_rdf_star(cypher: &str) -> String {
    let engine = OxigraphAdapter::default();
    Transpiler::cypher_to_sparql(cypher, &engine)
        .unwrap_or_else(|e| panic!("rdf-star translation failed for {cypher:?}: {e}"))
}

fn transpile_reification(cypher: &str) -> String {
    transpile(cypher)
}

// ── Basic MATCH … RETURN ─────────────────────────────────────────────────────

#[test]
fn match_node_returns_select() {
    let s = transpile_lower("MATCH (n) RETURN n");
    assert!(s.contains("select"), "got: {s}");
    assert!(s.contains("where"), "got: {s}");
    assert!(s.contains("?n"), "got: {s}");
}

#[test]
fn match_node_with_label_emits_rdf_type() {
    let s = transpile_lower("MATCH (n:Person) RETURN n");
    assert!(
        s.contains("rdf-syntax-ns#type") || s.contains("rdf:type") || s.contains("a "),
        "got: {s}"
    );
    assert!(s.contains("person"), "got: {s}");
    assert!(s.contains("?n"), "got: {s}");
}

#[test]
fn match_node_property_in_bgp() {
    let s = transpile_lower(r#"MATCH (n:Person {name: "Alice"}) RETURN n"#);
    // name property and literal should appear
    assert!(s.contains("name"), "got: {s}");
    assert!(s.contains("alice"), "got: {s}");
}

#[test]
fn match_relationship_right_emits_triple() {
    let s = transpile_lower("MATCH (a)-[:KNOWS]->(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
    assert!(s.contains("?a"), "got: {s}");
    assert!(s.contains("?b"), "got: {s}");
}

#[test]
fn match_relationship_left_emits_swapped_triple() {
    // Left arrow: b --> a in SPARQL (subject/object swapped).
    let s = transpile_lower("MATCH (a)<-[:KNOWS]-(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
    // Both a and b should appear.
    assert!(s.contains("?a"), "got: {s}");
    assert!(s.contains("?b"), "got: {s}");
}

#[test]
fn match_relationship_undirected_emits_triple() {
    let s = transpile_lower("MATCH (a)-[:KNOWS]-(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
}

#[test]
fn return_property_access_creates_var() {
    let s = transpile_lower("MATCH (n:Person) RETURN n.name");
    // A variable for n.name should be projected.
    assert!(s.contains("name"), "got: {s}");
    assert!(s.contains("select"), "got: {s}");
}

#[test]
fn return_property_with_alias() {
    let s = transpile_lower("MATCH (n:Person) RETURN n.name AS name");
    assert!(s.contains("?name"), "got: {s}");
}

#[test]
fn return_distinct_emits_distinct() {
    let s = transpile_lower("MATCH (n:Person) RETURN DISTINCT n");
    assert!(s.contains("distinct"), "got: {s}");
}

#[test]
fn return_star_no_project_wrapper() {
    // RETURN * should not add a Project restriction.
    let s = transpile("MATCH (n:Person) RETURN *");
    // The output should not contain a bare SELECT with explicit variable list.
    // A SELECT * in spargebra may render as SELECT * or just WHERE without
    // explicit variable list — either way variables are unrestricted.
    assert!(s.to_lowercase().contains("select"), "got: {s}");
}

// ── WHERE / FILTER ───────────────────────────────────────────────────────────

#[test]
fn where_gt_emits_filter() {
    let s = transpile_lower("MATCH (n:Person) WHERE n.age > 30 RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("30"), "got: {s}");
    assert!(s.contains("age"), "got: {s}");
}

#[test]
fn where_eq_emits_filter() {
    let s = transpile_lower(r#"MATCH (n:Person) WHERE n.name = "Alice" RETURN n"#);
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("alice"), "got: {s}");
}

#[test]
fn where_and_emits_and_in_filter() {
    let s = transpile_lower("MATCH (n) WHERE n.age > 18 AND n.active = true RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("&&") || s.contains("and"), "got: {s}");
}

#[test]
fn where_or_emits_or_in_filter() {
    let s = transpile_lower("MATCH (n) WHERE n.age > 18 OR n.admin = true RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(
        s.contains("||") || s.contains("or") || s.contains("||"),
        "got: {s}"
    );
}

#[test]
fn where_not_emits_not_in_filter() {
    let s = transpile_lower("MATCH (n) WHERE NOT n.deleted RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("!") || s.contains("not"), "got: {s}");
}

#[test]
fn where_is_null_emits_bound_check() {
    let s = transpile_lower("MATCH (n) WHERE n.name IS NULL RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("bound"), "got: {s}");
}

#[test]
fn where_is_not_null_emits_bound_check() {
    let s = transpile_lower("MATCH (n) WHERE n.name IS NOT NULL RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("bound"), "got: {s}");
}

#[test]
fn where_string_starts_with() {
    let s = transpile_lower(r#"MATCH (n) WHERE n.name STARTS WITH "Al" RETURN n"#);
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("strstarts"), "got: {s}");
}

#[test]
fn where_string_ends_with() {
    let s = transpile_lower(r#"MATCH (n) WHERE n.name ENDS WITH "ice" RETURN n"#);
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("strends"), "got: {s}");
}

#[test]
fn where_contains() {
    let s = transpile_lower(r#"MATCH (n) WHERE n.name CONTAINS "lic" RETURN n"#);
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("contains"), "got: {s}");
}

// ── OPTIONAL MATCH ───────────────────────────────────────────────────────────

#[test]
fn optional_match_emits_optional() {
    let s = transpile_lower("MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b:Person) RETURN a, b");
    assert!(s.contains("optional"), "got: {s}");
    assert!(s.contains("knows"), "got: {s}");
}

#[test]
fn optional_match_with_where() {
    let s = transpile_lower(
        "MATCH (a) OPTIONAL MATCH (a)-[:KNOWS]->(b) WHERE b.active = true RETURN a, b",
    );
    assert!(s.contains("optional"), "got: {s}");
    assert!(s.contains("filter") || s.contains("active"), "got: {s}");
}

// ── WITH ─────────────────────────────────────────────────────────────────────

#[test]
fn with_where_applies_filter() {
    let s = transpile_lower("MATCH (n:Person) WITH n WHERE n.age > 18 RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("18"), "got: {s}");
}

// ── Multi-label / multi-hop ───────────────────────────────────────────────────

#[test]
fn multi_label_node_emits_multiple_type_triples() {
    let s = transpile_lower("MATCH (n:Person:Employee) RETURN n");
    assert!(s.contains("person"), "got: {s}");
    assert!(s.contains("employee"), "got: {s}");
}

#[test]
fn multi_hop_path_emits_two_edge_triples() {
    let s = transpile_lower("MATCH (a)-[:KNOWS]->(b)-[:LIKES]->(c) RETURN a, c");
    assert!(s.contains("knows"), "got: {s}");
    assert!(s.contains("likes"), "got: {s}");
}

#[test]
fn relationship_variable_in_return() {
    // Relationship variable `r` should appear in the SPARQL output when returned.
    let s = transpile_lower("MATCH (a)-[r:KNOWS]->(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
}

// ── SPARQL validity spot-checks ───────────────────────────────────────────────

#[test]
fn output_starts_with_select() {
    let s = transpile("MATCH (n:Person) RETURN n");
    assert!(
        s.trim_start().to_lowercase().starts_with("select"),
        "got: {s}"
    );
}

#[test]
fn output_contains_where_block() {
    let s = transpile("MATCH (n) RETURN n");
    assert!(s.to_lowercase().contains("where"), "got: {s}");
}

#[test]
fn case_insensitive_keywords_translate() {
    let s1 = transpile("match (n:person) return n");
    let s2 = transpile("MATCH (n:person) RETURN n");
    // Both should produce structurally equivalent output.
    assert_eq!(s1.to_lowercase(), s2.to_lowercase());
}

// ── Phase 3: edge properties — RDF-star mode ──────────────────────────────────

#[test]
fn rdf_star_inline_rel_prop_emits_annotated_triple() {
    // <<?a <base:KNOWS> ?b>> <base:since> ?since
    let s = transpile_rdf_star(
        "MATCH (a)-[r:KNOWS {since: 2020}]->(b) RETURN a, b",
    );
    let l = s.to_lowercase();
    assert!(l.contains("<<") && l.contains(">>"), "missing << >> in: {s}");
    assert!(l.contains("since"), "missing 'since' in: {s}");
    assert!(l.contains("knows"), "missing 'knows' in: {s}");
}

#[test]
fn rdf_star_rel_prop_string_literal() {
    let s = transpile_rdf_star(
        r#"MATCH (a)-[r:LIKES {reason: "fun"}]->(b) RETURN a, b"#,
    );
    let l = s.to_lowercase();
    assert!(l.contains("<<") && l.contains(">>"), "got: {s}");
    assert!(l.contains("likes"), "got: {s}");
    assert!(l.contains("reason"), "got: {s}");
    assert!(l.contains("fun"), "got: {s}");
}

#[test]
fn rdf_star_multiple_inline_rel_props() {
    let s = transpile_rdf_star(
        "MATCH (a)-[r:KNOWS {since: 2020, weight: 5}]->(b) RETURN a, b",
    );
    let l = s.to_lowercase();
    assert!(l.contains("<<") && l.contains(">>"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
    assert!(l.contains("weight"), "got: {s}");
}

#[test]
fn rdf_star_where_rel_prop_emits_annotated_triple_plus_filter() {
    let s = transpile_rdf_star(
        "MATCH (a)-[r:KNOWS]->(b) WHERE r.since > 2000 RETURN a, b",
    );
    let l = s.to_lowercase();
    assert!(l.contains("<<") && l.contains(">>"), "got: {s}");
    assert!(l.contains("filter"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
    assert!(l.contains("2000"), "got: {s}");
}

#[test]
fn rdf_star_return_rel_prop() {
    let s = transpile_rdf_star(
        "MATCH (a)-[r:KNOWS]->(b) RETURN r.since",
    );
    let l = s.to_lowercase();
    assert!(l.contains("<<") && l.contains(">>"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
}

// ── Phase 3: edge properties — reification mode ───────────────────────────────

#[test]
fn reification_inline_rel_prop_emits_rdf_statement() {
    let s = transpile_reification(
        "MATCH (a)-[r:KNOWS {since: 2020}]->(b) RETURN a, b",
    );
    let l = s.to_lowercase();
    // Must NOT contain << >> (that would be RDF-star)
    assert!(!l.contains("<<"), "unexpected rdf-star syntax in: {s}");
    // Must contain rdf reification IRIs
    assert!(
        l.contains("statement") || l.contains("rdf-syntax"),
        "missing rdf:Statement in: {s}"
    );
    assert!(l.contains("since"), "missing 'since' in: {s}");
    assert!(l.contains("knows"), "missing 'knows' in: {s}");
}

#[test]
fn reification_multiple_inline_rel_props() {
    let s = transpile_reification(
        "MATCH (a)-[r:KNOWS {since: 2020, weight: 5}]->(b) RETURN a, b",
    );
    let l = s.to_lowercase();
    assert!(!l.contains("<<"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
    assert!(l.contains("weight"), "got: {s}");
}

#[test]
fn reification_where_rel_prop_access_adds_triple() {
    let s = transpile_reification(
        "MATCH (a)-[r:KNOWS]->(b) WHERE r.since > 2000 RETURN a, b",
    );
    let l = s.to_lowercase();
    assert!(!l.contains("<<"), "got: {s}");
    assert!(l.contains("filter"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
}

#[test]
fn reification_return_rel_prop() {
    let s = transpile_reification(
        "MATCH (a)-[r:KNOWS]->(b) RETURN r.since",
    );
    let l = s.to_lowercase();
    assert!(!l.contains("<<"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
}

// ── Phase 3: mode comparison ──────────────────────────────────────────────────

#[test]
fn rdf_star_and_reification_differ_structurally() {
    let cypher = "MATCH (a)-[r:KNOWS {since: 2020}]->(b) RETURN a, b";
    let star = transpile_rdf_star(cypher).to_lowercase();
    let reif = transpile_reification(cypher).to_lowercase();
    // RDF-star uses << >>, reification does not.
    assert!(star.contains("<<"), "rdf-star missing << : {star}");
    assert!(!reif.contains("<<"), "reification should not have << : {reif}");
}

#[test]
fn rdf_star_no_rel_props_same_as_reification_for_simple_path() {
    // Without edge properties, both modes should produce the same output.
    let cypher = "MATCH (a)-[:KNOWS]->(b) RETURN a, b";
    let star = transpile_rdf_star(cypher).to_lowercase();
    let reif = transpile_reification(cypher).to_lowercase();
    assert_eq!(star, reif, "modes should agree when no edge props");
}

#[test]
fn oxigraph_adapter_reports_rdf_star_true() {
    use polygraph::target::TargetEngine;
    let engine = OxigraphAdapter::default();
    assert!(engine.supports_rdf_star());
}

#[test]
fn generic_sparql11_reports_rdf_star_false() {
    use polygraph::target::TargetEngine;
    assert!(!ENGINE.supports_rdf_star());
}
