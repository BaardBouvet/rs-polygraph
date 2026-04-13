# TCK 100% Plan

**Current**: 362/463 passed (78.2%)
**Target**: 463/463 (100%)
**Gap**: 101 scenarios

---

## Critical Oversight: RDF-star / SPARQL-star

The project's core design (see `implementation-plan.md`, `AGENTS.md`, `README.md`) is
built around **SPARQL-star** as the primary edge-property encoding, with reification as
a fallback for legacy engines. However, the TCK runner's `TckEngine` was configured
with `supports_rdf_star() = false`, meaning the entire TCK test suite runs in the
weaker reification-only mode.

**Oxigraph 0.4 fully supports RDF-star/SPARQL-star.** Switching `TckEngine` to
`supports_rdf_star() = true` and encoding CREATE relationship properties as
annotated triples in INSERT DATA immediately enables:

- Relationship property access (`r.prop`) in RETURN and WHERE
- Edge property filtering (`WHERE r.since > 2020`)
- `type(r)` function via the annotated triple's predicate
- Correct results for multi-relationship property scenarios

This was the original architectural intent — the RDF-star module
(`rdf_mapping::rdf_star`) and the `RdfStar` target engine adapter already exist but
were not wired into the TCK runner.

**Action**: Switch TCK to RDF-star mode as Phase 0 below. This also changes the
INSERT DATA generator to emit annotated triples for relationship properties instead
of skipping them (the current `// Relationship properties are skipped` comment in
`emit_create_pattern`).

---

## Failure Categories

| Category | Count | Scenarios |
|----------|-------|-----------|
| Parse errors (missing grammar/expression support) | 29 | See §1 |
| Row count mismatch (complex — node/rel/path results) | 28 | See §4 |
| Result set mismatch (scalar comparison wrong) | 12 | See §5 |
| UNWIND of variable / non-literal expression | 9 | See §3 |
| Row count mismatch (scalar — logic/translation bug) | 10 | See §6 |
| Bounded variable-length path (`*N..M`) | 5 | See §7 |
| Complex return expression unsupported | 5 | See §2 |
| Semantic error not detected | 2 | See §8 |
| MERGE clause unsupported | 1 | See §9 |

---

## §1 — Parse Errors (29 scenarios)

These queries fail because the PEG grammar (`grammars/cypher.pest`) and/or the expression parser lack support for certain Cypher constructs.

### §1a — Function calls in expressions (19 scenarios)

The grammar has no general `function_call` expression atom. These functions appear in WHERE, RETURN, and MATCH contexts:

| Function | Cypher Example | Scenarios |
|----------|---------------|-----------|
| `type(r)` | `WHERE type(r) = 'KNOWS'` | MatchWhere1:155, :233, :268, Match2:75, :92, Return2:246 |
| `length(p)` | `WHERE length(p) = 10` | MatchWhere1:251, :268 |
| `last(r)` | `RETURN last(r)` | Match9:45 |
| `nodes(p)` | `RETURN nOdEs(p)` | Return4:77 |
| `count(DISTINCT p)` | `RETURN coUnt(dIstInct p)` | Return4:176 |
| `avg(n.age)` | `RETURN aVg(n.aGe)` | Return4:205 |
| `collect(x)` | `RETURN collect(child.name)` | Return6:246, :294 |
| `abs(x)` | `sum((1 - abs(...)))` | Return6:246 |
| `head(collect(...))` | nested function call | Return4:141 |
| `min(length(p))` | `min(length(p))` | Return6:158 |

**Fix**: Add a `function_call` rule to the grammar:

```pest
function_call = { ident ~ "(" ~ (kw_DISTINCT? ~ expression ~ ("," ~ expression)*)? ~ ")" }
```

Add `function_call` as an `atom` alternative. In the AST, add `Expression::FunctionCall { name: String, distinct: bool, args: Vec<Expression> }`.

In the translator, map known functions:
- `type(r)` → extract the predicate local name from `edge_map[r]`
- `length(p)` → count hops in path (or use property-path length via subquery)
- `nodes(p)` / `relationships(p)` → path decomposition (complex; may need RDF collections)
- `last(list)` → take last element of VALUES binding
- `head(list)` → take first element
- `collect(expr)` → `GROUP_CONCAT` or custom aggregation
- `abs(x)` → `ABS()` (direct SPARQL mapping)
- `toInteger(x)` → `xsd:integer` cast
- `toString(x)` → `STR()`

### §1b — Pattern expressions in WHERE (3 scenarios)

Cypher allows pattern predicates: `WHERE (a)-[:T]->(b:Label)` as a boolean test.

| Scenario | Query |
|----------|-------|
| MatchWhere4:67 | `WHERE ... AND (a)-[:T]->(b:TheLabel) OR (a)-[:T*]->(b:MissingLabel)` |
| MatchWhere5:70 | `WHERE i.var > 'te' AND i:TextNode` |
| MatchWhere6:50 | `MATCH (a)-->(b) WHERE b:B OPTIONAL MATCH (a)-->(c) WHERE c:C` |

**Fix**: 
- Label predicate in WHERE (`i:TextNode`) → `EXISTS { ?i a <base:TextNode> }` or just add a triple pattern
- Pattern predicate in WHERE (`(a)-[:T]->(b)`) → `EXISTS { ... }` subquery
- Add `label_predicate` and `pattern_predicate` as `atom` alternatives in grammar

