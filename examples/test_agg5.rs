//! Test Aggregation5[2] scenario directly with oxigraph
use oxigraph::{
    sparql::{QueryResults, SparqlEvaluator},
    store::Store,
};

fn main() {
    let store = Store::new().unwrap();

    store.update(r#"INSERT DATA {
  _:n0 <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesExist> .
  _:n0 <http://tck.example.org/num> "42"^^<http://www.w3.org/2001/XMLSchema#integer> .
  _:n1 <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesExist> .
  _:n1 <http://tck.example.org/num> "43"^^<http://www.w3.org/2001/XMLSchema#integer> .
  _:n2 <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesExist> .
  _:n2 <http://tck.example.org/num> "44"^^<http://www.w3.org/2001/XMLSchema#integer> .
}"#).unwrap();
    
    println!("=== Sub-SELECT approach ===");
    run_query(&store, r#"SELECT (GROUP_CONCAT(DISTINCT STR(?nnum)) AS ?gc_n) (GROUP_CONCAT(DISTINCT STR(?fnum)) AS ?gc_f) WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesExist> . }
  OPTIONAL { ?n <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesNotExist> . }
  OPTIONAL { { SELECT ?n ?nnum WHERE { ?n <http://tck.example.org/num> ?nnum . FILTER(BOUND(?n)) } } }
  OPTIONAL { { SELECT ?f ?fnum WHERE { ?f <http://tck.example.org/num> ?fnum . FILTER(BOUND(?f)) } } }
}"#);

    println!("\n=== Simple approach for reference ===");
    run_query(&store, r#"SELECT (GROUP_CONCAT(DISTINCT STR(?nnum)) AS ?gc_n) (GROUP_CONCAT(DISTINCT STR(?fnum)) AS ?gc_f) WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesExist> . }
  OPTIONAL { ?n <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesNotExist> . }
  OPTIONAL { ?n <http://tck.example.org/num> ?nnum }
  OPTIONAL { ?f <http://tck.example.org/num> ?fnum }
}"#);

    println!("\n=== Full Aggregation5[2] query style ===");
    run_query(&store, r#"SELECT (CONCAT("[", COALESCE(?__gc_1, ""), "]") AS ?a) (CONCAT("[", COALESCE(?__gc_3, ""), "]") AS ?b) WHERE {
  {SELECT (GROUP_CONCAT(DISTINCT CONCAT("'", STR(?__n_num_0), "'"); SEPARATOR = ", ") AS ?__gc_1)
         (GROUP_CONCAT(DISTINCT CONCAT("'", STR(?__f_num_2), "'"); SEPARATOR = ", ") AS ?__gc_3)
   WHERE {
    OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesExist> . }
    OPTIONAL { ?n <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesNotExist> . }
    OPTIONAL { { SELECT ?n ?__n_num_0 WHERE { ?n <http://tck.example.org/num> ?__n_num_0 . FILTER(BOUND(?n)) } } }
    OPTIONAL { { SELECT ?f ?__f_num_2 WHERE { ?f <http://tck.example.org/num> ?__f_num_2 . FILTER(BOUND(?f)) } } }
  }}
}"#);

    println!("\n=== Verify Test D: bound f in sub-SELECT ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesExist> . }
  OPTIONAL { SELECT ?f ?fnum WHERE { FILTER(BOUND(?f)) . ?f <http://tck.example.org/num> ?fnum } }
}"#);

    println!("\n=== Test: UNBOUND n in sub-SELECT - does it match wildcard? ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?n <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesNotExist> . }
  OPTIONAL { SELECT ?n ?nnum WHERE { FILTER(BOUND(?n)) . ?n <http://tck.example.org/num> ?nnum } }
}"#);

    println!("\n=== Test: UNBOUND n - sub-SELECT with FILTER BEFORE triple ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?n <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesNotExist> . }
  OPTIONAL { SELECT ?n ?nnum WHERE { FILTER(BOUND(?n)) ?n <http://tck.example.org/num> ?nnum } }
}"#);

    println!("\n=== External VALUES trick: prevent wildcard entirely ===");
    run_query(&store, r#"SELECT * WHERE {
  OPTIONAL { ?f <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesExist> . }
  OPTIONAL { ?n <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://tck.example.org/DoesNotExist> . }
  OPTIONAL { ?n <http://tck.example.org/num> ?nnum . FILTER(BOUND(?n)) }
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
                    let val = t.to_string();
                    format!("{}={}", v.as_str(), val)
                }).collect();
                println!("  row {}: {}", count, vars.join(", "));
            }
            println!("  Total: {} rows", count);
        }
        Ok(_) => println!("  Non-solution result"),
        Err(e) => println!("  Error: {}", e),
    }
}
// Test IF expression for numeric detection
