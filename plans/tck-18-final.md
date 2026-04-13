# Plan: Final 18 TCK Failures (446 → 463)

**Current**: 446 / 463 passed (96.3%)  
**Realistic target**: 457–460 / 463 (98.7–99.4%)  
**Stretch goal**: 463 / 463 (100%)

> 17 step failures + 1 Gherkin parse error = 18 total cucumber failures.

---

## Failure Inventory (refreshed)

| # | Feature:Line | Scenario | Cypher Query (key part) | Error / Symptom | Category |
|---|-------------|----------|------------------------|-----------------|----------|
| 1 | Match4:129 | [5] Varlen with property predicate | `[:WORKED_WITH* {year:1988}]` | 3 rows vs 1 — no per-hop property filter | P1 |
| 2 | Match4:171 | [7] Bound rel reuse in varlen | `MATCH ()-[r:EDGE]-()` then `[*0..1]-()-[r]-()-[*0..1]` | 0 vs 32 — bound rel `r` not threaded into second pattern | P3 |
| 3 | Match4:192 | [8] Rel list as path predicate | `[rs*]` (collected rel list) | 3 vs 1 — `[rs*]` syntax unimplemented | P5 |
| 4 | Match6:273 | [14] Undirected fixed varlen named path | `()-[:CONNECTED_TO*3..3]-(:End)` | 0 rows vs 4 — undirected fixed+typed varlen with parallel edges | P2 |
| 5 | Match7:232 | [11] Undirected rel + OPTIONAL + `r<>r2` | `WHERE r <> r2` | 4 vs 2 — relationship identity comparison not working | P2 |
| 6 | Match7:255 | [12] Variable-length OPTIONAL | `OPTIONAL MATCH (a)-[*]->(b)` | 3 vs 4 — property-path dedup loses multi-path endpoints | P4 |
| 7 | Match7:302 | [14] Varlen OPTIONAL with length predicate | `OPTIONAL MATCH (a)-[*3..]-(b)` returns nodes vs null | 3 vs 1 — undirected `*3..` matches too broadly | P2 |
| 8 | Match8:70 | [2] MATCH, MERGE, OPTIONAL MATCH | `MERGE (b)` | MERGE unsupported | P6 |
| 9 | Match9:45 | [1] Varlen rel vars are lists | `last(r)` on varlen variable | `last(r)` on relationship list unsupported | P5 |
| 10 | Match9:117 | [5] Varlen + label on both sides | `(a:Blue)-[r*]->(b:Green) RETURN count(r)` | 0 vs 1 — `count(r)` on path var returns 0 | P2 |
| 11 | Match9:138 | [6] Rel list + varlen + bound nodes | `[rs*]` with bound endpoints | Parse error: `expected OPTIONAL` | P5 |
| 12 | Match9:159 | [7] Same as [6], wrong direction | `[rs*]` wrong direction | Parse error: `expected OPTIONAL` | P5 |
| 13 | MatchWhere4:67 | [2] Disjunctive pattern predicates | `AND (a)-[:T]->(b) OR (a)-[:T*]->(b:Missing)` | 3 vs 1 — AND/OR precedence wrong in pattern predicates | P1 |
| 14 | Return2:135 | [7] List property concatenation | `a.list2 + a.list1` | null — list `+` not implemented | P3 |
| 15 | Return2:246 | [14] Type of deleted rel | `DELETE r RETURN type(r)` | DELETE unsupported | P6 |
| 16 | Return4:109 | [5] `nodes(p)` on path | `nOdEs(p)` function | unsupported complex expression | P5 |
| 17 | Return4:205 | [11] Reusing vars in RETURN | `head(collect({likeTime: l})).likeTime` | null — map property access on aggregated maps | P3 |
| 18 | Match5 (all) | Gherkin parse error | N/A — `#encoding: utf-8` + `®` char | Gherkin crate can't parse file | P0 |

---

## Priority Tiers

### P0 — Gherkin Parse Fix (0→? tests, trivial)

**Failure**: #18 (Match5.feature Gherkin parse error)

The `cucumber` crate can't parse `Match5.feature` because the file has `#encoding: utf-8` directive and contains `®` (registered trademark, U+00AE) in the license header. This causes a Gherkin parse error that silently skips all scenarios in the file.

**Fix**: Strip or replace the problematic character in the feature file (or fork the header). Alternatively, configure the cucumber crate's encoding handling. This is a test infrastructure issue.