### §1c — Bidirectional relationship patterns `<-->` (2 scenarios)

| Scenario | Query |
|----------|-------|
| Match6:230 | `MATCH p = (n)<-->(k)<--(n)` |
| Match6:251 | `MATCH topRoute = (:Start)<-[:CONNECTED_TO]-()-[:CONNECTED_TO*3..3]-(:End)` |

The `<-->` syntax (both directions simultaneously) likely fails to parse. The grammar handles `-->`, `<--`, `--` but not `<-->`.

**Fix**: Add `<-->` as a direction variant meaning "any direction" (same as `--` semantically, but explicit). Grammar: `rel_right_arrow = { "<-->" | "-->" }`.

### §1d — Map literal return (1 scenario)

| Scenario | Query |
|----------|-------|
| Return2:151 | `RETURN {a: 1, b: 'foo'}` |

A bare `RETURN` with no MATCH. May parse fine but translation fails because there is no `MATCH` clause.

**Fix**: Support standalone `RETURN` (no MATCH) by emitting a single-row `VALUES` binding.

### §1e — Nested list literal in WITH (1 scenario)

| Scenario | Query |
|----------|-------|
| Unwind1:88 | `WITH [[1, 2, 3], [4, 5, 6]] AS lol UNWIND lol AS x ...` |

Nested list `[[1,2,3],[4,5,6]]` may fail parsing.

**Fix**: Ensure `list_literal` is recursive (list of lists). Should already work if `expression` includes `list_literal`.

### §1f — Multi-match with comma-separated patterns (3 scenarios)

| Scenario | Query |
|----------|-------|
| Match3:365 | `MATCH (a), (b), (c) MATCH (a)-->(x), (b)-->(x), (c)-->(x)` |
| MatchWhere1:45 | `MATCH (a)<--()<--(b)-->()-->(c) WHERE a:A` |
| MatchWhere1:62 | `MATCH (n) WHERE n.name = 'Bar' RETURN n` (this should parse — investigate) |

These are mostly multi-hop chain patterns. `(a)<--()<--(b)-->()-->(c)` has 5 nodes in a chain. Should work with existing grammar if it handles chains correctly.

**Fix**: Debug individual parse failures — many may have the same root cause (function calls in WHERE or RETURN).

### §1g — OPTIONAL MATCH keyword (2 scenarios)

| Scenario | Query |
|----------|-------|
| Match7:473 | Multi-line with `OPTIONAL MATCH` + pattern |
| Match7:538 | `OPTIONAL MATCH (a)-->(x) OPTIONAL MATCH (x)-[r]->(b)` |
| MatchWhere6:73 | `MATCH (n:Single) OPTIONAL MATCH (n)-[r]-(m) WHERE m:NonExistent` |

**Fix**: Ensure `OPTIONAL MATCH` is parsed and translated to `LEFT JOIN` in SPARQL. If a label predicate appears in WHERE of OPTIONAL MATCH (e.g., `WHERE m:NonExistent`), add the type triple inside the optional pattern.

### §1h — Other expression atoms (2 scenarios)

| Scenario | Query |
|----------|-------|
| Return3:44 | `MATCH (a)-[r]->() RETURN a AS foo, r AS bar` — parse error at pos 185 is `expected [_]` |
| Return4:109 | `MATCH () RETURN count(*) AS columnName` — parse error at pos 32 |

These look like they should parse. Debug individually.

---

## §2 — Complex Return Expressions (5 scenarios)

Translation currently throws "Unsupported feature: complex return expression (Phase 4+)" for expressions it can't map to SPARQL projection.

| Scenario | Query | Issue |
|----------|-------|-------|
| Return2:39 | `RETURN 1 + (2 - (3 * (4 / (5 ^ (6 % null)))))` | Arithmetic with `^` and `%` operators + null propagation |
| Return2:167 | `RETURN count(a) > 0` | Comparison wrapping aggregate |
| Return2:212 | `RETURN [n, r, m] AS r` | List construction with graph objects |
| Return2:229 | `RETURN {node1: n, rel: r, node2: m} AS m` | Map construction with graph objects |
| Return6:123 | `RETURN a.name, {foo: a.name='Andres', kids: collect(child.name)}` | Map with aggregate inside |

**Fix**: Extend `translate_return_expression()` to handle:
1. **Arithmetic**: Map `^` → `SPARQL POWER()` (custom extension or just compute), `%` → modulo not in SPARQL 1.1 (return error or use extension)
2. **Comparison wrapping aggregate**: `count(a) > 0` → project count into variable, then `BIND(IF(?count > 0, true, false))`
3. **List/Map of graph objects**: These need full node serialization → skip or implement as concatenated strings

**Realistic assessment**: Items 3-5 (list/map of graph objects) are extremely hard without an RDF graph serialization framework. Consider marking these as known limitations.

---

## §3 — UNWIND of Variable/Non-literal (9 scenarios)

Currently `UNWIND` only supports literal lists like `UNWIND [1, 2, 3] AS x`. It fails for:

| Type | Count | Examples |
|------|-------|---------|
| UNWIND of variable from WITH | 5 | `WITH collect(b1) AS bees UNWIND bees AS b2` |
| UNWIND of non-literal (`null`, `[]`) | 2 | `UNWIND null AS nil`, `UNWIND [] AS empty` |
| UNWIND of nested list | 1 | `WITH [[1,2,3]] AS lol UNWIND lol AS x UNWIND x AS y` |
| UNWIND with parameter | 1 | `UNWIND $events AS event` (already skipped via param) |

**Fix**: Translate `UNWIND ?var AS ?x` into a SPARQL lateral join or `VALUES`-based expansion.

For `UNWIND ?list AS ?item` where `?list` came from `collect()`:
- Use SPARQL 1.1 subquery that binds each element

For `UNWIND null` → empty result (0 rows)
For `UNWIND []` → empty result (0 rows)
For `UNWIND [[1,2,3],[4,5,6]]` → need RDF list support or VALUES flattening

**Realistic assessment**: Variable UNWIND is fundamentally hard in SPARQL 1.1. The main approach would be:
- For literal nested lists: flatten at compile time into VALUES
- For variable references to `collect()`: requires SPARQL engine extension (e.g., property functions) which violates the "no engine modification" constraint

**Recommendation**: Implement compile-time literal UNWIND expansion (fixes `null`, `[]`, nested lists = ~3 scenarios). Variable UNWIND from `collect()` results should be documented as a known SPARQL 1.1 limitation.

---

## §4 — Row Count Mismatch (Complex Results) (28 scenarios)

These produce the wrong number of rows. The result comparison is lenient (row count only) because the expected values contain node/rel/path shapes that we can't reconstruct.

Root causes:
1. **Cartesian product explosion**: `MATCH (n)` with no constraints → matches ALL nodes (including auxiliary ones from CREATE setup), producing N² rows instead of expected count
2. **Variable-length path `*` semantics**: SPARQL property paths return all reachable pairs, but Cypher returns distinct paths
3. **OPTIONAL MATCH not producing correct Left Join**: Missing bindings should result in null, not extra rows
4. **Self-loop handling**: SPARQL `?a ?p ?a` matches self-referencing triples differently than Cypher
5. **Relationship uniqueness**: Cypher guarantees each relationship is traversed at most once per path; SPARQL property paths don't

**Fix strategy per root cause**:
1. **Constrained MATCH**: The `__node` sentinel ensures `MATCH (n)` finds all nodes. But when there are labels/properties, the sentinel creates extra matches. Review the sentinel triple emission — only emit when truly unconstrained.
2. **Property path dedup**: Use `SELECT DISTINCT` more aggressively for variable-length paths.
3. **OPTIONAL MATCH**: Verify the `LeftJoin` pattern is correct. Ensure filter conditions from the OPTIONAL MATCH's WHERE go inside the optional block.
4. **Self-loops**: Add `FILTER(?a != ?b)` when relationship-uniqueness semantics require it.
5. **Relationship uniqueness**: For multi-relationship patterns, emit `FILTER(?r1 != ?r2)` to enforce uniqueness.

---

## §5 — Result Set Mismatch (Scalar) (12 scenarios)

These produce the right number of rows but wrong values.

| Scenario | Query | Likely Issue |
|----------|-------|-------------|
| Match1:110 | `MATCH (n), (m) RETURN n.num, m.num` | Cross-product values wrong |
| Match7:629 | `OPTIONAL MATCH p = (a)-[r*]->(x)` | Path/rel values |
| Match8:101 | `MATCH () MERGE (b) WITH * ...` | MERGE unsupported |
| Match9:117 | `WITH [r1, r2] AS rs ... MATCH (first)-[rs*]->(second)` | Var-list as relationship pattern |
| Return2:135 | `RETURN a.num + 1 AS foo` | Arithmetic on properties |
| Return4:45 | `RETURN cOuNt(*)` | Column name case preservation |
| Return4:77 | `RETURN nOdEs(p)` | `nodes()` function |
| Return4:141 | `WITH head(collect({...}))` | Nested function + map construction |
| Return6:44 | `RETURN n.num, count(n)` | GROUP BY property + count |
| Return6:175 | Aggregation with multiple relationship matches | Complex aggregate expression |
| Return6:191 | `RETURN me.age, me.age + count(you.age)` | Mixed aggregate expression |
| Return6:77 | `RETURN n.num, count(*)` — 2 instead of 1 | GROUP BY not working correctly |

**Fix**: Most of these need:
- **Arithmetic in projection**: `a.num + 1` → `BIND(?a_num + 1 AS ?foo)` after property lookup
- **GROUP BY support**: `RETURN n.num, count(*)` requires SPARQL GROUP BY — currently not emitted
- **Column name preservation**: `cOuNt( * )` should preserve original case as the column name
- **Property arithmetic**: Property values fetched as RDF literals need numeric type handling

---

## §6 — Row Count Mismatch (Scalar) (10 scenarios)

| Got | Expected | Count | Likely Cause |
|-----|----------|-------|-------------|
| 2 | 0 | 2 | False positive matches on __node sentinel |
| 2 | 1 | 2 | Cartesian product with sentinel |
| 12 | 1 | 2 | Massive cartesian product (12 = 4 nodes × 3 combinations?) |
| 5 | 0 | 1 | Variable-length path over-counting |
| 3 | 1 | 1 | Triple over-counting |
| 1 | 0 | 1 | Should return empty but got one row |
| 0 | 1 | 1 | Should return one row but got empty |

