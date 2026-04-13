use polygraph::{Transpiler, target::TargetEngine};
struct TckEngine;
impl TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}
fn main() {
    let q = "MATCH (a)\nRETURN a.id IS NOT NULL AS a, a IS NOT NULL AS b";
    match Transpiler::cypher_to_sparql(q, &TckEngine) {
        Ok(output) => println!("SPARQL: {}\nLen: {}", output.sparql, output.sparql.len()),
        Err(e) => println!("ERROR: {e}"),
    }
}
