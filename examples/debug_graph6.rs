use polygraph::{sparql_engine::GenericSparql11, Transpiler};
const ENGINE: GenericSparql11 = GenericSparql11;
fn main() {
    let queries = [
        // Full WithOrderBy1[45] localtimes query
        r#"WITH [localtime({hour: 10, minute: 35}), localtime({hour: 12, minute: 31, second: 14, nanosecond: 645876123}), localtime({hour: 12, minute: 31, second: 14, nanosecond: 645876124}), localtime({hour: 12, minute: 35, second: 13}), localtime({hour: 12, minute: 30, second: 14, nanosecond: 645876123}), localtime({hour: 12, minute: 31, second: 15})] AS values
WITH values, size(values) AS numOfValues
UNWIND values AS value
WITH size([ x IN values WHERE x < value ]) AS x, value, numOfValues
  ORDER BY value
WITH numOfValues, collect(x) AS orderedX
RETURN orderedX = range(0, numOfValues-1) AS equal"#,
    ];
    for (i, q) in queries.iter().enumerate() {
        match Transpiler::cypher_to_sparql(q, &ENGINE) {
            Ok(r) => println!("OK[{}]: {}", i, r.sparql),
            Err(e) => println!("ERR[{}]: {e}", i),
        }
    }
}