**Root cause**: The `__node` sentinel creates extra triple patterns that participate in cartesian products. When a pattern like `MATCH (n), (m)` is used, each sentinel match multiplies.

**Fix**: Restructure sentinel handling — use `GRAPH` named graphs or `FILTER EXISTS` to separate type markers from structural triples.

---

## §7 — Bounded Variable-Length Paths (5 scenarios)

| Scenario | Pattern | Bound |
|----------|---------|-------|
| Match6:273 | `[:KNOWS*3..3]` | exactly 3 |
| Match6:308 | `[:KNOWS*1..2]` | 1 to 2 |
| Match6:364 | `[:KNOWS*..2]` | 0 to 2 |
| Match9:65 | `[*2..2]` | exactly 2 |
| Match9:81 | `[*2..2]` | exactly 2 |
| Match9:98 | `[*2..2]` | exactly 2 |

SPARQL 1.1 property paths support `*` (zero or more), `+` (one or more), `?` (zero or one), and `{n,m}` is NOT supported. Fixed-length and bounded paths need to be unrolled into explicit triple pattern chains.

**Fix**: For `*N..M`:
- `*2..2` → chain of exactly 2 triples: `?a :p ?mid . ?mid :p ?b`
- `*1..2` → UNION of 1-hop and 2-hop patterns
- `*3..3` → chain of 3 triples
- `*0..2` → UNION of 0, 1, and 2-hop patterns (0-hop = same node)

Emit `UNION` over each path length in the range. This is a known workaround for SPARQL 1.1's lack of bounded repetition.

---

## §8 — Semantic Errors Not Detected (2 scenarios)

| Scenario | Query | Error Type |
|----------|-------|-----------|
| Return1:56 | `MATCH () RETURN foo` | UndefinedVariable |
| Return6:353 | `RETURN me.age + you.age, me.age + you.age + count(*)` | AmbiguousAggregationExpression |

**Fix**:
1. **UndefinedVariable**: In `validate_semantics`, collect all bound variables from MATCH + WITH + UNWIND. For each `Expression::Variable(v)` in RETURN, check that `v` is in the bound set. Error if not.
2. **AmbiguousAggregationExpression**: The current check only fires when `non_agg_items.is_empty()`. Scenario [21] has `me.age + you.age` as a non-agg item AND `me.age + you.age + count(*)` as an agg-mixed item. The openCypher rule: the non-aggregate subexpressions inside the mixed item must be **exactly** one of the GROUP BY keys (non-agg return items). `me.age + you.age` **is** returned separately, but the second expression doesn't use it as a single reference — it re-derives it from `me.age` and `you.age` individually. Fix: check that the free variables in the aggregate-mixed expression exactly match variables in the non-agg items (not compound sub-expressions).

---

## §9 — MERGE Clause (1 scenario)

| Scenario | Query |
|----------|-------|
| Match8:70 | `MATCH () MERGE (b) WITH * OPTIONAL MATCH (a)--(b) RETURN count(*)` |

MERGE is a write operation (upsert). Supporting it requires SPARQL UPDATE with INSERT/WHERE.

**Fix**: Implement `MERGE` as:
```sparql
INSERT { ... } WHERE { OPTIONAL { ... } FILTER(!BOUND(?existing)) }
```
Then follow with a SELECT query. This is substantial work.

**Recommendation**: Low priority. Document as known limitation.

---

## Implementation Phases

### Phase 0 — Enable RDF-star in TCK (prerequisite for all other phases)

**Effort**: Low (< 1 session)
**Impact**: Unlocks relationship property scenarios; estimate +5-10 scenarios immediately

1. Change `TckEngine.supports_rdf_star()` to return `true`
2. Update `emit_create_pattern()` in `tests/tck/main.rs` to emit RDF-star annotated
   triples for relationship properties:
   ```turtle
   << _:n0 <base:KNOWS> _:n1 >> <base:since> "2020" .
   ```
   instead of the current `// Relationship properties are skipped` no-op
3. Verify the translator's existing RDF-star code path
   (`rdf_mapping::rdf_star::annotated_triple`) fires correctly for `r.prop` access
   in RETURN and WHERE
4. Re-run TCK and re-baseline the pass count

This is the **single highest-ROI change** because it activates an entire code path
(`rdf_star: true` in `TranslationState`) that was designed for exactly this purpose
but was never wired into the TCK runner. Oxigraph 0.4 supports RDF-star natively.

### Phase A — Grammar & Parser (targets ~29 parse-error scenarios)

**Effort**: Medium
**Impact**: +29 scenarios (if all translate correctly after parsing)

1. Add `function_call` expression atom to grammar
2. Add `Expression::FunctionCall` to AST
3. Add `<-->` direction support
4. Add `label_predicate` in WHERE (`n:Label` as boolean)
5. Add `pattern_predicate` in WHERE (`(a)-[:T]->(b)` as boolean)
6. Verify `OPTIONAL MATCH` with WHERE parses correctly
7. Verify standalone `RETURN` (no MATCH) parses correctly
8. Verify nested list literals parse correctly

### Phase B — Function Translation (depends on Phase A)

**Effort**: Medium-High
**Impact**: Enables many of the 29 parse-error scenarios to pass

Map Cypher functions to SPARQL:

