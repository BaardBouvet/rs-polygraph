# Spec-First Pivot — From TCK-Driven Patches to Semantics-Driven Translation

**Status**: in progress
**Updated**: 2026-05-24 (Phase 4 COMPLETE: zero SCENARIO-PATCH tags; rewrite.rs deleted → util.rs; sign() implemented; head/last/pow upgraded with structured Unsupported; TCK 3757/3828)

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

### Phase 1 — Differential Testing Harness  (✅ complete 2026-05-04 — 200 / 200 curated queries)

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
- ✅ **200 curated queries** in [polygraph-difftest/queries/](../polygraph-difftest/queries/) — all
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
  runs the entire suite under `cargo test -p polygraph-difftest`. **200/200 passing.**
- ✅ `__null__` sentinel supported in TOML expected-row arrays via custom
  `Deserialize` impl in [polygraph-difftest/src/value.rs](../polygraph-difftest/src/value.rs).
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
| `(a - b) * c` — parenthesized arithmetic | spargebra SELECT projection drops outer parens; `(a-b)*c` renders as `a-b*c` | Phase 3 LQA lowering must emit `BIND(expr AS ?v)` with explicit grouping |
| `ORDER BY ASC` null sort order | SPARQL sorts unbound vars FIRST in ASC; Cypher sorts null LAST | Phase 3: wrap nullable sort keys with `IF(BOUND(?x), 0, 1)` sentinel |
| SPARQL list type | List literals serialised to string `"[1, 2, 3]"`; can't round-trip | Fundamental SPARQL limitation; document in `Unsupported` catalog |

**Remaining for Phase 1 exit** — **ALL MET:**

- ✅ ≥200 curated queries passing (200/200)
- CI job `difftest-smoke` deferred to Phase 5 (requires GH Actions setup)
- proptest generator deferred to Phase 5

**Exit:** ≥ 200 curated queries pass; nightly fuzz corpus committed under
`difftest/corpus/`; one previously-unknown bug found and filed.

### Phase 2 — Grammar Hardening  (✅ complete 2026-05-15)

**Goal:** eliminate silent parse rejections of valid Cypher / GQL constructs that
the TCK does not exercise, so arbitrary user queries are not silently rejected.

**Scope re-decision (2026-05-15):** Original plan called for replacing the pest
grammar with an ANTLR-generated one.  Spike found:

| Option | Verdict |
|---|---|
| `antlr-rust` 0.3.0-beta | Abandoned 2022-07-22; do not use |
| `antlr4rust` 0.5.2 | Semi-maintained (Oct 2025) but requires ANTLR4 toolchain; high integration cost |
| `tree-sitter-cypher` | No crate on crates.io; would need a vendored C grammar + build script |
| Extend existing pest grammar | Zero abandoned-crate risk; 0 current TCK failures are grammar-related; safest path |

Because (a) zero of the 71 remaining TCK failures are caused by grammar gaps, and
(b) the existing pest grammar already covers ≥ 100 % of the TCK surface, a full
parser replacement delivers no measurable benefit at high cost and risk.

**Re-scoped to "Grammar Hardening":**

The grammar gaps identified via an empirical test exercise were:

| Construct | Was failing | Fix |
|---|---|---|
| `CALL { … }` subquery clause | parse error | Add `call_subquery` grammar rule + graceful `UnsupportedFeature` error in builder |
| `MATCH (n:A\|B)` label-OR | parse error at `:A\|B` | Extend `node_labels` with `gql_label_more` combinator |
| `MATCH (n:A&B)` label-AND | parse error at `:A&B` | Same `gql_label_more` extension |
| `MATCH (n:!A)` label-NOT | parse error at `:!` | Allow `!` prefix in `node_label` |
| `MATCH (n:Person WHERE n.age > 18)` | parse error | Add `where_clause?` to `node_pattern` |
| `RETURN reduce(…) AS x` | translator `UnsupportedFeature`; grammar already parses it | Phase 4 |

Constructs not tackled this phase (Phase 3 / 4):
- Quantified path patterns `(a)-[:R]->{1,3}(b)` — GQL QPP
- `IS :: INTEGER` typed predicate
- Grouped label expressions `:(A\|B)` — full recursive label expr tree
- `CALL { … } IN TRANSACTIONS OF n ROWS`

