use oxigraph::store::Store;
use oxigraph::sparql::QueryResults;
use polygraph::Transpiler;
fn main() {
    struct TCK;
    impl polygraph::target::TargetEngine for TCK {
        fn supports_rdf_star(&self) -> bool { true }
        fn supports_federation(&self) -> bool { false }
        fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
    }
    let engine = TCK;
    
    let store = Store::new().unwrap();
    
    // Simulate the TCK setup
    let create_cypher = "CREATE (a {name: 'A'}),\n  (b {name: 'B'}),\n  (c {name: 'C'}),\n  (a)-[:KNOWS]->(b),\n  (a)-[:HATES]->(c),\n  (a)-[:WONDERS]->(c)";
    
    // Use the actual TCK create_to_insert_data logic via a direct call
    let sparql = Transpiler::cypher_to_sparql(
        "MATCH (n)-[r]->(x) WHERE type(r) = 'KNOWS' OR type(r) = 'HATES' RETURN r",
        &engine
    ).unwrap().sparql;
    println!("Query SPARQL: {sparql}");
    
    // Manually insert the data  
    let insert = r#"INSERT DATA {
        _:a <http://tck.example.org/__node> <http://tck.example.org/__node> .
        _:a <http://tck.example.org/name> "A" .
        _:b <http://tck.example.org/__node> <http://tck.example.org/__node> .
        _:b <http://tck.example.org/name> "B" .
        _:c <http://tck.example.org/__node> <http://tck.example.org/__node> .
        _:c <http://tck.example.org/name> "C" .
        _:a <http://tck.example.org/KNOWS> _:b .
        _:a <http://tck.example.org/HATES> _:c .
        _:a <http://tck.example.org/WONDERS> _:c .
    }"#;
    store.update(insert).unwrap();
    
    match store.query(&sparql) {
        Ok(QueryResults::Solutions(mut sols)) => {
            let vars: Vec<_> = sols.variables().iter().map(|v| v.as_str().to_owned()).collect();
            let mut count = 0;
            for sol_r in sols.by_ref() {
                let _ = sol_r.unwrap();
                count += 1;
            }
            println!("Result rows: {count}");
        }
        Err(e) => println!("Error: {e}"),
        _ => {}
    }
}
