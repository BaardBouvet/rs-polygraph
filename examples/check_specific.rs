use polygraph::{Transpiler, sparql_engine::TargetEngine};

struct TckEngine;
impl TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}

fn main() {
    // Match4[5]: varlen with inline property predicate
    let q5 = "MATCH (a:Artist)-[:WORKED_WITH* {year: 1988}]->(b:Artist)\nRETURN *";
    // Match4[7]: bound relationship in varlen
    let q7 = "MATCH ()-[r:EDGE]-()\nMATCH p = (n)-[*0..1]-()-[r]-()-[*0..1]-(m)\nRETURN count(p) AS c";
    // Match7[12]: optional variable length
    let q12 = "MATCH (a:A), (b:B)\nOPTIONAL MATCH (a)-[r*]-(b)\nWHERE r IS NULL AND a <> b\nRETURN b";
    
    for (name, q) in [("Match4[5]", q5), ("Match4[7]", q7), ("Match7[12]", q12)] {
        println!("\n=== {} ===", name);
        match Transpiler::cypher_to_sparql(q, &TckEngine) {
            Ok(tr) => println!("SPARQL:\n{}", tr.sparql),
            Err(e) => println!("TRANSLATION ERROR: {:?}", e),
        }
    }
}


