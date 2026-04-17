use polygraph::Transpiler;

struct TckEngine;
impl polygraph::sparql_engine::TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}
const ENGINE: TckEngine = TckEngine;

fn main() {
    let q = "MATCH (n), (m) WHERE (n)-[:REL1*]-(m) RETURN n, m";
    match Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => println!("SPARQL:\n{}", r.sparql),
        Err(e) => println!("ERR: {}", e),
    }
}