**Action**: Check how many scenarios are in Match5.feature. If all were already passing before the parse error, fixing this adds no test gains but removes the "1 parsing error" from the summary. If some scenarios were being silently skipped, this could reveal new passes (or new failures).

**Effort**: Trivial — 5 minutes.

---

### P1 — Per-hop Property Filters & Pattern Predicate Precedence (2 tests: #1, #13)

#### #1: Match4[5] — `[:WORKED_WITH* {year:1988}]`

**Problem**: `MATCH (a:Artist)-[:WORKED_WITH* {year:1988}]->(b:Artist)` should only traverse edges where `year=1988`. Our translator ignores the property predicate on variable-length relationships.

**Approach — Path Unrolling with RDF-star annotation filters**:

For a bounded varlen `[:T* {prop:val}]`, unroll to fixed-length UNION chains where each hop has an RDF-star annotation filter:

```sparql
# 1-hop:
{ ?a <WORKED_WITH> ?b . << ?a <WORKED_WITH> ?b >> <year> 1988 . }
UNION
# 2-hop:
{ ?a <WORKED_WITH> ?m1 . << ?a <WORKED_WITH> ?m1 >> <year> 1988 .
  ?m1 <WORKED_WITH> ?b  . << ?m1 <WORKED_WITH> ?b >> <year> 1988 . }
UNION
# ... up to upper bound (default cap: 10)
```

Since we already have `emit_bounded_path_union` for direction combos, this is an extension of the same approach: add property filter triples per hop.

**Effort**: Medium. Requires threading `rel_properties` from the AST through `emit_bounded_path_union`.

#### #13: MatchWhere4[2] — AND/OR precedence with pattern predicates

**Problem**: `WHERE a.id = 0 AND (a)-[:T]->(b:TheLabel) OR (a)-[:T*]->(b:MissingLabel)` returns 3 results instead of 1. Our EXISTS translation works, but AND/OR precedence is wrong.

In Cypher, `AND` binds tighter than `OR`. So this is:  
`(a.id = 0 AND EXISTS{...}) OR EXISTS{...T*...MissingLabel}`

The `OR EXISTS{...MissingLabel}` branch should match nothing (no `:MissingLabel` nodes exist), so only the AND branch matters. But we're getting 3 rows — probably the OR is being parsed as `a.id = 0 AND (patternA OR patternB)`.

**Fix**: Ensure the expression parser respects AND > OR precedence. Cypher precedence: NOT > AND > OR > XOR. The expression parser needs to build nested trees correctly.

**Effort**: Small-medium. Debug the expression tree for this query and fix the precedence in `translate_expr` or the parser.

---

### P2 — Variable-Length Path Semantics Fixes (4 tests: #4, #5, #7, #10)

#### #4: Match6[14] — Undirected typed varlen returning 0 rows

**Query**: `(:Start)<-[:CONNECTED_TO]-()-[:CONNECTED_TO*3..3]-(:End)`

This is a mixed pattern: one directed hop `<-[:CONNECTED_TO]-` followed by an undirected 3-hop varlen `[:CONNECTED_TO*3..3]-`. The graph has parallel edges from `mid` to `db2` (two `CONNECTED_TO` edges), so there are 4 distinct 3-hop undirected paths.

**Problem**: Our `emit_bounded_path_union` handles typed+bounded correctly, but the 0-row result suggests it's not matching at all. Likely the preceding directed hop isn't correctly joined with the varlen.

**Fix**: Debug the generated SPARQL for this pattern. Ensure that `emit_bounded_path_union` works for typed undirected patterns with preceding fixed hops.

**Effort**: Medium — likely a join variable name mismatch.

#### #5: Match7[11] — Undirected rel + OPTIONAL + `r <> r2`

**Query**: `MATCH (a)-[r {name:'r1'}]-(b) OPTIONAL MATCH (b)-[r2]-(c) WHERE r <> r2`

Expected: 2 rows. Got: 4 rows.

**Problem**: Undirected MATCH `(a)-[r]-(b)` matches both directions of each edge. For a single edge A→B with name='r1': two rows (A,B) and (B,A). Then for each, OPTIONAL MATCH `(b)-[r2]-(c) WHERE r <> r2` expands. The issue is that `r <> r2` compares relationship *identity*, but we're comparing the edge variable as a node IRI.

