use polygraph::{Transpiler, target::TargetEngine};
struct TckEngine;
impl TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}
fn main() {
    let queries = vec![
        ("Return3:44", "MATCH (a)\nRETURN a.id IS NOT NULL AS a, a IS NOT NULL AS b"),
        ("Return4:45", "MATCH (a)\nWITH a.name AS a\nRETURN a"),
        ("Return6:77", "MATCH (a)\nWITH a.num AS a, count(*) AS count\nRETURN count"),
        ("Return4:141", "MATCH p = (n)-->(b)\nRETURN aVg(    n.aGe     )"),
        ("Return2:135", "MATCH (a)\nRETURN a.list2 + a.list1 AS foo"),
        ("Match7:302", "MATCH (a:Single)\nOPTIONAL MATCH (a)-[*3..]-(b)\nRETURN b"),
    ];
    for (name, q) in queries {
        match Transpiler::cypher_to_sparql(q, &TckEngine) {
            Ok(sparql) => println!("=== {name} ===\n{sparql}\n"),
            Err(e) => println!("=== {name} ERROR: {e}\n"),
        }
    }
}
