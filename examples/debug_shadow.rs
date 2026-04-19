use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        "MATCH (a:A) WITH a, a.num2 % 3 AS x WITH a, a.num + a.num2 AS x ORDER BY x LIMIT 3 RETURN a, x",
        "WITH 1 AS a WITH 2 AS a RETURN a",
    ];
    for q in queries.iter() {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("Q: {q}\nSPARQL: {}\n\n", r.sparql),
            Err(e) => println!("Q: {q}\nERR: {e}\n\n"),
        }
    }
}
