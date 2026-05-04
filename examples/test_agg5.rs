use oxigraph::store::Store;
use oxigraph::sparql::QueryResults;
fn main() {
    let s = Store::new().unwrap();
    s.update("INSERT DATA {
      <http://e/n0> <http://e/name> \"a\" . <http://e/n0> <http://e/num> \"33\"^^<http://www.w3.org/2001/XMLSchema#integer> .
      <http://e/n1> <http://e/name> \"a\" .
      <http://e/n2> <http://e/name> \"a\" . <http://e/n2> <http://e/num> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .
    }").unwrap(); 

    // Test: does GROUP BY + SUM with OPTIONAL work?
    let queries = vec![
        ("SUM no optional (2 nodes)", "SELECT (SUM(?num) AS ?total) WHERE { ?n <http://e/name> ?name . ?n <http://e/num> ?num } GROUP BY ?name"),
        ("SUM with optional (3 nodes, one unbound)", "SELECT (SUM(?num) AS ?total) WHERE { ?n <http://e/name> ?name . OPTIONAL { ?n <http://e/num> ?num } } GROUP BY ?name"),
        ("SUM+name no optional", "SELECT ?name (SUM(?num) AS ?total) WHERE { ?n <http://e/name> ?name . ?n <http://e/num> ?num } GROUP BY ?name"),
    ];
    for (label, q) in &queries {
        println!("{label}:");
        match s.query(*q) {
            Ok(QueryResults::Solutions(sols)) => {
                for sol in sols { let sol = sol.unwrap(); let b: Vec<_> = sol.iter().map(|(n,v)| format!("{n}={v}")).collect(); println!("  {:?}", b); }
            }
            Err(e) => println!("  Error: {e}"),
            _ => {}
        }
    }
}
