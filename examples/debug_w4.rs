use polygraph::{Transpiler, target::GenericSparql11};

fn main() {
    let engine = GenericSparql11;
    
    // Verify pattern predicate parsing
    let q = "MATCH (a) WHERE (a)-[:T*]->(b:Foo) RETURN a";
    let ast = Transpiler::parse_cypher(q).unwrap();
    println!("AST: {:#?}", ast);
    
    match Transpiler::cypher_to_sparql(q, &engine) {
        Ok(output) => println!("\nSPARQL: {}", output.sparql),
        Err(e) => println!("\nERROR: {}", e),
    }
    
    // MatchWhere4[2]
    let q2 = "MATCH (a), (b) WHERE a.id = 0 AND (a)-[:T]->(b:TheLabel) OR (a)-[:T*]->(b:MissingLabel) RETURN DISTINCT b";
    match Transpiler::cypher_to_sparql(q2, &engine) {
        Ok(output) => println!("\nMatchWhere4[2]: {}", output.sparql),
        Err(e) => println!("\nMatchWhere4[2] ERROR: {}", e),
    }

    // Match6[14]: Undirected typed *3..3
    let q3 = "MATCH topRoute = (:Start)<-[:CONNECTED_TO]-()-[:CONNECTED_TO*3..3]-(:End) RETURN topRoute";
    match Transpiler::cypher_to_sparql(q3, &engine) {
        Ok(output) => println!("\nMatch6[14] SPARQL:\n{}", output.sparql),
        Err(e) => println!("\nMatch6[14] ERROR: {}", e),
    }

    // Match7[14]: *3.. undirected
    let q4 = "MATCH (a:Single) OPTIONAL MATCH (a)-[*3..]-(b) RETURN b";
    match Transpiler::cypher_to_sparql(q4, &engine) {
        Ok(output) => println!("\nMatch7[14] SPARQL:\n{}", output.sparql),
        Err(e) => println!("\nMatch7[14] ERROR: {}", e),
    }

    // Match9[5]: count(r) on varlen
    let q5 = "MATCH (a:Blue)-[r*]->(b:Green) RETURN count(r)";
    match Transpiler::cypher_to_sparql(q5, &engine) {
        Ok(output) => println!("\nMatch9[5] SPARQL:\n{}", output.sparql),
        Err(e) => println!("\nMatch9[5] ERROR: {}", e),
    }

    // Match4[5]: per-hop property filter
    let q6 = "MATCH (a:Artist)-[:WORKED_WITH* {year: 1988}]->(b:Artist) RETURN *";
    match Transpiler::cypher_to_sparql(q6, &engine) {
        Ok(output) => println!("\nMatch4[5] SPARQL:\n{}", output.sparql),
        Err(e) => println!("\nMatch4[5] ERROR: {}", e),
    }

    // Return4[11]: head(collect) peephole
    let q7 = "MATCH (person:Person)<--(message)<-[like]-(:Person) WITH like.creationDate AS likeTime, person AS person ORDER BY likeTime, message.id WITH head(collect({likeTime: likeTime})) AS latestLike, person AS person RETURN latestLike.likeTime AS likeTime ORDER BY likeTime";
    match Transpiler::cypher_to_sparql(q7, &engine) {
        Ok(output) => println!("\nReturn4[11] SPARQL:\n{}", output.sparql),
        Err(e) => println!("\nReturn4[11] ERROR: {}", e),
    }
}
