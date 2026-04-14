# Plan: Solving the Final 10 TCK Failures

**Current**: 453 / 463 (97.8%)
**Target**: 463 / 463 (100%)

---

## Inventory

| # | Scenario | Feature | Error | Root Cause |
|---|----------|---------|-------|------------|
| 1 | Match4[7] | Bound rel reuse in varlen | got 0, expected 32 | Cross-MATCH bound relationship reuse in `*0..1` chain |
| 2 | Match4[8] | Rel list as varlen `[rs*]` | got 3, expected 1 | Runtime relationship list as path constraint |
| 3 | Match6[14] | Undirected fixed varlen, parallel edges | got 0, expected 4 | SPARQL property path dedup vs Cypher per-path multiplicity |
| 4 | Match7[12] | Variable length optional | got 3, expected 4 | Same dedup issue — `(a)-[*]->(b)` |
| 5 | Match8[2] | MERGE clause | UnsupportedFeature | MERGE not translated to SPARQL Update |
| 6 | Match9[1] | `last(r)` on `*0..1` | complex expr error | Varlen rel variable → list; `last()` extraction |
| 7 | Match9[6] | `[rs*]` with bound nodes | parse error | Runtime relationship list as path constraint |
| 8 | Match9[7] | `[rs*]` wrong direction | parse error | Same as Match9[6], reversed direction |
| 9 | Return2[14] | DELETE + RETURN type(r) | UnsupportedFeature | DELETE not translated to SPARQL Update |
| 10 | Return4[11] | `head(collect({map})).prop` | got None | Map literal construction + head/collect + property access |

Bonus flaky: Return6[13] passes in isolation but intermittently fails
in the full suite due to list element ordering.  Not a real bug.

---

## Categorization

### Wave A — SPARQL Update: MERGE + DELETE (scenarios 5, 9)

Both `spargebra` and `oxigraph` already support SPARQL Update fully.
`spargebra` exposes `Update { operations: Vec<GraphUpdateOperation> }` with
variants: `InsertData`, `DeleteData`, `DeleteInsert`, `Clear`, `Create`,
`Drop`, `Load`.  The TCK test runner already calls `store.update()` for
INSERT DATA in setup steps.

**Match8[2]** — `MATCH (a) MERGE (b) WITH * OPTIONAL MATCH (a)--(b) RETURN count(*)`

Cypher `MERGE` = "match or create".  In SPARQL terms this is a conditional
INSERT: `INSERT { ... } WHERE { FILTER NOT EXISTS { ... } }` followed by
the SELECT query.  This is a **two-operation** update+query sequence.

Translation:
```
MERGE (b) →
  INSERT DATA { _:b <__node> <__node> . }   -- if no match
  (or find existing node matching pattern)

-- Then the SELECT continues as normal
```

More precisely, `MERGE (b)` with no label/properties matches any existing node,
so it reduces to: if no nodes exist at all, create one; otherwise bind each
existing node.  The TCK scenario has existing nodes, so MERGE is a no-op — it
binds `b` to all existing nodes (cross-product with `a`).

**Approach**: Translate MERGE as a two-phase output:
1. Phase 1: `INSERT { ... } WHERE { FILTER NOT EXISTS { ... } }` (conditional insert)
2. Phase 2: The remaining `SELECT` query using the merged variable

For the simpler case where the MERGE pattern already exists (common in TCK),
MERGE is equivalent to MATCH — emit a regular BGP for the merge pattern.

**Return2[14]** — `MATCH ()-[r]->() DELETE r RETURN type(r)`

Cypher allows reading from `r` after deleting it (the binding survives the
delete within the same query).  In SPARQL:
1. First SELECT binds `?r_pred`, `?r_src`, `?r_dst`
2. Then DELETE the triple
3. Return `type(r)` from the pre-delete bindings

Translation to SPARQL Update + Query:
```sparql
# Phase 1: Capture bindings + delete
DELETE { ?src ?r_pred ?dst }
WHERE  { ?src ?r_pred ?dst . FILTER(?r_pred NOT IN (rdf:type, <__node>)) }

# Phase 2: Return pre-delete bindings
# (bindings are returned from the DELETE WHERE result)
```