| Cypher | SPARQL |
|--------|--------|
| `type(r)` | Extract local name from predicate IRI (from edge_map) |
| `length(p)` | Count hops in path pattern — unroll |
| `nodes(p)` | Collect path nodes — very hard in SPARQL 1.1 |
| `last(list)` | Custom — take last VALUES binding |
| `head(list)` | Custom — take first VALUES binding |
| `collect(expr)` | `GROUP_CONCAT` (lossy) or extension |
| `abs(x)` | `ABS()` in SPARQL |
| `toString(x)` | `STR()` in SPARQL |
| `toInteger(x)` | `xsd:integer` cast |

### Phase C — Bounded Paths (targets 5 scenarios)

**Effort**: Medium
**Impact**: +5 scenarios

Unroll `*N..M` into UNION of fixed-length chain patterns.

### Phase D — GROUP BY & Aggregation (targets ~8 scenarios)

**Effort**: Medium
**Impact**: +8 scenarios  

1. Emit `GROUP BY` when RETURN mixes aggregate and non-aggregate expressions
2. Arithmetic in RETURN projection via `BIND`
3. `count(DISTINCT expr)` support
4. Mixed aggregate expressions (`age + count(*)`)

### Phase E — UNWIND of Variables (targets ~6 scenarios)

**Effort**: High
**Impact**: +6 scenarios (compile-time evaluable: +3)

1. `UNWIND null` → empty result set
2. `UNWIND []` → empty result set  
3. Nested literal UNWIND → flatten at compile time
4. Variable UNWIND from collect() → SPARQL 1.1 limitation

### Phase F — Relationship Uniqueness & Cartesian Products (targets ~20 scenarios)

**Effort**: High
**Impact**: +20 scenarios (row count fixes)

1. Emit `FILTER(?r1 != ?r2)` for multi-relationship patterns
2. Fix sentinel `__node` triple to not participate in general queries
3. Use `DISTINCT` for property-path results
4. Correct OPTIONAL MATCH LEFT JOIN placement with WHERE filters

### Phase G — Semantic Validation (targets 2 scenarios)

**Effort**: Low
**Impact**: +2 scenarios

1. UndefinedVariable check
2. AmbiguousAggregationExpression compound expression check

### Phase H — Complex Return Expressions (targets 3 scenarios)

**Effort**: Medium
**Impact**: +3 realistically (2 with graph objects are very hard)

1. Arithmetic with null propagation
2. Comparison over aggregate (`count(a) > 0`)
3. List/Map of graph objects — document as limitation

### Phase I — MERGE (targets 1 scenario)

**Effort**: High
**Impact**: +1 scenario

Implement MERGE as conditional INSERT + SELECT. Low priority.

---

## Recommended Execution Order

| Order | Phase | Scenarios Fixed | Running Total | % |
|-------|-------|-----------------|---------------|---|
| **0** | **0 — Enable RDF-star** | **+5..10** | **367-372** | **79-80%** |
| 1 | G — Semantic checks | +2 | 374 | 80.8% |
| 2 | A — Grammar/parser | +29 (parse only) | — | — |
| 3 | B — Function translation | +~15 (parse+translate) | 389 | 84.0% |
| 4 | C — Bounded paths | +5 | 394 | 85.1% |
| 5 | D — GROUP BY / aggregation | +8 | 402 | 86.8% |
| 6 | F — Uniqueness / cartesian | +20 | 422 | 91.1% |
| 7 | E — UNWIND variables | +3..6 | 428 | 92.4% |
| 8 | H — Complex return exprs | +3 | 431 | 93.1% |
| 9 | I — MERGE | +1 | 432 | 93.3% |

**Realistic ceiling with SPARQL-star**: ~432/463 (93%) — significantly higher than
the previous 91% estimate because RDF-star resolves relationship property scenarios
that were previously blocked.

**Hard 100% blockers** (~31 scenarios, require approximations or extensions):
- `nodes(p)` / `relationships(p)` — path decomposition (no SPARQL equivalent)
- UNWIND of `collect()` result — requires runtime list iteration
- Map/List of graph objects in RETURN — requires node serialization
- Some complex aggregation patterns with nested `collect()` + map construction

**Path to 100%**: The remaining ~31 scenarios after Phase I break down as:
- ~15 fundamentally hard (path decomposition, runtime UNWIND, graph-object
  serialization) — would need engine-specific custom functions or compile-time
  approximations
- ~16 potentially fixable with deeper translation work (complex nested
  aggregations, multi-statement transaction semantics, DELETE+RETURN)

For truly engine-agnostic 100%, the project would need to define a small set of
custom SPARQL functions (e.g., `pg:nodes()`, `pg:type()`) that target engines
implement. This is consistent with the `TargetEngine` trait design — engines
that support these extensions get fuller coverage.

---

## Estimated Total Effort

| Phase | Effort |
|-------|--------|
| **0 — RDF-star** | **< 1 session** |
| A — Grammar | 2-3 sessions |
| B — Functions | 2-3 sessions |
| C — Bounded paths | 1 session |
| D — GROUP BY | 1-2 sessions |
| E — UNWIND | 1-2 sessions |
| F — Uniqueness | 2-3 sessions |
| G — Semantic | < 1 session |
| H — Complex return | 1 session |
| I — MERGE | 1 session |
| **Total** | **~13-17 sessions** |

