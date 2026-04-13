# Plan: Handling the Last 27 TCK Failures (436 → 463)

**Current**: 436 / 463 passed (94.2%)
**Target**: 463 / 463 (100%), with realistic ceiling discussed below

---

## Failure Inventory

| # | Feature | Scenario | Root Cause | Category |
|---|---------|----------|-----------|----------|
| 1 | Match4:2 | Simple variable length pattern | Untyped `[*]` → single triple instead of transitive closure | A |
| 2 | Match4:5 | Varlen pattern with property predicate | `[*{year:1988}]` property filter on paths not implemented | A |
| 3 | Match4:7 | Varlen patterns including bound relationship | Re-use of bound `r` in second MATCH with `[*0..1]` | A |
| 4 | Match4:8 | Matching rels into list + varlen using list | `[rs*]` (collected rel list as path predicate) unsupported | E |
| 5 | Match6:14 | Named path with undirected fixed varlen | `<-[:T]-()-[:T*3..3]-` combined fixed+varlen (0 rows) | A |
| 6 | Match6:15 | Variable-length named path | `[*0..]` named path returns 0 rows | A |
| 7 | Match6:20 | Varlen relationship without bounds | `[*N..]` returning 3 rows instead of 2 | A |
| 8 | Match7:11 | Undirected rel + OPTIONAL MATCH | Undirected `r {name: 'r1'}` + OPTIONAL returns 4 vs 2 | B |
| 9 | Match7:12 | Variable length optional relationships | `OPTIONAL MATCH (a)-[*]->(b)` returns 2 vs 4 | AB |
| 10 | Match7:14 | Varlen optional with length predicates | `OPTIONAL MATCH (a)-[*3..]->(b)` returns 2 vs 1 | AB |
| 11 | Match8:2 | MATCH, MERGE, OPTIONAL MATCH | MERGE clause unsupported | F |
| 12 | Match9:1 | Varlen rel variables are lists of rels | `last(r)` on varlen rel variable unsupported | E |
| 13 | Match9:3 | Collect rels as list — undirected 2-hop | `[r:REL*2..2]` undirected returns 1 vs 2 | A |
| 14 | Match9:5 | Varlen with label predicate on both sides | `(a:Blue)-[r*]->(b:Green)` count returns 0 vs 1 | A |
| 15 | Match9:6 | Rels into list + varlen with bound nodes | `[rs*]` syntax (parse error) | E |
| 16 | Match9:7 | Same as 9:6, wrong direction | `[rs*]` syntax (parse error) | E |
| 17 | MatchWhere1:12 | Path length predicate in WHERE | `WHERE length(p) = 1` on named path (0 vs 1) | C |
| 18 | MatchWhere4:2 | Disjunctive predicates with patterns | `(a)-[:T]->(b:Label)` pattern in WHERE not parsed | D |
| 19 | Return2:7 | Adding list properties in projection | `a.list2 + a.list1` returns null (lists stored as strings) | C |
| 20 | Return2:13 | Projecting map of nodes and rels | `{node1: n, rel: r, node2: m}` nodes are IRIs, not reconstructable | C |
| 21 | Return2:14 | Type of deleted relationships | DELETE clause unsupported | F |
| 22 | Return4:5 | `nodes(p)` function on path | `nodes()` function not implemented | C |
| 23 | Return4:11 | Reusing variable names in RETURN | `head(collect({likeTime: likeTime}))` → map property access | C |
| 24 | Return6:13 | Min length of paths | `min(length(p))` → path length function returns null | C |
| 25 | Unwind1:4 | Unwinding collected unwound expression | `WITH collect(row) AS rows UNWIND rows` — runtime list | G |
| 26 | Unwind1:5 | Unwinding collected expression | Same: `WITH collect(row) AS rows UNWIND rows` | G |
| 27 | Unwind1:12 | Unwind doesn't remove vars from scope | Same: `WITH a, collect(b1) AS bees UNWIND bees` | G |

### Category Key

| Code | Category | Count | Fixable? |
|------|----------|-------|----------|
| **A** | Variable-length path semantics (untyped `[*]`) | 9 | Partially — see §1 |
| **AB** | Variable-length + OPTIONAL MATCH interaction | 2 | Partially — see §1 |
| **B** | OPTIONAL MATCH row-count correctness | 1 | Medium — see §2 |
| **C** | Complex expression / function gaps | 6 | Mixed — see §3 |
| **D** | Pattern expressions in WHERE (parse) | 1 | Medium — see §4 |
| **E** | Collected-rel-list-as-path (`[rs*]`) | 3 | Hard — see §5 |
| **F** | MERGE / DELETE (SPARQL Update) | 2 | Out of scope — see §6 |
| **G** | UNWIND of runtime-collected lists | 3 | Hard — see §7 |

