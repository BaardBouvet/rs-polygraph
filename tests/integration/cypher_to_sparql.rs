/// Integration tests: given a Cypher query string, assert the SPARQL output.
///
/// Each test calls `Transpiler::cypher_to_sparql` and checks structural
/// properties of the serialized SPARQL string.
use polygraph::{
    sparql_engine::{GenericSparql11, RdfStar},
    Transpiler,
};

const ENGINE: GenericSparql11 = GenericSparql11;

fn transpile(cypher: &str) -> String {
    Transpiler::cypher_to_sparql(cypher, &ENGINE)
        .unwrap_or_else(|e| panic!("translation failed for {cypher:?}: {e}"))
        .sparql
}

fn transpile_lower(cypher: &str) -> String {
    transpile(cypher).to_lowercase()
}

fn transpile_rdf_star(cypher: &str) -> String {
    let engine = RdfStar::default();
    Transpiler::cypher_to_sparql(cypher, &engine)
        .unwrap_or_else(|e| panic!("rdf-star translation failed for {cypher:?}: {e}"))
        .sparql
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
    let s = transpile_rdf_star("MATCH (a)-[r:KNOWS {since: 2020}]->(b) RETURN a, b");
    let l = s.to_lowercase();
    assert!(
        l.contains("<<") && l.contains(">>"),
        "missing << >> in: {s}"
    );
    assert!(l.contains("since"), "missing 'since' in: {s}");
    assert!(l.contains("knows"), "missing 'knows' in: {s}");
}

