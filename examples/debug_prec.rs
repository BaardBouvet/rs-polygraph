use polygraph::{sparql_engine::GenericSparql11, Transpiler};
use oxigraph::{sparql::{QueryResults, SparqlEvaluator}, store::Store};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let store = Store::new().unwrap();
    let q = "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b \
WITH collect((a >= b IS NULL) = (a >= (b IS NULL))) AS eq, \
     collect((a >= b IS NULL) <> ((a >= b) IS NULL)) AS neq \
RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result";
    match Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => {
            println!("SPARQL:\n{}\n", r.sparql);
            #[expect(deprecated)]
            match store.query_opt(r.sparql.as_str(), SparqlEvaluator::new()) {
                Ok(QueryResults::Solutions(mut s)) => {
                    while let Some(row) = s.next() {
                        let row = row.unwrap();
                        let vars: Vec<_> = row.iter().map(|(v, t)| format!("{}={}", v.as_str(), t)).collect();
                        println!("  RESULT: {}", vars.join(", "));
                    }
                }
                Err(e) => println!("  SPARQL ERROR: {}", e),
                _ => {}
            }
        }
        Err(e) => println!("ERR: {e}"),
    }
}
