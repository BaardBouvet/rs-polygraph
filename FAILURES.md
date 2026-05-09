# TCK Failure Reference

**Updated**: 2026-05-09  
**Baseline**: 3756 / 3828 (frozen, committed in `tests/tck/baseline/scenarios.jsonl`)  
**Current**: **3820 / 3828 passing** (99.8 %)  
**Remaining failures**: **8** — all are permanent or require multi-phase runtime execution  
**Legacy fallback events**: **432** — see [plans/legacy-removal.md](plans/legacy-removal.md) for elimination roadmap

L-levels from [plans/fundamental-limitations.md](plans/fundamental-limitations.md):

| Level | Meaning |
|-------|---------|
| **L1** | Fixable within the static, single-round-trip transpiler model |
| **L2** | Requires multi-phase (Continuation) runtime execution |
| **L3** | Permanently infeasible — structural RDF/SPARQL limitation |

---

## The 8 remaining failures

### 1. Match4[8] — Pre-bound relationship list in varlen  `L3`

**Feature**: [Match4.feature](tests/tck/features/clauses/match/Match4.feature) (`@skipGrammarCheck`)

```cypher
MATCH ()-[r1]->()-[r2]->()
WITH [r1, r2] AS rs
  LIMIT 1
MATCH (first)-[rs*]->(second)
RETURN first, second
```

**Why it cannot be fixed**: The pattern `[rs*]` with `rs` being a pre-bound list of
relationship objects is absent from the openCypher grammar (hence `@skipGrammarCheck`).
It asks the engine to traverse *exactly* those specific edges, in sequence, as a path.
SPARQL property paths have no concept of "use this set of named edges"; the only
representation would be an explicit triple chain bound by specific IRI values, which
requires knowing the edge IDs at query-compile time. Static transpilation has no means
to reify or enumerate the contents of a runtime list into a path pattern.

---

### 2. Match5[27] — Multigraph traversal after DELETE+CREATE reshaping  `L3`

**Feature**: [Match5.feature](tests/tck/features/clauses/match/Match5.feature)  
*(The TCK scenario comment reads: "This gets hard to follow for a human mind. The answer is named graphs, but it's not crucial to fix.")*

**Setup** (two `having_executed` steps): Reverses all non-A edges with `DELETE r CREATE (b)-[:LIKES]->(a)`, then fans out every D node to two E children.

**Query**: `MATCH (a:A) MATCH (a)-[:LIKES]->()<-[:LIKES*3]->(c) RETURN c.name` — expects 16 rows.

**Why it cannot be fixed**: The setup creates multiple `:LIKES` edges between the same
pair of nodes (e.g., `(d)-[:LIKES]->(e1)` and `(d)-[:LIKES]->(e2)` are fine, but
intermediate reversal steps produce two parallel `:LIKES` edges between the same pair
when a node has two outgoing edges that both get reversed onto the same target).  In RDF
a predicate triple `<s> <p> <o>` is a set — duplicate edges between the same pair under
the same predicate are silently deduplicated. The 16-result query relies on counting
distinct paths through these parallel edges. Named graphs (each edge in a separate graph)
are the only RDF mechanism that preserves edge multiplicity; they require engine-level
support not present in any standard SPARQL 1.1 engine.

---

### 3. Match6[14] — Undirected fixed-length varlen over parallel edges  `L3`

**Feature**: [Match6.feature](tests/tck/features/clauses/match/Match6.feature)

**Setup**:
```cypher
CREATE (db1:Start), (db2:End), (mid), (other)
CREATE (mid)-[:CONNECTED_TO]->(db1),
       (mid)-[:CONNECTED_TO]->(db2),
       (mid)-[:CONNECTED_TO]->(db2),   -- parallel edge
       (mid)-[:CONNECTED_TO]->(other),
       (mid)-[:CONNECTED_TO]->(other)  -- parallel edge
```

**Query**: `MATCH topRoute = (:Start)<-[:CONNECTED_TO]-()-[:CONNECTED_TO*3..3]-(:End) RETURN topRoute` — expects 4 rows.

**Why it cannot be fixed**: The setup deliberately creates two parallel `:CONNECTED_TO`
edges between `mid→db2` and two between `mid→other`. The four expected result paths are
formed by independently traversing these parallel edges during the fixed-length varlen
hop. In RDF, `<mid> <CONNECTED_TO> <db2>` is a single triple regardless of how many
times it was asserted. The four parallel edges collapse to two distinct triples, and the
varlen path enumerator sees only those two triples, producing fewer paths than expected.
This is the canonical multigraph limitation (L3).