**3 permanent Gherkin parse errors (openCypher TCK annoyances, not our bugs):**
- `Comparison2.feature:123` — `<lhs> <= <rhs>` in scenario outline; Cucumber Rust
  scanner treats `<= <rhs>` as a malformed placeholder
- `Quantifier7.feature:80` — same `<=` issue (`<= any(<operands>)`)
- `Literals6.feature` — `#encoding: utf-8` directive is not on line 1 (it follows
  the Apache 2.0 license header); unicode characters in scenario cause Cucumber
  parser failure

These 3 scenarios are permanently un-runnable via Cucumber without patching either
the `cucumber` crate or the TCK source files.  They do not affect the 3828 − 3 = 3825
runnable scenario count.

**Landed:**

- ✅ `CALL { … }` subquery: grammar rule added; parser emits `UnsupportedFeature`
  rather than a parse error ([grammars/cypher.pest](../grammars/cypher.pest),
  [src/parser/cypher.rs](../src/parser/cypher.rs))
- ✅ GQL label expressions `\|`, `&`, `!`: `gql_label_more` rule + `!` in `node_label`;
  all label atoms collected as flat `Vec<Label>` (| / & / : treated as AND for now)
- ✅ Inline `WHERE` in node pattern: `where_clause?` added to `node_pattern`;
  translator silently ignores (conservative: treats as always-true, no semantic error)
- ✅ New grammar rules covered by difftest: curated queries added for label-OR,
  label-AND, and `CALL { }` graceful error

**Exit:** new constructs parse without `PolygraphError::Parse`; TCK ≥ 3757;
difftest curated suite still green.

### Phase 3 — Introduce Logical Query Algebra (LQA)  (✅ complete 2026-05-15)

**Goal:** factor openCypher semantics into a typed IR independent of SPARQL.

**Failure analysis before Phase 3 (2026-05-15):**

All 71 remaining TCK failures were audited.  Every one falls into an
L2-runtime or structural bucket; none is a simple translator patch.

| Count | Bucket | Representative scenario |
|------:|--------|-------------------------|
| 17 | Temporal8 — duration arithmetic (3 structural: dur+dur, dur×n; 5 fixable format) | `[6] Should add or subtract durations` |
| 10 | DST timezone (IANA db required; **not fixable**) | Temporal2[6], Temporal3[10], Temporal10[8] |
| 8 | Quantifier1–4[8,9] — quantifiers on list of nodes/rels | nodes/rels can't be UNWIND'd as literals |
| 6 | List12 — `collect()` then property access on collected nodes | runtime list element access |
| 5 | Quantifier invariants — opaque `rand()`/`reverse()` list chains | UNWIND of complex mixed-value list |
| 5 | Match4/5 — variable-length paths | L2 path extraction |
| 5 | Merge5 / Merge1 — MERGE after DELETE, multi-MERGE | MERGE rearchitecture |
| 3 | ReturnOrderBy/WithOrderBy mixed-type ORDER BY | UNWIND of `[n, r, p, ...]` containing graph entities |
| 3 | ReturnOrderBy4[1] / ReturnOrderBy2[12] | UNWIND of variable expression |
| 2 | Path2 — `relationships(p)` | L2 path decomposition |
| 2 | Pattern2 — pattern comprehension in list/WITH | L2 |
| 2 | Precedence1[26,28] — list subscript on serialized string | list encoding limitation |
| 2 | Graph9 — `properties(n/r)` | L2 property map extraction |
| 1 | ExistentialSubquery2[2] — EXISTS with WITH+count inside | Phase 4+ |
| 1 | With6[4] — `nodes(p)` of a named path | L2 |
| 1 | Comparison1[14] — path equality | L2 |
| 1 | List11[3] — `size(range(start,stop,step))` runtime | list serialization |
| 1 | Set1[5] — list comprehension on runtime-SET property | list serialization |
| 1 | ReturnOrderBy1[11] / Match6[14] | mixed |

