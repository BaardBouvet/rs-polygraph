use oxigraph::store::Store;
use oxigraph::sparql::QueryResults;

fn test(store: &Store, name: &str, q: &str) {
    let mut out = Vec::new();
    match store.query(q) {
        Ok(QueryResults::Solutions(mut sols)) => {
            while let Some(s) = sols.next() {
                match s {
                    Ok(sol) => {
                        let vars: Vec<String> = sol.iter().map(|(v,t)| format!("{}={}", v.as_str(), t.to_string())).collect();
                        out.push(vars.join(" | "));
                    }
                    Err(e) => out.push(format!("ERR:{}", e)),
                }
            }
        }
        Ok(_) => out.push("Non-tabular".to_string()),
        Err(e) => out.push(format!("QueryErr:{}", e)),
    }
    println!("[{}] rows={}: {}", name, out.len(), out.join(" || "));
}

fn main() {
    let store = Store::new().unwrap();
    
    let xsd = "<http://www.w3.org/2001/XMLSchema#boolean>";
    let t = format!("\"true\"^^{}", xsd);
    let f = format!("\"false\"^^{}", xsd);
    
    // Test 1: Simple BIND in GC
    test(&store, "simple BIND in GC",
        &format!("SELECT (GROUP_CONCAT(STR(?v); SEPARATOR=\",\") AS ?gc) WHERE {{ VALUES (?a) {{ ({t} ) ({f}) }} BIND(!?a AS ?v) }}"));
    
    // Test 2: Two VALUES + BIND(||) 
    test(&store, "2 VALUES + BIND(||)",
        &format!("SELECT (GROUP_CONCAT(STR(?x); SEPARATOR=\",\") AS ?gc) WHERE {{ VALUES (?a) {{ ({t}) (UNDEF) }} VALUES (?b) {{ ({t}) (UNDEF) }} BIND((?a || ?b) AS ?x) }}"));
    
    // Test 3: BIND in GC no-agg (raw VALUES)
    test(&store, "no-agg VALUES with UNDEF",
        &format!("SELECT ?a ?b ?x WHERE {{ VALUES (?a) {{ ({t}) (UNDEF) }} VALUES (?b) {{ ({t}) (UNDEF) }} BIND((?a || ?b) AS ?x) }}"));
    
    // Test 4: Two VALUES no UNDEF + complex expr GC
    test(&store, "2 VALUES no UNDEF + complex GC", 
        &format!("SELECT (GROUP_CONCAT(STR(?v); SEPARATOR=\",\") AS ?gc) WHERE {{ VALUES (?a) {{ ({t}) ({f}) }} VALUES (?b) {{ ({t}) ({f}) }} BIND((?a || ?b) AS ?x) BIND(!((?a || !BOUND(?b)) = !BOUND(?x)) AS ?v) }}"));

    // Test 5: Two VALUES WITH UNDEF + complex expr GC (same as failing test 3 from before)
    test(&store, "2 VALUES WITH UNDEF + complex GC",
        &format!("SELECT (GROUP_CONCAT(STR(?v); SEPARATOR=\",\") AS ?gc) WHERE {{ VALUES (?a) {{ ({t}) (UNDEF) }} VALUES (?b) {{ ({t}) (UNDEF) }} BIND((?a || ?b) AS ?x) BIND(!((?a || !BOUND(?b)) = !BOUND(?x)) AS ?v) }}"));
}
