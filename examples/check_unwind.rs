use polygraph::{Transpiler, target::TargetEngine};
struct TckEngine;
impl TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}
fn main() {
    let queries = vec![
        ("Unwind1:69", "WITH [1, 2, 3] AS first, [4, 5, 6] AS second\nUNWIND (first + second) AS x\nRETURN x"),
        ("Unwind1:149", "WITH [[1, 2, 3], [4, 5, 6]] AS lol\nUNWIND lol AS x\nUNWIND x AS y\nRETURN y"),
        ("Unwind1:210", "WITH [1, 2, 3] AS list\nUNWIND list AS x\nRETURN *"),
        ("Unwind1:251", "WITH [1, 2] AS xs, [3, 4] AS ys, [5, 6] AS zs\nUNWIND xs AS x\nUNWIND ys AS y\nUNWIND zs AS z\nRETURN *"),
    ];
    for (name, q) in queries {
        match Transpiler::cypher_to_sparql(q, &TckEngine) {
            Ok(sparql) => println!("=== {name} OK ===\n{sparql}\n"),
            Err(e) => println!("=== {name} ERROR: {e}\n"),
        }
    }
}
