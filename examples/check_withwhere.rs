use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        "MATCH (a) WITH a.name2 AS name WHERE a.name2 = 'B' RETURN *",
        "MATCH (a) WITH a.name2 AS name WHERE name = 'B' RETURN *",
        "MATCH (m:Movie { rating: 4 }) WITH * MATCH (n) RETURN toFloat(n.rating) AS float",
    ];
    for q in &queries {
        print!("Q: {} => ", q);
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("{}", r.sparql),
            Err(e) => println!("Err: {}", e),
        }
    }
}

fn check_orderby_count() {
    let q = "MATCH (n) RETURN n.division, count(*) ORDER BY count(*) DESC, n.division ASC";
    match polygraph::Transpiler::cypher_to_sparql(q, &ENGINE) {
        Ok(r) => println!("SPARQL:\n{}", r.sparql),
        Err(e) => println!("Err: {}", e),
    }
}
