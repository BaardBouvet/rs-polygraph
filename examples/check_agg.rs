use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        "MATCH (n) RETURN n.name, sum(n.num)",
        "MATCH (n) RETURN n.name, sum(DISTINCT n.num)",
        "UNWIND [1, 2, 3, null] AS x WITH x RETURN x, 1 AS c",
    ];
    for q in queries.iter() {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("{}: {}", q, r.sparql),
            Err(e) => println!("{}: ERR {}", q, e),
        }
    }
}
