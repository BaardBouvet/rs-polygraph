# Spec-First Pivot — From TCK-Driven Patches to Semantics-Driven Translation

**Status**: in progress
**Updated**: 2026-05-04 (Phase 1: 98 curated queries; fixed ORDER BY leak + chained-string CONCAT)

This plan replaces the project's *de facto* methodology — "find the next failing
TCK scenario, patch the translator until it passes" — with a spec-anchored,
algebra-mediated, differentially-tested pipeline. It does **not** discard the
existing module decomposition (parser / AST / translator / rdf_mapping / target);
it inserts a logical IR between AST and SPARQL, replaces the ad-hoc parser, and
re-grounds testing on openCypher / GQL semantics rather than scenario fixtures.

The TCK is preserved throughout as a regression floor: no phase may land that
drops the current pass rate (3734 / 3828, 97.5 %).

---

## 1. Why pivot

The current translator was built by reverse-engineering scenarios. That produced
a working ~97.5 % TCK transpiler but with three structural risks for arbitrary
user queries:

1. **Hand-rolled pest grammar** ([grammars/cypher.pest](../grammars/cypher.pest))
   has been grown to accept what the TCK writes. Constructs the TCK does not
   exercise (deeply nested `CALL { … }`, label expressions with `&`/`|`/`!`,
   list comprehensions inside map projections, certain `FOREACH` shapes,
   parameter-typed pattern predicates, schema/index DDL, procedure calls) are
   silently rejected or misparsed.
2. **AST → SPARQL is a single hop** through visitors plus an ad-hoc rewrite
   pass ([src/translator/cypher/rewrite.rs](../src/translator/cypher/rewrite.rs)).
   Many rules in `rewrite.rs` and `semantics.rs` are scenario-specific patches
   rather than normalizations derivable from the spec; they are likely to
   silently misbehave on query shapes they were not authored against.
3. **TCK pass-rate is the only correctness oracle.** The TCK is thin in several
   user-visible areas (large `WITH` chains with aggregation+ordering, null
   propagation through `CASE`, parameterized queries, `FOREACH` inside `MERGE`,
   bag semantics around `DISTINCT` + `OPTIONAL MATCH`). A 97.5 % TCK score
   gives no quantitative bound on *real-query* correctness.

The remediation has three pillars: a **spec-grounded logical algebra IR**,
a **grammar generated from the openCypher / GQL reference**, and a
**differential testing harness** against a real Cypher engine.

---

## 2. Target Architecture

```
Input query (Cypher / GQL)
   │
[parser]                                       ── Phase 2 ──
   │   ANTLR-generated Cypher / GQL parser, span-preserving
   ▼
Cypher AST  /  GQL AST                         (existing, hardened)
   │
[normalizer]                                   ── Phase 3 ──
   │   desugar list/pattern/map comprehensions, normalize CASE,
   │   lift WITH/RETURN aliases, resolve scoping, type-annotate
   ▼
Normalized AST (typed)
   │
[lowering]                                     ── Phase 3 ──
   │   AST → Logical Query Algebra (LQA)
   ▼
Logical Query Algebra (LQA)                    ── Phase 3 (new) ──
   │   bag-semantics operators: Scan, Expand, Selection, Projection,
   │   GroupBy, OrderBy, Limit, Distinct, Union, OptionalJoin,
   │   Subquery, Foreach, Merge, Update, …
   │
[lowering]                                     ── Phase 4 ──
   │   LQA → SPARQL algebra, parameterized by TargetEngine capabilities
   ▼
spargebra::GraphPattern  (+ updates)
   │
[target]                                       (existing)
   ▼
SPARQL 1.1 / SPARQL-star string
```

The LQA is the load-bearing addition. It is the only place where openCypher
semantics are encoded; everything below it is mechanical lowering.

---

## 3. Phases

Each phase has an explicit **exit criterion** and a **TCK floor**. No phase
merges if the TCK pass count drops below the value at phase start.

### Phase 0 — Baseline & Instrumentation  (✅ complete 2026-05-04)

**Goal:** establish the metrics needed to detect regressions during the pivot.

- ✅ Baseline frozen at [tests/tck/baseline/scenarios.jsonl](../tests/tck/baseline/scenarios.jsonl)
  via the `POLYGRAPH_TCK_RESULTS_PATH` env var (writer in [tests/tck/main.rs](../tests/tck/main.rs)).
  **3756 / 3828 passing (98.1 %), 72 failing.**
