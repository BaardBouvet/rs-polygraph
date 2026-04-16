//! Test what happens with nulls in GROUP_CONCAT  
use oxigraph::{sparql::{QueryResults, SparqlEvaluator}, store::Store};

fn main() {
    let store = Store::new().unwrap();
    
    println!("=== Test UNDEF handling in GROUP_CONCAT ===\n");
    
    // Old approach: CONCAT("'", STR(?x), "'") -- does UNDEF skip?
    let q1 = r#"SELECT (GROUP_CONCAT(CONCAT("'", STR(?x), "'"); SEPARATOR=", ") AS ?gc) WHERE { VALUES (?x) { (UNDEF) ("1"^^<http://www.w3.org/2001/XMLSchema#integer>) (UNDEF) } }"#;
    println!("Old approach (CONCAT STR):");
    run_query(&store, q1);
    
    // New approach: IF(isLiteral && ..., STR, CONCAT) -- does UNDEF skip?
    let q2 = r#"SELECT (GROUP_CONCAT(IF((isLITERAL(?x) && DATATYPE(?x) = <http://www.w3.org/2001/XMLSchema#integer>), STR(?x), CONCAT("'", STR(?x), "'")) ; SEPARATOR=", ") AS ?gc) WHERE { VALUES (?x) { (UNDEF) ("1"^^<http://www.w3.org/2001/XMLSchema#integer>) (UNDEF) } }"#;
    println!("New approach (IF ISLITERAL):");
    run_query(&store, q2);
    
    // What does isLITERAL(UNDEF) return?
    let q3 = r#"SELECT (isLITERAL(?x) AS ?lit) WHERE { VALUES (?x) { (UNDEF) ("1"^^<http://www.w3.org/2001/XMLSchema#integer>) } }"#;
    println!("isLITERAL of UNDEF:");
    run_query(&store, q3);
    
    // What does IF(false, "yes", "no") with UNDEF condition return?
    let q4 = r#"SELECT (IF(isLITERAL(?x), STR(?x), "null") AS ?v) WHERE { VALUES (?x) { (UNDEF) ("1"^^<http://www.w3.org/2001/XMLSchema#integer>) } }"#;
    println!("IF(isLiteral(undef), ...):");
    run_query(&store, q4);
}

fn run_query(store: &Store, q: &str) {
    #[expect(deprecated)]
    match store.query_opt(q, SparqlEvaluator::new()) {
        Ok(QueryResults::Solutions(mut s)) => {
            while let Some(row) = s.next() {
                let row = row.unwrap();
                let vars: Vec<_> = row.iter().map(|(v, t)| format!("{}={}", v.as_str(), t)).collect();
                println!("  {}", vars.join(", "));
            }
        }
        Err(e) => println!("  Error: {}", e),
        _ => {}
    }
    println!();
}
/* We need more tests - let me recreate the file */
