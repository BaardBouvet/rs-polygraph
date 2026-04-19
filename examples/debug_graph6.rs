use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        r#"MATCH (n) RETURN labels(n)"#,
        r#"OPTIONAL MATCH (n:DoesNotExist) RETURN labels(n), labels(null)"#,
        r#"RETURN relationships(null)"#,
        r#"MATCH (a)-[r]->() WITH [r, 1] AS list RETURN type(list[0])"#,
    ];
    for (i, q) in queries.iter().enumerate() {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("OK[{}]: {}", i, r.sparql),
            Err(e) => println!("ERR[{}]: {e}", i),
        }
    }
}