- ✅ Diff tool [tools/tck_diff.sh](../tools/tck_diff.sh) with `--freeze` and
  default diff modes; exits non-zero on any regression.
- ✅ Working-agreement headers added to
  [src/translator/cypher/rewrite.rs](../src/translator/cypher/rewrite.rs) and
  [src/translator/cypher/semantics.rs](../src/translator/cypher/semantics.rs)
  defining the `// NORMALIZATION(<spec-ref>):` / `// SCENARIO-PATCH(<TCK-ids>):`
  marker convention.
- ✅ First obvious scenario-patch tagged: Quantifier9–12 tautology fold in
  [src/translator/cypher/mod.rs](../src/translator/cypher/mod.rs).
- ✅ [plans/scenario-debt.md](scenario-debt.md) catalogues every
  `examples/check_*`, `examples/debug_*`, and `examples/test_*` probe with a
  disposition (delete │ promote → unit / integration / difftest).

**Exit:** baseline committed, instrumentation in place, debt list filed.

**Followup work merged into Phase 4:** the broader audit of `rewrite.rs` /
`semantics.rs` to tag every existing transformation with a NORMALIZATION or
SCENARIO-PATCH marker is left to Phase 4 since it requires the LQA
normalization pass as the migration target.

### Phase 1 — Differential Testing Harness  (🟡 in progress — 98 / ≥200 curated queries)

**Goal:** stop measuring correctness purely against the TCK.

**Landed:**

- ✅ Workspace converted; new crate [polygraph-difftest/](../polygraph-difftest/).
- ✅ [`PropertyGraph`](../polygraph-difftest/src/fixture.rs) fixture model with
  Cypher `CREATE` and SPARQL `INSERT DATA` projections.
- ✅ RDF projection in [polygraph-difftest/src/rdf_projection.rs](../polygraph-difftest/src/rdf_projection.rs)
  matching the TCK harness encoding:
  - `<node_iri> <base:__node> <base:__node>` sentinel for every node (required by
    all MATCH patterns that the translator emits).
  - Label → `rdf:type`; property → base-IRI predicate; edge → typed predicate.
  - Edge properties → RDF-star reification `<< s <base:REL> o >> <base:key> "val"`.
- ✅ [`Comparison`](../polygraph-difftest/src/oracle.rs) bag/ordered oracle with
  Cypher null-propagating equality and column-name parity.
- ✅ [`run_one`](../polygraph-difftest/src/runner.rs) end-to-end runner: transpile via
  `polygraph::Transpiler::cypher_to_sparql`, execute against in-process Oxigraph,
  hydrate result rows, compare against the curated expectation.
- ✅ Live Neo4j HTTP driver in [polygraph-difftest/src/neo4j.rs](../polygraph-difftest/src/neo4j.rs)
  behind `live-neo4j` feature; reads `NEO4J_URL` / `NEO4J_USER` / `NEO4J_PASSWORD`.
- ✅ **98 curated queries** in [polygraph-difftest/queries/](../polygraph-difftest/queries/) — all
  passing against the in-process Oxigraph oracle. Coverage includes:
  - Basic MATCH, WHERE (int/string/bool/float/range/regex), ORDER BY (ASC/DESC/multi-col)
  - Aggregates: count, count(DISTINCT), sum, min, max, avg, sum/avg per group
  - OPTIONAL MATCH, OPTIONAL MATCH + coalesce (limitations documented)
  - WITH chains: rename, filter (HAVING-equivalent), ORDER+LIMIT in WITH
  - UNWIND list literal, UNWIND range, UNWIND+MATCH
  - String functions: toLower, toUpper, size, trim, replace, substring, left,
    contains, startsWith, endsWith, concat (+), regex =~
  - Math functions: abs, floor, ceil, sqrt, round, modulo
  - Type conversion: toString, toInteger, toFloat
  - Relationship patterns: typed, type-OR ([:A|B]), undirected, incoming direction,
    anonymous target/relationship, property-on-relationship inline predicate,
    edge property via RDF-star
  - CASE: generic WHEN form, simple (CASE expr WHEN) form
  - Two-hop, three-node chain with intermediate-node filter
  - Cross-product (Cartesian) MATCH
  - SKIP, LIMIT, SKIP+LIMIT
  - Multi-label nodes, label predicate in WHERE
  - Literal RETURN (no MATCH), range() return, range() in UNWIND
  - IS NULL, IS NOT NULL on property
  - NOT, NOT(conjunction), NOT IN
  - Boolean / float property filters
