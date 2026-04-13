use polygraph::{Transpiler, target::TargetEngine};
use oxigraph::store::Store;
use oxigraph::sparql::QueryResults;

struct TckEngine;
impl TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}

fn main() {
    // Return4:141 — MATCH p = (n)-->(b) RETURN aVg(    n.aGe     )
    // Expected: null (no matches)
    let store = Store::new().unwrap();
    // Graph is empty — CREATE () only.
    store.update("INSERT DATA { _:n0 <http://tck.example.org/__node> <http://tck.example.org/__node> . }").unwrap();
    
    let q = "MATCH p = (n)-->(b)\nRETURN aVg(    n.aGe     )";
    let sparql = Transpiler::cypher_to_sparql(q, &TckEngine).unwrap().sparql;
    println!("SPARQL: {}", sparql);
    
    match store.query(&sparql).unwrap() {
        QueryResults::Solutions(mut sols) => {
            for sol in sols.by_ref() {
                let sol = sol.unwrap();
                println!("Row: {:?}", sol);
            }
        }
        _ => println!("Non-solution result"),
    }
}