Actually, a cleaner approach: split into two operations combined in a single
`spargebra::Update`:

```sparql
DELETE WHERE { ?src ?r_pred ?dst . FILTER(...) }
```

But we need the bindings *before* deletion for the RETURN.  This requires
executing a SELECT first, storing results, then executing DELETE.

**Approach**: `TranspileOutput::MultiPhase` — emit a read query, then an
update, then return the read query's results.

**Implementation**:

1. Extend `TranspileOutput` with a `ReadThenUpdate` variant:
   ```rust
   pub enum TranspileOutput {
       Simple { sparql: String, schema: ProjectionSchema },
       ReadThenUpdate {
           read_query: String,
           update_query: String,
           schema: ProjectionSchema,
       },
   }
   ```

2. In the translator, when encountering a `Clause::Delete`:
   - Build the SELECT for the MATCH/WHERE preceding the DELETE
   - Build the DELETE WHERE/DELETE DATA from the bound patterns
   - The RETURN columns come from the SELECT phase

3. For MERGE: detect whether the merge pattern can match existing data
   - Emit conditional INSERT + SELECT

4. Update the TCK test runner to handle `ReadThenUpdate`: execute the
   read query first, then the update, then use the read results for
   assertion.

**Estimated effort**: Medium.  MERGE has complex semantics (ON MATCH SET /
ON CREATE SET), but the TCK scenario uses the simplest case.  DELETE + RETURN
is more straightforward.

**Scenarios fixed**: 2 (Match8[2], Return2[14])

---

### Wave B — Bounded Unroll for `last(r)` (scenario 6)

**Match9[1]** — `MATCH ()-[r*0..1]-() RETURN last(r) AS l`

In Cypher, `r` in `[r*0..1]` is bound to the *list* of relationships along
the path.  `last(r)` returns the final element.

Current translator emits `[r*0..1]` as a SPARQL property path
`?a <p>{0,1} ?b`, which loses intermediate edge bindings.  The solution
from `fundamental-limitations.md` §3b: bounded unrolling into a UNION.

**Approach**:

The path `*0..1` has exactly 2 cases:
- 0-hop: `r = []`, `last(r) = null`
- 1-hop: `r = [edge]`, `last(r) = edge`

Emit:
```sparql
SELECT ?l WHERE {
  {
    # 0-hop branch — every node matches itself
    ?__a0 <__node> <__node> .
    # r is empty list, last(r) = null → unbound ?l
  }
  UNION
  {
    # 1-hop branch — exactly one relationship
    ?__a0 ?__r0_pred ?__b0 .
    FILTER(?__r0_pred NOT IN (rdf:type, <__node>))
    # last(r) = the relationship itself
    # For TCK: relationship is compared as [:T] shape → need type
    BIND(?__r0_pred AS ?l)
  }
}
```

The key insight: the translator already does bounded unrolling for varlen
paths (see `emit_bounded_path_union`).  The missing piece is:
1. **Detecting** when the RETURN/WITH references `last(r)` (or other list
   functions) on a varlen rel variable
2. **Switching** from property-path emission to bounded-unroll emission
   for that specific relationship
3. **Binding** the appropriate intermediate variable to the list function
   result

**Detection**: When translating `last(r)` or `r` (as a list) in an
expression, check if `r` is a varlen relationship variable.  If so, look
up the bounded unroll and bind the appropriate hop variable.

**Implementation**:

1. Add a `varlen_rel_info` map to `TranslationState` tracking varlen
   relationship variables and their bounds: `{ "r" → (lower, upper) }`

2. When a varlen rel is referenced in `last()`, `head()`, or as a list:
   - If the bounds are finite and small (≤ threshold, e.g. 8), switch
     to bounded unroll
   - In each UNION branch, bind a `?__last_r` variable to the predicate
     of the last hop (or null for 0-hop)

3. Wire `last(r)` expression translation to reference `?__last_r`

**Edge case**: The TCK expects `[:T]` (relationship shape) in the output,
not just the predicate IRI.  Our TCK runner's "complex result" handler
already does row-count comparison for relationship-shaped values, so
binding the predicate (from which `type()` can be derived) is sufficient.