Phase 0 (RDF-star) should be done **first** as it changes the baseline for
all subsequent measurements and may resolve scenarios currently categorized
under other failure buckets (especially §4 row-count mismatches involving
relationship properties).

---

## §10 — SPARQL Extension Functions: Deep Dive

### What we actually need

After analyzing the ~31 scenarios beyond the 93% ceiling, the **truly hard
blockers** reduce to approximately **10 scenarios across 5 capability gaps**.
The rest are addressable with deeper translation work (complex expressions,
GROUP BY improvements, etc.) or standard SPARQL.

### Gap 1: `type(r)` — Relationship type extraction

**Scenarios**: MatchWhere1:155, :233, :268, Match2:75, :92, Return2:246

**Cypher**: `WHERE type(r) = 'KNOWS'` / `RETURN type(r)`

**Analysis**: This is **not actually a hard blocker**. It can be solved with
standard SPARQL:

```sparql
# Given edge_map[r] = { pred: <base:KNOWS> }
# type(r) → extract local name from the predicate IRI
BIND(REPLACE(STR(?r_pred), "^.*[/#]", "") AS ?type_r)
```

Since our translator already tracks `edge_map[r].pred` (the predicate
`NamedNode`), we know the relationship type IRI at translation time. For a
single-type relationship, `type(r)` is a compile-time constant. For
multi-type (`[:A|B]`), we bind the predicate variable and extract the local
name.

**Engine coverage**:
- **Standard SPARQL 1.1**: `REPLACE(STR(?pred), "^.*[/#]", "")` ✓
- **Jena**: `afn:localname(?pred)` ✓
- **Oxigraph**: standard REPLACE works ✓

**Verdict**: Solvable in Phase B (function translation). **No extension needed.**

---

### Gap 2: `collect()` — List aggregation

**Scenarios**: Return6:123 (`{kids: collect(child.name)}`), Return6:246,
Return6:294

**Cypher**: `collect(expr)` — aggregates values into an ordered list.

**Analysis**: SPARQL 1.1 has `GROUP_CONCAT` which concatenates strings with a
separator. This is **lossy** (can't distinguish `"a,b"` from `["a","b"]`) and
doesn't work for non-string types. However, for TCK compliance the comparison
is against string representations.

**Engine coverage**:
- **Standard SPARQL 1.1**: `GROUP_CONCAT(?val; separator=",")` — lossy
- **Jena**: Custom aggregates via Java ServiceLoader — can implement true
  `collect()` returning RDF lists
- **Oxigraph**: `with_custom_aggregate_function()` — Rust `AggregateFunctionAccumulator`
  trait. Can build a proper list:
  ```rust
  struct CollectAccumulator { items: Vec<Term> }
  impl AggregateFunctionAccumulator for CollectAccumulator {
      fn accumulate(&mut self, element: Term) { self.items.push(element); }
      fn finish(&mut self) -> Option<Term> {
          // serialize as "[item1, item2, ...]" string literal
          Some(Literal::new_simple_literal(format!("[{}]",
              self.items.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ")
          )).into())
      }
  }
  ```

**Complication**: Return6:123 uses `collect()` **inside a map literal**:
```cypher
RETURN a.name, {foo: a.name='Andres', kids: collect(child.name)}
```
This requires both `collect()` as an aggregate **and** map construction in
RETURN. The map-in-RETURN is a separate issue (§2).

**Verdict**: `collect()` alone is solvable with `GROUP_CONCAT` (lossy) for
standard SPARQL, or custom aggregate on Oxigraph/Jena. The scenarios that use
it inside map literals remain blocked by map construction (Gap 5).

---

### Gap 3: UNWIND of runtime variables (4 scenarios)

| Scenario | Query | Kind |
|----------|-------|------|
| Unwind1 [5] | `WITH collect(row) AS rows UNWIND rows AS node` | UNWIND of `collect()` result |
| Unwind1 [11] | `WITH [1,2,3] AS list UNWIND list AS x RETURN *` | UNWIND of WITH-bound list variable |
| Unwind1 [12] | `WITH a, collect(b1) AS bees UNWIND bees AS b2 MATCH ...` | UNWIND + re-MATCH |
| Unwind1 [13] | `WITH [1,2] AS xs, [3,4] AS ys UNWIND xs AS x UNWIND ys AS y` | Cascaded variable UNWIND |

**Analysis**: SPARQL has no native UNNEST/LATERAL JOIN. The fundamental
problem is: given a binding `?list = "[1, 2, 3]"`, generate 3 rows with
`?x = 1`, `?x = 2`, `?x = 3`.

**Approaches**:

**A. Compile-time expansion** (works for literals only):
When the UNWIND expression is a literal list or a WITH-bound literal, the
translator can expand at compile time:
```cypher
WITH [1, 2, 3] AS list UNWIND list AS x
```
→
```sparql
VALUES (?x ?list) { (1 "[1, 2, 3]") (2 "[1, 2, 3]") (3 "[1, 2, 3]") }
```
This handles scenarios [11] and [13] (both use WITH-bound literal lists).

**B. Two-phase execution** (works for `collect()` results):
1. Execute query up to the `collect()`, get the result
2. Inject the collected values as `VALUES` into a second query
3. Execute the second query

This is a **runtime** strategy. The `TargetEngine` trait would need:
```rust
fn supports_two_phase_unwind(&self) -> bool { false }
```
Or rs-polygraph returns a `QueryPlan` with multiple stages instead of a
single SPARQL string. This is a significant architectural change.

**C. Jena property functions**:
```sparql
?list list:member ?item
```
Jena's `list:member` iterates RDF Collection members. But the collected
values would need to be stored as RDF Collections (rdf:first/rdf:rest
linked lists), not as string literals.

**D. Oxigraph custom function** — no equivalent to property functions.
Oxigraph only has scalar and aggregate custom functions, not property
functions that generate multiple bindings.

**Verdict**:
- Scenarios [11] and [13]: **Solvable** via compile-time expansion (literal
  lists bound in WITH). No extension needed.
- Scenario [5] and [12]: **Hard.** Requires either two-phase execution
  (architectural change) or engine-specific list iteration (Jena only).

---

### Gap 4: `nodes(p)` / `relationships(p)` — Path decomposition (1 scenario)

**Scenario**: Return4 [5]:
```cypher
MATCH p = (n)-->(b) RETURN nOdEs( p )
```
Expected: list of nodes traversed in the path.

**Analysis**: SPARQL property paths (`?a :p+ ?b`) find endpoints but **do not
expose intermediate nodes**. There is no standard way to extract the sequence
of nodes along a path.

**Approaches**:

**A. Fixed-length unrolling**: For paths of known length, unroll into
explicit triple chains:
```sparql
# For a 1-hop path p = (n)-->(b):
SELECT ?n ?b WHERE { ?n ?p ?b }
# nodes(p) = [?n, ?b]
```
For the specific scenario (1-hop `-->`), `nodes(p)` is just `[n, b]`.
The translator knows the path length at compile time for fixed-length
patterns.

**B. Custom function on Oxigraph**: Register `<pg:nodes>` that takes the
path endpoints and predicate, then queries the store to reconstruct the path.
But this requires access to the store during function evaluation, which
Oxigraph's `with_custom_function` closure doesn't provide.

**C. Jena ARQ**: No built-in path decomposition either. Would need a custom
property function with access to the dataset.

**Complication**: The expected result is a **list of node objects** (not
scalars), which is the graph-object serialization problem (Gap 5).

**Verdict**: For fixed-length paths, the node list is known at compile time.
For variable-length paths, this is **fundamentally unsolvable** in standard
SPARQL. However, the specific TCK scenario uses `(n)-->(b)` (1-hop), so
the result is simply `[n, b]` — but returning it as a **list of nodes** hits
Gap 5.

---

### Gap 5: Graph object serialization in RETURN (3 scenarios)

| Scenario | Query | Expected |
|----------|-------|----------|
| Return2 [12] | `RETURN [n, r, m] AS r` | `[(:A), [:T], (:B)]` |
| Return2 [13] | `RETURN {node1: n, rel: r, node2: m} AS m` | `{node1: (:A), rel: [:T], node2: (:B)}` |
| Return6 [6] | `RETURN a.name, {foo: ..., kids: collect(...)}` | map with aggregate |

**Analysis**: Cypher's `RETURN` can project **graph objects** (nodes,
relationships) and construct **lists and maps** containing them. RDF/SPARQL
has no equivalent — SPARQL SELECT returns RDF terms (URIs, literals, blank
nodes), not structured graph objects with labels, properties, and types.

