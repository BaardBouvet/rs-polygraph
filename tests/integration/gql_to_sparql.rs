/// Integration tests: given a GQL query string, assert SPARQL output.
///
/// Each test calls `Transpiler::gql_to_sparql` and checks structural
/// properties of the serialised SPARQL string.  Because GQL clauses are
/// lowered to Cypher equivalents during parsing, the expected SPARQL output
/// is identical to what the Cypher integration tests produce for equivalent
/// Cypher queries.
use polygraph::{
    target::{GenericSparql11, RdfStar},
    Transpiler,
};

const ENGINE: GenericSparql11 = GenericSparql11;

fn transpile(gql: &str) -> String {
    Transpiler::gql_to_sparql(gql, &ENGINE)
        .unwrap_or_else(|e| panic!("GQL translation failed for {gql:?}: {e}"))
}

fn transpile_lower(gql: &str) -> String {
    transpile(gql).to_lowercase()
}

fn transpile_rdf_star(gql: &str) -> String {
    let engine = RdfStar::default();
    Transpiler::gql_to_sparql(gql, &engine)
        .unwrap_or_else(|e| panic!("GQL rdf-star translation failed for {gql:?}: {e}"))
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
fn match_node_with_colon_label() {
    // Standard `:Label` notation is supported alongside GQL `IS Label`.
    let s = transpile_lower("MATCH (n:Person) RETURN n");
    assert!(
        s.contains("rdf-syntax-ns#type") || s.contains("rdf:type") || s.contains(" a "),
        "got: {s}"
    );
    assert!(s.contains("person"), "got: {s}");
}

#[test]
fn match_node_with_is_label() {
    // GQL `IS Label` notation: lowered to `:Label` during parsing.
    let s = transpile_lower("MATCH (n IS Person) RETURN n");
    assert!(
        s.contains("rdf-syntax-ns#type") || s.contains("rdf:type") || s.contains(" a "),
        "got: {s}"
    );
    assert!(s.contains("person"), "got: {s}");
}

#[test]
fn match_node_is_and_colon_produce_same_sparql() {
    // IS Label and :Label should produce identical SPARQL.
    let s1 = transpile("MATCH (n IS Person) RETURN n");
    let s2 = transpile("MATCH (n:Person) RETURN n");
    assert_eq!(s1, s2, "IS Label and :Label should produce the same SPARQL");
}

#[test]
fn match_node_is_multi_label() {
    // `IS Person & Employee` — both labels become rdf:type constraints.
    let s = transpile_lower("MATCH (n IS Person & Employee) RETURN n");
    assert!(s.contains("person"), "got: {s}");
    assert!(s.contains("employee"), "got: {s}");
}

// ── FILTER / WHERE ───────────────────────────────────────────────────────────

#[test]
fn where_on_match_clause() {
    // GQL WHERE attached to MATCH — same as Cypher.
    let s = transpile_lower("MATCH (n:Person) WHERE n.age > 30 RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("30"), "got: {s}");
    assert!(s.contains("age"), "got: {s}");
}

#[test]
fn standalone_filter_clause() {
    // GQL `FILTER` is a standalone clause that maps to a WITH's WHERE predicate.
    let s = transpile_lower("MATCH (n:Person) FILTER n.age > 30 RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("30"), "got: {s}");
    assert!(s.contains("age"), "got: {s}");
}

#[test]
fn filter_and_where_equivalent_output() {
    // FILTER and WHERE after MATCH should produce SPARQL with equivalent
    // filter predicates somewhere in the output.
    let s_filter = transpile_lower("MATCH (n:Person) FILTER n.age > 30 RETURN n");
    let s_where = transpile_lower("MATCH (n:Person) WHERE n.age > 30 RETURN n");
    // Both should contain the predicate components.
    assert!(
        s_filter.contains("30") && s_filter.contains("age"),
        "got: {s_filter}"
    );
    assert!(
        s_where.contains("30") && s_where.contains("age"),
        "got: {s_where}"
    );
}

#[test]
fn filter_eq_string() {
    let s = transpile_lower(r#"MATCH (n:Person) FILTER n.name = "Alice" RETURN n"#);
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("alice"), "got: {s}");
}

#[test]
fn filter_and_compound() {
    let s = transpile_lower("MATCH (n) FILTER n.age > 18 AND n.active = true RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("&&") || s.contains("and"), "got: {s}");
}

#[test]
fn filter_or_compound() {
    let s = transpile_lower("MATCH (n) FILTER n.age > 18 OR n.admin = true RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("||") || s.contains("or"), "got: {s}");
}

#[test]
fn filter_not() {
    let s = transpile_lower("MATCH (n) FILTER NOT n.deleted RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("!") || s.contains("not"), "got: {s}");
}

#[test]
fn filter_is_null() {
    let s = transpile_lower("MATCH (n) FILTER n.name IS NULL RETURN n");
    assert!(s.contains("filter"), "got: {s}");
    assert!(s.contains("bound"), "got: {s}");
}

// ── RETURN projections ───────────────────────────────────────────────────────

#[test]
fn return_property_access() {
    let s = transpile_lower("MATCH (n:Person) RETURN n.name");
    assert!(s.contains("name"), "got: {s}");
    assert!(s.contains("select"), "got: {s}");
}

#[test]
fn return_property_with_alias() {
    let s = transpile_lower("MATCH (n:Person) RETURN n.name AS name");
    assert!(s.contains("?name"), "got: {s}");
}

#[test]
fn return_distinct() {
    let s = transpile_lower("MATCH (n:Person) RETURN DISTINCT n");
    assert!(s.contains("distinct"), "got: {s}");
}

#[test]
fn return_star() {
    let s = transpile_lower("MATCH (n:Person) RETURN *");
    assert!(s.contains("select"), "got: {s}");
}

// ── Relationship patterns ────────────────────────────────────────────────────

#[test]
fn rel_with_colon_type() {
    let s = transpile_lower("MATCH (a)-[:KNOWS]->(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
    assert!(s.contains("?a"), "got: {s}");
    assert!(s.contains("?b"), "got: {s}");
}

#[test]
fn rel_with_is_type() {
    // GQL `IS TYPE` edge syntax should produce the same SPARQL as `:TYPE`.
    let s = transpile_lower("MATCH (a)-[r IS KNOWS]->(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
    assert!(s.contains("?a"), "got: {s}");
    assert!(s.contains("?b"), "got: {s}");
}

#[test]
fn rel_is_and_colon_produce_same_sparql() {
    let s1 = transpile("MATCH (a)-[r IS KNOWS]->(b) RETURN a, b");
    let s2 = transpile("MATCH (a)-[r:KNOWS]->(b) RETURN a, b");
    assert_eq!(s1, s2, "IS TYPE and :TYPE should produce the same SPARQL");
}

#[test]
fn rel_left_direction() {
    let s = transpile_lower("MATCH (a)<-[:KNOWS]-(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
    assert!(s.contains("?a"), "got: {s}");
    assert!(s.contains("?b"), "got: {s}");
}

#[test]
fn rel_undirected() {
    let s = transpile_lower("MATCH (a)-[:KNOWS]-(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
}

// ── OPTIONAL MATCH ───────────────────────────────────────────────────────────

#[test]
fn optional_match_emits_optional() {
    let s = transpile_lower("MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b:Person) RETURN a, b");
    assert!(s.contains("optional"), "got: {s}");
    assert!(s.contains("knows"), "got: {s}");
}

// ── ORDER BY / SKIP / LIMIT ──────────────────────────────────────────────────

#[test]
fn order_by_asc() {
    let s = transpile_lower("MATCH (n:Person) RETURN n ORDER BY n.name ASC");
    assert!(s.contains("order"), "got: {s}");
    assert!(s.contains("name"), "got: {s}");
}

#[test]
fn order_by_desc() {
    let s = transpile_lower("MATCH (n:Person) RETURN n ORDER BY n.age DESC");
    assert!(s.contains("order"), "got: {s}");
    assert!(s.contains("desc"), "got: {s}");
}

#[test]
fn limit_emits_limit() {
    let s = transpile_lower("MATCH (n:Person) RETURN n LIMIT 10");
    assert!(s.contains("limit"), "got: {s}");
    assert!(s.contains("10"), "got: {s}");
}

#[test]
fn skip_emits_offset() {
    let s = transpile_lower("MATCH (n:Person) RETURN n SKIP 5");
    assert!(s.contains("offset"), "got: {s}");
    assert!(s.contains("5"), "got: {s}");
}

#[test]
fn order_by_skip_limit_combined() {
    let s = transpile_lower("MATCH (n:Person) RETURN n ORDER BY n.name SKIP 0 LIMIT 25");
    assert!(s.contains("order"), "got: {s}");
    assert!(s.contains("limit 25"), "got: {s}");
}

// ── Aggregation ──────────────────────────────────────────────────────────────

#[test]
fn count_star() {
    let s = transpile_lower("MATCH (n:Person) RETURN count(*)");
    assert!(s.contains("count"), "got: {s}");
}

#[test]
fn count_expr() {
    let s = transpile_lower("MATCH (n:Person) RETURN count(n)");
    assert!(s.contains("count"), "got: {s}");
}

// ── NEXT (GQL scope boundary) ────────────────────────────────────────────────

#[test]
fn next_creates_scope_boundary() {
    // NEXT separates two query blocks; corresponds to Cypher's WITH *.
    let s = transpile_lower("MATCH (n:Person) RETURN n NEXT MATCH (m:Movie) RETURN m");
    // Both concepts should appear in the unified SPARQL query.
    assert!(s.contains("person"), "got: {s}");
    assert!(s.contains("movie"), "got: {s}");
}

// ── IS Label in compound query ───────────────────────────────────────────────

#[test]
fn match_rel_and_is_labels_combined() {
    let s =
        transpile_lower("MATCH (a IS Person)-[r IS KNOWS]->(b IS Person) RETURN a.name, b.name");
    assert!(s.contains("person"), "got: {s}");
    assert!(s.contains("knows"), "got: {s}");
    assert!(s.contains("name"), "got: {s}");
}

// ── RDF-star path through the GQL entry point ───────────────────────────────

#[test]
fn rdf_star_engine_with_gql() {
    let s = transpile_rdf_star(
        "MATCH (a IS Person)-[r IS KNOWS {since: 2020}]->(b IS Person) RETURN a, b, r",
    )
    .to_lowercase();
    // RDF-star encoding should reference the edge properties somehow.
    assert!(s.contains("person"), "got: {s}");
    assert!(s.contains("knows"), "got: {s}");
}

// ── Error path ───────────────────────────────────────────────────────────────

#[test]
fn invalid_gql_returns_error() {
    let result = Transpiler::gql_to_sparql("THIS IS NOT VALID GQL !!!", &ENGINE);
    assert!(result.is_err(), "expected parse error");
}
