use oxigraph::store::Store;
use oxigraph::sparql::QueryResults;
use polygraph::{Transpiler, sparql_engine::TargetEngine};

struct TckEngine;
impl TargetEngine for TckEngine {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn base_iri(&self) -> Option<&str> { Some("http://polygraph.example/") }
}
const ENGINE: TckEngine = TckEngine;

const BASE: &str = "http://polygraph.example/";
const NODE: &str = "http://polygraph.example/__node";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

fn insert(s: &Store, sparql: &str) {
    s.update(sparql).unwrap_or_else(|e| eprintln!("INSERT_ERR: {e}"));
}

fn run(s: &Store, cypher: &str) {
    match Transpiler::cypher_to_sparql(cypher, &ENGINE) {
        Ok(r) => {
            let sparql = r.sparql().unwrap();
            match s.query(sparql) {
                Ok(QueryResults::Solutions(sols)) => {
                    let rows: Vec<_> = sols
                        .map(|sol| {
                            sol.unwrap()
                                .iter()
                                .map(|(n, v)| format!("{n}={v}"))
                                .collect::<Vec<_>>()
                        })
                        .collect();
                    println!("  {} rows: {:?}", rows.len(), &rows[..rows.len().min(5)]);
                }
                Ok(_) => println!("  non-select"),
                Err(e) => println!("  EXEC_ERR: {e}"),
            }
        }
        Err(e) => println!("  TRANS_ERR: {e}"),
    }
}

fn main() {
    // Aggregation3[1]: MATCH (n) RETURN n.name, sum(n.num)
    println!("Aggregation3[1]:");
    let s = Store::new().unwrap();
    insert(&s, &format!("INSERT DATA {{
      <{BASE}n0> <{NODE}> <{NODE}> .
      <{BASE}n0> <{BASE}name> \"a\" .
      <{BASE}n0> <{BASE}num> \"33\"^^<{XSD}integer> .
      <{BASE}n1> <{NODE}> <{NODE}> .
      <{BASE}n1> <{BASE}name> \"a\" .
      <{BASE}n2> <{NODE}> <{NODE}> .
      <{BASE}n2> <{BASE}name> \"a\" .
      <{BASE}n2> <{BASE}num> \"42\"^^<{XSD}integer> .
    }}"));
    run(&s, "MATCH (n) RETURN n.name, sum(n.num)");
    println!("  Expected: [['name=a', 'sum=75']]");

    // Return4[7]: MATCH p = (n)-->(b) RETURN avg(n.age)
    println!("Return4[7]:");
    let s2 = Store::new().unwrap();
    insert(&s2, &format!("INSERT DATA {{ <{BASE}n0> <{NODE}> <{NODE}> . }}"));
    run(&s2, "MATCH p = (n)-->(b) RETURN avg(n.age)");
    println!("  Expected: 1 row with null");

    // Match3[28]: MATCH (n) WHERE n.prop = null RETURN n  
    println!("Match3[28]:");
    let s3 = Store::new().unwrap();
    insert(&s3, &format!("INSERT DATA {{
      <{BASE}n0> <{NODE}> <{NODE}> . <{BASE}n0> <{BASE}prop> \"1\"^^<{XSD}integer> .
      <{BASE}n1> <{NODE}> <{NODE}> .
    }}"));
    run(&s3, "MATCH (n) WHERE n.prop = null RETURN n");
    println!("  Expected: 1 row (node without prop)");

    // Return2[7]: MATCH (a) RETURN a.list2 + a.list1 AS foo  
    println!("Return2[7]:");
    let s4 = Store::new().unwrap();
    insert(&s4, &format!("INSERT DATA {{
      <{BASE}n0> <{NODE}> <{NODE}> .
      <{BASE}n0> <{BASE}list1> \"[1, 2, 3]\" .
      <{BASE}n0> <{BASE}list2> \"[4, 5]\" .
    }}"));
    run(&s4, "MATCH (a) RETURN a.list2 + a.list1 AS foo");
    println!("  Expected: [4, 5, 1, 2, 3]");
    
    // Graph4[4]: type(r) with OPTIONAL MATCH
    println!("Graph4[4]:");
    let s5 = Store::new().unwrap();
    insert(&s5, &format!("INSERT DATA {{
      <{BASE}n0> <{NODE}> <{NODE}> .
      <{BASE}n1> <{NODE}> <{NODE}> .
      <{BASE}n0> <{BASE}T> <{BASE}n1> .
    }}"));
    run(&s5, "MATCH (a) OPTIONAL MATCH (a)-[r:T]->() RETURN type(r)");
    println!("  Expected: 2 rows ['T', null]");
}