---

### 4. Merge5[3] — MERGE match count on multigraph setup  `L3`

**Feature**: [Merge5.feature](tests/tck/features/clauses/merge/Merge5.feature)

**Setup**:
```cypher
CREATE (a:A), (b:B)
CREATE (a)-[:TYPE]->(b)
CREATE (a)-[:TYPE]->(b)   -- deliberately creates a second parallel edge
```

**Query**: `MATCH (a:A), (b:B) MERGE (a)-[r:TYPE]->(b) RETURN count(r)` — expects `2`.

**Why it cannot be fixed**: The setup inserts two parallel `:TYPE` edges between the same
`(A)→(B)` pair. In RDF the triple `<A> <TYPE> <B>` is written once. After setup there is
exactly one `:TYPE` triple in the store. `MERGE` matches that one triple, binds `r` once,
and `count(r)` returns 1. The expected answer of 2 requires the engine to distinguish two
co-existing edges that share the same subject, predicate, and object — a multigraph
property that is structurally absent from the RDF data model and from SPARQL semantics.

---

### 5. Merge5[21] — DELETE+MERGE with multigraph setup  `L3 + L2`

**Feature**: [Merge5.feature](tests/tck/features/clauses/merge/Merge5.feature)

**Setup**: Creates `(A)-[:T {name:'rel1'}]->(B)` and `(A)-[:T {name:'rel2'}]->(B)` — two
parallel `:T` edges between the same pair.

**Query**:
```cypher
MATCH (a)-[t:T]->(b)
DELETE t
MERGE (a)-[t2:T {name: 'rel3'}]->(b)
RETURN t2.name
```

Expected: 2 rows of `'rel3'`.

**Why it cannot be fixed**: There are two independent obstacles, either of which is
individually fatal:

1. **L3 — multigraph setup**: The two parallel `:T` edges collapse to a single
   `<A> <T> <B>` triple at INSERT time. `MATCH (a)-[t:T]->(b)` sees only one match,
   not two. `RETURN t2.name` can therefore produce at most 1 row, never 2.

2. **L2 — snapshot semantics**: Even if the multigraph were representable, the SPARQL
   UPDATE snapshot model means the MERGE's WHERE clause sees the *pre-DELETE* graph and
   matches the `:T` edge being deleted — making NOT EXISTS false, so MERGE does nothing.
   Two-phase L2 execution (DELETE first, then MERGE in a second round-trip) would fix
   this half, but the multigraph half remains.

Fixing Merge5[21] would require both named-graph multigraph support (L3, engine change)
and two-phase execution (L2, implementable). The L3 dependency makes it permanently out of
reach for this transpiler.

---

### 6. Merge1[14] — MERGE must not match deleted nodes  `L2`

**Feature**: [Merge1.feature](tests/tck/features/clauses/merge/Merge1.feature)

**Setup**: Two `:A` nodes with properties `{num:1}` and `{num:2}`.

**Query**:
```cypher
MATCH (a:A)
DELETE a
MERGE (a2:A)
RETURN a2.num
```

Expected: 2 rows of `null` (MERGE creates one fresh `:A`; the result cross-products with
the 2-row DELETE context but `a2.num` is null since the new node has no property).

**Why it is hard**: openCypher specifies that each clause sees the graph state produced by
the previous clause. After `DELETE a`, `a` no longer exists, and `MERGE (a2:A)` should
search an empty `:A` set, create a new node, and bind `a2` to it. SPARQL UPDATE uses
snapshot semantics: the WHERE clause of every INSERT in the same update batch reads the
graph *before* any deletes in that batch. Our emitted `MERGE` INSERT therefore still sees
the two `:A` nodes under deletion and matches them, producing wrong rows.

**Mitigation (L2)**: Two-phase execution — emit DELETE as the first SPARQL UPDATE,
execute it, then emit MERGE as a second update against the now-mutated store. The
`TranspileOutput::Continuation` API supports this; it needs a write-path emitter that
splits DELETE and subsequent MERGE into separate `Continuation` phases.

---

### 7. List12[1] — List comprehension captures pre-SET values  `L2`

**Feature**: [List12.feature](tests/tck/features/expressions/list/List12.feature)

**Setup**: One node `(:Label1 {name: 'original'})`.

**Query**:
```cypher
MATCH (a:Label1)
WITH collect(a) AS nodes
WITH nodes, [x IN nodes | x.name] AS oldNames
UNWIND nodes AS n
SET n.name = 'newName'
RETURN n.name, oldNames
```