- ✅ [polygraph-difftest/tests/smoke.rs](../polygraph-difftest/tests/smoke.rs)
  runs the entire suite under `cargo test -p polygraph-difftest`. **98/98 passing.**
- ✅ `difftest` CLI binary with human-readable per-query report and a 0/1 exit code.

**Known translator limitations found and documented during Phase 1 expansion:**

| Query pattern | Behaviour | Notes |
|---|---|---|
| `OPTIONAL MATCH (n)-[r]->(m)` + `m.prop` in outer OPTIONAL | `m.prop` outer OPTIONAL re-binds to all matching nodes when `m` is null | Structural bug: property OPTIONALs should be scoped inside the OPTIONAL MATCH block |
| `collect(x)` → `size(collect(x))` | `STRLEN` of the serialized string, not list length | GROUP_CONCAT serializes list; size() treats it as a string |
| `^` power operator | `<urn:polygraph:unsupported-pow>` stub, rejected by Oxigraph | SPARQL has no POW(); Phase 4 candidate |
| `head([...])` / `last([...])` | String slice hack / unsupported | Phase 4 candidate |
| `sign(expr)` on non-literal | "complex return expression (Phase 4+)" error | Phase 4 candidate |
| `ORDER BY non-RETURN-expr` | ✅ **Fixed 2026-05-04**: removed edge-map guard in `clauses.rs` pre-ORDER-BY loop; all property sort keys now pre-translated and included in inner `Project`, triggering outer-project hiding. TCK: 72→71 failing. | [`clauses.rs` pre-order loop](../src/translator/cypher/clauses.rs) |
| chained string `+` (`a + ' ' + b`) | ✅ **Fixed 2026-05-04**: added recursive `expr_is_string_producer` free function in `mod.rs`; string detection now propagates through any depth of `Add`. | [`mod.rs` Add branch](../src/translator/cypher/mod.rs) |

**Remaining for Phase 1 exit (≥ 200 curated queries):**

- Grow [polygraph-difftest/queries/](../polygraph-difftest/queries/) to ≥ 200
  seeds covering the gaps the TCK under-tests (multi-clause `WITH`+aggregation,
  `OPTIONAL MATCH` + null propagation, `CASE` in projections, list/pattern/map
  comprehensions, parameterized queries, `FOREACH` inside `MERGE`).
- Stand up CI job `difftest-smoke` running on every PR.
- (Phase 5) proptest-driven generator emitting *typed* Cypher; currently deferred.

**Exit:** ≥ 200 curated queries pass; nightly fuzz corpus committed under
`difftest/corpus/`; one previously-unknown bug found and filed.

### Phase 2 — Grammar Migration

**Goal:** replace the hand-rolled pest grammar with one generated from the
openCypher / GQL reference grammars.

- Add `antlr-rust` (or `tree-sitter-cypher` + a thin wrapper — pick during
  spike) as the parser backend.
- Vendor the openCypher reference grammar
  (`Cypher.g4`) and the GQL ISO grammar excerpt under `grammars/upstream/`.
- Generate parser into `src/parser/generated/` and write a thin adapter that
  produces the existing `ast::cypher` types, preserving `pest`-style spans.
- Run **both** parsers in parallel under a feature flag for one phase; assert
  AST equivalence on the entire TCK corpus and the difftest curated suite.
- Remove the pest grammar once parity is achieved on TCK + difftest.

**Risks / mitigations:**
- ANTLR-rust runtime maturity → spike first, fall back to `tree-sitter` if
  ergonomic costs are too high.
- AST shape drift → keep the AST module stable; absorb grammar differences in
  the adapter.

**Exit:** pest grammar deleted; TCK ≥ 3734; difftest curated suite still green.

### Phase 3 — Introduce Logical Query Algebra (LQA)

**Goal:** factor openCypher semantics into a typed IR independent of SPARQL.

