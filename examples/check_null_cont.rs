use polygraph::{Transpiler, target::TargetEngine};
struct TckEngine;
impl TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}
fn main() {
    let qs = vec![
        // Match9:159
        ("Match9:159", "MATCH (a)-[r1]->()-[r2]->(b)\nWITH [r1, r2] AS rs, a AS second, b AS first\n  LIMIT 1\nMATCH (first)-[rs*]->(second)\nRETURN first, second"),
        // Return4:45 
        ("Return4:45", "MATCH (a) WITH a.name AS a RETURN a"), 
    ];
    for (name, q) in qs {
        match Transpiler::cypher_to_sparql(q, &TckEngine) {
            Ok(s) => println!("=== {name} ===\n{s}\n"),
            Err(e) => println!("=== {name} ERROR: {e}\n"),
        }
    }
}
