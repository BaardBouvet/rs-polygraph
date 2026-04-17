use polygraph::Transpiler;
struct TckEngine;
impl polygraph::sparql_engine::TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}
const ENGINE: TckEngine = TckEngine;
fn main() {
    let qs = [
        "RETURN single(x IN [34, 0, null, 5, 900] WHERE x < 10) AS result",
        "RETURN single(x IN [4, 0, null, -15, 9] WHERE x < 10) AS result",
        "RETURN single(x IN [null, null] WHERE x IS NULL) AS result",
        "RETURN single(x IN [null, 2] WHERE x IS NULL) AS result",
    ];
    for q in &qs {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("Q: {}\n  SPARQL: {}\n", q, r.sparql),
            Err(e) => println!("Q: {} error: {}\n", q, e),
        }
    }
}
