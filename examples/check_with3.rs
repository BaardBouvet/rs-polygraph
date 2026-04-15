use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        "MATCH (a)-[r]->(b:X) WITH a, r, b MATCH (a)-[r]->(b) RETURN r AS rel ORDER BY rel.id",
        "MATCH ()-[r1]->(:X) WITH r1 AS r2 MATCH ()-[r2]->() RETURN r2 AS rel",
    ];
    for q in queries.iter() {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("Q: {q}\nSPARQL: {}\n", r.sparql),
            Err(e) => println!("Q: {q}\nERR: {e}\n"),
        }
    }
}
