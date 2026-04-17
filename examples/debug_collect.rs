use polygraph::Transpiler;
use oxigraph::store::Store;
use oxigraph::sparql::QueryResults;

struct TckEngine;
impl polygraph::sparql_engine::TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://tck.example.org/") }
}
const ENGINE: TckEngine = TckEngine;

fn run_query(store: &Store, name: &str, q: &str) {
    print!("\n=== {} === ", name);
    match Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => {
            match store.query(&r.sparql) {
                Ok(QueryResults::Solutions(mut sols)) => {
                    let mut found = false;
                    while let Some(Ok(sol)) = sols.next() {
                        let res = sol.get("result").map(|t| t.to_string()).unwrap_or("UNDEF".to_string());
                        print!("result={}", res);
                        found = true;
                    }
                    if !found { print!("(0 rows)"); }
                }
                Ok(_) => print!("Non-tabular"),
                Err(e) => print!("Query error: {}", e),
            }
            println!();
        }
        Err(e) => println!("ERR: {}", e),
    }
}

fn run_query_verbose(store: &Store, name: &str, q: &str) {
    println!("\n=== {} ===", name);
    match Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => {
            println!("SPARQL:\n{}", r.sparql);
            match store.query(&r.sparql) {
                Ok(QueryResults::Solutions(mut sols)) => {
                    while let Some(Ok(sol)) = sols.next() {
                        let res = sol.get("result").map(|t| t.to_string()).unwrap_or("UNDEF".to_string());
                        println!("  result={}", res);
                    }
                }
                Ok(_) => println!("Non-tabular"),
                Err(e) => println!("Query error: {}", e),
            }
        }
        Err(e) => println!("ERR: {}", e),
    }
}

fn main() {
    let store = Store::new().unwrap();
    let queries = vec![
        ("= IS NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a = b IS NULL) = (a = (b IS NULL))) AS eq, collect((a = b IS NULL) <> ((a = b) IS NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        ("= IS NOT NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a = b IS NOT NULL) = (a = (b IS NOT NULL))) AS eq, collect((a = b IS NOT NULL) <> ((a = b) IS NOT NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        ("<= IS NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a <= b IS NULL) = (a <= (b IS NULL))) AS eq, collect((a <= b IS NULL) <> ((a <= b) IS NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        ("<= IS NOT NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a <= b IS NOT NULL) = (a <= (b IS NOT NULL))) AS eq, collect((a <= b IS NOT NULL) <> ((a <= b) IS NOT NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        (">= IS NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a >= b IS NULL) = (a >= (b IS NULL))) AS eq, collect((a >= b IS NULL) <> ((a >= b) IS NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        (">= IS NOT NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a >= b IS NOT NULL) = (a >= (b IS NOT NULL))) AS eq, collect((a >= b IS NOT NULL) <> ((a >= b) IS NOT NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        ("< IS NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a < b IS NULL) = (a < (b IS NULL))) AS eq, collect((a < b IS NULL) <> ((a < b) IS NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        ("< IS NOT NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a < b IS NOT NULL) = (a < (b IS NOT NULL))) AS eq, collect((a < b IS NOT NULL) <> ((a < b) IS NOT NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        ("> IS NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a > b IS NULL) = (a > (b IS NULL))) AS eq, collect((a > b IS NULL) <> ((a > b) IS NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        ("> IS NOT NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a > b IS NOT NULL) = (a > (b IS NOT NULL))) AS eq, collect((a > b IS NOT NULL) <> ((a > b) IS NOT NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        ("<> IS NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a <> b IS NULL) = (a <> (b IS NULL))) AS eq, collect((a <> b IS NULL) <> ((a <> b) IS NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
        ("<> IS NOT NULL [23]", "UNWIND [true, false, null] AS a UNWIND [true, false, null] AS b WITH collect((a <> b IS NOT NULL) = (a <> (b IS NOT NULL))) AS eq, collect((a <> b IS NOT NULL) <> ((a <> b) IS NOT NULL)) AS neq RETURN all(x IN eq WHERE x) AND any(x IN neq WHERE x) AS result"),
    ];
    for (name, q) in queries {
        run_query(&store, name, q);
    }
}
