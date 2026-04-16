use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let q = r#"OPTIONAL MATCH (f:DoesExist)
OPTIONAL MATCH (n:DoesNotExist)
RETURN collect(DISTINCT n.num) AS a, collect(DISTINCT f.num) AS b"#;
    match Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => println!("SPARQL:\n{}", r.sparql),
        Err(e) => println!("Error: {}", e),
    }
}
