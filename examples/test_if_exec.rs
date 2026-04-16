//! Test the IF(isLiteral && datatype...) approach in SPARQL directly
use oxigraph::{sparql::{QueryResults, SparqlEvaluator}, store::Store};

fn main() {
    let store = Store::new().unwrap();
    
    // Test 1: UNWIND [1, 2, null] collect with IF on inner query
    let q = r#"SELECT ?c WHERE { { SELECT (CONCAT("[", COALESCE(?gc, ""), "]") AS ?c) WHERE { {SELECT (GROUP_CONCAT(IF((isLITERAL(?x) && (((DATATYPE(?x) = <http://www.w3.org/2001/XMLSchema#integer>) || (DATATYPE(?x) = <http://www.w3.org/2001/XMLSchema#double>)) || (DATATYPE(?x) = <http://www.w3.org/2001/XMLSchema#boolean>))), STR(?x), CONCAT("'", STR(?x), "'")); SEPARATOR = ", ") AS ?gc) WHERE { VALUES ( ?x ) { ( "1"^^<http://www.w3.org/2001/XMLSchema#integer> ) ( "2"^^<http://www.w3.org/2001/XMLSchema#integer> ) ( UNDEF )  } FILTER(BOUND(?x)) }} } } }"#;
    println!("Test 1 (UNWIND [1, 2, null]):");
    run_query(&store, q);
    
    // Test 2: Simple VALUES with IF 
    let q2 = r#"SELECT (GROUP_CONCAT(IF((isLITERAL(?x) && DATATYPE(?x) = <http://www.w3.org/2001/XMLSchema#integer>), STR(?x), CONCAT("'", STR(?x), "'")) ; SEPARATOR=", ") AS ?gc) WHERE { VALUES (?x) { ("1"^^<http://www.w3.org/2001/XMLSchema#integer>) ("hello") } }"#;
    println!("Test 2 (direct VALUES IF):");
    run_query(&store, q2);
    
    // Test 3: Check IF expression without GROUP_CONCAT 
    let q3 = r#"SELECT (IF(isLITERAL(?x) && DATATYPE(?x) = <http://www.w3.org/2001/XMLSchema#integer>, STR(?x), CONCAT("'", STR(?x), "'")) AS ?v) WHERE { VALUES (?x) { ("1"^^<http://www.w3.org/2001/XMLSchema#integer>) ("hello") } }"#;
    println!("Test 3 (no GROUP_CONCAT):");
    run_query(&store, q3);
}

fn run_query(store: &Store, q: &str) {
    #[expect(deprecated)]
    match store.query_opt(q, SparqlEvaluator::new()) {
        Ok(QueryResults::Solutions(mut s)) => {
            let mut count = 0;
            while let Some(row) = s.next() {
                let row = row.unwrap();
                count += 1;
                let vars: Vec<_> = row.iter().map(|(v, t)| format!("{}={}", v.as_str(), t)).collect();
                println!("  row {}: {}", count, vars.join(", "));
            }
            if count == 0 { println!("  (no rows)"); }
        }
        Err(e) => println!("  Error: {}", e),
        _ => println!("  Non-solution"),
    }
}