---

## §1 — Variable-Length Path Semantics (11 failures)

**Failures**: #1, #2, #3, #5, #6, #7, #9, #10, #13, #14 (Category A/AB)

### Problem

The current translator has two modes for variable-length relationships:

1. **Typed `[*]`** (`-[:KNOWS*]->`): Uses SPARQL property paths
   (`<knows>+`, `<knows>*`). These work for reachability but fail when the
   query needs *all distinct paths* (Cypher returns one row per path, not
   per reachable endpoint).

2. **Untyped `[*]`** (`-[*]->`): Falls through to a single triple pattern
   with a predicate variable. This only matches 1-hop, never transitive.

### Root Cause: RDF vs Property Graph Mismatch

SPARQL property paths (`?a <p>+ ?b`) return *distinct bindings*, not paths.
`MATCH (a)-[*]->(x)` with A→B→C→D returns 3 rows (B, C, D), but a SPARQL
`?a <CONTAINS>+ ?x` also returns 3 rows — so **typed variable-length
paths already work** for simple reachability queries.

The failures are caused by:

- **Untyped `[*]`** (#1, #6, #9, #10): Needs `NegatedPropertySet` to match
  any relationship type, but this also matches property predicates and
  internal markers (`rdf:type`, `__node`). Attempted fix regressed because
  node properties like `name` got traversed as relationship hops.

- **Property filters on paths** (#2): `[:WORKED_WITH* {year: 1988}]` requires
  checking RDF-star annotations on each hop of the path. SPARQL property
  paths cannot express per-hop property filters.

- **Bound-relationship re-use in varlen** (#3): The second MATCH re-uses
  `r` from the first MATCH inside a `[*0..1]` pattern — requires
  relationship isomorphism tracking across MATCH clauses.

- **Undirected bounded varlen** (#5, #13): Fixed-length undirected paths
  (`*3..3`, `*2..2`) via UNION chains only emit one direction.

- **Count on varlen rel** (#14): `count(r)` where `r` is a path variable —
  `r` is never bound because property path patterns don't bind predicates.

### Fix Plan

#### 1a. Untyped variable-length paths (5 fixes: #1, #6, #7, #9, #10)

**Approach**: Use `NegatedPropertySet` excluding internal predicates:

```sparql
# Exclude: __node marker, rdf:type, and all *known property predicates*
# from the current graph's CREATE setup.
?a (!(tck:__node | rdf:type | tck:name | tck:num | ...))+ ?x
```

The key insight we missed: the exclusion set must include **every property
IRI used in the test graph**, not just `__node` and `rdf:type`. Since the
translation happens per-query and the translator doesn't know which
properties exist, we have two options:

- **Option A (pragmatic)**: Exclude a fixed set of known marker predicates
  (`__node`, `rdf:type`) and accept that queries on graphs with property
  predicates that share names with relationship types will over-match. This
  is actually the correct behavior for most real graphs (properties and
  relationships use different predicates).

- **Option B (correct)**: In the TCK test runner, pass the set of
  CREATE-emitted property IRIs to the translator as context, and add them
  to the exclusion set. This requires a minor API change.

**Recommendation**: Option A first for the 5 simple cases (no property
filter), then evaluate remaining failures.

**Risk**: The previous attempt with Option A regressed 4 tests. Need to
investigate *which* tests regressed and why — likely the OPTIONAL MATCH
cases where the NegatedPropertySet matched too broadly.

#### 1b. Undirected bounded varlen (#5, #13)

`emit_bounded_path_union` only emits forward chains. For undirected
patterns, also emit reverse chains (or UNION both directions per hop).

#### 1c. Property filters on paths (#2)

**Hard**. SPARQL property paths cannot express per-hop property constraints.
Requires **path unrolling**: for each hop count 1..N, emit a fixed-length
chain of triples with per-hop RDF-star annotation filters.

```sparql
# For -[:T* {year: 1988}]-> unrolled to 2 hops:
?a <T> ?mid . << ?a <T> ?mid >> <year> 1988 .
?mid <T> ?b  . << ?mid <T> ?b >> <year> 1988 .
```

Upper bound must be finite (or capped at a reasonable default like 10).

#### 1d. Bound-relationship re-use (#3)

Requires variable binding across MATCH clauses. The first MATCH binds `r`,
and the second MATCH must constrain the variable-length pattern to use `r`.
This is fundamentally different from SPARQL property paths. Would need
unrolling + FILTER matching.

**Recommendation**: Defer. This is an edge case.

---

## §2 — OPTIONAL MATCH Row-Count Bugs (1 failure)

**Failure**: #8 (Match7:11)

### Problem

```cypher
MATCH (a)-[r {name: 'r1'}]-(b)
OPTIONAL MATCH (b)-[r2]-(c)
WHERE r <> r2
RETURN a, b, c
```

Returns 4 rows instead of 2. The undirected MATCH `(a)-[r]-(b)` doubles
rows (A↔B and B↔A), and since both `r` and `r2` match the same edge in
both directions, the relationship inequality filter `r <> r2` doesn't
deduplicate properly.

### Fix Plan

The undirected relationship UNION currently emits both directions with a
filter `?src != ?dst` to prevent self-loop duplicates. But it doesn't
prevent `(A,B)` + `(B,A)` duplicates on the *same* edge. Need to add an
ordering filter: for undirected non-self-loop edges, only emit the row
where `STR(?src) < STR(?dst)`.

**Estimated effort**: Small change to `translate_relationship_pattern`'s
undirected branch.

**Risk**: May break other undirected tests that expect both directions.
Need to verify semantics: Cypher undirected MATCH returns both directions
for each edge, so the current behavior (4 rows) might actually be *correct*
for the MATCH — the issue may be in the OPTIONAL MATCH interaction.

---

## §3 — Complex Expression / Function Gaps (6 failures)

### #17 — MatchWhere1:12: `WHERE length(p) = 1`

**Problem**: `length(p)` on a named path variable. The translator maps
`length()` → `STRLEN()` which operates on strings, not paths.

**Fix**: Path length can be computed from the number of relationship hops.
For a named path `p = (a)-[...]->(b)`, the length is the number of
relationship-pattern elements in the path. For fixed-length patterns, emit
a constant. For variable-length patterns, this requires counting via
aggregate or VALUES.

**Approach**: When the argument to `length()` is a path variable (not a
string), emit the count of hops. For simple fixed-length paths, emit an
integer literal. For variable-length paths, would need a counter variable
incremented per hop — complex.

**Effort**: Medium for fixed-length paths, hard for variable-length.

### #19 — Return2:7: `a.list2 + a.list1`

**Problem**: List properties are stored as serialized strings (e.g.
`"[4, 5]"`). The `+` operator does SPARQL `ADD` (numeric), not list
concatenation.

**Fix**: Detect when both operands of `+` are property accesses to list
properties. Parse the serialized strings at query time using SPARQL string
functions (`CONCAT`, `SUBSTR`, etc.) to concatenate them.

**Approach**:
```sparql
# a.list2 = "[4, 5]", a.list1 = "[1, 2, 3]"
# Result: CONCAT(
#   SUBSTR(?list2, 1, STRLEN(?list2) - 1),  -- "[4, 5"
#   ", ",
#   SUBSTR(?list1, 2)                        -- "1, 2, 3]"
# )
```

**Effort**: Medium — requires detecting list-typed properties (not
currently tracked) or attempting string manipulation unconditionally.

### #20 — Return2:13: `{node1: n, rel: r, node2: m}`

**Problem**: Map contains node/relationship variables. These are blank node
IRIs in SPARQL, but the TCK expects `(:A)`, `[:T]`, `(:B)` formatting.
Since `is_complex_tck_value` detects `{node1: (:A), ...}` as complex, only
row count is compared — but we produce `None` because the map CONCAT
receives blank node IRIs, not formatted graph objects.

**Fix**: The map expression translation already uses `CONCAT` + `STR()`.
For variable references, `STR(?n)` gives the blank node ID or IRI. The
issue is the result doesn't match because the value check sees `None`.
Need to ensure the CONCAT actually produces a non-null string.

**Effort**: Small — debug why the CONCAT produces null.

### #22 — Return4:5: `nodes(p)`

**Problem**: `nodes()` function on a path variable. Returns the ordered
list of nodes in the path. Not implementable in standard SPARQL (property
paths don't bind intermediate nodes).

**Effort**: Hard. Would require unrolling the path to fixed-length chains
and collecting intermediate node variables.

**Recommendation**: Defer.

### #23 — Return4:11: Complex aggregation with map property access

```cypher
WITH head(collect({likeTime: likeTime})) AS latestLike, person AS person
RETURN latestLike.likeTime AS likeTime
```

**Problem**: `head(collect({...}))` collects maps into a GROUP_CONCAT
string, then `head()` extracts the first element (via STRBEFORE), then
`.likeTime` is a property access on the result. But `.likeTime` on a
string like `"{likeTime: 20160614}"` isn't extractable via SPARQL property
access triples.

**Fix**: Detect the pattern `head(collect(map_expr)).key` and rewrite it
to extract the corresponding aggregate directly. For this specific case:

```sparql
# Instead of head(collect({likeTime: ...})).likeTime
# Emit: the first likeTime value (using SAMPLE or GROUP_CONCAT + STRBEFORE)
```

**Effort**: Hard — requires pattern recognition in the translator.

### #24 — Return6:13: `min(length(p))`

**Problem**: `length(p)` on a variable-length path returns null because
path length isn't computable in SPARQL. The `min()` aggregate then
propagates null.

**Fix**: Same as #17. If path length can be computed, this follows.

---

## §4 — Pattern Expressions in WHERE (1 failure)

**Failure**: #18 (MatchWhere4:2)

### Problem

```cypher
WHERE a.id = 0
  AND (a)-[:T]->(b:TheLabel)
  OR (a)-[:T*]->(b:MissingLabel)
```

Pattern expressions inside WHERE (`(a)-[:T]->(b:TheLabel)`) are not
supported by the parser. These are Cypher "pattern predicates" which test
for the existence of a path.

### Fix Plan

1. **Grammar**: Add a `pattern_predicate` rule to `expression` that allows
   a path pattern inside WHERE.
2. **Parser**: Build it as a new `Expression::PatternPredicate(Pattern)`.
3. **Translator**: Translate to `EXISTS { ?a <T> ?b . ?b rdf:type <TheLabel> }`.

The variable-length variant `(a)-[:T*]->(b:MissingLabel)` would need
property path support inside EXISTS.

**Effort**: Medium. Grammar + parser + translator.

---

## §5 — Collected-Rel-List-as-Path (3 failures)

**Failures**: #4, #15, #16 (Match4:8, Match9:6, Match9:7)

### Problem

```cypher
WITH [r1, r2] AS rs LIMIT 1
MATCH (first)-[rs*]->(second)
```

`[rs*]` uses a collected list of relationships as the predicate for a
variable-length MATCH. This is an advanced Cypher feature: the path must
traverse exactly the relationships in the list, in order.

### Fix Plan

**Not feasible in standard SPARQL 1.1**. This would require a custom
SPARQL extension or pre-processing the relationship list to emit a fixed
chain of triple patterns.

**Recommendation**: Mark as unsupported (out of SPARQL 1.1 scope).
#15 and #16 are parse errors for `[rs*]` — the grammar should reject this
cleanly rather than crashing.

---

## §6 — MERGE / DELETE Clauses (2 failures)

**Failures**: #11 (Match8:2), #21 (Return2:14)

### Problem

MERGE and DELETE are SPARQL Update operations, not query operations. The
translator is a query transpiler.

### Fix Plan

These require SPARQL Update support (INSERT/DELETE/WHERE) which is
Phase 4+ in the roadmap. Not currently in scope.

**Recommendation**: Retain as "unsupported" with a clean error message.
These 2 failures are permanent until SPARQL Update is added.

---

## §7 — UNWIND of Runtime-Collected Lists (3 failures)

**Failures**: #25, #26, #27 (Unwind1:4, Unwind1:5, Unwind1:12)

### Problem

```cypher
MATCH (row)
WITH collect(row) AS rows
UNWIND rows AS node
RETURN node.id
```

`collect(row)` produces a runtime list via GROUP_CONCAT. UNWIND then needs
to iterate over this list. But SPARQL 1.1 has no LATERAL join or list
iteration operator.

### Fix Plan

#### Approach A: Subquery Rewrite (Preferred)

Detect the pattern `WITH collect(x) AS xs / UNWIND xs AS y` and eliminate
it — the combined effect is an identity (each original `x` row becomes a
`y` row). Rewrite as:

```sparql
# Original: MATCH (row) WITH collect(row) AS rows UNWIND rows AS node
# Equivalent: MATCH (row) BIND(?row AS ?node)
```

This works for the simple collect-then-unwind pattern. For Unwind1:12,
there's an additional MATCH after the UNWIND that uses the unwound variable:

```cypher
MATCH (a:S)-[:X]->(b1)
WITH a, collect(b1) AS bees
UNWIND bees AS b2
MATCH (a)-[:Y]->(b2)
RETURN a, b2
```

This is equivalent to:

```cypher
MATCH (a:S)-[:X]->(b1)
MATCH (a)-[:Y]->(b1)
RETURN a, b1 AS b2
```

The `collect + UNWIND` pattern here serves no purpose (it collects and
re-expands). The optimizer could detect this and elide the WITH/UNWIND.

#### Approach B: GROUP_CONCAT + Split (Fallback)

If the collect is followed by a UNWIND that cannot be elided:
1. GROUP_CONCAT with a known separator (e.g. `\x1F` Unit Separator)
2. In a subsequent query or using SPARQL string functions, split and rebind

This is fragile and limited. Approach A is strongly preferred.

**Effort**: Medium for Approach A (pattern detection in translator).

---

## Priority Matrix

| Priority | Failures | Effort | Fixes |
|----------|----------|--------|-------|
| **P0** | #25, #26, #27 | Medium | Collect+UNWIND elision (§7A) → 3 fixes |
| **P1** | #1, #6, #7, #9, #10 | Medium | Untyped `[*]` via NegatedPropertySet (§1a) → up to 5 fixes |
| **P1** | #18 | Medium | Pattern predicates in WHERE (§4) → 1 fix |
| **P2** | #8 | Small | Undirected row dedup (§2) → 1 fix |
| **P2** | #5, #13 | Small | Undirected bounded varlen (§1b) → 2 fixes |
| **P2** | #20 | Small | Debug map CONCAT null (§3) → 1 fix |
| **P3** | #17, #24 | Medium | Path length() function (§3) → 2 fixes |
| **P3** | #2 | Hard | Per-hop property filters (§1c) → 1 fix |
| **P3** | #14 | Medium | Count on varlen rel (§3) → 1 fix |
| **P4** | #19 | Medium | List property concatenation (§3) → 1 fix |
| **P4** | #23 | Hard | head(collect({map})).key (§3) → 1 fix |
| **Defer** | #3, #22 | Hard | Bound-rel re-use, nodes() function |
| **Wontfix** | #4, #11, #12, #15, #16, #21 | N/A | MERGE, DELETE, [rs*], last(r) |

---

## Realistic Ceiling

| Category | Fixable | Deferred | Wontfix |
|----------|---------|----------|---------|
| Variable-length paths (A/AB) | 7–9 | 2 | 0 |
| OPTIONAL MATCH (B) | 1 | 0 | 0 |
| Complex expressions (C) | 3–4 | 2 | 0 |
| Pattern predicates (D) | 1 | 0 | 0 |
| Collected-rel-as-path (E) | 0 | 0 | 3 |
| MERGE/DELETE (F) | 0 | 0 | 2 |
| Runtime UNWIND (G) | 3 | 0 | 0 |
| **Total** | **15–18** | **4** | **5** |

**Projected ceiling**: 451–454 / 463 (97.4%–98.1%)

With maximum effort on P0–P3 items: **~453 / 463 (97.8%)**

The 5 wontfix items (MERGE, DELETE, `[rs*]` syntax, `last(r)`) would
require SPARQL Update support or language extensions beyond SPARQL 1.1.

---

## Execution Order

### Phase 1: Quick wins (est. +7)
1. Collect+UNWIND elision (#25, #26, #27)
2. Undirected bounded varlen fix (#5, #13)
3. Debug map CONCAT null (#20)
4. Undirected optional dedup (#8)

### Phase 2: Untyped variable-length paths (est. +5)
5. NegatedPropertySet for untyped `[*]` with expanded exclusion set
6. Test against #1, #6, #7, #9, #10 — fix regressions

### Phase 3: Expression gaps (est. +3–4)
7. Pattern predicates in WHERE (#18)
8. Path length for fixed-length paths (#17, #24)
9. Count on varlen rel variable (#14)

### Phase 4: Hard problems (est. +2)
10. Per-hop property filters (#2)
11. List property concatenation (#19)