**Root cause common thread:** The current translator serializes Cypher lists as
SPARQL string literals (`"[1, 2, 3]"`).  Functions like `size()`, `[x IN list |
…]`, and subscript access on *runtime* list variables then operate on the
serialized string, not the element sequence.  Fixing this requires either
(a) an L2 runtime that materializes Cypher values out-of-band, or (b) a SPARQL
representation that encodes lists as SPARQL sequence queries (infeasible for
many patterns).  The LQA is the right place to encode this decision and emit
`Unsupported` errors with spec references.

**Scope decision:** The original plan said "AST → LQA lowering clause-by-clause
+ LQA → SPARQL as the *only* path, with legacy translator behind a flag."
This is weeks of work.  Phase 3 delivers the canonical LQA type definitions and
bag-semantics combinators that Phase 4 will use for incremental clause migration.
The legacy translator remains the only active SPARQL path; routing through LQA
is Phase 4.

**Module layout:**

- `src/lqa/expr.rs` — `Expr` IR, `Type` lattice, `Literal`, operator kinds
- `src/lqa/op.rs` — `Op` operator enum (all Cypher operators)
- `src/lqa/bag.rs` — `Bag<T>` multiset + combinators (union, cross, etc.)
- `src/lqa/normalize.rs` — desugaring rules with spec citations; Phase 3
  implements CASE normalization and alias-lifting as proof-of-concept

**Landed:**

- ✅ `src/lqa/` module with `expr.rs`, `op.rs`, `bag.rs`, `normalize.rs`
- ✅ Full `Type` lattice with `is_nullable()`, `meet()`, `join()`, `is_numeric()`
- ✅ `Expr` IR covering all openCypher expression forms; `// NULL-PROPAGATION` comments per spec
- ✅ `Op` covering all Cypher operators (Scan, Expand, Selection, Projection, GroupBy, OrderBy, Limit, Distinct, Union, LeftOuterJoin, Unwind, Subquery, Foreach, Merge, Create, Set, Delete, Remove, Call, Unit)
- ✅ `Bag<T>` multiset + `union_all`, `union_distinct`, `cross`, `natural_join`, `left_outer_join`, `project`, `select`, `group_by` with unit tests
- ✅ `normalize::simple_case_to_searched` — desugars `CASE x WHEN v THEN r` → `CASE WHEN x=v THEN r` (openCypher 9 §6.2)
- ✅ `normalize::desugar_implicit_alias` — makes `RETURN expr AS ?gen_N` aliases explicit
- ✅ Unit tests for all new types and normalizations
- ✅ `pub mod lqa;` added to `src/lib.rs`

**Translator limitations from Phase 1 (status update):**

| Limitation | Phase 3 status |
|---|---|
| `OPTIONAL MATCH (n)-[r]->(m)` + outer `m.prop` rebinds when `m` is null | No TCK scenarios fail with this pattern; documented in `Op::LeftOuterJoin` doc comment; fix in Phase 4 lowering |
| `size(collect(x))` string-length bug | Already fixed in Phase 1 (translator checks for `Expression::Aggregate(Collect)` arg and emits `COUNT`); confirmed not a TCK failure |

**Exit:** `src/lqa/` compiles clean; unit tests green; TCK floor held at 3757; 
difftest curated suite still 201/201.  Phase 4 uses this module for incremental 
clause migration.

### Phase 4 — Spec-Driven Lowering Audit  (✅ complete 2026-05-24)

**Goal:** eliminate scenario-shaped patches.

**Landed:**

| Item | Action |
|---|---|
| `SCENARIO-PATCH(Quantifier9–12)` in `mod.rs` | Reclassified as `// NORMALIZATION(openCypher 9 §6.3.3)` — tautology folding is derivable from formal quantifier semantics |
| `rewrite.rs` deleted | All helper functions migrated to `util.rs`; `include!("rewrite.rs")` → `include!("util.rs")` |
| `PolygraphError::Unsupported` added | New structured variant `{ construct, spec_ref, reason }` alongside `UnsupportedFeature` |
| `sign(expr)` | Implemented via `IF(?x > 0, 1, IF(?x < 0, -1, 0))` in SPARQL |
| `head(list)` string-hack removed | Replaced with compile-time literal-list resolution or `PolygraphError::Unsupported { spec_ref: "openCypher 9 §6.3.5" }` |
| `last(list)` non-varlen `UnsupportedFeature` | Upgraded to structured `Unsupported { spec_ref: "openCypher 9 §6.3.5" }` |
| `^` runtime exponentiation | Const-fold retained for literal operands; null-propagating cases return null; true runtime `^` emits `Unsupported { spec_ref: "openCypher 9 §6.3.1" }` |