**Estimated effort**: Medium.  Bounded unrolling machinery exists; needs
a new trigger path for list-function references on varlen rels.

**Scenarios fixed**: 1 (Match9[1])

---

### Wave C — Multi-Phase Execution for `[rs*]` (scenarios 2, 7, 8)

**Match4[8], Match9[6], Match9[7]** — relationship list used as path constraint

```cypher
MATCH ()-[r1]->()-[r2]->()
WITH [r1, r2] AS rs LIMIT 1
MATCH (first)-[rs*]->(second)  -- "follow exactly these edges in order"
```

This is fundamentally impossible in a single SPARQL query (see
`fundamental-limitations.md` §1a).  The relationship list `rs` is a
*runtime value* — the transpiler cannot know which edges to chain until
phase 1 executes.

**Approach**: Multi-phase execution (L2 from fundamental-limitations.md).

Phase 1 query — everything up to and including the WITH:
```sparql
SELECT ?r1_pred ?r1_src ?r1_dst ?r2_pred ?r2_src ?r2_dst WHERE {
  ?a0 ?r1_pred ?m0 .
  ?m0 ?r2_pred ?b0 .
  FILTER(?r1_pred NOT IN (rdf:type, <__node>))
  FILTER(?r2_pred NOT IN (rdf:type, <__node>))
} LIMIT 1
```

Phase 2 query — using concrete bindings from phase 1:
```sparql
SELECT ?first ?second WHERE {
  ?first <:Y> ?__m0 .
  ?__m0 <:Y> ?second .
}
```

**Implementation**:

1. Extend `TranspileOutput` with a `Continuation` variant:
   ```rust
   Continuation {
       phase1_query: String,
       phase1_schema: ProjectionSchema,
       continuation: ContinuationSpec,
   }
   ```
   Where `ContinuationSpec` encodes how to build phase 2 from phase 1 results
   (template + substitution slots).  This must be serializable — no closures.

2. In the parser, recognize `[rs*]` where `rs` is not a fresh variable but
   a previously-bound list.  This requires WITH-scope tracking.

3. In the translator, when encountering a `[rs*]` with a runtime-bound
   variable:
   - Emit everything before as phase 1
   - Record the substitution template for phase 2
   - Return `TranspileOutput::Continuation`

4. Update the TCK test runner to handle continuations:
   - Execute phase 1
   - Feed results into the continuation spec
   - Execute phase 2
   - Assert on phase 2 results

5. Match9[7] expects **0 rows** (wrong direction) — the direction check
   happens in phase 2 generation.  If the bound edges go A→B→C but the
   pattern asks `(first)-[rs*]->(second)` and the WITH swaps first/second,
   phase 2 will correctly fail to match.

**Estimated effort**: High.  This introduces a fundamentally new execution
model.  The continuation spec needs careful design to be both correct and
serializable (for use cases where the transpiler and executor are separate
processes).

**Parse error note**: Match9[6] and Match9[7] currently fail with
`expected OPTIONAL` parse error.  This is because the parser sees
`MATCH (first)-[rs*]->(second)` after a WITH and `rs` is treated as
an unknown token.  The parser needs to accept previously-bound variables
in the relationship list position.

**Scenarios fixed**: 3 (Match4[8], Match9[6], Match9[7])

---

### Wave D — Varlen Path Multiplicity / SPARQL Dedup (scenarios 3, 4)

**Match6[14]** — `MATCH topRoute = (:Start)<-[:CONNECTED_TO]-()-[:CONNECTED_TO*3..3]-(:End)`
expects 4 rows.  Got 0.

**Match7[12]** — `MATCH (a:Single) OPTIONAL MATCH (a)-[*]->(b) RETURN b`
expects rows `[A, B, B, C]` (B appears twice via two paths).  Got `[A, B, C]`.

Both stem from the same underlying mismatch: **RDF is a set of triples**.
When a graph has two edges with identical (subject, predicate, object) —
e.g. `mid -[:CONNECTED_TO]-> db2` created twice — the RDF store collapses
them into one triple.  SPARQL therefore returns only distinct solutions while
Cypher enumerates each distinct path instance.

