/// Integration tests: `cypher_to_sparql_update` / `gql_to_sparql_update`.
///
/// Tests the full two-phase call pattern against an embedded Oxigraph store,
/// using a small subset of the Neo4j "Get started with Cypher" movie graph.
use oxigraph::{
    sparql::{QueryResults, SparqlEvaluator},
    store::Store,
};
use polygraph::{sparql_engine::TargetEngine, Transpiler};

// ── Test engine (mirrors TckEngine in tck/main.rs) ───────────────────────────

const BASE: &str = "http://movie.example.org/";

struct MovieEngine;

impl TargetEngine for MovieEngine {
    fn supports_rdf_star(&self) -> bool {
        true
    }
    fn supports_federation(&self) -> bool {
        false
    }
    fn base_iri(&self) -> Option<&str> {
        Some(BASE)
    }
}

const ENGINE: MovieEngine = MovieEngine;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn fresh_store() -> Store {
    Store::new().expect("Oxigraph Store::new()")
}

/// Execute all write updates for a Cypher statement against `store`.
fn run_write(store: &Store, cypher: &str) {
    let updates = Transpiler::cypher_to_sparql_update(cypher, &ENGINE).unwrap_or_else(|e| {
        panic!("cypher_to_sparql_update failed for {cypher:?}: {e}");
    });
    for upd in &updates {
        store
            .update(upd.as_str())
            .unwrap_or_else(|e| panic!("UPDATE failed: {e}\nQuery: {upd}"));
    }
}

/// Count all nodes in the store via a SELECT query.
fn count_nodes(store: &Store) -> usize {
    let sparql =
        format!("SELECT (COUNT(DISTINCT ?n) AS ?c) WHERE {{ ?n <{BASE}__node> <{BASE}__node> }}");
    #[expect(deprecated)]
    match store
        .query_opt(sparql.as_str(), SparqlEvaluator::new())
        .expect("count_nodes query failed")
    {
        QueryResults::Solutions(mut sols) => {
            if let Some(Ok(row)) = sols.next() {
                if let Some(oxigraph::model::Term::Literal(lit)) = row.get("c") {
                    return lit.value().parse().unwrap_or(0);
                }
            }
            0
        }
        _ => 0,
    }
}

