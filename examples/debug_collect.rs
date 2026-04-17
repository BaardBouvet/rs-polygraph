use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        "MATCH (n)-[r:T]->() RETURN r.name AS name",  // normal match
        "MATCH (n) RETURN [(n)-[r:T]->() | r.name] AS list",  // comprehension
    ];
    for q in &queries {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("Q: {}\nSPARQL: {}\n", q, r.sparql),
            Err(e) => println!("ERR: {}", e),
        }
    }
}

