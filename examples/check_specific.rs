use polygraph::{Transpiler, sparql_engine::TargetEngine};

struct TckEngine;
impl TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}

fn main() {
    // Match4[8]: WITH [r1, r2] AS rs MATCH (first)-[rs*]->(second) 
    let q = r#"MATCH ()-[r1]->()-[r2]->()
WITH [r1, r2] AS rs
  LIMIT 1
MATCH (first)-[rs*]->(second)
RETURN first, second"#;
    
    match Transpiler::cypher_to_sparql(q, &TckEngine) {
        Ok(tr) => {
            let sparql = &tr.sparql;
            println!("Length: {}", sparql.len());
            // Print in 100-char chunks for readability
            for (i, chunk) in sparql.as_bytes().chunks(100).enumerate() {
                println!("[{}..{}] {}", i*100, (i+1)*100, std::str::from_utf8(chunk).unwrap());
            }
        },
        Err(e) => println!("TRANSLATION ERROR: {:?}", e),
    }
}