**Analysis**: In our RDF-star mapping, relationship variables are bound to the triple's subject (or a reification node). Relationship identity comparison `r <> r2` needs to compare the actual edge identity, not just `!=` on arbitrary variables.

**Fix**: When comparing relationship variables with `<>`, emit `FILTER(!(?r_s = ?r2_s && ?r_p = ?r2_p && ?r_o = ?r2_o))` — compare all three components (subject, predicate, object) of the reified edge. Or, if edge variables are mapped to unique edge IRIs (via RDF-star `<< s p o >>`), a simple `!=` suffices.

**Effort**: Medium-hard. Requires tracking which variables are relationship variables and generating correct comparison expressions.

#### #7: Match7[14] — Undirected `*3..` matches too broadly

**Query**: `OPTIONAL MATCH (a:Single)-[*3..]-(b)` on a graph where the longest path from `:Single` is 2 hops.

Expected: `null` (no match at 3+ hops). Got: 3 rows.

**Problem**: The undirected untyped `*3..` varlen is over-matching. The NPS-based approach (`!(rdf:type|__node)`) without upper bound falls back to `OneOrMore` path, which matches at any depth ≥1, not ≥3. The *3..* lower bound of 3 isn't applied.

**Fix**: For `*N..` with N>1 (no upper bound), we need bounded unrolling. SPARQL property paths don't support minimum-only bounds. Options:
1. **Unroll to UNION**: 3-hop, 4-hop, ..., up to a cap (e.g., 20 hops). This is the same emit_bounded_path_union approach.
2. **Compose paths**: `path{3,} = path*3 / path*` — use a 3-hop fixed prefix followed by `*` closure. Spargebra supports `Sequence(FixedLengthPath(3), ZeroOrMore(path))`.

Actually SPARQL 1.1 property paths don't have repetition counts, but we can compose:
```
?a (nps/nps/nps)/(nps*) ?b
```  
where `nps = !(rdf:type|__node)`. This means "at least 3 hops".

**Effort**: Medium. Requires composing path expressions for `*N..` patterns.

#### #10: Match9[5] — `count(r)` on varlen returns 0

**Query**: `MATCH (a:Blue)-[r*]->(b:Green) RETURN count(r)`

Expected: 1. Got: 0.

**Problem**: In a typed varlen `[r*]`, `r` should be bound to the list of relationships along the path. In SPARQL property path mode, `r` is never bound — property paths don't bind intermediate edge variables.

**Fix approach A**: For `count(r)` where `r` is a varlen relationship variable, emit `COUNT(*)` instead — counting the number of matched rows (source-target pairs).

**Fix approach B**: When a varlen relationship variable is referenced in the RETURN clause (not just used for matching), switch from property-path mode to bounded-unrolling mode, which does bind intermediate variables.

Approach A is simpler and works for this specific case. The expected result is `count(r) = 1` meaning one path was found (Blue→Red→Green has 2 hops, so `r` is a list of 2 rels, but `count(r)` counts the number of rows, not list elements).

Wait — re-reading: `count(r)` in Cypher counts the number of non-null values of `r`. Since there's one matching path (Blue→Red→Green), `r` is non-null once, so `count(r) = 1`. In our SPARQL, if the property path `?a <T>+ ?b` matches `(Blue, Green)`, that's 1 row, but `?r` is unbound. So `COUNT(?r) = 0` because `?r` is null.

**Fix**: For varlen patterns where the relationship variable is referenced in expressions, bind a placeholder variable. For typed property paths where we don't know intermediate edges, bind the predicate IRI as a constant:  
```sparql
?a <T>+ ?b . BIND(<T> AS ?r)
```
Then `COUNT(?r) = 1` because `?r` is bound.

Actually, better: the relationship variable for a property-path pattern could be bound to a synthetic "path found" marker: `BIND(true AS ?r_bound)`. Then `count(r)` translates to `COUNT(?r_bound)`.

**Effort**: Small-medium. Need to detect when a varlen relationship variable is used in expressions and bind a marker.

---

### P3 — Expression / Function Gaps (3 tests: #2, #14, #17)

#### #2: Match4[7] — Bound relationship reuse

**Query**: `MATCH ()-[r:EDGE]-() MATCH p = (n)-[*0..1]-()-[r]-()-[*0..1]-(m) RETURN count(p) AS c`

Expected: 32. Got: 0.

