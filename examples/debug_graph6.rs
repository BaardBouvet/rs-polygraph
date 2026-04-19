use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        // Match6 varlen 3-hop path: should expand to 3 consecutive edges
        r#"MATCH topRoute = (:Start)<-[:CONNECTED_TO]-()-[:CONNECTED_TO*3..3]-(:End) RETURN topRoute"#,
        // Simpler case: exact 2-hop
        r#"MATCH p = (a)-[:T*2..2]->(b) RETURN a, b"#,
    ];
    for (i, q) in queries.iter().enumerate() {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("OK[{}]: {}", i, r.sparql),
            Err(e) => println!("ERR[{}]: {e}", i),
        }
    }
}