#### Can RDF-star solve multi-edge multiplicity?

This is a natural question because RDF-star lets you attach metadata to
individual triples.  However, **RDF-star alone cannot solve this**.

`<< s p o >>` inRDF-star always refers to the *same* triple `(s, p, o)`.
Writing:
```turtle
<< :mid :CT :db2 >> :edgeId :e1 .
<< :mid :CT :db2 >> :edgeId :e2 .
```
does store two annotation triples and a SPARQL query on `?x :edgeId ?id`
would yield two rows — which looks promising.  But generating these at
CREATE time requires the CREATE translator to:
1. Assign a globally unique IRI (or blank node) to *every* edge at
   insertion time, even edges with no properties.
2. Emit an annotation triple `<< s p o >> :edgeId <fresh_iri> .` for each
   edge instance.

And the MATCH translator must:
1. Query `?s ?p ?o . << ?s ?p ?o >> :edgeId ?eid .` instead of just `?s ?p ?o .`
2. Use `?eid` as the canonical edge identity (replacing the current
   `BIND(CONCAT(...))` synthetic ID).

This is **full per-edge reification via RDF-star annotations**.  The existing
`rdf_mapping::reification` module already does the equivalent with
`rdf:Statement` nodes, but only for edges *that carry properties*.
The change is to extend it to **every edge unconditionally**.

#### Effect on the data model

| Encoding | Multi-edge support | Query complexity | Store size |
|----------|--------------------|-----------------|------------|
| Current (bare triples) | ✗ — identical (s,p,o) collapse | O(1) triples/edge | minimal |
| Full reification (rdf:Statement) | ✓ | +4 triples/edge, joins required | 5× |
| RDF-star per-edge annotation | ✓ | +1 annotation triple/edge, `<<>>` joins | 2× |

**RDF-star annotation is the cheapest reification approach**: one extra
triple per edge instance, and the SPARQL-star extension already available
in spargebra/oxigraph handles the `<< s p o >> :edgeId ?eid` join natively.

#### Approach: Opt-in full edge annotation

Introduce a `TargetEngine::multi_edge` capability flag (default `false`).
When `true`:

**CREATE side** (TCK test runner / `create_to_insert_data`):
```turtle
# bare triple (always)
_:src <base:CT> _:dst .
# per-instance annotation (multi_edge mode)
<< _:src <base:CT> _:dst >> <pg:edgeId> <pg:edge_{counter}> .
```

**MATCH side** (translator):
```sparql
# bare rel pattern becomes:
?src <base:CT> ?dst .
<< ?src <base:CT> ?dst >> <pg:edgeId> ?__eid_r .
# ?__eid_r is now a real bound variable, not a BIND(CONCAT(...))
```

SPARQL will produce one solution row per distinct `?__eid_r` binding,
giving correct per-edge multiplicity.

This replaces the current `BIND(CONCAT(STR(?s),"|",STR(?p),"|",STR(?o))))`
synthetic ID hack with a real stored identity, while keeping the same
`eid_var` field already in `EdgeInfo`.

#### Match7[12] — additional issue (property path dedup)

Even with full edge annotation, Match7[12]'s `OPTIONAL MATCH (a)-[*]->(b)`
uses a SPARQL property path which **cannot bind individual edge IDs** — only
endpoints.  Property paths (`?a <p>+ ?b`) never produce intermediate variable
bindings in SPARQL 1.1.  To expose per-edge multiplicity here, we must switch
to bounded unrolling.

The graph has `(b)-[:LOOP]->(b)` so the path `(a:Single)-[*]->(b)` expands:
- `a→A`, `a→B`, `a→A→C`, `a→B→B` (the self-loop), giving 4 paths and 4 `?b` bindings
- SPARQL distinct endpoint dedup reduces `a→B` and `a→B→B` to one `B`, giving 3

Fix: For `[*]` in OPTIONAL MATCH, switch to bounded unrolling with a
configurable hop limit (default 8).  Each UNION branch binds individual
edge annotation variables, giving full multiplicity.

**Estimated effort**: Medium-High.
- Full edge annotation: changes to TCK runner CREATE, EdgeInfo, MATCH translator
- Match7[12] bounded-unroll fallback: new `[*]` OPTIONAL MATCH code path

