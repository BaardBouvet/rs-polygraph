fn main() {
    use oxigraph::store::Store;
    let store = Store::new().unwrap();
    
    let tests = vec![
        ("T3_pass", "SELECT ?a WHERE { { SELECT ?a WHERE { { SELECT ?a WHERE { VALUES (?a) { (1) } } } } LIMIT 1 } }"),
        ("T3_3var", "SELECT ?a ?b ?c WHERE { { SELECT ?a ?b ?c WHERE { { SELECT ?a ?b ?c WHERE { VALUES (?a ?b ?c) { (1 2 3) } } } } LIMIT 1 } }"),
        ("T_bind_inner", "SELECT ?a ?b WHERE { { SELECT ?a ?b WHERE { { SELECT ?b ?a WHERE { VALUES (?a ?b) { (1 2) } BIND(?a AS ?b) } } } LIMIT 1 } }"),
        ("T_outervals", "SELECT ?a ?b WHERE { { SELECT ?a ?b WHERE { { SELECT ?a ?b WHERE { VALUES (?a ?b) { (1 2) } } } } LIMIT 1 } VALUES (?c) { (3) } }"),
    ];
    
    for (name, q) in &tests {
        match store.query(*q) {
            Ok(oxigraph::sparql::QueryResults::Solutions(mut sols)) => {
                let mut count = 0;
                while let Some(Ok(_)) = sols.next() { count += 1; }
                println!("{name}: OK ({count} rows)");
            }
            Err(e) => println!("{name}: ERROR: {e}"),
            _ => {}
        }
    }
}
