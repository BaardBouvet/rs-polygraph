use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries: &[(&str, &str)] = &[
        ("Match3[5]", "MATCH (a)-[r {name: 'r'}]-(b) RETURN a, b"),
        ("Match7[11]", "MATCH (a)-[r {name: 'r1'}]-(b) OPTIONAL MATCH (b)-[r2]-(c) WHERE r <> r2 RETURN a, b, c"),
        ("Match3[15]", "MATCH (x:A)-[r1]->(y)-[r2]-(z) RETURN x, r1, y, r2, z"),
        ("Match3[16]", "MATCH (x)-[r1]-(y)-[r2]-(z) RETURN x, r1, y, r2, z"),
        ("CountingSubgraph1[10]", "MATCH (:A)-->()--() RETURN count(*)"),
    ];
    for (name, q) in queries {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("OK {name}: {}", r.sparql().unwrap_or("<none>")),
            Err(e) => println!("ERR {name}: {e}"),
        }
    }
}
// This would be bad - let me not append to the file