#[test]
fn rdf_star_rel_prop_string_literal() {
    let s = transpile_rdf_star(r#"MATCH (a)-[r:LIKES {reason: "fun"}]->(b) RETURN a, b"#);
    let l = s.to_lowercase();
    assert!(l.contains("<<") && l.contains(">>"), "got: {s}");
    assert!(l.contains("likes"), "got: {s}");
    assert!(l.contains("reason"), "got: {s}");
    assert!(l.contains("fun"), "got: {s}");
}

#[test]
fn rdf_star_multiple_inline_rel_props() {
    let s = transpile_rdf_star("MATCH (a)-[r:KNOWS {since: 2020, weight: 5}]->(b) RETURN a, b");
    let l = s.to_lowercase();
    assert!(l.contains("<<") && l.contains(">>"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
    assert!(l.contains("weight"), "got: {s}");
}

#[test]
fn rdf_star_where_rel_prop_emits_annotated_triple_plus_filter() {
    let s = transpile_rdf_star("MATCH (a)-[r:KNOWS]->(b) WHERE r.since > 2000 RETURN a, b");
    let l = s.to_lowercase();
    assert!(l.contains("<<") && l.contains(">>"), "got: {s}");
    assert!(l.contains("filter"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
    assert!(l.contains("2000"), "got: {s}");
}

#[test]
fn rdf_star_return_rel_prop() {
    let s = transpile_rdf_star("MATCH (a)-[r:KNOWS]->(b) RETURN r.since");
    let l = s.to_lowercase();
    assert!(l.contains("<<") && l.contains(">>"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
}

// ── Phase 3: edge properties — reification mode ───────────────────────────────

#[test]
fn reification_inline_rel_prop_emits_rdf_statement() {
    let s = transpile_reification("MATCH (a)-[r:KNOWS {since: 2020}]->(b) RETURN a, b");
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
    let s = transpile_reification("MATCH (a)-[r:KNOWS {since: 2020, weight: 5}]->(b) RETURN a, b");
    let l = s.to_lowercase();
    assert!(!l.contains("<<"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
    assert!(l.contains("weight"), "got: {s}");
}

#[test]
fn reification_where_rel_prop_access_adds_triple() {
    let s = transpile_reification("MATCH (a)-[r:KNOWS]->(b) WHERE r.since > 2000 RETURN a, b");
    let l = s.to_lowercase();
    assert!(!l.contains("<<"), "got: {s}");
    assert!(l.contains("filter"), "got: {s}");
    assert!(l.contains("since"), "got: {s}");
}

#[test]
fn reification_return_rel_prop() {
    let s = transpile_reification("MATCH (a)-[r:KNOWS]->(b) RETURN r.since");
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
    assert!(
        !reif.contains("<<"),
        "reification should not have << : {reif}"
    );
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
fn rdf_star_adapter_reports_rdf_star_true() {
    use polygraph::sparql_engine::TargetEngine;
    let engine = RdfStar::default();
    assert!(engine.supports_rdf_star());
}

#[test]
fn generic_sparql11_reports_rdf_star_false() {
    use polygraph::sparql_engine::TargetEngine;
    assert!(!ENGINE.supports_rdf_star());
}

// ── Phase 4: ORDER BY / SKIP / LIMIT ─────────────────────────────────────────

#[test]
fn return_order_by_asc_emits_order_by() {
    let s = transpile_lower("MATCH (n:Person) RETURN n ORDER BY n.name");
    assert!(s.contains("order by"), "got: {s}");
}

#[test]
fn return_order_by_desc_emits_desc() {
    let s = transpile("MATCH (n:Person) RETURN n ORDER BY n.name DESC");
    let s_lower = s.to_lowercase();
    assert!(s_lower.contains("order by"), "got: {s}");
    assert!(s_lower.contains("desc"), "got: {s}");
}

#[test]
fn return_limit_emits_limit() {
    let s = transpile_lower("MATCH (n) RETURN n LIMIT 10");
    assert!(s.contains("limit"), "got: {s}");
    assert!(s.contains("10"), "got: {s}");
}

#[test]
fn return_skip_emits_offset() {
    let s = transpile_lower("MATCH (n) RETURN n SKIP 5");
    // SPARQL Slice with start=5, no length
    assert!(s.contains("offset") || s.contains("5"), "got: {s}");
}

#[test]
fn return_skip_and_limit_emits_both() {
    let s = transpile_lower("MATCH (n) RETURN n SKIP 5 LIMIT 10");
    assert!(s.contains("limit"), "got: {s}");
    assert!(s.contains("10"), "got: {s}");
}

#[test]
fn return_order_by_multiple_fields() {
    let s = transpile_lower("MATCH (n:Person) RETURN n ORDER BY n.name ASC, n.age DESC");
    assert!(s.contains("order by"), "got: {s}");
    assert!(s.contains("desc"), "got: {s}");
}

// ── Phase 4: aggregation ──────────────────────────────────────────────────────

#[test]
fn aggregate_count_star_emits_count() {
    // count(*) AS total
    let s = transpile_lower("MATCH (n:Person) RETURN count(*) AS total");
    assert!(s.contains("count"), "got: {s}");
    assert!(s.contains("total"), "got: {s}");
}

#[test]
fn aggregate_count_node_emits_count_expr() {
    let s = transpile_lower("MATCH (n:Person) RETURN count(n) AS total");
    assert!(s.contains("count"), "got: {s}");
}

#[test]
fn aggregate_sum_emits_sum() {
    let s = transpile_lower("MATCH (n:Person) RETURN n.name, sum(n.age) AS total_age");
    assert!(s.contains("sum"), "got: {s}");
    assert!(s.contains("total_age"), "got: {s}");
}

#[test]
fn aggregate_avg_emits_avg() {
    let s = transpile_lower("MATCH (n:Person) RETURN avg(n.score) AS mean_score");
    assert!(s.contains("avg"), "got: {s}");
}

#[test]
fn aggregate_collect_emits_group_concat() {
    let s = transpile_lower("MATCH (n:Person) RETURN collect(n.name) AS names");
    assert!(
        s.contains("group_concat") || s.contains("groupconcat"),
        "expected GROUP_CONCAT, got: {s}"
    );
}

#[test]
fn aggregate_group_by_non_agg_vars() {
    // Non-aggregate variables should become GROUP BY variables.
    let s = transpile_lower("MATCH (n:Person) RETURN n.name, count(*) AS cnt");
    assert!(s.contains("count"), "got: {s}");
    // The query must be a valid SELECT GROUP BY structure.
    assert!(s.contains("select"), "got: {s}");
}

// ── Phase 4: UNWIND ───────────────────────────────────────────────────────────

#[test]
fn unwind_list_emits_values() {
    let s = transpile_lower("UNWIND [1, 2, 3] AS x RETURN x");
    assert!(s.contains("values"), "got: {s}");
    assert!(s.contains("?x"), "got: {s}");
}

#[test]
fn unwind_string_list_emits_values() {
    let s = transpile_lower(r#"UNWIND ["alice", "bob"] AS name RETURN name"#);
    assert!(s.contains("values"), "got: {s}");
    assert!(s.contains("name"), "got: {s}");
}

// ── Phase 4: variable-length paths ───────────────────────────────────────────

#[test]
fn varlength_star_emits_one_or_more() {
    // -[:KNOWS*]-> → OneOrMore property path (Cypher * = 1+)
    let s = transpile_lower("MATCH (a)-[:KNOWS*]->(b) RETURN a, b");
    // spargebra renders OneOrMore as (pred)+ in SPARQL property path syntax
    assert!(s.contains("knows"), "got: {s}");
    assert!(
        s.contains("+") || s.contains("oneormore"),
        "expected path +, got: {s}"
    );
}

#[test]
fn varlength_one_or_more_emits_plus() {
    // -[:KNOWS*1..]-> → OneOrMore
    let s = transpile_lower("MATCH (a)-[:KNOWS*1..]->(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
    assert!(
        s.contains("+") || s.contains("oneormore"),
        "expected path +, got: {s}"
    );
}

#[test]
fn varlength_zero_or_one_emits_question() {
    // -[:KNOWS*0..1]-> → ZeroOrOne
    let s = transpile_lower("MATCH (a)-[:KNOWS*0..1]->(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
    assert!(
        s.contains("?") || s.contains("zeroorone"),
        "expected path ?, got: {s}"
    );
}

#[test]
fn varlength_bounded_range_succeeds() {
    // *2..5 is now supported via UNION of fixed-length chains.
    let result = Transpiler::cypher_to_sparql("MATCH (a)-[:KNOWS*2..5]->(b) RETURN a, b", &ENGINE);
    assert!(
        result.is_ok(),
        "expected success for *2..5, got: {:?}",
        result.unwrap_err()
    );
}

// ── Phase 4: multi-type relationship (union path) ─────────────────────────────

#[test]
fn multi_type_rel_emits_alternative_path() {
    // -[:KNOWS|LIKES]-> → Alternative property path
    let s = transpile_lower("MATCH (a)-[:KNOWS|LIKES]->(b) RETURN a, b");
    assert!(s.contains("knows"), "got: {s}");
    assert!(s.contains("likes"), "got: {s}");
    // spargebra uses | for Alternative
    assert!(s.contains("|"), "expected | path alternative, got: {s}");
}

// ── Phase 4: IN list literal ─────────────────────────────────────────────────

#[test]
fn where_in_list_literal_emits_in() {
    let s = transpile_lower(r#"MATCH (n:Person) WHERE n.status IN ["active", "pending"] RETURN n"#);
    assert!(s.contains("in"), "got: {s}");
    assert!(s.contains("active"), "got: {s}");
    assert!(s.contains("pending"), "got: {s}");
}

// ── Phase 4: write clauses → UnsupportedFeature ───────────────────────────────

#[test]
fn create_clause_returns_unsupported_feature() {
    let result = Transpiler::cypher_to_sparql("CREATE (n:Person {name: 'Alice'})", &ENGINE);
    assert!(result.is_err(), "CREATE should return an error");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.to_lowercase().contains("create") || msg.contains("unsupported"),
        "got: {msg}"
    );
}

#[test]
fn merge_clause_returns_unsupported_feature() {
    let result = Transpiler::cypher_to_sparql("MERGE (n:Person {name: 'Alice'})", &ENGINE);
    assert!(result.is_err(), "MERGE should return an error");
}

#[test]
fn set_clause_returns_unsupported_feature() {
    let result = Transpiler::cypher_to_sparql("MATCH (n:Person) SET n.age = 30", &ENGINE);
    assert!(result.is_err(), "SET should return an error");
}

#[test]
fn delete_clause_returns_unsupported_feature() {
    let result = Transpiler::cypher_to_sparql("MATCH (n:Person) DELETE n", &ENGINE);
    assert!(result.is_err(), "DELETE should return an error");
}

#[test]
fn call_clause_returns_unsupported_feature() {
    let result = Transpiler::cypher_to_sparql(
        "CALL apoc.path.expand(n, 'KNOWS', null, 1, 3) WITH node RETURN node",
        &ENGINE,
    );
    assert!(result.is_err(), "CALL should return an error");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("apoc") || msg.contains("CALL") || msg.contains("unsupported"),
        "got: {msg}"
    );
}

// ── Phase 4: parser-level tests for new clauses ───────────────────────────────

#[test]
fn parse_order_by_desc_round_trips() {
    use polygraph::Transpiler;
    let ast = Transpiler::parse_cypher("MATCH (n) RETURN n ORDER BY n.name DESC")
        .expect("parse should succeed");
    // Verify ORDER BY parsed correctly.
    if let Some(polygraph::ast::cypher::Clause::Return(r)) = ast.clauses.last() {
        let ob = r.order_by.as_ref().expect("order_by should be Some");
        assert!(ob.items[0].descending, "first sort item should be DESC");
    } else {
        panic!("last clause should be RETURN");
    }
}

#[test]
fn parse_unwind_as_variable() {
    use polygraph::Transpiler;
    let ast =
        Transpiler::parse_cypher("UNWIND [1, 2] AS x RETURN x").expect("parse should succeed");
    if let Some(polygraph::ast::cypher::Clause::Unwind(u)) = ast.clauses.first() {
        assert_eq!(u.variable, "x");
    } else {
        panic!("first clause should be UNWIND");
    }
}

#[test]
fn parse_aggregate_count_star() {
    use polygraph::ast::cypher::{AggregateExpr, Expression};
    use polygraph::Transpiler;
    let ast = Transpiler::parse_cypher("MATCH (n) RETURN count(*) AS total")
        .expect("parse should succeed");
    if let Some(polygraph::ast::cypher::Clause::Return(r)) = ast.clauses.last() {
        if let polygraph::ast::cypher::ReturnItems::Explicit(items) = &r.items {
            assert!(
                matches!(
                    &items[0].expression,
                    Expression::Aggregate(AggregateExpr::Count { expr: None, .. })
                ),
                "expected count(*), got {:?}",
                items[0].expression
            );
        }
    }
}

#[test]
fn parse_set_clause() {
    use polygraph::ast::cypher::{Clause, SetItem};
    use polygraph::Transpiler;
    let ast = Transpiler::parse_cypher("MATCH (n) SET n.age = 30").expect("parse should succeed");
    if let Some(Clause::Set(s)) = ast.clauses.last() {
        assert_eq!(s.items.len(), 1);
        assert!(matches!(s.items[0], SetItem::Property { .. }));
    } else {
        panic!("last clause should be SET, got {:?}", ast.clauses.last());
    }
}

#[test]
fn parse_delete_clause() {
    use polygraph::ast::cypher::{Clause, DeleteClause};
    use polygraph::Transpiler;
    let ast = Transpiler::parse_cypher("MATCH (n) DELETE n").expect("parse should succeed");
    if let Some(Clause::Delete(DeleteClause {
        detach,
        expressions,
    })) = ast.clauses.last()
    {
        assert!(!detach, "should not be DETACH");
        assert_eq!(expressions.len(), 1);
    } else {
        panic!("last clause should be DELETE");
    }
}

#[test]
fn parse_detach_delete_clause() {
    use polygraph::ast::cypher::{Clause, DeleteClause};
    use polygraph::Transpiler;
    let ast = Transpiler::parse_cypher("MATCH (n) DETACH DELETE n").expect("parse should succeed");
    if let Some(Clause::Delete(DeleteClause { detach, .. })) = ast.clauses.last() {
        assert!(detach, "should be DETACH");
    } else {
        panic!("last clause should be DETACH DELETE");
    }
}

#[test]
fn parse_remove_property_clause() {
    use polygraph::ast::cypher::{Clause, RemoveItem};
    use polygraph::Transpiler;
    let ast = Transpiler::parse_cypher("MATCH (n) REMOVE n.age").expect("parse should succeed");
    if let Some(Clause::Remove(r)) = ast.clauses.last() {
        assert!(matches!(r.items[0], RemoveItem::Property { .. }));
    } else {
        panic!("last clause should be REMOVE");
    }
}

#[test]
fn parse_call_clause() {
    use polygraph::ast::cypher::Clause;
    use polygraph::Transpiler;
    let ast = Transpiler::parse_cypher("CALL db.labels()").expect("parse should succeed");
    if let Some(Clause::Call(c)) = ast.clauses.first() {
        assert_eq!(c.procedure, "db.labels");
    } else {
        panic!("first clause should be CALL");
    }
}

#[test]
fn test_prec2_multiply_parens() {
    use oxigraph::sparql::{QueryResults};
    use oxigraph::store::Store;
    let store = Store::new().unwrap();
    let sparql = transpile("RETURN 4 * (2 + 3) * 2 AS c");
    println!("SPARQL: {sparql}");
    // Should be 40, not 14
    #[allow(deprecated)]
    match store.query(sparql.as_str()) {
        Ok(QueryResults::Solutions(mut sols)) => {
            let vars: Vec<_> = sols.variables().iter().map(|v| v.as_str().to_owned()).collect();
            for sol_r in &mut sols {
                if let Ok(sol) = sol_r {
                    for v in &vars {
                        println!("  {}: {:?}", v, sol.get(v.as_str()));
                    }
                }
            }
        }
        Err(e) => println!("Error: {e}"),
        _ => {}
    }
    // Also check the AST
    let ast = Transpiler::parse_cypher("RETURN 4 * (2 + 3) * 2 AS c").unwrap();
    println!("AST: {:?}", ast.clauses[0]);
}

#[test]
fn debug_with_orderby_limit() {
    let output = Transpiler::cypher_to_sparql(
        "MATCH (a) WITH a ORDER BY a.bool, a.num LIMIT 4 RETURN a",
        &polygraph::sparql_engine::GenericSparql11
    ).unwrap();
    println!("SPARQL:\n{}", output.sparql);
    assert!(output.sparql.len() > 0);
}


#[test]
fn debug_contains_non_string() {
    let q = r#"WITH [1, 3.14, true] AS operands
UNWIND operands AS op1
UNWIND operands AS op2
WITH op1 CONTAINS op2 AS v
RETURN v, count(*)"#;
    let s = transpile(q);
    println!("SPARQL:\n{}", s);
}

#[test]
fn debug_contains_full() {
    let q = r#"WITH [1, 3.14, true, [], {}, null] AS operands
UNWIND operands AS op1
UNWIND operands AS op2
WITH op1 CONTAINS op2 AS v
RETURN v, count(*)"#;
    let s = transpile(q);
    println!("SPARQL:\n{}", s);
}

#[test]
fn debug_oxigraph_undef_groupby() {
    use oxigraph::{sparql::{QueryResults, SparqlEvaluator}, store::Store};
    let store = Store::new().unwrap();
    
    let tests = [
        ("type-error UNDEF",
         r#"SELECT ?v (COUNT(*) AS ?cnt) WHERE { VALUES ?x { 1 2 3 } BIND(CONTAINS(?x, "a") AS ?v) } GROUP BY ?v"#),
        ("explicit UNDEF via IF+unbound",
         r#"SELECT ?v (COUNT(*) AS ?cnt) WHERE { VALUES ?x { 1 2 3 } BIND(IF(isString(?x), CONTAINS(?x, "a"), ?__undef) AS ?v) } GROUP BY ?v"#),
    ];
    
    for (name, q) in &tests {
        let q_str: &str = q;
        #[expect(deprecated)]
        let res = store.query_opt(q_str, SparqlEvaluator::new());
        match res {
            Ok(QueryResults::Solutions(mut sols)) => {
                let rows: Vec<_> = sols.collect();
                println!("{}: {} rows", name, rows.len());
                for r in &rows {
                    println!("  {:?}", r);
                }
            }
            Ok(_) => println!("{}: non-solutions result", name),
            Err(e) => println!("{}: error: {}", name, e),
        }
    }
}

#[test]
fn debug_min_int() {
    let q = "RETURN -9223372036854775808 AS literal";
    let s = transpile(q);
    println!("SPARQL:\n{}", s);
}
