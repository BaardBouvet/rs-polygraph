use polygraph::parser::parse_cypher;
fn main() {
    let tests = vec![
        "MATCH (n) WHERE n.age > 18 RETURN n",
        "MATCH (n:Person) WHERE n.age > 30 RETURN n",
        "MATCH (n:Person) WITH n WHERE n.age > 18 RETURN n",
    ];
    for q in tests {
        match parse_cypher(q) {
            Ok(_) => println!("OK: {q}"),
            Err(e) => println!("FAIL: {q}\n  {e}"),
        }
    }
}
