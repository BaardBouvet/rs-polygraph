use polygraph::{Transpiler, sparql_engine::TargetEngine};
struct E; static ENGINE: E = E;
impl TargetEngine for E {
    fn base_iri(&self) -> Option<&str> { None }
    fn supports_rdf_star(&self) -> bool { false }
    fn supports_federation(&self) -> bool { false }
    fn finalize(&self, s: String) -> Result<String, polygraph::PolygraphError> { Ok(s) }
}
fn main() {
    let queries: &[(&str, &str)] = &[
        ("simple_none_false", "WITH [1,2,3] AS list WITH none(x IN list WHERE false) AS result RETURN result"),
        ("case_when_basic", "WITH [1,2,3] AS list WITH CASE WHEN 1 > 0 THEN [1] ELSE [2] END + list AS list2 RETURN list2"),
        ("reverse_var", "WITH [1,2,3] AS list WITH reverse(list) AS r RETURN r"),
        ("list_comp_return", "RETURN [x IN [1,2,3] | x * 2] AS result"),
        ("list_comp_where_static", "WITH [1,2,3] AS list WITH [y IN list WHERE y > 1 | y] AS r RETURN r"),
        ("list_comp_where_rand", "WITH [1,2,3] AS list WITH [y IN list WHERE rand() > 0.5 | y] AS r RETURN r"),
        ("properties_n", "MATCH (n:A) RETURN properties(n) AS m"),
        ("collect_names", "MATCH (n) RETURN [x IN collect(n.name) | x + '!'] AS names"),
    ];
    for (name, q) in queries {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("{name}: OK → {}", &r.sparql[..r.sparql.len().min(150)]),
            Err(e) => println!("{name}: ERROR → {e}"),
        }
    }
}
