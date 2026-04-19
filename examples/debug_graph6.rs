use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        r#"MATCH (a:A)-[r:REL]->(b:B)
WITH a AS b, b AS tmp, r AS r
WITH b AS a, r
LIMIT 1
MATCH (a)-[r]->(b)
RETURN a, r, b"#,
    ];
    for (i, q) in queries.iter().enumerate() {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("OK[{}]: {}", i, r.sparql),
            Err(e) => println!("ERR[{}]: {e}", i),
        }
    }
}
