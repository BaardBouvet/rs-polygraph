# polygraph-difftest

Phase 1 deliverable of [plans/spec-first-pivot.md](../plans/spec-first-pivot.md):
the differential testing harness that gives `rs-polygraph` an oracle distinct
from the openCypher TCK.

## Layout

```
polygraph-difftest/
├── Cargo.toml
├── src/
│   ├── lib.rs              # public re-exports
│   ├── fixture.rs          # PropertyGraph / NodeSpec / EdgeSpec
│   ├── value.rs            # Value (Cypher/SPARQL value model + bag-eq)
│   ├── rdf_projection.rs   # PropertyGraph → SPARQL INSERT DATA
│   ├── oracle.rs           # bag/ordered comparison
│   ├── runner.rs           # transpile → Oxigraph → compare
│   ├── neo4j.rs            # live driver (feature: live-neo4j)
│   └── bin/difftest.rs     # CLI
├── queries/                # curated TOML query specs
└── tests/smoke.rs          # `cargo test -p polygraph-difftest`
```

## Running

```sh
# Curated suite — does not need Neo4j; runs against pre-recorded expectations.
cargo test -p polygraph-difftest
cargo run  -p polygraph-difftest --bin difftest

# Live Neo4j cross-check (needs a reachable Neo4j 5.x at NEO4J_URL).
cargo build -p polygraph-difftest --features live-neo4j
NEO4J_URL=http://localhost:7474 NEO4J_USER=neo4j NEO4J_PASSWORD=secret \
  cargo run -p polygraph-difftest --features live-neo4j --bin difftest
```

## Adding a curated query

Each `queries/*.toml` file follows this schema:

```toml
name        = "short-id"
description = "what this query exercises"
spec_ref    = "openCypher 9 §X.Y …"   # provenance for the expected result

cypher = """
MATCH (n:Person) RETURN n.name AS name
"""

[fixture]
nodes = [ { id = "a", labels = ["Person"], properties = { name = "Alice" } } ]
edges = []

[expected]
columns = ["name"]
rows    = [["Alice"]]
order   = "bag"   # or "ordered" for queries with ORDER BY
```

The `spec_ref` field is required and is the *justification* for the expected
result. If you cannot cite a spec section or a Neo4j round-trip transcript, the
query does not belong in the curated suite — file it as a fuzz seed instead
(Phase 5).

## Phase 1 status

| Deliverable                                               | Status |
|-----------------------------------------------------------|--------|
| Workspace + crate skeleton                                | ✅ done |
| PropertyGraph fixture + RDF projection                    | ✅ done |
| Bag-equality oracle (Cypher null/order semantics)         | ✅ done |
| Oxigraph runner + curated suite (≥ 6 seeds)               | ✅ done |
| Live Neo4j HTTP driver behind `live-neo4j` feature        | ✅ done (untested in dev container) |
| `cargo test -p polygraph-difftest` smoke test             | ✅ done |
| Proptest-driven generator                                 | ⏳ Phase 5 |
| Curated suite ≥ 200 seeds                                 | ⏳ in progress |
| CI nightly job                                            | ⏳ Phase 1 followup |

The plan's exit criterion of ≥ 200 curated queries is the next session's work
— each new seed must carry a `spec_ref` justification and is reviewed against
the openCypher 9 reference semantics, not derived from TCK fixtures.