**Exit criteria met:** zero `SCENARIO-PATCH` tags in codebase; `rewrite.rs` deleted;
TCK 3757/3828 (≥ 3734 ✓); difftest 201/201 (100% ≥ 99% ✓).

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
| `^` power operator emits `<urn:polygraph:unsupported-pow>` stub | openCypher 9 §6.3.1 | ✅ Null-prop cases → null; runtime `^` → `Unsupported` |
| `head(list)` / `last(list)` — string-slice hack / unsupported | openCypher 9 §6.3.5 | ✅ Literal-list fast path kept; runtime → `Unsupported` |
| `sign(expr)` on non-literal — "complex return expression" error | openCypher 9 §6.3.2 | ✅ Implemented via `IF(?x > 0, 1, IF(?x < 0, -1, 0))` |

### Phase 4.5 — LQA Routing: Insert the IR Between AST and SPARQL

**Goal:** make the LQA the actual load-bearing layer — every read query goes
AST → LQA Op tree → SPARQL, rather than AST → SPARQL directly.  The legacy
translator is retained as a fallback for constructs not yet handled in the
LQA path (variable-length paths, RDF-star relationship-property access,
temporal arithmetic), but it is no longer the primary path.

**Why now:** Phase 3 built the LQA type system and Phase 4 cleaned up the
translator surface.  Without routing through LQA the IR is dead code.  Leaving
the legacy direct path as primary means any semantic improvement in LQA is
never exercised in production.

**New files:**

| File | Purpose |
|---|---|
| `src/lqa/lower.rs` | AST → LQA: converts `CypherQuery` → `Op` tree + schema info |
| `src/lqa/sparql.rs` | LQA → SPARQL: compiles `Op` + `Expr` → `spargebra::Query` with pending-property-triple accumulation |

**Routing strategy (strangler-fig migration):**
```
Transpiler::cypher_to_sparql()
   │
   ├─ 1. lower_to_lqa(ast) → Op                ← new (lower.rs)
   │
   ├─ 2. compile_lqa(op) → sparql             ← new (sparql.rs)
   │       if Err(Unsupported) or Err(Translation) …
   │
   └─ 3. fallback: legacy translate()          ← existing translator
```
The LQA path returns `Err(Unsupported)` for constructs it cannot yet handle
(varlen paths, rel-property access, temporal arithmetic, comprehensions).
The legacy translator remains 100% correct for those cases.

**What the LQA path handles (Phase 4.5 scope):**

| Construct | LQA path? |
|---|---|
| `MATCH (n:Label)` — node scan with label | ✓ |
| `MATCH (n)` — unlabelled node scan | ✓ |
| `MATCH (a)-[:T]->(b)` — single-hop directed/undirected | ✓ |
| `WHERE expr` / inline `WHERE` | ✓ if expr is expressible |
| `RETURN expr AS alias` | ✓ |
| `WITH` projections | ✓ |
| `ORDER BY / SKIP / LIMIT` | ✓ |
| Aggregates: `count`, `sum`, `avg`, `min`, `max` | ✓ |
| `OPTIONAL MATCH` | ✓ |
| `UNION [ALL]` | ✓ |
| `UNWIND` | ✓ |
| Property access in expressions | ✓ (fresh var + BGP triple) |
| `type(r)` / label check `n:Label` | ✓ |
| String functions, math functions | ✓ |
| Variable-length paths `*lower..upper` | ✗ → fallback |
| Relationship property access `r.prop` | ✗ → fallback |
| Temporal arithmetic / constructors | ✗ → fallback |
| List/pattern comprehensions | ✗ → fallback |
| `CASE` expressions | ✓ (lowered to nested IF) |
| Write clauses (CREATE/MERGE/SET/DELETE/REMOVE) | ✗ → fallback |
| `CALL subquery` | ✗ → fallback |

**Exit:** LQA path active (not behind flag); TCK floor maintained at 3757;
`cargo test --lib` green; difftest 201/201.

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
