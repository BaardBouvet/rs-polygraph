use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let q1 = "MATCH (a:A)-[r:REL]->(b:B)\nWITH a AS b, b AS tmp, r AS r\nWITH b AS a, r\nLIMIT 1\nMATCH (a)-[r]->(b)\nRETURN a, r, b";
    let q2 = "MATCH (a:A)\nWITH a, a.num2 % 3 AS x\nWITH a, a.num + a.num2 AS x\n  ORDER BY x\n  LIMIT 3\nRETURN a, x";
    for q in [q1, q2] {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("{}\n", r.sparql),
            Err(e) => println!("ERR: {e}\n"),
        }
    }
}