The expected output `(:A)` means "a node with label A" — this requires
reconstructing the node's labels and properties from the RDF store and
serializing them into Cypher's display format.

**Approaches**:

**A. Approximate with IRI serialization**:
```sparql
# Instead of (:A), return the blank node IRI
SELECT ?n WHERE { ?n a <base:A> }
```
The TCK test runner already handles this via `is_complex_tck_value()` — it
compares row counts only for complex results. These scenarios currently fail
on **row count**, not on value comparison.

**B. Construct Cypher display strings**:
Build `CONCAT("(:", STR(?label), ")")` in SPARQL to approximate the display
format. Very fragile and doesn't generalize.

**C. Custom function on Oxigraph**:
```rust
// pg:node_display(?node) → "(:A {name: 'Alice'})"
.with_custom_function(NamedNode::new("http://polygraph.example/node_display")?, |args| {
    // Would need store access to look up labels and properties — NOT POSSIBLE
    // from a scalar function closure
})
```
Oxigraph custom functions only take `&[Term]` — they don't have access to
the store. This approach is **not feasible**.

**D. Post-processing in the TCK runner**:
After getting SPARQL results, the TCK runner could query the store for each
returned node/relationship to reconstruct its Cypher display form. This is
a **test-infrastructure** solution, not a translator solution.

**Verdict**: **Fundamentally unsolvable** in the translator. The Cypher→SPARQL
boundary cannot preserve graph object structure. These 3 scenarios represent
a semantic mismatch between Cypher's object model and RDF's term model.

For the TCK runner specifically, approach D (post-processing) could work,
but it's test-specific, not part of the transpiler's output.

---

### Gap 6: DELETE + RETURN (1 scenario)

**Scenario**: Return2 [14]:
```cypher
MATCH ()-[r]->() DELETE r RETURN type(r)
```
Expected: `type(r) = 'T'` with side effect `-relationships 1`.

**Analysis**: This requires DELETE (SPARQL UPDATE) **followed by** RETURN
(SPARQL SELECT) in a single query, with the deleted binding still available.
SPARQL UPDATE and SPARQL SELECT are separate operations.