Expected: `n.name = 'newName'`, `oldNames = ['original']`.

**Why it is hard**: `oldNames` must capture the `.name` property *before* `SET n.name =
'newName'` executes. A single-pass SPARQL UPDATE+SELECT model cannot produce this: the
SELECT that provides `oldNames` runs after the UPDATE mutates the store, reading
`'newName'` instead of `'original'`. There is no SPARQL mechanism to snapshot a value
before a mutation and return it after.

**Mitigation (L2)**: Three-phase continuation — Phase 1 collects `nodes` and computes
`oldNames` via SELECT; Phase 2 applies `SET n.name = 'newName'` as an UPDATE; Phase 3
returns the final RETURN projection by joining the Phase 1 result with Phase 2's
post-state. This requires the L2 Continuation API to materialise intermediate result sets
in Rust and pass them between phases.

---

### 8. List12[2] — List comprehension filter captures pre-SET values  `L2`

**Feature**: [List12.feature](tests/tck/features/expressions/list/List12.feature)

**Query**:
```cypher
MATCH (a:Label1)
WITH collect(a) AS nodes
WITH nodes, [x IN nodes WHERE x.name = 'original'] AS noopFiltered
UNWIND nodes AS n
SET n.name = 'newName'
RETURN n.name, size(noopFiltered)
```

Expected: `n.name = 'newName'`, `size(noopFiltered) = 1`.

**Why it is hard**: Same pre-write state issue as List12[1]. The filter
`WHERE x.name = 'original'` must evaluate before `SET n.name = 'newName'` is applied.
Post-SET, no node satisfies `x.name = 'original'`, so `noopFiltered` is empty and
`size(noopFiltered) = 0`. The fix is identical: three-phase L2 continuation.

---

## Legacy fallback inventory  (432 events)

Every test execution that routes through the legacy translator
(`crates/polygraph/src/translator/`) is a blocker for its deletion. Run
`POLYGRAPH_TRACE_LEGACY=1 cargo test --test tck` to reproduce.

Current breakdown by construct (May 2026):

| Construct | Events | L-level | Notes |
|-----------|-------:|---------|-------|
| Quantifier over non-constant list | 97 | L2 | `nodes(p)`, `relationships(p)`, runtime list |
| List comprehension on variable list | 94 | L2 | `[x IN collect(…) \| expr]` |
| UNWIND with variable/expression list | 30 | L2 | Non-literal UNWIND source |
| Non-literal in VALUES/UNWIND context | 28 | L2 | Variable list in VALUES |
| `write_delete_with_return` | 23 | L1 | DELETE+RETURN in same query |
| Path value in projection | 17 | L2 | Named path in RETURN/WITH |
| `write_merge_with_outer_match` | 15 | L1 | MERGE inside outer MATCH scope |
| PatternComprehension expression | 13 | L2 | `[(a)-->(b) \| b.prop]` |
| `varlen_named_relvar` (safety pre-pass) | 13 | L1 | `[r*]` with named `r` |
| `write_set_replace_or_merge_map` | 12 | L1 | `SET n = map` / `SET n += map` |
| `relvar_after_with` (safety pre-pass) | 11 | L1 | Rel var referenced after WITH |
| Exists expression | 9 | L1 | `EXISTS { pattern }` |
| `unbounded_varlen_unlabeled` (safety pre-pass) | 9 | L1 | Bare `[*]` guard |
| Dynamic list concatenation | 8 | L2 | `list1 + list2` with runtime operands |
| `write_set_complex_expr` | 5 | L1 | Complex value expression in SET |
| `write_delete_complex_expr` | 5 | L1 | Complex delete target expression |
| `collect()` aggregate | 6 | L2 | In non-RETURN position |
| List ordering comparison | 4 | L1 | Type-ranked ORDER BY extension |
| Subscript expression | 4 | L1 | `list[idx]` in SPARQL context |
| List/map equality with null | 3 | L1 | Null-aware equality |
| Aggregate in non-standard position | 3 | L1 | Aggregates outside GROUP BY |
| `write_merge_rel_unbound_nodes` | 2 | L1 | Partially-bound MERGE endpoints |
| `write_delete_rel_undirected_untyped` | 2 | L1 | `DELETE` on `--` pattern |
| `labels()` function | 2 | L2 | Runtime label extraction |
| `head()` function | 2 | L1 | First element of list |
| Misc (write_select_complex, UNWIND_write, …) | 7 | L1/L2 | Long-tail constructs |

**Elimination roadmap**: see [plans/legacy-removal.md](plans/legacy-removal.md).
