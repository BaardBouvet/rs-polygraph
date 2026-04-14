use polygraph::{Transpiler, sparql_engine::TargetEngine};

struct TckEngine;
impl TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}

fn main() {
    use oxigraph::store::Store;
    use oxigraph::model::*;
    use oxigraph::sparql::{QueryResults, SparqlEvaluator};
    
    // Create the RDF graph matching TCK setup
    let store = Store::new().unwrap();
    let base = "http://tck.example.org/";
    
    let rdf_type = NamedNode::new_unchecked("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let node_class = NamedNode::new_unchecked(format!("{base}Node"));
    let edge_pred = NamedNode::new_unchecked(format!("{base}EDGE"));
    let node_sentinel = NamedNode::new_unchecked(format!("{base}__node"));
    let default_graph = GraphName::DefaultGraph;
    
    let n0 = BlankNode::new_unchecked("n0");
    let n1 = BlankNode::new_unchecked("n1");
    let n2 = BlankNode::new_unchecked("n2");
    let n3 = BlankNode::new_unchecked("n3");
    
    for n in [&n0, &n1, &n2, &n3] {
        let nn = NamedOrBlankNode::BlankNode(n.clone());
        store.insert(&Quad::new(nn.clone(), rdf_type.clone(), node_class.clone(), default_graph.clone())).unwrap();
        store.insert(&Quad::new(nn.clone(), node_sentinel.clone(), node_sentinel.clone(), default_graph.clone())).unwrap();
    }
    for (s, o) in [(&n0, &n1), (&n1, &n2), (&n2, &n3)] {
        store.insert(&Quad::new(
            NamedOrBlankNode::BlankNode(s.clone()),
            edge_pred.clone(),
            Term::BlankNode(o.clone()),
            default_graph.clone()
        )).unwrap();
    }
    
    // Debug step by step
    // Generate and check SPARQL
    let q47 = r#"MATCH ()-[r:EDGE]-()
MATCH p = (n)-[*0..1]-()-[r]-()-[*0..1]-(m)
RETURN count(p) AS c"#;
    
    let sparql = Transpiler::cypher_to_sparql(q47, &TckEngine)
        .expect("translation failed")
        .sparql;
    
    println!("SPARQL:\n{}\n", sparql);
    
    #[expect(deprecated)]
    match store.query_opt(sparql.as_str(), SparqlEvaluator::new()) {
        Ok(QueryResults::Solutions(mut solutions)) => {
            for sol_result in solutions.by_ref() {
                println!("Result: {:?}", sol_result.unwrap());
            }
        },
        Ok(_) => println!("Not solutions"),
        Err(e) => println!("SPARQL Error: {}", e),
    }
    for (name, sparql) in &[] as &[(&str, &str)] {
        print!("'{}': ", name);
        #[expect(deprecated)]
        match store.query_opt(*sparql, SparqlEvaluator::new()) {
            Ok(QueryResults::Solutions(mut solutions)) => {
                let mut cnt = 0;
                for sol_result in solutions.by_ref() {
                    let row = sol_result.unwrap();
                    println!("{:?}", row);
                    cnt += 1;
                    if cnt > 3 { break; }
                }
                if cnt == 0 { println!("(0 rows)"); }
            },
            Ok(_) => println!("Not solutions"),
            Err(e) => println!("SPARQL Error: {}", e),
        }
    }
}