- New module `src/lqa/` with:
  - `op.rs` — operator enum (`Scan`, `Expand`, `Selection`, `Projection`,
    `GroupBy`, `OrderBy`, `Limit`, `Distinct`, `Union`, `LeftOuterJoin`,
    `Subquery`, `Foreach`, `Merge`, `SetProperty`, `RemoveProperty`,
    `CreateNode`, `CreateEdge`, `Delete`, `Call`, …).
  - `expr.rs` — expression IR with explicit null-propagation rules and a
    `Type` lattice (`Node`, `Relationship`, `Path`, `List<T>`, `Map`,
    primitives, `Any`, `Null`).
  - `bag.rs` — bag-semantics combinators used by both lowering and the
    differential oracle.
  - `normalize.rs` — desugaring rules (list/pattern/map comprehensions, CASE,
    aliasing, scoping). Each rule has a citation to the openCypher 9 / GQL
    semantic clause it implements.
- AST → LQA lowering implemented clause-by-clause with a unit test per clause
  that asserts the expected operator tree, **not** the resulting SPARQL.
- LQA → spargebra lowering becomes the *only* path to SPARQL. The current
  AST → spargebra translator is retained behind a `--legacy-translator` flag
  for one phase to allow A/B comparison.

**Translator limitations to fix in this phase** (deferred from Phase 1):

| Limitation | Root cause | Fix strategy |
|---|---|---|
| `OPTIONAL MATCH (n)-[r]->(m)` + outer `m.prop` rebinds to all matching nodes when `m` is null | Property OPTIONALs are emitted outside the OPTIONAL MATCH block, losing null scope | In LQA `LeftOuterJoin`, property lookups on nullable variables must be scoped inside the join branch; generate `OPTIONAL { … }` sub-patterns rather than trailing property-lookup OPTIONALs |
| `size(collect(x))` returns string length instead of list cardinality | `collect()` maps to `GROUP_CONCAT`; `size()` then calls `STRLEN` on the serialized string | In LQA `GroupBy`, track which aggregates produce lists; lower `count(collect(x))` directly to `COUNT(x)` and `size(collect(x))` to `COUNT(x)` |


**Exit:** every TCK scenario routes through LQA; legacy translator flag
removed; TCK ≥ 3734; curated difftest still green.

### Phase 4 — Spec-Driven Lowering Audit

**Goal:** eliminate scenario-shaped patches.

- Walk every `// SCENARIO-PATCH(...)` tag from Phase 0:
  - If the patch is derivable from a normalization rule, move it into
    `lqa/normalize.rs` with a spec citation.
  - If not, file it as a `polygraph-difftest` curated query and either
    generalize the rule or mark the construct `Unsupported(reason)` with a
    typed error variant.
- Extend `PolygraphError` with `Unsupported { construct, spec_ref, reason }`
  so callers can distinguish "transpiler bug" from "semantically infeasible
  in static SPARQL" (per [fundamental-limitations.md](fundamental-limitations.md)).
- Delete `src/translator/cypher/rewrite.rs` and merge any surviving rules
  into `lqa/normalize.rs` or the lowering pass.

**Translator limitations to fix or classify in this phase** (deferred from Phase 1):

| Limitation | Spec ref | Fix / classification |
|---|---|---|
| `^` power operator emits `<urn:polygraph:unsupported-pow>` stub | openCypher 9 §6.3.1 | SPARQL has no `POW()`; lower to `XEXP(XLOG(x) * y)` approximation, or emit `Unsupported` with a typed error |
| `head(list)` / `last(list)` — string-slice hack / unsupported | openCypher 9 §6.3.5 | Requires list-index access in SPARQL; emit `Unsupported` with `spec_ref = "openCypher 9 §6.3.5"` until L2 runtime support is available |
| `sign(expr)` on non-literal — "complex return expression" error | openCypher 9 §6.3.2 | Implement `SIGN` via `IF(?x > 0, 1, IF(?x < 0, -1, 0))` in the LQA → spargebra lowering |


**Exit:** zero `SCENARIO-PATCH` tags; `rewrite.rs` deleted; TCK ≥ 3734;
difftest curated suite ≥ 99 % pass.

### Phase 5 — Coverage Expansion via Differential Fuzzing

**Goal:** push correctness beyond what the TCK measures.

- Grow the proptest generator to emit:
  - Multi-clause queries with `WITH … WHERE … ORDER BY … LIMIT` chains.
  - `OPTIONAL MATCH` with subsequent aggregation.
  - List / pattern / map comprehensions, including nested.
  - `CASE` expressions inside projections and predicates.
  - Parameterized queries (driven by a parameter-binding API).
- Track a **bag-equality pass rate** against Neo4j over the corpus; treat it
  as a first-class metric in [ROADMAP.md](../ROADMAP.md) alongside TCK %.
