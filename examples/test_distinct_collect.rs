use oxigraph::{sparql::{QueryResults, SparqlEvaluator}, store::Store};
fn main() {
    let store = Store::new().unwrap();
    
    // Direct: GROUP_CONCAT DISTINCT IF with UNDEF values (no filter)
    let q1 = r#"SELECT (CONCAT("[", COALESCE(?gc, ""), "]") AS ?c) WHERE {
      {SELECT (GROUP_CONCAT(DISTINCT IF((isLITERAL(?x) && (DATATYPE(?x) = <http://www.w3.org/2001/XMLSchema#integer>)), STR(?x), CONCAT("'", STR(?x), "'")); SEPARATOR = ", ") AS ?gc) WHERE {
        VALUES ( ?x ) { ( UNDEF ) ( "1"^^<http://www.w3.org/2001/XMLSchema#integer> ) ( UNDEF ) }
      }}}"#;
    println!("New: GROUP_CONCAT DISTINCT IF (no filter):");
    run_query(&store, q1);
    
    // Old: GROUP_CONCAT DISTINCT STR with UNDEF (no filter) 
    let q2 = r#"SELECT (CONCAT("[", COALESCE(?gc, ""), "]") AS ?c) WHERE {
      {SELECT (GROUP_CONCAT(DISTINCT CONCAT("'", STR(?x), "'"); SEPARATOR = ", ") AS ?gc) WHERE {
        VALUES ( ?x ) { ( UNDEF ) ( "1"^^<http://www.w3.org/2001/XMLSchema#integer> ) ( UNDEF ) }
      }}}"#;
    println!("Old: GROUP_CONCAT DISTINCT CONCAT STR (no filter):");
    run_query(&store, q2);
    
    // New with FILTER(BOUND):
    let q3 = r#"SELECT (CONCAT("[", COALESCE(?gc, ""), "]") AS ?c) WHERE {
      {SELECT (GROUP_CONCAT(DISTINCT IF((isLITERAL(?x) && (DATATYPE(?x) = <http://www.w3.org/2001/XMLSchema#integer>)), STR(?x), CONCAT("'", STR(?x), "'")); SEPARATOR = ", ") AS ?gc) WHERE {
        VALUES ( ?x ) { ( UNDEF ) ( "1"^^<http://www.w3.org/2001/XMLSchema#integer> ) ( UNDEF ) }
        FILTER(BOUND(?x))
      }}}"#;
    println!("New + FILTER(BOUND):");
    run_query(&store, q3);
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
