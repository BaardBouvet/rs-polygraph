# Fundamental Limitations of rs-polygraph

**Status**: reference  
**Updated**: 2026-04-14

This document describes the hard boundaries of what `rs-polygraph` can express
as a **static transpiler** (one Cypher input → one SPARQL string output).

Three levels of mitigation exist, in increasing implementation cost:

| Level | Mechanism | Scope |
|-------|-----------|-------|
| L1 | Bounded unrolling (compile-time) | Any varlen with a finite upper bound |
| L2 | Multi-phase execution (runtime round-trip) | `[rs*]` family |
| L3 | Engine extension (`pg:pathEdges` etc.) | General unbounded path decomposition |

The remaining sections describe which limitation requires which level.

---

## 1. Variable-Length Path Runtime Decomposition

### 1a — Collected Relationship List as Path Constraint (`[rs*]`)

**Cypher pattern**

```cypher
MATCH (a)-[r:REL]->(b)        -- binds 'r' to a specific edge
WITH r
MATCH (first)-[rs*]->(second) -- 'rs' must follow exactly those edges
```

When `rs` is derived from a prior `WITH`, it is a list of relationship
*instances* known only at runtime.  Following "exactly these edges in this
order" requires the query executor to iterate over that list during graph
traversal.

**Why SPARQL 1.1 cannot express this**

SPARQL property paths are compiled from static syntax.  There is no mechanism to
say "follow the predicates in this variable-length list of bindings."  A
workaround (emit one fixed-length chain per known list length) is impossible
because the list length is a runtime value from a prior result set.

**Affected TCK scenarios**

| Scenario | Key query fragment |
|----------|--------------------|
| Match4 [8]  | `[rs*]` with bound endpoints |
| Match9 [6]  | `[rs*]` with bound endpoints |
| Match9 [7]  | `[rs*]` in opposite direction |

**Verdict (static transpiler)**: Blocked.  
**Verdict (multi-phase L2)**: Solvable — see §3.

---

### 1b — Last Element of a Variable-Length Relationship List (`last(r)`)

**Cypher pattern**

```cypher
MATCH ()-[r*0..1]-()
RETURN last(r)
```

In Cypher, a variable-length relationship variable (`r` above) is bound to the
*ordered list* of all edges along the matched path.  `last(r)` extracts the
final edge in that list.

**Why SPARQL 1.1 cannot express this in general**

SPARQL property paths (`?a <p>+ ?b`, `?a <p>* ?b`, etc.) return only the
endpoints; intermediate edges are not bound to any variable.  For an unbounded
`[r*]` path there is no variable to call `last()` on.

**However: the TCK bound is `*0..1`, which is a bounded-unroll problem**

The one and only `last()` scenario in the entire 220-file openCypher TCK corpus
is Match9[1] (in a file explicitly labelled "Match *deprecated* scenarios").
Its upper bound is `1`, so the complete path space is:

| Hops | `r` | `last(r)` |
|------|-----|-----------|
| 0    | `[]` | `null`    |
| 1    | `[edge]` | that edge |

A static bounded-unroll UNION handles this with zero round-trips.  `List8.feature`
("List Last") in the upstream TCK is a 31-line header stub with **no scenarios**.
`last()` on a general unbounded varlen path is not tested anywhere in the TCK —
consistent with the function being deprecated.

**Affected TCK scenarios**

| Scenario | Key query fragment |
|----------|--------------------|  
| Match9 [1] | `RETURN last(r)` where `r` is `[r*0..1]` |

**Verdict (static transpiler)**: Solvable with L1 bounded unrolling — see §3.  
**Verdict (general unbounded `[r*]`)**: Not tested in the TCK; theoretically requires L3 engine extension, but this is not a compliance concern.
| Distinct-endpoint dedup | Bounded-unroll UNION to expose all paths |
| `count(r)` on varlen | Bind a sentinel value (`BIND(<T> AS ?r)`) |
| `nodes(p)` on fixed-length path | CONCAT the named node variables |

The critical difference for `[rs*]` and `last(r)` (unbounded) is that the
**list itself is a runtime value**, not a compile-time constant.  The translator
emits a static SPARQL string before any data is seen, so it cannot inspect the
list length or the specific edge IRIs involved.

---

## 3. Multi-Phase Execution as a Mitigation

Instead of emitting one static SPARQL string, the transpiler can emit a
**continuation**: a first SPARQL query whose result rows are fed back to the
transpiler to generate the final SPARQL query.

```
cypher_input
    │
    ▼
Transpiler::transpile()  ─── returns TranspileOutput::Continuation ───►
    │                                                                    │
    │   phase1_query (SPARQL string)                                     │
    │       │                                                            │
    │       ▼                                                            │
    │   SPARQL engine executes phase1                                    │
    │       │                                                            │
    │       ▼                                                            │
    │   phase1 result rows ──► continuation closure ──► phase2_query   ◄┘
    │                                                        │
    │                                                        ▼
    │                                               SPARQL engine executes phase2
    │                                                        │
    └────────────────────────────────────────────────────── ▼
                                                      final result
```

This maps cleanly onto a `TranspileOutput` enum:

```rust
pub enum TranspileOutput {
    /// Current behaviour: a single complete SPARQL query string.
    Complete(String),

    /// Run `phase1_query`; pass every result row to `continue_fn` to obtain
    /// the final `TranspileOutput` (which may itself be a Continuation).
    Continuation {
        phase1_query: String,
        continue_fn: Box<dyn Fn(Vec<BindingRow>) -> Result<TranspileOutput, PolygraphError>>,
    },
}
```

### 3a — `[rs*]` is fully solvable with exactly two queries

Consider:

```cypher
MATCH (a)-[r:REL]->(b)
WITH collect(r) AS rs
MATCH (first)-[rs*]->(second)
RETURN first, second
```

**Phase 1 query** — translate everything up to the `[rs*]` pattern:

```sparql
SELECT ?rs WHERE {
  ?a <base:REL> ?b .
  # ... group-concat the edge IRIs into an ordered list per row
}
```

Result rows might be:

| `?rs` |
|-------|
| `"<base:REL_1> <base:REL_2>"` |
| `"<base:REL_1> <base:REL_3>"` |

**Phase 2 query** — the continuation closure receives these rows and generates
one concrete fixed-length chain per distinct edge list, UNIONed together:

```sparql
SELECT ?first ?second WHERE {
  { ?first <base:REL_1> ?_m0 . ?_m0 <base:REL_2> ?second . }
  UNION
  { ?first <base:REL_1> ?_m0 . ?_m0 <base:REL_3> ?second . }
}
```

This is **always two round-trips**, regardless of edge list length.  No engine
extension is needed.  The phase 2 query is a standard SPARQL 1.1 BGP.

**Important constraint**: the `rs` variable must be bound to a concrete,
finite list in phase 1.  If the prior `WITH` clause itself produces an
unbounded path, phase 1 is also unbounded — but that is a separate (and
already-handled) varlen problem, not a new one.

### 3b — `last(r)` on the TCK case (`*0..1`) is solvable with bounded unrolling

The specific failing scenario is:

```cypher
MATCH ()-[r*0..1]-() RETURN last(r)
```

The upper bound is **1**, so bounded unrolling (L1, no multi-phase needed)
produces a two-branch UNION:

```sparql
SELECT ?last_r WHERE {
  {
    # 0-hop: path is empty, last(r) = null
    ?_a ?_sentinel ?_a .
    BIND(<pg:null> AS ?last_r)
  } UNION {
    # 1-hop: exactly one edge; last(r) = that edge's predicate
    ?_a ?last_r ?_b .
    FILTER(?last_r NOT IN (rdf:type, <base:__node>))
  }
}
```

This requires **no multi-phase** and **no engine extension**.

Note: the full upstream TCK has 220 feature files.  `last()` appears in exactly
one scenario across all of them (Match9[1], bounded `*0..1`, deprecated).  The
`List8.feature` file for the `last()` list function contains **zero scenarios**.
There is no unbounded `last(r)` test anywhere in the TCK, so the static
transpiler ceiling is not constrained by this case.

---

## 4. What Would Unlock Each Case

| Limitation | Static transpiler (L1) | Multi-phase (L2) | Engine extension (L3) |
|------------|:----------------------:|:----------------:|:---------------------:|
| `[rs*]` (3 TCK scenarios) | ✗ | ✓ exactly 2 queries | ✓ `pg:followEdges` |
| `last(r)` on `*0..1` (1 TCK scenario) | ✓ bounded unroll | not needed | not needed |
| `last(r)` on unbounded `[r*]` | ✗ (not in TCK — deprecated) | ✗ impractical | ✓ `pg:pathEdges` |

SPARQL 1.2 `LATERAL` joins (in draft as of 2026) could in principle allow
runtime-parameterised sub-queries and would change the `[rs*]` row from L2 to
L1, but still would not bind intermediate property-path edges.

---

## 5. Impact on TCK Compliance

Our vendored corpus is **24 of 220** feature files in the upstream TCK.

Under a **static transpiler only** (against our current 463-scenario subset):

| Scenario | Category | Status |
|----------|----------|--------|
| Match4 [8]  | `[rs*]` | blocked — needs L2 or L3 |
| Match9 [1]  | `last(r)` `*0..1` | fixable with L1 unrolling — no SPARQL limit |
| Match9 [6]  | `[rs*]` | blocked — needs L2 or L3 |
| Match9 [7]  | `[rs*]` | blocked — needs L2 or L3 |

Practical ceiling under SPARQL 1.1 static translation: **460/463 (99.4%)**
(Match9[1] is an L1 implementation gap, not a fundamental limit).

With **multi-phase execution** added to `rs-polygraph` (a `TranspileOutput::Continuation`
variant plus a two-query runtime harness in the calling application):

| Scenario | Resolved? |
|----------|-----------|
| Match4 [8]  | ✓ phase-2 concrete chain query |
| Match9 [1]  | ✓ L1 bounded unrolling (no multi-phase needed) |
| Match9 [6]  | ✓ phase-2 concrete chain query |
| Match9 [7]  | ✓ phase-2 concrete chain query |

Practical ceiling with both L1 and L2 implemented: **463/463 (100%)** for the
current vendored subset.

The only truly irreducible static-SPARQL limitation  — `last(r)` on a general
unbounded `[r*]` — has **zero TCK coverage** and is consistent with the function
being deprecated in openCypher.
