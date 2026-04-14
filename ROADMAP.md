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

## Phase 8 — Full openCypher TCK Suite Expansion

**Goal**: Expand TCK coverage from 463 scenarios (4 clause categories) to the full 3,650 scenarios across all 37 categories. Current coverage is 12.7% of the upstream TCK.

**Scope summary**:

| Phase | Categories | New scenarios | Difficulty |
|-------|-----------|---------------|------------|
| A — Low-hanging fruit | return-orderby, return-skip-limit, with, with-skip-limit, with-where, with-orderBy, union, expressions/literals, expressions/boolean | 572 | Low |
| B — Expression engine | comparison, null, mathematical, precedence, string, aggregation, conditional, typeConversion, list, map, countingSubgraphMatches | 558 | Medium |
| C — Advanced features | call, graph, pattern, existentialSubqueries, path, quantifier, triadicSelection | 670 | Hard |
| D — Write ops & temporal | create, delete, merge, remove, set, temporal | 1,370 | Very Hard |

### Phase A — Low-Hanging Fruit (572 scenarios)

Features from Phases 2–4 are already implemented; this phase primarily vendorizes feature files and patches edge-case parse failures.

- [ ] Automate TCK vendorization with `scripts/vendor-tck.sh` (clone opencypher/openCypher, copy all feature files)
- [ ] Vendorize 42 feature files for phase A categories
- [ ] Harden harness for `Scenario Outline:` + `Examples:` tables (cucumber crate handles natively; verify)
- [ ] Fix grammar edge-cases: `CASE WHEN … END`, `IS NULL` / `IS NOT NULL`, `UNION` / `UNION ALL`
- [ ] Fix `WITH` + aggregation combos (`WITH count(*) AS c`)
- [ ] Target: ≥ 90% pass rate on phase A categories

### Phase B — Expression Engine (558 scenarios)

- [ ] Grammar additions to `grammars/cypher.pest`: `function_call`, `case_expression`, `list_comprehension`, `map_literal`
- [ ] New AST nodes: `Expression::FunctionCall`, `Expression::CaseExpression`, `Expression::ListComprehension`, `Expression::MapLiteral`
- [ ] Translator mappings for string functions: `toString→STR`, `toUpper→UCASE`, `toLower→LCASE`, `trim/ltrim/rtrim→REPLACE`, `left/right/substring→SUBSTR`, `replace→REPLACE`, `STARTS WITH→STRSTARTS`, `ENDS WITH→STRENDS`, `CONTAINS→CONTAINS`, `=~→REGEX`
- [ ] Translator mappings for numeric functions: `abs→ABS`, `ceil→CEIL`, `floor→FLOOR`, `round→ROUND`, `rand→RAND`, `x % y→arithmetic`, `sign→IF chain`
- [ ] Translator mappings for type conversion: `toInteger→xsd:integer`, `toFloat→xsd:double`, `toBoolean→xsd:boolean`
- [ ] `CASE WHEN` → nested `IF()` expression
- [ ] `coalesce(a, b)` → `COALESCE(a, b)`
- [ ] `x IS NULL` / `x IS NOT NULL` → `!BOUND(x)` / `BOUND(x)`
- [ ] Vendorize 64 feature files; target: ≥ 75% pass rate on phase B categories

### Phase C — Advanced Features (670 scenarios)

- [ ] Graph functions: `type(r)` via `PREDICATE()` (SPARQL 1.2); `labels(n)` via subquery; `id(n)` via IRI/BNode; `properties(n)` / `keys(n)` via property subquery
- [ ] `nodes(p)` / `relationships(p)` for bounded (unrolled) paths — intermediate variables are available
- [ ] Pattern predicates: `EXISTS { … }` / `NOT EXISTS { … }` → `FILTER EXISTS { }` / `FILTER NOT EXISTS { }`
- [ ] Existential subqueries: `EXISTS { MATCH … WHERE … }` → SPARQL `EXISTS` block
- [ ] Quantifier expressions for compile-time literal lists: `all(x IN list WHERE …)`, `any`, `none`, `single` — unroll at translation time; mark runtime-list variants as `UnsupportedFeature`
- [ ] `CALL` procedure stubs: parse and emit `UnsupportedFeature` for unknown procedures (counts as "correctly rejected")
- [ ] Triadic selection patterns
- [ ] Vendorize remaining phase C feature files; target: ≥ 40% pass rate on phase C categories (quantifiers are fundamentally hard for runtime lists)

### Phase D — Write Operations & Temporal (1,370 scenarios)

- [ ] Implement `cypher_to_sparql_update()` API returning SPARQL Update strings
- [ ] `CREATE` → `INSERT DATA { }` with RDF-star annotated triples for relationship properties
- [ ] `DELETE` → `DELETE DATA { }` / `DELETE WHERE { }`
- [ ] `SET` / `REMOVE` → `DELETE { old } INSERT { new } WHERE { }`
- [ ] `MERGE` → `INSERT { } WHERE { NOT EXISTS { } }` pattern
- [ ] TCK harness: add `Then the side effects should be:` step validating graph mutations against Oxigraph store
- [ ] Temporal types: map `xsd:date`, `xsd:dateTime`, `xsd:time`, `xsd:duration` for basic constructors and `YEAR/MONTH/DAY/HOURS/MINUTES/SECONDS` accessors
- [ ] Document known gaps: `LocalTime`/`LocalDateTime` (no xsd equivalent), duration arithmetic, temporal truncation
- [ ] Vendorize remaining phase D feature files; target: ≥ 50% for write ops, ≥ 30% for temporal

**Full-TCK compliance tracker** (updated each release):

| Release | Pass | Fail | Total | % |
|---------|------|------|-------|---|
| dev     | 461  | 2    | 463   | 99.6% (current 4-category subset) |
| target  | —    | —    | 3,650 | — (all 37 categories) |

**Milestone**: All 37 TCK categories vendorized; ≥ 60% pass rate across the complete suite.

---

## Future Considerations

- **SPARQL-star federation** (`SERVICE` keyword pass-through)
- **GQL write operations** (`INSERT`, `SET`, `DELETE` graph modifications)
- **Query planning hints** for specific engines (e.g., Jena TDB2 optimizations)
- **WASM target** for use in browser or edge environments
- **Python/JS bindings** via PyO3 / wasm-bindgen