**Approach**: Execute as two operations:
1. `SELECT` to capture the bindings (save `type(r)`)
2. `DELETE` to remove the triples
3. Return the saved bindings

This is a **multi-phase execution** problem similar to Gap 3B.

**Verdict**: Requires multi-phase execution. Not solvable as a single SPARQL
query.

---

### Gap 7: MERGE clause (1 scenario)

**Scenario**: Match8 [2]:
```cypher
MATCH (a) MERGE (b) WITH * OPTIONAL MATCH (a)--(b) RETURN count(*)
```

**Analysis**: MERGE is a conditional upsert. In SPARQL, this would be:
```sparql
INSERT { _:b <__node> <__node> }
WHERE { FILTER NOT EXISTS { ?b <__node> <__node> } }
```
Then a SELECT for the RETURN. Again, multi-phase execution.

**Verdict**: Requires SPARQL UPDATE + multi-phase execution.

---

### Summary: True Hard Blockers

| Gap | Scenarios | Root Cause | Solvable? |
|-----|-----------|-----------|-----------|
| 1. `type(r)` | 6 | Function mapping | **Yes** — standard SPARQL `REPLACE(STR())` |
| 2. `collect()` | 3 | List aggregation | **Partially** — `GROUP_CONCAT` or custom aggregate; blocked when inside map literal |
| 3. UNWIND variables | 4 | No SQL-like UNNEST | **2 yes** (compile-time expansion), **2 hard** (runtime collect) |
| 4. `nodes(p)` | 1 | Path decomposition | **No** for variable-length; fixed-length computable but hits Gap 5 |
| 5. Graph object display | 3 | Cypher vs RDF model | **No** — fundamental semantic mismatch |
| 6. DELETE+RETURN | 1 | Multi-phase execution | **Possible** with architectural change |
| 7. MERGE | 1 | SPARQL UPDATE | **Possible** with architectural change |

**Truly unsolvable in a single SPARQL query**: 5 scenarios (Gaps 4, 5)
**Solvable with architectural change** (multi-phase): 4 scenarios (Gaps 3 partial, 6, 7)
**Solvable with standard SPARQL**: 8+ scenarios (Gaps 1, 2 partial, 3 partial)

### Revised Realistic Ceiling

With all phases A–I complete **plus** `type(r)` via REPLACE/STR and
compile-time UNWIND expansion:

**~456/463 (98.5%)**

The remaining **7 scenarios** are:
1. Return2 [12]: `RETURN [n, r, m]` — graph objects in list
2. Return2 [13]: `RETURN {node1: n, ...}` — graph objects in map
3. Return6 [6]: `RETURN {foo: ..., kids: collect(...)}` — map with aggregate
4. Unwind1 [5]: `UNWIND collect(row)` — runtime variable UNWIND
5. Unwind1 [12]: `UNWIND bees` (from collect) + re-MATCH — runtime UNWIND
6. Return2 [14]: `DELETE r RETURN type(r)` — multi-phase execution
7. Match8 [2]: `MERGE (b)` — SPARQL UPDATE multi-phase

Of these, **items 6 and 7** could be solved with a `QueryPlan` architecture
that returns a sequence of SPARQL operations rather than a single string.
**Item 4 and 5** could be solved with two-phase execution or Jena's
`list:member`. **Items 1-3** are the true semantic boundary — Cypher returns
structured graph objects, SPARQL returns RDF terms.

### Engine Extension Mapping

| Extension | IRI | Type | Oxigraph | Jena | Generic |
|-----------|-----|------|----------|------|---------|
| `type(r)` | — | Not needed | `REPLACE(STR())` | `afn:localname()` | `REPLACE(STR())` |
| `collect()` | `<pg:collect>` | Custom aggregate | `with_custom_aggregate_function` | Java ServiceLoader | `GROUP_CONCAT` (lossy) |
| `head(list)` | `<pg:head>` | Custom function | `with_custom_function` | `list:index` | Not available |
| `last(list)` | `<pg:last>` | Custom function | `with_custom_function` | `list:index` | Not available |
| `unnest(list)` | `<pg:unnest>` | Property function | **Not available** | `list:member` | Not available |
| `nodes(p)` | `<pg:nodes>` | Requires store access | **Not feasible** | **Not feasible** | Not available |

### TargetEngine Trait Extensions

```rust
pub trait TargetEngine {
    // Existing
    fn supports_rdf_star(&self) -> bool;
    fn supports_federation(&self) -> bool;
    fn base_iri(&self) -> Option<&str>;
    
    // New: extension capabilities
    /// Whether the engine has a list-member property function (e.g., Jena's list:member)
    /// that can iterate RDF Collection members, enabling UNWIND of runtime variables.
    fn supports_list_unnest(&self) -> bool { false }
    
    /// IRI for the list-member property function. Default: Jena's list:member.
    fn list_unnest_iri(&self) -> Option<&str> { None }
    
    /// Whether the engine supports custom aggregate functions (e.g., collect()).
    fn supports_custom_aggregates(&self) -> bool { false }
    
    /// Whether the transpiler should emit multi-phase query plans
    /// (Vec<String> instead of single String) for DELETE+RETURN, MERGE, etc.
    fn supports_multi_phase(&self) -> bool { false }
}
```
