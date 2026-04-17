use oxigraph::{sparql::{QueryResults, SparqlEvaluator}, store::Store};
use polygraph::Transpiler;

struct TckEngine;
impl polygraph::sparql_engine::TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}
const ENGINE: TckEngine = TckEngine;

#[allow(deprecated)]
fn run(store: &Store, q: &str) {
    let result = match Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => r,
        Err(e) => { println!("  gen error: {}", e); return; },
    };
    println!("  SPARQL: {}", result.sparql);
    match store.query_opt(&result.sparql, SparqlEvaluator::new()) {
        Ok(QueryResults::Solutions(mut sols)) => {
            let vars: Vec<String> = sols.variables().iter().map(|v| v.as_str().to_owned()).collect();
            for s in sols.by_ref() {
                let sol = s.unwrap();
                let row: Vec<_> = vars.iter().map(|v| sol.get(v.as_str()).map(|t| t.to_string())).collect();
                println!("  row: {:?}", row);
            }
        }
        Ok(_) => {},
        Err(e) => println!("  SPARQL exec error: {}", e),
    }
}

fn main() {
    let b = "http://tck.example.org/";
    let store = Store::new().unwrap();
    store.update(&format!(r#"INSERT DATA {{
        _:n1 <{b}__node> <{b}__node> . _:n2 <{b}__node> <{b}__node> . _:n1 <{b}T> _:n2 .
    }}"#)).unwrap();
    
    println!("=== noWITH ===");
    run(&store, "MATCH ()-[a]->() MATCH ()-[b]->() WHERE a = b RETURN count(b)");
    println!("\n=== withWITH ===");
    run(&store, "MATCH ()-[a]->() WITH a MATCH ()-[b]->() WHERE a = b RETURN count(b)");
}
