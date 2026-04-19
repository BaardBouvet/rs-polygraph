use oxigraph::{sparql::{QueryResults, SparqlEvaluator}, store::Store};
use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    // Test Precedence1[26] with comp = <>
    let queries = vec![
        // Test with comp = <>
        ("comp=<>", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b UNWIND [[], [true], [false], [null], [true, false], [true, false, null]] AS c WITH collect((a <> b IN c) = (a <> (b IN c))) AS eq, collect((a <> b IN c) <> ((a <> b) IN c)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        // Test with comp = =
        ("comp==", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b UNWIND [[], [true], [false], [null], [true, false], [true, false, null]] AS c WITH collect((a = b IN c) = (a = (b IN c))) AS eq, collect((a = b IN c) <> ((a = b) IN c)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        // Test simple in
        ("simple", "UNWIND [true, false] AS b UNWIND [[true], [false]] AS c RETURN b IN c AS result"),
        // boolop = AND
        ("boolop=AND", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b UNWIND [[], [true], [false], [null], [true, false], [true, false, null]] AS c WITH collect((a AND b IN c) = (a AND (b IN c))) AS eq, collect((a AND b IN c) <> ((a AND b) IN c)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
    ];
    
    let store = Store::new().unwrap();
    for (name, q) in queries {
        println!("--- {name} ---");
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(result) => {
                let sparql = &result.sparql;
                println!("SPARQL: {sparql}");
                #[expect(deprecated)]
                match store.query_opt(sparql, SparqlEvaluator::new()) {
                    Ok(QueryResults::Solutions(mut sol)) => {
                        while let Some(row_result) = sol.next() {
                            let r = row_result.unwrap();
                            println!("  result = {:?}", r.get("result"));
                            println!("  eq = {:?}", r.get("eq"));
                            println!("  neq = {:?}", r.get("neq"));
                        }
                    }
                    _ => println!("  unexpected query result"),
                }
            }
            Err(e) => println!("Translation error: {e}"),
        }
        println!();
    }
}
