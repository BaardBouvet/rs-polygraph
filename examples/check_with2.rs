use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        "MATCH (a:Begin) WITH a.num AS property MATCH (b) WHERE b.id = property RETURN b",
        "MATCH (a:End)-[r]->(b:End) RETURN a, r, b",
    ];
    for q in queries.iter() {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("Q: {q}\nSPARQL: {}\n", r.sparql),
            Err(e) => println!("Q: {q}\nERR: {e}\n"),
        }
    }
}
