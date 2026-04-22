use polygraph::{Transpiler, sparql_engine::TargetEngine};
use oxigraph::{store::Store, sparql::{QueryResults, SparqlEvaluator}};

struct TckEngine;
impl TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}

fn main() {
    let queries = vec![
        "RETURN duration.inSeconds(date('1984-10-11'), date('2015-06-24')) AS duration",
        "RETURN duration.inDays(date('1984-10-11'), date('2015-06-24')) AS duration",
        "RETURN duration.inDays(datetime('2014-07-21T21:40:36.143+0200'), date('2015-06-24')) AS d",
    ];
    
    let store = Store::new().unwrap();
    
    for q in queries {
        let sparql = Transpiler::cypher_to_sparql(q, &TckEngine).unwrap().sparql;
        println!("SPARQL: {sparql}");
        #[expect(deprecated)]
        if let Ok(QueryResults::Solutions(mut sols)) = store.query_opt(&sparql, SparqlEvaluator::new()) {
            let vars: Vec<_> = sols.variables().iter().map(|v| v.as_str().to_owned()).collect();
            for sol in sols.by_ref() {
                let sol = sol.unwrap();
                for v in &vars {
                    println!("  {v} = {:?}", sol.get(v.as_str()));
                }
            }
        }
        println!();
    }
}
