fn main() {
    use polygraph::{target::TargetEngine, Transpiler};
    struct TckEngine;
    impl TargetEngine for TckEngine {
        fn supports_rdf_star(&self) -> bool {
            true
        }
        fn supports_federation(&self) -> bool {
            false
        }
        fn base_iri(&self) -> Option<&str> {
            Some("http://tck.example.org/")
        }
    }
    let queries = [
        "MATCH (n:Person)-->() WHERE n.name = 'Bob' RETURN n",
        "MATCH (a)-[r]->(b) WHERE type(r) = 'KNOWS' RETURN b",
        "MATCH (n) RETURN n.num, count(*)",
    ];
    for q in &queries {
        println!("=== {q} ===");
        match Transpiler::cypher_to_sparql(q, &TckEngine) {
            Ok(output) => println!("{}", output.sparql),
            Err(e) => println!("ERROR: {e}"),
        }
        println!();
    }
}
