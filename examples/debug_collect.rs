use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        "MATCH ()-[a]->() WITH a MATCH ()-[b]->() WHERE a = b RETURN count(b)",
    ];
    for q in &queries {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("Q: {}\nSPARQL: {}\n", q, r.sparql),
            Err(e) => println!("ERR: {}", e),
        }
    }
}

