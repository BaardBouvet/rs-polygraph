//! Test FILTER(BOUND) behavior with oxigraph
use oxigraph::{
    sparql::{QueryResults, SparqlEvaluator},
    store::Store,
};

fn main() {
    let store = Store::new().unwrap();

    store.update(r#"INSERT DATA {
  <http://e.g/n0> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesExist> .
  <http://e.g/n0> <http://e.g/num> 42 .
  <http://e.g/n1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesExist> .
  <http://e.g/n1> <http://e.g/num> 43 .
  <http://e.g/n2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesExist> .
  <http://e.g/n2> <http://e.g/num> 44 .
}"#).unwrap();
    
    println!("=== Test A: Simple OPTIONAL chain, 3 rows fval ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesExist> }
  OPTIONAL { ?f <http://e.g/num> ?fval }
}"#);

    println!("\n=== Test B: FILTER(BOUND) group before triple, bound f ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesExist> }
  OPTIONAL { { FILTER(BOUND(?f)) } ?f <http://e.g/num> ?fval }
}"#);


    println!("\n=== Test B2: Direct triple with bound f ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesExist> }
  ?f <http://e.g/num> ?fval
}"#);

    println!("\n=== Test B3: Inner OPTIONAL just triple, bound f ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesExist> }
  OPTIONAL { ?f <http://e.g/num> ?fval }
}"#);

    println!("\n=== Test B4: OPTIONAL filter-join - does it get fval? ===");
    run_query(&store, r#"SELECT ?f ?fval WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesExist> }
  OPTIONAL { { FILTER(BOUND(?f)) } ?f <http://e.g/num> ?fval }
}"#);

    println!("\n=== Test C: FILTER(BOUND) group + nested OPTIONAL, bound f ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesExist> }
  OPTIONAL { { FILTER(BOUND(?f)) } OPTIONAL { ?f <http://e.g/num> ?fval } }
}"#);
    
    println!("\n=== Test D: Sub-select FILTER, bound f ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesExist> }
  OPTIONAL { SELECT ?f ?fval WHERE { FILTER(BOUND(?f)) . ?f <http://e.g/num> ?fval } }
}"#);

    println!("\n=== Test E: FILTER(BOUND) group nested OPTIONAL, UNBOUND n ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?n <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesNotExist> }
  OPTIONAL { { FILTER(BOUND(?n)) } OPTIONAL { ?n <http://e.g/num> ?nval } }
}"#);

    println!("\n=== Test F: FILTER(BOUND) group before triple, UNBOUND n ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?n <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e.g/DoesNotExist> }
  OPTIONAL { { FILTER(BOUND(?n)) } ?n <http://e.g/num> ?nval }
}"#);
}

#[expect(deprecated)]
fn run_query(store: &Store, q: &str) {
    match store.query_opt(q, SparqlEvaluator::new()) {
        Ok(QueryResults::Solutions(mut s)) => {
            let mut count = 0;
            while let Some(row) = s.next() {
                let row = row.unwrap();
                count += 1;
                let vars: Vec<_> = row.iter().map(|(v, t)| {
                    format!("{}={}", v.as_str(), t)
                }).collect();
                println!("  row {}: {}", count, vars.join(", "));
            }
            println!("  Total: {} rows", count);
        }
        Ok(_) => println!("  Non-solution result"),
        Err(e) => println!("  Error: {}", e),
    }
}