**Problem**: The second MATCH reuses relationship variable `r` from the first MATCH. `r` is already bound to a specific edge. The pattern `[*0..1]-()-[r]-()-[*0..1]` means: 0-or-1 hops, then the specific edge `r`, then 0-or-1 hops. This is a join constraint.

In our SPARQL, `r` from the first MATCH is bound (e.g., as a triple or edge variable). The second MATCH needs to constrain one hop to be exactly `r`. This requires:
1. The first MATCH binds edge identity variables for `r` (source, predicate, target).
2. The second MATCH's middle triple directly uses those bound variables.

**Approach**: When a relationship variable appears in a second MATCH, detect that it's already bound and emit a direct triple pattern using the bound subject/object variables from the first MATCH instead of a free variable.

**Effort**: Hard. Cross-MATCH variable binding plus unrolling.

#### #14: Return2[7] — List property concatenation `a.list2 + a.list1`

**Problem**: List properties stored as serialized strings. `+` operator does numeric addition, not list concat.

**Approach**: Detect when `+` operands are property accesses that hold list values (heuristically: the string starts with `[`). Emit SPARQL string manipulation:

```sparql
# a.list2 = "[4, 5]", a.list1 = "[1, 2, 3]"
BIND(
  CONCAT(
    SUBSTR(?list2, 1, STRLEN(?list2) - 1),
    ", ",
    SUBSTR(?list1, 2)
  ) AS ?foo
)
# Result: "[4, 5, 1, 2, 3]"
```

But the problem is we don't know at translate-time whether `a.list2` is a list or a number. We'd need runtime detection:

```sparql
BIND(
  IF(STRSTARTS(?list2, "["),
    CONCAT(SUBSTR(?list2, 1, STRLEN(?list2)-1), ", ", SUBSTR(?list1, 2)),
    ?list2 + ?list1
  ) AS ?foo
)
```

**Effort**: Medium. Requires `+` operator handling to check for lists.

#### #17: Return4[11] — `head(collect({likeTime: l})).likeTime`

**Problem**: Multi-step expression: collect maps → head → property access.

```cypher
WITH head(collect({likeTime: likeTime})) AS latestLike, person AS person
RETURN latestLike.likeTime AS likeTime
```

The `collect` aggregation produces `GROUP_CONCAT`. Then `head()` extracts the first element via `STRBEFORE`. Then `.likeTime` tries property access on the resulting string — which can't work.

**Approach — Pattern recognition**: Detect this specific pattern and rewrite:
- `head(collect({key: val})).key` → just return the first `val` using `SAMPLE(val)` or substring extraction from `GROUP_CONCAT`.

More specifically:
1. `collect({likeTime: likeTime})` → `GROUP_CONCAT(STR(?likeTime); separator=",")` (or similar)
2. `head(...)` → `STRBEFORE(..., ",")`
3. `.likeTime` → the extracted value is already the likeTime

So the chain `head(collect({k: v})).k` simplifies to: `SAMPLE(?v)` (first value in group).

**Effort**: Medium-hard. Requires detect-and-rewrite pattern in `translate_return_item` or `translate_expr`.

Could also take a simpler approach: `collect(expr)` in a WITH clause followed by `head()` property access — detect this as a peephole like we did for collect+UNWIND, and rewrite to `SAMPLE(expr.key)`.

---

### P4 — Property Path Distinct-Endpoint Problem (1 test: #6)

#### #6: Match7[12] — `OPTIONAL MATCH (a)-[*]->(b)` returns 3 vs 4

**Query**: `MATCH (a:Single) OPTIONAL MATCH (a)-[*]->(b) RETURN b`

Graph: Single→A→C, Single→B→(LOOP)B

Expected: A, B, B, C (4 rows). Got: 3 rows (A, B, C).

**Problem**: SPARQL property paths return *distinct* (source, target) pairs. There are two distinct paths from Single to B: (Single→B direct, and Single→A→...→B? No — the graph is Single→A→C and Single→B with B→B LOOP). 

Actually: Single→A (1 hop), Single→B (1 hop), Single→A→C (2 hops), Single→B→B (2 hops via LOOP). So the distinct endpoints are {A, B, C}. But Cypher expects B to appear twice because there are two distinct paths reaching B (direct and via loop).

SPARQL property paths deduplicate endpoints. This is a **fundamental SPARQL limitation**: `?a <p>+ ?b` returns distinct `?b` bindings, not one per path.