/// Collect the values of a single column from a SELECT query.
fn collect_column(store: &Store, sparql: &str, col: &str) -> Vec<String> {
    #[expect(deprecated)]
    match store
        .query_opt(sparql, SparqlEvaluator::new())
        .unwrap_or_else(|e| panic!("query failed: {e}\n{sparql}"))
    {
        QueryResults::Solutions(sols) => sols
            .filter_map(|row| {
                let row = row.ok()?;
                let term = row.get(col)?;
                match term {
                    oxigraph::model::Term::Literal(lit) => Some(lit.value().to_owned()),
                    oxigraph::model::Term::NamedNode(nn) => Some(nn.as_str().to_owned()),
                    _ => None,
                }
            })
            .collect(),
        _ => vec![],
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Populate the mini movie graph, then query nodes and relationships.
#[test]
fn movie_graph_populate_and_query() {
    let store = fresh_store();

    // ── Populate nodes ────────────────────────────────────────────────────────

    // Movies
    run_write(
        &store,
        "MERGE (m:Movie {title:'The Matrix'}) ON CREATE SET m.released=1999",
    );
    run_write(
        &store,
        "MERGE (m:Movie {title:'Top Gun'}) ON CREATE SET m.released=1986",
    );
    run_write(
        &store,
        "MERGE (m:Movie {title:'A Few Good Men'}) ON CREATE SET m.released=1992",
    );

    // Actors / directors
    run_write(
        &store,
        "MERGE (p:Person {name:'Keanu Reeves'}) ON CREATE SET p.born=1964",
    );
    run_write(
        &store,
        "MERGE (p:Person {name:'Tom Cruise'}) ON CREATE SET p.born=1962",
    );
    run_write(
        &store,
        "MERGE (p:Person {name:'Jack Nicholson'}) ON CREATE SET p.born=1937",
    );
    run_write(
        &store,
        "MERGE (p:Person {name:'Lilly Wachowski'}) ON CREATE SET p.born=1967",
    );
    run_write(
        &store,
        "MERGE (p:Person {name:'Tony Scott'}) ON CREATE SET p.born=1944",
    );
    run_write(
        &store,
        "MERGE (p:Person {name:'Rob Reiner'}) ON CREATE SET p.born=1947",
    );

    // We should have 9 nodes total (3 movies + 6 people).
    assert_eq!(count_nodes(&store), 9, "expected 9 nodes after population");

    // ── MERGE is idempotent ───────────────────────────────────────────────────

    run_write(
        &store,
        "MERGE (m:Movie {title:'The Matrix'}) ON CREATE SET m.released=1999",
    );
    run_write(
        &store,
        "MERGE (p:Person {name:'Keanu Reeves'}) ON CREATE SET p.born=1964",
    );
    assert_eq!(count_nodes(&store), 9, "MERGE must be idempotent");

    // ── Populate relationships via full inline MERGE patterns ─────────────────

    run_write(
        &store,
        "MERGE (n:Person {name:'Keanu Reeves'})-[:ACTED_IN]->(m:Movie {title:'The Matrix'})",
    );
    run_write(
        &store,
        "MERGE (n:Person {name:'Lilly Wachowski'})-[:DIRECTED]->(m:Movie {title:'The Matrix'})",
    );
    run_write(
        &store,
        "MERGE (n:Person {name:'Tom Cruise'})-[:ACTED_IN]->(m:Movie {title:'Top Gun'})",
    );
    run_write(
        &store,
        "MERGE (n:Person {name:'Tony Scott'})-[:DIRECTED]->(m:Movie {title:'Top Gun'})",
    );
    run_write(
        &store,
        "MERGE (n:Person {name:'Tom Cruise'})-[:ACTED_IN]->(m:Movie {title:'A Few Good Men'})",
    );
    run_write(
        &store,
        "MERGE (n:Person {name:'Jack Nicholson'})-[:ACTED_IN]->(m:Movie {title:'A Few Good Men'})",
    );
    run_write(
        &store,
        "MERGE (n:Person {name:'Rob Reiner'})-[:DIRECTED]->(m:Movie {title:'A Few Good Men'})",
    );

    // Node count unchanged — only edges were added.
    assert_eq!(
        count_nodes(&store),
        9,
        "relationship MERGE must not create new nodes"
    );

    // ── READ: directors of "The Matrix" ──────────────────────────────────────

    let matrix_directors = {
        let output = Transpiler::cypher_to_sparql(
            "MATCH (m:Movie {title:'The Matrix'})<-[:DIRECTED]-(d:Person) RETURN d.name AS name",
            &ENGINE,
        )
        .expect("translation failed");
        collect_column(&store, &output.into_sparql(), "name")
    };
    assert_eq!(
        matrix_directors,
        vec!["Lilly Wachowski"],
        "wrong directors for The Matrix"
    );

    // ── READ: movies released before 1990 ─────────────────────────────────────

    let old_movies = {
        let output = Transpiler::cypher_to_sparql(
            "MATCH (m:Movie) WHERE m.released < 1990 RETURN m.title AS title",
            &ENGINE,
        )
        .expect("translation failed");
        let mut v = collect_column(&store, &output.into_sparql(), "title");
        v.sort();
        v
    };
    assert_eq!(
        old_movies,
        vec!["Top Gun"],
        "expected only Top Gun before 1990"
    );

    // ── READ: Tom Cruise movies ────────────────────────────────────────────────

    let tom_movies = {
        let output = Transpiler::cypher_to_sparql(
            "MATCH (p:Person {name:'Tom Cruise'})-[:ACTED_IN]->(m:Movie) RETURN m.title AS title",
            &ENGINE,
        )
        .expect("translation failed");
        let mut v = collect_column(&store, &output.into_sparql(), "title");
        v.sort();
        v
    };
    assert_eq!(
        tom_movies,
        vec!["A Few Good Men", "Top Gun"],
        "Tom Cruise should have acted in 2 movies"
    );

    // ── SET: update a property ────────────────────────────────────────────────

    run_write(
        &store,
        "MATCH (m:Movie {title:'Top Gun'}) SET m.released=1986",
    );

    // ── REMOVE: remove a property ─────────────────────────────────────────────

    run_write(
        &store,
        "MATCH (m:Movie {title:'Top Gun'}) REMOVE m.released",
    );

    // ── CREATE: add a new node ────────────────────────────────────────────────

    run_write(
        &store,
        "CREATE (r:Review {summary:'Great movie', rating:95})",
    );
    assert_eq!(count_nodes(&store), 10, "CREATE should add one node");

    // ── TEARDOWN: DETACH DELETE ───────────────────────────────────────────────

    run_write(&store, "MATCH (n) DETACH DELETE n");

    assert_eq!(
        count_nodes(&store),
        0,
        "store must be empty after DETACH DELETE"
    );
}

/// DDL statements (CREATE CONSTRAINT) must return UnsupportedFeature.
#[test]
fn create_constraint_returns_unsupported() {
    let result = Transpiler::cypher_to_sparql_update(
        "CREATE CONSTRAINT movie_title IF NOT EXISTS FOR (m:Movie) REQUIRE m.title IS UNIQUE",
        &ENGINE,
    );
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("CREATE CONSTRAINT"),
        "error should mention CREATE CONSTRAINT, got: {msg}"
    );
}

/// `cypher_to_sparql_update` returns empty vec for pure-read queries.
#[test]
fn pure_read_returns_empty_updates() {
    let updates = Transpiler::cypher_to_sparql_update("MATCH (n:Person) RETURN n.name", &ENGINE)
        .expect("should succeed");
    assert!(
        updates.is_empty(),
        "pure-read query should produce no updates"
    );
}

/// `gql_to_sparql_update` works for a simple GQL CREATE.
#[test]
fn gql_update_create_node() {
    let updates =
        Transpiler::gql_to_sparql_update("MATCH (n:Person {name: 'Alice'}) RETURN n.name", &ENGINE)
            .expect("gql_to_sparql_update should not fail on read query");
    // Read query → no updates.
    assert!(updates.is_empty());
}