- Each fuzz-discovered failure becomes either a curated regression test
  (after fix) or a documented `Unsupported` construct.

**Exit:** ≥ 10 000-query nightly corpus, ≥ 99.5 % bag-equality;
`Unsupported` set documented in `docs/unsupported.md`.

### Phase 6 — Public API Hardening

**Goal:** make the library safe to depend on for non-TCK users.

- Stabilize the public surface in [src/lib.rs](../src/lib.rs):
  `transpile_cypher`, `transpile_gql`, `TranspileOptions`,
  `TranspileOutput`, `TargetEngine`, `PolygraphError`.
- Document the supported subset and the `Unsupported` catalog.
- Cut `0.x` → `0.y` release with a CHANGELOG entry calling out the pivot.

**Exit:** semver-stable API; docs build clean; one external integration
example (e.g. against Apache Jena or Stardog via `TargetEngine`).

---

## 4. Sequencing & Dependencies

```
Phase 0 ──► Phase 1 ──► Phase 2 ──► Phase 3 ──► Phase 4 ──► Phase 5 ──► Phase 6
              │            │            ▲
              │            └────────────┘  (Phase 2 may proceed in parallel
              │                             with Phase 3 once Phase 1 lands)
              ▼
        nightly difftest CI
```

Phase 1 (difftest harness) is the highest-leverage step and must land first;
without it the rest of the pivot has no oracle distinct from the TCK.

---

## 5. Non-Goals

- Rewriting the AST module. The existing `ast::cypher` and `ast::gql` types
  are adequate; only the parser feeding them changes in Phase 2.
- Replacing `spargebra`. It remains the SPARQL-side IR.
- Supporting Cypher procedures (`CALL db.…`) or `LOAD CSV`. These remain in
  the `Unsupported` set and are not in scope.
- Schema/index DDL. Out of scope; `Unsupported`.
- Runtime continuation work tracked in
  [l2-runtime-support.md](l2-runtime-support.md) is orthogonal and proceeds
  independently.

---

## 6. Risks

| Risk | Mitigation |
|------|------------|
| ANTLR-rust runtime immaturity blocks Phase 2 | Spike `tree-sitter-cypher` adapter as fallback; both produce the same AST |
| LQA introduction temporarily regresses TCK | Legacy translator behind feature flag for one phase; CI gate forbids regression |
| Differential testing flakiness from Neo4j Docker | Pin Neo4j version; cache fixtures; mark transient failures `nightly-only` |
| Scope creep into runtime / GQL features | This plan is parser+translator only; runtime work stays in `l2-runtime-support.md` |
| Generator emits queries Neo4j and Oxigraph disagree on for legitimate reasons (e.g. ordering of unordered results) | Compare under bag semantics; explicit ORDER BY normalization in oracle |

---

## 7. Success Metrics

- TCK pass rate ≥ 97.5 % maintained across every phase.
- Differential bag-equality ≥ 99.5 % on a ≥ 10 000-query nightly corpus.
- Zero `SCENARIO-PATCH` tags in the codebase post-Phase 4.
- `Unsupported` constructs documented and stable; no new ones added without
  a spec citation.
- Public `0.y` release shipped from Phase 6 with a third-party integration
  example.

---

## 8. Out-of-Band Cleanups (do alongside, not gating)

- Move `examples/debug_*` and `examples/check_*` one-offs into
  `tests/regression/` as proper unit tests, or delete them once their scenario
  is covered by curated difftest queries.
- Delete `grammars/cypher.pest.bak` and `examples/check_agg.rs.bak.ignore`.
- Audit `src/translator/cypher/temporal.rs` against the openCypher temporal
  spec; temporal arithmetic is one of the areas where TCK coverage is thin.

---

## 9. Cross-References

- Architectural baseline: [implementation-plan.md](implementation-plan.md)
- Hard semantic limits driving the `Unsupported` set:
  [fundamental-limitations.md](fundamental-limitations.md)
- Engine capability negotiation consumed by Phase 4 lowering:
  [target-engines.md](target-engines.md)
- Runtime-side companion (orthogonal): [l2-runtime-support.md](l2-runtime-support.md)
- Result hydration consumed by difftest oracle: [result-mapping.md](result-mapping.md)
- Final-mile TCK work continues until Phase 0 freezes the baseline:
  [final-mile.md](final-mile.md)
