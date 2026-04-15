
use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let q = "MATCH (n) RETURN n.division, count(*) ORDER BY count(*) DESC, n.division ASC";
    match Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => println!("{}", r.sparql),
        Err(e) => println!("ERR: {}", e),
    }
}
