use polygraph::Transpiler;
use oxigraph::store::Store;
use oxigraph::sparql::QueryResults;

struct TckEngine;
impl polygraph::sparql_engine::TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}
const ENGINE: TckEngine = TckEngine;

fn run_query(store: &Store, name: &str, q: &str) {
    print!("\n=== {} === ", name);
    match Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => {
            match store.query(&r.sparql) {
                Ok(QueryResults::Solutions(mut sols)) => {
                    let mut found = false;
                    while let Some(Ok(sol)) = sols.next() {
                        let res = sol.get("result").map(|t| t.to_string()).unwrap_or("UNDEF".to_string());
                        print!("result={}", res);
                        found = true;
                    }
                    if !found { print!("(0 rows)"); }
                }
                Ok(_) => print!("Non-tabular"),
                Err(e) => print!("Query error: {}", e),
            }
            println!();
        }
        Err(e) => println!("ERR: {}", e),
    }
}

fn run_query_verbose(store: &Store, name: &str, q: &str) {
    println!("\n=== {} ===", name);
    match Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => {
            println!("SPARQL:\n{}", r.sparql);
            match store.query(&r.sparql) {
                Ok(QueryResults::Solutions(mut sols)) => {
                    while let Some(Ok(sol)) = sols.next() {
                        let res = sol.get("result").map(|t| t.to_string()).unwrap_or("UNDEF".to_string());
                        println!("  result={}", res);
                    }
                }
                Ok(_) => println!("Non-tabular"),
                Err(e) => println!("Query error: {}", e),
            }
        }
        Err(e) => println!("ERR: {}", e),
    }
}

fn main() {
    let store = Store::new().unwrap();
    
    // With7[1] - test: variable renaming across WITHs
    let q = "MATCH (a:A)-[r:REL]->(b:B) WITH a AS b, b AS tmp, r AS r WITH b AS a, r LIMIT 1 MATCH (a)-[r]->(b) RETURN a, r, b";
    match Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => {
            println!("SPARQL:\n{}", r.sparql);
        }
        Err(e) => println!("ERR: {}", e),
    }
}