**Workaround — Bounded unrolling**: Instead of `<p>+`, unroll to fixed-length chains:
```sparql
# 1 hop:
{ ?a <p> ?b }
UNION
# 2 hops:  
{ ?a <p> ?mid . ?mid <p> ?b }
UNION
# ... up to cap
```

This returns one row per path (including duplicates at the same endpoint via different routes), which matches Cypher semantics.

**Problem**: Requires a finite upper bound. For `[*]` (1 or more), we need to cap. The graph has max depth 2 from Single, so cap=2 suffices here. But in general we'd need `cap = |V|` (number of nodes), which is unknown at query compile time.

**Practical approach**: Use a configurable cap (default 10 or 20). Document that variable-length paths beyond the cap won't find all paths. This is a pragmatic trade-off for SPARQL 1.1 compliance.

**Effort**: Medium-hard. Requires switching from property-path mode to bounded-unrolling for untyped `[*]` paths, which is more complex than the NPS property-path approach but semantically more correct.

**Note**: This same unrolling approach also helps #7 (minimum-bound paths) and #1 (per-hop property filters).

---

### P5 — Fundamentally Hard / Novel Features (4 tests: #3, #9, #11, #12, #16)

#### #3, #11, #12: `[rs*]` — Collected relationship list as path constraint

`MATCH (first)-[rs*]->(second)` where `rs` is a list of previously-collected relationship variables.

This means: follow exactly the relationships in `rs` in order. SPARQL 1.1 has no mechanism for this — it would require VALUES over relationship IRIs and ordered graph traversal.

**Theoretical approach**: For a list of N known relationships, emit a fixed-length chain where each hop is constrained to a specific relationship:
```sparql
# rs = [r1, r2]
?first ?p1 ?mid . FILTER(?p1_s = ?r1_s && ?p1_o = ?r1_o) .
?mid ?p2 ?second . FILTER(?p2_s = ?r2_s && ?p2_o = ?r2_o) .
```

But the list length isn't known at query compile time (it comes from a previous MATCH + WITH).

**Verdict**: Not feasible in static SPARQL translation. Would require runtime query rewriting or SPARQL federation. **Skip**.

#### #9: Match9[1] — `last(r)` on varlen relationship list

`MATCH ()-[r*0..1]-() RETURN last(r)`

`r` is a list of relationships along the path. `last()` extracts the last element. In SPARQL property paths, intermediate relationships aren't bound.

**Approach for `*0..1` specifically**: Path is either 0 hops (r=[]) or 1 hop (r=[edge]). For 0 hops, `last(r)` is null. For 1 hop, `last(r)` is the single edge.

Unroll `*0..1`:
```sparql
# 0 hops (self-match):
{ BIND(?a AS ?b) . BIND(BNODE() AS ?last_r) }  # actually last(r) = null
UNION
# 1 hop:
{ ?a ?p ?b . BIND(?p AS ?last_r) }
```

**Effort**: Medium for `*0..1` specifically. Extending to general `last(r)` on `*N..M` requires tracking the last hop variable in bounded unrolling.

#### #16: Return4[5] — `nodes(p)` 

`MATCH p = (n)-->(b) RETURN nodes(p)`

For a fixed-length 1-hop path, `nodes(p) = [n, b]`. For variable-length paths, requires collecting intermediate nodes.

**Approach for fixed-length paths**: `nodes(p)` on a 1-hop path `(n)-->(b)` → emit a list `[?n, ?b]` as a CONCAT string.

```sparql
BIND(CONCAT("[", STR(?n), ", ", STR(?b), "]") AS ?nodes_p)
```

But the result is `n` and `b` as node variables (blank node IRIs). The TCK test expects an empty result (the query returns no rows because `(n)-->(b)` matches nothing in a graph with a single unconnected node), so `nodes(p)` should produce 0 rows.

Wait — the test has `CREATE ()` (single node, no edges), and `MATCH p = (n)-->(b)` matches nothing. So expected result is empty (0 rows). Our error says "complex return expression" — we never get to execution.

**Fix**: Simply handle `nodes(p)` as a recognized function that emits a list expression based on the path variables. For 0 rows, it doesn't matter what we emit — it just needs to not error.

**Effort**: Small-medium. Register `nodes` as a supported function that emits a CONCAT-based list of the path's node variables.

---

### P6 — MERGE / DELETE (2 tests: #8, #15)

#### #8: Match8[2] — MERGE clause
#### #15: Return2[14] — DELETE clause

