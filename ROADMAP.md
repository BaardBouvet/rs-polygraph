# Roadmap

This roadmap tracks the phased delivery of `rs-polygraph`. Each phase produces a usable artifact and ends with a clear milestone. See [plans/implementation-plan.md](plans/implementation-plan.md) for design details.

---

## Phase 1 — Foundation & openCypher Parser

**Goal**: Parse a useful subset of openCypher into a typed AST.

- [ ] Initialize Cargo workspace with module structure (`ast`, `parser`, `translator`, `target`, `rdf_mapping`, `error`)
- [ ] Define `PolygraphError` with `thiserror`
- [ ] Write `grammars/cypher.pest` covering core clauses: `MATCH`, `WHERE`, `RETURN`, `WITH`
- [ ] Implement `pest`-based parser producing `CypherQuery` AST
- [ ] Unit tests for all core AST node types
- [ ] Parser round-trip tests for the covered subset

**Milestone**: `polygraph::parser::cypher::parse(query)` returns a typed AST for basic `MATCH … RETURN` queries.

---

## Phase 2 — openCypher → SPARQL Algebra Translator

**Goal**: Translate Phase 1's AST into valid SPARQL 1.1 algebra via `spargebra`.

- [ ] Define `AstVisitor` trait in `translator/visitor.rs`
- [ ] Implement node/label/property → RDF triple pattern mappings
- [ ] Implement directed and undirected relationship → triple pattern mappings
- [ ] Map `WHERE` predicates to `FILTER` expressions
- [ ] Map `RETURN` projections to `SELECT` variables
- [ ] Map `OPTIONAL MATCH` to `OPTIONAL { }` graph pattern
- [ ] Map `WITH` to sub-select or `BIND`
- [ ] Integration tests: given a Cypher string, assert the serialized SPARQL output

**Milestone**: `Transpiler::cypher_to_sparql(q, engine)` works for single-hop queries. Output validates against the SPARQL 1.1 grammar.

---

## Phase 3 — RDF-star & Reification Edge Properties

**Goal**: Support edge properties with both RDF-star and standard reification modes.

- [ ] Implement `rdf_mapping::rdf_star` encoder for edge property triples
- [ ] Implement `rdf_mapping::reification` fallback
- [ ] Implement `TargetEngine` trait with `supports_rdf_star()` capability flag
- [ ] Implement `target::oxigraph::Oxigraph` adapter (RDF-star enabled)
- [ ] Implement `target::GenericSparql11` adapter (reification fallback)
- [ ] Tests for both encoding modes on edge properties

**Milestone**: Relationship properties transpile correctly for both RDF-star and legacy engines.

---

## Phase 4 — Extended openCypher Coverage

**Goal**: Reach broad openCypher feature parity beyond basic `MATCH … RETURN`.

- [ ] Variable-length path patterns (`-[:REL*1..3]->`)
- [ ] `MERGE`, `CREATE`, `SET`, `DELETE` write clauses → SPARQL Update
- [ ] `UNWIND` → `VALUES` or sub-select
- [ ] Aggregation functions (`count`, `sum`, `avg`, `collect`) → SPARQL aggregates
- [ ] `ORDER BY`, `SKIP`, `LIMIT` → SPARQL modifiers
- [ ] List, map, and string literal expressions
- [ ] `CALL` procedure stubs (emit warning for unsupported procedures)
- [ ] Expand grammar and parser accordingly
- [ ] Regression tests for each new feature

**Milestone**: Handles the majority of real-world read Cypher queries. Publicly announce alpha.

---

## Phase 5 — ISO GQL Parser & Translator

**Goal**: Add ISO GQL (ISO/IEC 39075:2024) as a supported input language.

- [ ] Write `grammars/gql.pest` for core GQL constructs
- [ ] Define `GqlQuery` AST types in `ast/gql.rs`
- [ ] Implement GQL parser in `parser/gql.rs`
- [ ] Implement `AstVisitor` for GQL in `translator/gql.rs`, reusing shared mapping logic
- [ ] `Transpiler::gql_to_sparql(q, engine)` public API
- [ ] Integration tests mirroring Phase 2 tests for GQL equivalents

**Milestone**: Basic GQL `MATCH … RETURN` queries transpile to valid SPARQL.

---

## Phase 6 — openCypher TCK Compliance

**Goal**: Systematically verify semantic correctness against the official test suite.

- [ ] Integrate the `cucumber` crate for Gherkin-driven tests
- [ ] Download and vendorize TCK feature files from [opencypher/openCypher](https://github.com/opencypher/openCypher/tree/master/tck)
- [ ] Spin up an embedded Oxigraph instance in tests for SPARQL execution
- [ ] Implement step definitions for TCK `Given`/`When`/`Then` patterns
- [ ] Achieve ≥ 80% TCK pass rate
- [ ] Track and document skipped/failing scenarios with issue references

**TCK compliance tracker** (updated each release):

| Release | Pass | Fail | Skip | % |
|---------|------|------|------|---|
| 0.1.0   | —    | —    | —    | — |

**Milestone**: Published compliance report. ≥ 80% pass rate.

---

## Phase 7 — Performance & Production Hardening

**Goal**: Ready for embedding in production database kernels.

- [ ] Add `criterion` benchmarks for translation throughput (queries/sec)
- [ ] Profile and optimize hot paths in the translator visitor
- [ ] Enforce `#![forbid(unsafe_code)]` crate-wide
- [ ] `#![deny(clippy::all, clippy::pedantic)]` with justified exceptions
- [ ] Fuzz the parser with `cargo-fuzz` / `arbitrary`
- [ ] Audit all `unwrap`/`expect` calls — replace with proper error propagation
- [ ] Verify `no_std` compatibility (or document the requirement for `std`)
- [ ] Publish `0.1.0` to crates.io

**Milestone**: `0.1.0` stable release on crates.io.

---

## Future Considerations

- **SPARQL-star federation** (`SERVICE` keyword pass-through)
- **GQL write operations** (`INSERT`, `SET`, `DELETE` graph modifications)
- **Query planning hints** for specific engines (e.g., Jena TDB2 optimizations)
- **WASM target** for use in browser or edge environments
- **Python/JS bindings** via PyO3 / wasm-bindgen
