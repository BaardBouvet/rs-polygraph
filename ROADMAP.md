# Roadmap

This roadmap tracks the phased delivery of `rs-polygraph`. Each phase produces a usable artifact and ends with a clear milestone. See [plans/implementation-plan.md](plans/implementation-plan.md) for design details.

---

## Phase 1 — Foundation & openCypher Parser

**Goal**: Parse a useful subset of openCypher into a typed AST.

- [x] Initialize Cargo workspace with module structure (`ast`, `parser`, `translator`, `target`, `rdf_mapping`, `error`)
- [x] Define `PolygraphError` with `thiserror`
- [x] Write `grammars/cypher.pest` covering core clauses: `MATCH`, `WHERE`, `RETURN`, `WITH`
- [x] Implement `pest`-based parser producing `CypherQuery` AST
- [x] Unit tests for all core AST node types
- [x] Parser round-trip tests for the covered subset

**Milestone**: `polygraph::parser::cypher::parse(query)` returns a typed AST for basic `MATCH … RETURN` queries. ✅

---

## Phase 2 — openCypher → SPARQL Algebra Translator

**Goal**: Translate Phase 1's AST into valid SPARQL 1.1 algebra via `spargebra`.

- [x] Define `AstVisitor` trait in `translator/visitor.rs`
- [x] Implement node/label/property → RDF triple pattern mappings
- [x] Implement directed and undirected relationship → triple pattern mappings
- [x] Map `WHERE` predicates to `FILTER` expressions
- [x] Map `RETURN` projections to `SELECT` variables
- [x] Map `OPTIONAL MATCH` to `OPTIONAL { }` graph pattern
- [x] Map `WITH` to sub-select or `BIND`
- [x] Integration tests: given a Cypher string, assert the serialized SPARQL output

**Milestone**: `Transpiler::cypher_to_sparql(q, engine)` works for single-hop queries. Output validates against the SPARQL 1.1 grammar. ✅

---

## Phase 3 — RDF-star & Reification Edge Properties

**Goal**: Support edge properties with both RDF-star and standard reification modes.

- [x] Implement `rdf_mapping::rdf_star` encoder for edge property triples
- [x] Implement `rdf_mapping::reification` fallback
- [x] Implement `TargetEngine` trait with `supports_rdf_star()` capability flag
- [x] Implement `target::rdf_star::RdfStar` generic adapter (RDF-star enabled; engine-agnostic)
- [x] Implement `target::GenericSparql11` adapter (reification fallback)
- [x] Tests for both encoding modes on edge properties

**Milestone**: Relationship properties transpile correctly for both RDF-star and legacy engines. ✅

---

## Phase 4 — Extended openCypher Coverage

**Goal**: Reach broad openCypher feature parity beyond basic `MATCH … RETURN`.

- [x] Variable-length path patterns (`-[:REL*]->`, `-[:REL*1..]->`, `-[:REL*0..1]->`) → SPARQL ZeroOrMore / OneOrMore / ZeroOrOne property paths
- [x] Multi-type relationship union (`-[:A|B]->`) → SPARQL Alternative property path
- [x] `MERGE`, `CREATE`, `SET`, `DELETE`, `REMOVE` write clauses → parsed, return UnsupportedFeature (SPARQL Update deferred to engine integration)
- [x] `UNWIND [literal list] AS var` → SPARQL `VALUES`
- [x] Aggregation functions `count(*)`, `count(expr)`, `sum`, `avg`, `min`, `max`, `collect` → SPARQL aggregate expressions + `GROUP BY`
- [x] `ORDER BY` (ASC/DESC, multi-field) → SPARQL `OrderBy`
- [x] `SKIP` / `LIMIT` → SPARQL `Slice`
- [x] List literals in `IN [a, b, c]` → SPARQL `IN()` expression with multiple members
- [x] `CALL` procedure stubs → parsed, return UnsupportedFeature with procedure name
- [x] Expand grammar (`cypher.pest`) and parser for all new constructs
- [x] Regression tests for each new feature (45 new tests: 10 AST unit + 35 integration)

**Milestone**: Handles the majority of real-world read Cypher queries. Publicly announce alpha. ✅

---

## Phase 5 — ISO GQL Parser & Translator ✅

**Goal**: Add ISO GQL (ISO/IEC 39075:2024) as a supported input language.

- [x] Write `grammars/gql.pest` for core GQL constructs (MATCH, FILTER/WHERE, RETURN, NEXT, IS labels, multi-labels, ORDER BY, SKIP, LIMIT, aggregation, write clauses)
- [x] Define `GqlQuery` AST types in `ast/gql.rs` (wraps `Vec<Clause>` for zero-duplication design)
- [x] Implement GQL parser in `parser/gql.rs` with IS→`:Label` lowering, FILTER→WITH WHERE, NEXT→WITH *, IS edge types (19 unit tests)
- [x] Implement `translator/gql.rs` delegating to Cypher translator via shared clause types
- [x] `Transpiler::gql_to_sparql(q, engine)` public API wired up in `lib.rs`
- [x] 34 integration tests in `tests/integration/gql_to_sparql.rs` covering IS labels, multi-labels, FILTER, WHERE, NEXT, rel IS TYPE, OPTIONAL MATCH, ORDER BY/SKIP/LIMIT, aggregation, RDF-star

**Milestone**: Basic GQL `MATCH … RETURN` queries transpile to valid SPARQL. ✅ 199 tests passing.

---

## Phase 6 — openCypher TCK Compliance

**Goal**: Systematically verify semantic correctness against the official test suite.