Both require SPARQL Update support. Currently these clauses return `UnsupportedFeature` errors.

**Approach — Multi-statement SPARQL Update**: 

For MERGE: emit INSERT DATA + SELECT to implement "create if not exists" semantics. Requires sending SPARQL Update to the engine before the query.

For DELETE: emit DELETE WHERE + SELECT. The DELETE happens first, then the SELECT returns the cached type info.

**Architecture**: The `transpile()` function would need to return a `TranspileOutput` that contains both update operations and a final query, instead of just a single SPARQL query string.

This is the "Multi-Phase TranspileOutput" approach from `tck-final-four.md`.

**Effort**: Large. Requires new output type, engine integration changes.

---

## Implementation Sequence

### Wave 1 — Quick Wins (est. +3-6 tests)

| Step | Failure | Approach | Confidence |
|------|---------|----------|------------|
| 1a | #18 Match5 parse | Fix Gherkin encoding issue | 99% |  
| 1b | #13 MatchWhere4[2] | Fix AND/OR precedence in expression parser | 90% |
| 1c | #10 Match9[5] | Bind marker variable for varlen rel in expressions | 85% |
| 1d | #16 Return4[5] | Register `nodes()` function with path node CONCAT | 80% |

### Wave 2 — Bounded Unrolling Engine (est. +4-6 tests)

| Step | Failure | Approach | Confidence |
|------|---------|----------|------------|
| 2a | #1 Match4[5] | Per-hop property filter via RDF-star in bounded unrolling | 80% |
| 2b | #4 Match6[14] | Debug/fix undirected typed varlen with preceding directed hop | 75% |
| 2c | #7 Match7[14] | Compose `nps{3,}` as `nps/nps/nps/nps*` for minimum bounds | 80% |
| 2d | #6 Match7[12] | Bounded unrolling for untyped `[*]` to preserve path multiplicity | 60% |

### Wave 3 — Expression Rewrites (est. +2-3 tests)

| Step | Failure | Approach | Confidence |
|------|---------|----------|------------|
| 3a | #14 Return2[7] | List concat `+` via SPARQL string manipulation with runtime `[` detection | 70% |
| 3b | #17 Return4[11] | Peephole: `head(collect({k:v})).k` → `SAMPLE(?v)` | 65% |

### Wave 4 — Hard Cross-MATCH + SPARQL Update (est. +2-4 tests)

| Step | Failure | Approach | Confidence |
|------|---------|----------|------------|
| 4a | #5 Match7[11] | Relationship identity comparison via triple-component equality | 55% |
| 4b | #2 Match4[7] | Cross-MATCH bound rel reuse + varlen unrolling | 40% |
| 4c | #8, #15 Match8/Return2 | Multi-phase TranspileOutput for MERGE+DELETE | 50% |

### Wave 5 — Maybe Never (est. 0-1 tests)

| Failure | Why it's hard |
|---------|---------------|
| #3, #11, #12 (rs*) | Runtime relationship list as path — no static SPARQL equivalent |
| #9 (last(r)) | Varlen relationship list element access — needs runtime path decomposition |

---

## Projected Outcomes

| Scenario | Tests Fixed | Total | % |
|----------|-------------|-------|---|
| Wave 1 only | +3-4 | 449-450 | 97.0% |
| Waves 1+2 | +7-10 | 453-456 | 97.8-98.5% |
| Waves 1+2+3 | +9-13 | 455-459 | 98.3-99.1% |
| Waves 1-4 | +11-17 | 457-463 | 98.7-100% |
| Waves 1-5 (perfect) | +18 | 464* | 100%+ |

*\*463 scenarios + fixing Match5 Gherkin could reveal additional hidden scenarios*

## Key Architectural Insight

Many of the remaining failures share a common root: **SPARQL property paths return distinct endpoint pairs, but Cypher returns one row per distinct path**. The solution for most P2/P4 failures is a unified **bounded path unrolling engine** that:

1. Enumerates all path lengths from `min` to `max` (capped)
2. Emits one UNION arm per length
3. Each arm is a chain of explicit triple patterns with unique intermediate variables
4. Optionally adds per-hop property filters (for #1)
5. Optionally handles direction combos for undirected (already have this in `emit_bounded_path_union`)
6. Binds intermediate relationship/node variables for `count(r)`, `last(r)`, `nodes(p)`

Building this engine well in Wave 2 creates leverage for Waves 3-5.