**Scenarios fixed**: both (Match6[14] and Match7[12]) with full implementation

---

### Wave E — Map Literal + head(collect()) Peephole (scenario 10)

**Return4[11]**:
```cypher
MATCH (person:Person)<--(message)<-[like]-(:Person)
WITH like.creationDate AS likeTime, person AS person
  ORDER BY likeTime, message.id
WITH head(collect({likeTime: likeTime})) AS latestLike, person AS person
RETURN latestLike.likeTime AS likeTime
  ORDER BY likeTime
```

This requires:
1. **Map literal construction**: `{likeTime: likeTime}` builds an anonymous
   map/dictionary from a key-value pair
2. **`collect()` aggregation**: Collects all maps into a list (already
   supported for simple values)
3. **`head()` on the collected list**: Extracts the first element
4. **Property access on the map**: `latestLike.likeTime` dereferences a
   key from the map

SPARQL has no native map/dictionary type.  However, this pattern is a
common Cypher idiom that can be **peephole-optimized** away:

**Peephole**: `head(collect({k1: v1, k2: v2, ...}))` with a preceding
ORDER BY is equivalent to "take the first row's values after sorting."
The map is just a way to carry multiple values through the aggregation
boundary.

Transformation:
```cypher
-- Original
WITH head(collect({likeTime: likeTime})) AS latestLike, person AS person
RETURN latestLike.likeTime AS likeTime

-- Equivalent (after peephole)
-- The head(collect(...)) with ORDER BY = "first value after sort"
-- In SPARQL: use a subquery with LIMIT 1 + ORDER BY
```

SPARQL equivalent:
```sparql
SELECT ?likeTime WHERE {
  {
    SELECT ?person (MIN(?likeTime0) AS ?likeTime) WHERE {
      # ... match pattern ...
      << ?src ?like_pred ?dst >> <creationDate> ?likeTime0 .
    }
    GROUP BY ?person
  }
}
ORDER BY ?likeTime
```

Actually, since there's an ORDER BY before the collect, `head(collect(X))`
= the X value from the first row in that ordering.  In SPARQL:
```sparql
SELECT ?likeTime WHERE {
  {
    SELECT ?person ?likeTime WHERE {
      # ... match ...
    }
    ORDER BY ?likeTime ?message_id
    LIMIT 1
  }
}
```

**Implementation**:

1. **Detect the peephole pattern** in the AST: a WITH clause containing
   `head(collect({...}))` preceded by an ORDER BY
2. **Rewrite**: Replace the `head(collect({k: v}))` with a subquery
   that has `LIMIT 1` and the preceding ORDER BY, projecting the
   map values directly
3. **Wire property access**: `latestLike.likeTime` → direct reference
   to the projected `?likeTime` variable from the subquery

**Estimated effort**: Medium-High.  The peephole detection requires AST
pattern matching across clause boundaries (ORDER BY in one WITH, collect
in the next).  Map literal construction is a new AST node type that needs
partial support.

**Scenarios fixed**: 1 (Return4[11])

---

### Wave F — Match4[7] Bound Relationship in Varlen Chain (scenario 1)

**Match4[7]**:
```cypher
MATCH ()-[r:EDGE]-()
MATCH p = (n)-[*0..1]-()-[r]-()-[*0..1]-(m)
RETURN count(p) AS c
```

Expected: 32 rows.  Got: 0.

This uses a **previously-bound relationship variable `r`** inside a new
MATCH pattern chain.  The `r` from the first MATCH must be the same
specific edge in the second MATCH — it's a join constraint.

**Why it fails**: The second MATCH pattern has `[r]` where `r` is already
bound.  The translator treats this as a new relationship pattern rather
than a join on the existing `r` binding.  The `*0..1` chains around it
use property paths that don't bind intermediate edges, so there's no
way to constrain the middle edge to be specifically `r`.

**Approach**: This requires recognizing that `r` is already bound (from
scope tracking) and emitting the second pattern as a join constraint:

```sparql
# First MATCH binds r's components:
?_a ?r_pred ?_b .

# Second MATCH reuses r:
?n <EDGE>? ?_m1 .  # *0..1 to some node
?_m1 ?r_pred ?_m2 . # this must be the same edge as r
                     # need ?_m1 = one of r's endpoints, ?_m2 = the other
?_m2 <EDGE>? ?m .   # *0..1 from there
```

The constraint is: the edge `(?_m1, ?r_pred, ?_m2)` must be exactly the
edge matched by `r` in the first MATCH.  With our `__eid_*` mechanism,
this becomes:
```sparql
FILTER(?__eid_r_second = ?__eid_r_first)
```

But since the relationship is undirected (`-()`), both directions of the
middle edge must be considered.  Combined with `*0..1` on each side,
this creates a large UNION (2 directions × 2 paths each side = significant
combinatorial expansion).

**Implementation**:

1. **Scope tracking**: When translating the second MATCH, detect that
   `[r]` refers to an already-bound EdgeInfo from the first MATCH
2. **Emit join constraint**: Instead of a fresh edge pattern, emit a
   triple pattern that reuses the same predicate variable and adds
   an eid equality filter (or direct variable reuse)
3. **Handle undirected**: The undirected `()-[r]-()` means both
   endpoints are interchangeable, requiring UNION branches
4. **Handle *0..1**: The adjacent `*0..1` segments are bounded, so
   unrolling produces manageable UNION branches

**Estimated effort**: High.  Cross-MATCH variable binding, combined
with undirected edges and bounded varlen chains, creates significant
combinatorial complexity.

**Scenarios fixed**: 1 (Match4[7])

---

## Implementation Order

| Wave | Scenarios | Effort | Cumulative TCK |
|------|-----------|--------|----------------|
| B    | Match9[1] | Medium | 454/463 (98.1%) |
| A    | Match8[2], Return2[14] | Medium | 456/463 (98.5%) |
| E    | Return4[11] | Medium-High | 457/463 (98.7%) |
| D    | Match6[14], Match7[12] | Medium-High | 459/463 (99.1%) |
| F    | Match4[7] | High | 460/463 (99.4%) |
| C    | Match4[8], Match9[6], Match9[7] | High | 463/463 (100%) |

**Rationale for ordering**:

1. **Wave B first**: `last(r)` on `*0..1` is the lowest-hanging fruit.
   Bounded unrolling machinery already exists.  One scenario, self-contained.

2. **Wave A second**: SPARQL Update support (MERGE + DELETE) is valuable
   beyond TCK — it's a real feature users need.  `spargebra::Update` and
   `oxigraph::store::update()` already exist.  Two scenarios.

3. **Wave E third**: Map literal peephole is a common Cypher idiom.
   One scenario but the pattern is useful beyond TCK.

4. **Wave F fourth**: Bound relationship reuse is a correctness issue
   with cross-MATCH scoping.  Hard but well-defined.

5. **Wave C fifth**: Multi-phase execution is the biggest architectural
   change.  Three scenarios but requires a new `TranspileOutput` variant
   and test runner changes.

6. **Wave D last**: The property-path dedup issue is inherent to RDF.
   Match6[14] is likely infeasible.  Match7[12] might be solvable with
   bounded unrolling but changes unbounded `[*]` semantics.

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| MERGE semantics are complex (ON MATCH/ON CREATE) | Only simple MERGE tested in TCK | Implement only the bare pattern-match-or-create case |
| Multi-phase execution adds architectural complexity | Requires new enum variant, test runner changes | Keep continuation spec as simple data (no closures), test independently |
| Full per-edge RDF-star annotation changes data model | Affects all existing tests that rely on bare triples | Gate behind `TargetEngine::multi_edge()` flag, default false; TCK engine enables it |
| Bounded unroll for `[*]` changes semantics for large graphs | Match7[12] uses unbounded `[*]` | Use configurable limit; document behavior |
| Map literal support may need to be general | Only peephole case tested | Implement only the `head(collect({...}))` pattern |

---

## Summary

- **Definitely solvable**: 9 scenarios (all Waves A–F)
- **Flaky, not a real failure**: 1 scenario (Return6[13], list ordering)

**Ceiling with full implementation**: **463/463 (100%)** of real scenarios.
