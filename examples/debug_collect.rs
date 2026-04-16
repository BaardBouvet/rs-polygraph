use polygraph::{sparql_engine::GenericSparql11, Transpiler};
use oxigraph::{sparql::{QueryResults, SparqlEvaluator}, store::Store};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        "WITH collect(1) AS c RETURN c",
        "UNWIND [1, 2, null] AS x WITH collect(x) AS c RETURN c",
        "UNWIND ['a', 'b'] AS x WITH collect(x) AS c RETURN c",
        "UNWIND [null, 1, null] AS x RETURN collect(DISTINCT x) AS c",
    ];
    let store = Store::new().unwrap();
    for q in queries.iter() {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => {
                println!("Q: {}\nSPARQL: {}", q, r.sparql);
                #[expect(deprecated)]
                match store.query_opt(r.sparql.as_str(), SparqlEvaluator::new()) {
                    Ok(QueryResults::Solutions(mut s)) => {
                        while let Some(row) = s.next() {
                            let row = row.unwrap();
                            let vars: Vec<_> = row.iter().map(|(v, t)| format!("{}={}", v.as_str(), t)).collect();
                            println!("  RESULT: {}", vars.join(", "));
                        }
                    }
                    Err(e) => println!("  ERROR: {}", e),
                    _ => {}
                }
                println!();
            }
            Err(e) => println!("Q: {}\nERR: {}\n", q, e),
        }
    }
}