- [x] Integrate the `cucumber` crate for Gherkin-driven tests
- [x] Download and vendorize TCK feature files from [opencypher/openCypher](https://github.com/opencypher/openCypher/tree/master/tck)
- [x] Spin up an embedded Oxigraph instance in tests for SPARQL execution
- [x] Implement step definitions for TCK `Given`/`When`/`Then` patterns
- [x] Track and document skipped/failing scenarios with issue references
- [x] Achieve ≥ 80% TCK pass rate
- [x] Achieve ≥ 90% TCK pass rate
- [x] Achieve ≥ 95% TCK pass rate (currently 99.6%)

**TCK compliance tracker** (updated each release):

| Release | Pass | Fail | Total | % |
|---------|------|------|-------|---|
| 0.1.0   | 362  | 101  | 463   | 78.2% |
| dev     | 461  | 2    | 463   | 99.6% |

**Remaining 2 failures** — fundamental static-transpiler limitations:
- Match4[8]: `[rs*]` runtime list as path constraint (requires multi-phase execution, see plans/fundamental-limitations.md §1a)
- Match6[14]: undirected *3..3 with parallel edges (RDF collapses duplicate triples; multigraph not representable in RDF)

**Milestone**: Published compliance report. ≥ 80% pass rate.

---

## Phase 7 — Full openCypher TCK Suite Expansion

**Goal**: Expand TCK coverage from 463 scenarios (4 clause categories) to ≥ 80% pass rate across all 3,650 scenarios in the complete suite. See [plans/tck-full-plan.md](plans/tck-full-plan.md) for the detailed phased breakdown and translator mapping tables.

**Current coverage**: 461/463 (99.6%) across the 4-category subset; 12.7% of the full suite.

- [ ] **Phase A** — Vendorize low-effort categories (return-orderby, with, union, literals, boolean); fix grammar edge-cases; 572 new scenarios; target ≥ 90%
- [ ] **Phase B** — Expression engine: string/numeric/type-conversion functions, `CASE WHEN`, list comprehensions, map literals; 558 new scenarios; target ≥ 75%
- [ ] **Phase C** — Advanced features: graph functions (`type(r)`, `labels(n)`), `EXISTS` / `NOT EXISTS`, quantifiers (compile-time lists), procedure stubs; 670 new scenarios; target ≥ 40%
- [ ] **Phase D** — Write operations (`CREATE/DELETE/SET/MERGE` → SPARQL Update) and temporal types; 1,370 new scenarios; target ≥ 40%
- [x] **Phase F** — Code-health refactor: split `src/translator/cypher.rs` (16,209 lines) into 8 focused subfiles under `src/translator/cypher/` using `include!` macro technique; pre-refactor dead-code removal (−1,038 lines); zero TCK regression

**Full-TCK compliance tracker** (updated each release):

| Release | Pass | Fail | Total | % | Notes |
|---------|------|------|-------|---|-------|
| dev     | 461  | 2    | 463   | 99.6% | 4-category subset |
| dev     | 1632 | 116  | 1748  | 93.4% | full suite, grammar expanded |
| dev     | 2179 | 224  | 2505  | 87.0% | Phase C vendored: graph/pattern/quantifier |
| dev     | 2199 | 204  | 2505  | 87.8% | Phase C: SyntaxError checks, 3VL single() |
| dev     | 2228 | 175  | 2505  | 88.9% | date/time functions, WITH list propagation fixes |
| dev     | 2248 | 155  | 2505  | 89.7% | null_vars tracking, subscript property access, temporal constructors in translate_function_call, const_int_vars for range(), path nullable tracking |
| dev     | 2548 | 1241 | 3789  | 67.2% | Phase D vendored (create/delete/merge/remove/set/temporal); comprehensive temporal constructors (ISO week/ordinal/quarter/duration) |
| dev     | 2595 | 999  | 3789  | 68.5% | Write clause support: REMOVE/SET via SPARQL Update, CREATE/DELETE semantic validation (VariableAlreadyBound, NoSingleRelationshipType, etc.) |
| dev     | 2627 | 967  | 3789  | 69.3% | MERGE validation (+NoSingleRelType, +VariableAlreadyBound, +path var), MERGE INSERT/MATCH in skip_writes, SetLabel support, SET/MERGE RHS undefined var checks |
| dev     | 3430 | 164  | 3789  | 90.5% | All Temporal5 scenarios fixed: JDN pos/neg split (Oxigraph right-associative subtraction bug), d.quarters/d.weeks added to TEMPORAL_PROPS; date/time component extraction working for all temporal types |
| dev     | 3431 | 163  | 3789  | 90.6% | Phase F: translator split into 8 focused files (mod.rs 4059L, clauses.rs 1753L, temporal.rs 3343L, patterns.rs 1549L, functions.rs 1528L, semantics.rs 1554L, rewrite.rs 826L, return_proj.rs 593L); −1,038 dead lines |
| dev     | 3433 | 161  | 3789  | 90.7% | fix(temporal): xsd:time literals missing :00 seconds — time({hour:10,timezone:'+01:00'}) UNDEF for comparison; +2 passes |
| dev     | 3435 | 156  | 3739  | 95.7% | TCK runner expanded: 1415→3739 scenarios via new write-clause + temporal + graph/path/quantifier shards; stack overflow fixes (larger thread stack, Call-proc skip step); list null encoding fix |
| target  | —    | —    | 3,650 | ≥ 80% | all 37 categories |

**Milestone**: ≥ 80% pass rate across the full 3,650-scenario suite.

---

## Phase 8 — Performance & Production Hardening

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
