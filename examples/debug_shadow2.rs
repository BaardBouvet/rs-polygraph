use oxigraph::{sparql::QueryResults, store::Store};

fn main() {
    let store = Store::new().unwrap();
    // Simplified test for SPARQL variable rebinding
    let sqls = [
        // Case 1: Nested SELECT rebinding ?a (like WITH 1 AS a, WITH 2 AS a)
        r#"SELECT ("2"^^<http://www.w3.org/2001/XMLSchema#integer> AS ?a) WHERE { { SELECT ("1"^^<http://www.w3.org/2001/XMLSchema#integer> AS ?a) WHERE {} } }"#,
        // Case 2: Like the failing query - rebind ?x in nested subquery
        r#"SELECT ?a (?__num1 + ?__num2 AS ?x) WHERE { { SELECT ?a ("5"^^<http://www.w3.org/2001/XMLSchema#integer> AS ?x) WHERE { BIND ("test"^^<http://www.w3.org/2001/XMLSchema#string> AS ?a) } } BIND ("3"^^<http://www.w3.org/2001/XMLSchema#integer> AS ?__num1) BIND ("4"^^<http://www.w3.org/2001/XMLSchema#integer> AS ?__num2) }"#,
    ];
    for (i, q) in sqls.iter().enumerate() {
        match store.query(*q) {
            Ok(QueryResults::Solutions(mut sol)) => {
                if let Some(Ok(row)) = sol.next() {
                    println!("Case {}: {:?}", i+1, row);
                } else {
                    println!("Case {}: no rows", i+1);
                }
            }
            Err(e) => println!("Case {} error: {e}", i+1),
            _ => {}
        }
    }
}
