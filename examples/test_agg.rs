use oxigraph::store::Store;
use oxigraph::sparql::QueryResults;
use polygraph::{Transpiler, target::GenericSparql11};
fn main() {
    let store = Store::new().unwrap();
    // 5 nodes
    let insert = "INSERT DATA { _:a <http://polygraph.example/__node> <http://polygraph.example/__node> . _:b <http://polygraph.example/__node> <http://polygraph.example/__node> . _:c <http://polygraph.example/__node> <http://polygraph.example/__node> . _:d <http://polygraph.example/__node> <http://polygraph.example/__node> . _:e <http://polygraph.example/__node> <http://polygraph.example/__node> . }";
    store.update(insert).unwrap();
    
    let engine = GenericSparql11;
    let sparql = Transpiler::cypher_to_sparql("MATCH (n) RETURN count(n) / 60 / 60 AS count", &engine).unwrap();
    println!("SPARQL: {sparql}");
    
    match store.query(&sparql) {
        Ok(QueryResults::Solutions(mut sols)) => {
            let vars: Vec<_> = sols.variables().iter().map(|v| v.as_str().to_owned()).collect();
            println!("Vars: {vars:?}");
            for sol_r in sols.by_ref() {
                let sol = sol_r.unwrap();
                let row: Vec<_> = vars.iter().map(|v| sol.get(v.as_str()).map(|t| t.to_string())).collect();
                println!("Row: {row:?}");
            }
        }
        Err(e) => println!("Error: {e}"),
        _ => {}
    }
}
