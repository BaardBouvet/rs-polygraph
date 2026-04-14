# Protocol Extension: Path-Decomposition Primitives for the Postgres Triplestore

**Status**: planned  
**Updated**: 2026-04-14

This document specifies the two custom SPARQL functions and one custom aggregate
that are needed to lift the fundamental SPARQL 1.1 limitations described in
[fundamental-limitations.md](fundamental-limitations.md), assuming the target
engine is our own Postgres-backed triplestore.

The extensions are **narrow and principled** — they do not add a general
property-graph query engine on top of Postgres.  They expose exactly what
SPARQL 1.1 is missing: the ability to (a) walk a runtime-supplied list of edge
IRIs in order, and (b) bind intermediate edges during a property-path traversal.

---

## Background: What SPARQL 1.1 Is Missing

| Cypher construct | Problem |
|-----------------|---------|
| `MATCH (a)-[rs*]->(b)` where `rs` comes from a prior WITH | Path to follow is a *runtime list* of edge IRIs — SPARQL paths are static |
| `RETURN last(r)` on `[r*]` | Property paths never bind intermediate edge variables |

Both problems share a root cause: **SPARQL property paths are compiled, not
data-driven**.  The fix is a small set of server-side functions that accept
runtime bindings and perform graph traversal inside the engine — where the data
already lives.

---

## Extension 0: `TargetEngine` Capability Flag

Add one new capability flag to the existing `TargetEngine` trait in
`src/target/mod.rs`:

```rust
/// Returns `true` if the engine supports the pg:pathDecompose / pg:followEdges
/// custom functions described in plans/pg-extension-protocol.md.
fn supports_path_decomposition(&self) -> bool {
    false   // conservative default
}
```

The Postgres triplestore target struct overrides this to `true`.  The translator
checks this flag before emitting any extended syntax, falling back to the current
"skip / UnsupportedFeature" behaviour on engines that don't have the extension.

---

## Extension 1: `pg:followEdges` — Walk a Runtime Edge List

### Purpose

Handles the `[rs*]` pattern family.  Given a *runtime* list of edge IRIs (bound
from a prior query result), traverse exactly those edges in order and bind path
endpoints.

### SPARQL syntax

```sparql
PREFIX pg: <http://polygraph.example/ext/>

SELECT ?first ?second WHERE {
  # 'rs' is a bag of edge IRIs from a prior WITH clause
  ?rs pg:followEdges (?first ?second) .
}
```

Or, when edge identity is available as an RDF-star binding bag:

```sparql
  pg:followEdges(?rs_bag, ?first, ?second) .
```

Both forms bind `?first` (the start of the chain) and `?second` (the end), and
require `?rs_bag` to be bound to a serialised edge list (see below).

### Edge list encoding

A collected relationship list is serialised by the `pg:edgeBag` aggregate
(Extension 3 below) as an `xsd:string` of space-separated edge IRIs in
traversal order:

```
"<http://pg.ex/e1> <http://pg.ex/e2> <http://pg.ex/e3>"
```

The function validates that the IRIs reference existing triples in the store
and that consecutive edges are connected (object of edge N = subject of edge
N+1).

### Postgres implementation sketch

```sql
-- Registered as a SPARQL extension function in the engine's function table
CREATE OR REPLACE FUNCTION pg_ext.follow_edges(
    edge_list  text,   -- space-separated edge IRI list from binding
    OUT first  text,   -- subject of first edge
    OUT second text    -- object of last edge
)
RETURNS SETOF RECORD
LANGUAGE sql STABLE AS $$
  WITH edges AS (
    -- parse the IRI list into an ordered array
    SELECT
      unnest(string_to_array(trim(both '<>' from e), '> <')) AS edge_iri,
      ordinality
    FROM regexp_split_to_table(edge_list, '\s+') WITH ORDINALITY AS t(e, ordinality)
  ),
  resolved AS (
    SELECT
      t.subject_iri,
      t.object_iri,
      e.ordinality
    FROM edges e
    JOIN rdf_triples t ON t.predicate_iri = e.edge_iri   -- composite index on predicate+ordinality
    ORDER BY e.ordinality
  ),
  chain_check AS (
    -- verify connectivity: object[n] = subject[n+1]
    SELECT
      r1.object_iri = r2.subject_iri AS connected,
      r1.object_iri,
      r2.subject_iri
    FROM resolved r1
    JOIN resolved r2 ON r2.ordinality = r1.ordinality + 1
  )
  SELECT
    (SELECT subject_iri FROM resolved WHERE ordinality = 1) AS first,
    (SELECT object_iri  FROM resolved ORDER BY ordinality DESC LIMIT 1) AS second
  WHERE NOT EXISTS (SELECT 1 FROM chain_check WHERE NOT connected)
$$;
```

The engine's SPARQL evaluation layer calls `pg_ext.follow_edges` during solution
mapping, once per row of the incoming binding.

### Translator changes in `rs-polygraph`

In `translator/cypher.rs`, when emitting a `[rs*]` pattern and
`engine.supports_path_decomposition()` is true:

```sparql
# Previously: UnsupportedFeature error
# Now:
?first pg:followEdges(?rs, ?second) .
```

Where `?rs` is the variable that was bound by the prior `collect()` + `WITH`
clause.

---

## Extension 2: `pg:pathEdges` — Bind Intermediate Edges During Traversal

### Purpose

Handles `last(r)` (and, more generally, `nodes(p)`, `relationships(p)`,
`length(p)`, per-hop property filters on unbounded varlen).  Instead of a
SPARQL property path that returns only endpoints, this function returns one row
per edge, binding each edge's subject, predicate, and object.

### SPARQL syntax

```sparql
PREFIX pg: <http://polygraph.example/ext/>

SELECT ?a ?b ?last_edge WHERE {
  # Enumerate every edge on every path from ?a to ?b via <base:REL>
  pg:pathEdges(<base:REL>, ?a, ?b, ?edge_subj, ?edge_pred, ?edge_obj, ?depth) .

  # last(r): bind to the edge with the highest depth in each path group
  BIND(?edge_pred AS ?last_edge)
}
ORDER BY ?a ?b DESC(?depth)
```

Parameters:

| Parameter | Direction | Meaning |
|-----------|-----------|---------|
| `predicate` | IN (IRI or `*` wildcard) | Edge type(s) to traverse, or `pg:ANY` for all |
| `?start` | IN (bound) or OUT (free) | Source node |
| `?end` | IN (bound) or OUT (free) | Destination node |
| `?edge_subj` | OUT | Subject of this hop's triple |
| `?edge_pred` | OUT | Predicate of this hop's triple |
| `?edge_obj` | OUT | Object of this hop's triple |
| `?depth` | OUT | 1-based position of this hop in the path |

Optional modifiers passed as named keyword arguments:

```sparql
pg:pathEdges(<base:REL>, ?a, ?b, ?es, ?ep, ?eo, ?d ;
             pg:minLength 3 ;
             pg:maxLength 10 ;
             pg:direction pg:OUTGOING )  .
```

Omitting `pg:maxLength` signals "unbounded" — the engine applies a configurable
safety cap (default: graph diameter or 50, whichever is smaller) to prevent
infinite loops on cyclic graphs.

### Postgres implementation sketch

```sql
CREATE OR REPLACE FUNCTION pg_ext.path_edges(
    pred_iri   text,          -- NULL = any predicate
    start_iri  text,          -- NULL = free
    end_iri    text,          -- NULL = free
    min_len    int DEFAULT 1,
    max_len    int DEFAULT 50,
    direction  text DEFAULT 'OUTGOING'  -- 'OUTGOING' | 'INCOMING' | 'ANY'
)
RETURNS TABLE (
    edge_subj  text,
    edge_pred  text,
    edge_obj   text,
    depth      int
)
LANGUAGE sql STABLE AS $$
  WITH RECURSIVE traverse(subj, pred, obj, depth, path_key, visited) AS (
    -- base case: 1-hop edges from start (or all edges if start is free)
    SELECT
      t.subject_iri,
      t.predicate_iri,
      t.object_iri,
      1,
      t.subject_iri || '|' || t.predicate_iri || '|' || t.object_iri,
      ARRAY[t.subject_iri || '>' || t.object_iri]
    FROM rdf_triples t
    WHERE (pred_iri IS NULL OR t.predicate_iri = pred_iri)
      AND (start_iri IS NULL OR
           CASE direction
             WHEN 'INCOMING' THEN t.object_iri  = start_iri
             ELSE                 t.subject_iri = start_iri
           END)
      AND t.predicate_iri NOT IN (
        'http://www.w3.org/1999/02/22-rdf-syntax-ns#type',
        'http://polygraph.example/__node'
      )
    UNION ALL
    -- recursive case
    SELECT
      t.subject_iri,
      t.predicate_iri,
      t.object_iri,
      tr.depth + 1,
      tr.path_key || '|' || t.subject_iri || '|' || t.predicate_iri || '|' || t.object_iri,
      tr.visited || (t.subject_iri || '>' || t.object_iri)
    FROM traverse tr
    JOIN rdf_triples t ON (
      CASE direction
        WHEN 'INCOMING' THEN t.object_iri  = tr.subj
        WHEN 'ANY'      THEN t.subject_iri = tr.obj OR t.object_iri = tr.subj
        ELSE                 t.subject_iri = tr.obj
      END
    )
    WHERE tr.depth < max_len
      AND NOT (t.subject_iri || '>' || t.object_iri) = ANY(tr.visited)  -- cycle guard
      AND (pred_iri IS NULL OR t.predicate_iri = pred_iri)
  )
  SELECT subj, pred, obj, depth
  FROM traverse
  WHERE depth >= min_len
    AND (end_iri IS NULL OR
         CASE direction
           WHEN 'INCOMING' THEN subj = end_iri
           ELSE                 obj  = end_iri
         END)
$$;
```

The cycle guard (`visited` array) prevents infinite loops on graphs with cycles
(e.g. the Match7 LOOP node).  For large graphs an HLL sketch or bloom-filter
alternative would be preferred, but an array is correct for development.

### Handling `last(r)` with this extension

```cypher
MATCH ()-[r*0..1]-()
RETURN last(r)
```

Compiled SPARQL (with extension):

```sparql
PREFIX pg: <http://polygraph.example/ext/>

SELECT ?last_r WHERE {
  {
    # 0-hop case: last(r) = null
    BIND(<pg:null> AS ?r_edge_pred)
    BIND(0 AS ?r_depth)
  } UNION {
    # 1+ hop case: enumerate all hops
    pg:pathEdges(pg:ANY, ?_a, ?_b, ?r_es, ?r_edge_pred, ?r_eo, ?r_depth ;
                 pg:minLength 0 ;
                 pg:maxLength 1)
  }
  # Extract only the deepest hop per path (= last edge)
}
GROUP BY ?_a ?_b
HAVING ?r_depth = MAX(?r_depth)
BIND(?r_edge_pred AS ?last_r)
```

The translator emits this pattern when:
1. `engine.supports_path_decomposition()` is true, AND
2. the expression involves `last()`, `nodes()`, `relationships()`, or
   `length()` on a varlen relationship variable, OR
3. the varlen has a per-hop property filter.

---

## Extension 3: `pg:edgeBag` — Aggregate a Path into a Serialised Edge List

### Purpose

The companion aggregate to `pg:followEdges`.  Collects a path traversal
(produced by ordinary MATCH or by `pg:pathEdges`) into the wire format expected
by `followEdges`.

### SPARQL syntax

```sparql
SELECT ?a ?b (pg:edgeBag(?edge_pred ORDER BY ?depth) AS ?rs) WHERE {
  pg:pathEdges(<base:REL>, ?a, ?b, ?es, ?edge_pred, ?eo, ?depth) .
}
GROUP BY ?a ?b
```

Produces one row per `(?a, ?b)` pair with `?rs` bound to a serialised edge list:

```
"<http://pg.ex/REL> <http://pg.ex/REL>"
```

This is then usable as the input to `pg:followEdges` in a subsequent sub-select
or `WITH` clause — closing the round-trip.

### Postgres implementation sketch

```sql
CREATE OR REPLACE AGGREGATE pg_ext.edge_bag(pred_iri text ORDER BY depth int) (
  SFUNC    = array_append,
  STYPE    = text[],
  FINALFUNC = pg_ext.iri_list_to_string,
  INITCOND  = '{}'
);

CREATE OR REPLACE FUNCTION pg_ext.iri_list_to_string(iris text[])
RETURNS text LANGUAGE sql IMMUTABLE AS $$
  SELECT string_agg('<' || i || '>', ' ') FROM unnest(iris) AS i
$$;
```

---

## Integration with `rs-polygraph`

### New `TargetEngine` impl

```rust
// src/target/postgres.rs  (new file)
use super::TargetEngine;

/// Target adapter for our Postgres-backed triplestore with
/// pg:pathEdges / pg:followEdges / pg:edgeBag extensions.
pub struct PostgresTriplestore {
    base_iri: Option<String>,
}

impl PostgresTriplestore {
    pub fn new(base_iri: Option<impl Into<String>>) -> Self {
        Self { base_iri: base_iri.map(Into::into) }
    }
}

impl TargetEngine for PostgresTriplestore {
    fn supports_rdf_star(&self) -> bool { true }
    fn supports_federation(&self) -> bool { false }
    fn supports_path_decomposition(&self) -> bool { true }

    fn base_iri(&self) -> Option<&str> {
        self.base_iri.as_deref()
    }
}
```

### Translator dispatch (sketch)

In `translator/cypher.rs`, the branching for varlen relationship patterns gains
a third arm:

```
if engine.supports_path_decomposition()        => emit pg:pathEdges / pg:followEdges
else if upper_bound is Some(N) && N <= MAX_UNROLL => emit N-step bounded UNION
else                                               => emit SPARQL property path (NPS)
                                                      + UnsupportedFeature for last(r)
```

### Namespace declaration

The translator prefixes any query that uses extensions with:

```sparql
PREFIX pg: <http://polygraph.example/ext/>
```

This prefix is only emitted when `supports_path_decomposition()` is true.

---

## Postgres Schema Assumptions

The implementation sketches above assume an `rdf_triples` table with at minimum:

```sql
CREATE TABLE rdf_triples (
    subject_iri   text NOT NULL,
    predicate_iri text NOT NULL,
    object_iri    text,           -- NULL when object is a literal
    object_lit    text,           -- NULL when object is an IRI
    object_dt     text,           -- datatype IRI for typed literals
    object_lang   text,           -- language tag for lang-string literals
    graph_iri     text NOT NULL DEFAULT 'default'
);

CREATE INDEX rdf_triples_spo ON rdf_triples (subject_iri, predicate_iri, object_iri);
CREATE INDEX rdf_triples_pos ON rdf_triples (predicate_iri, object_iri, subject_iri);
CREATE INDEX rdf_triples_osp ON rdf_triples (object_iri, subject_iri, predicate_iri);
```

The recursive CTE in `pg_ext.path_edges` relies on `rdf_triples_pos` (forward
traversal) and `rdf_triples_osp` (backward traversal for `INCOMING` /  `ANY`
direction).

---

## Considered Alternatives

| Alternative | Reason not chosen |
|-------------|------------------|
| SPARQL 1.1 `SERVICE` federation to a path-service | Adds a network hop; path data is already in Postgres |
| Full GQL/openCypher engine in Postgres (e.g. Apache AGE) | Much larger scope; loses SPARQL interoperability |
| SPARQL 1.2 `LATERAL` joins | Spec not yet finalised; no production implementations as of 2026 |
| Bounded unrolling only | Does not help `[rs*]` (runtime list length unknown at compile time) |

---

## Open Questions

1. **Cycle semantics** — Cypher uses "no repeated relationships" (not "no
   repeated nodes") as its path uniqueness rule.  The current cycle guard uses
   node visited-set semantics.  Should be updated to track `(subj, pred, obj)`
   triples instead.

2. **`pg:maxLength` cap** — What is the right default for production?  Graph
   diameter is expensive to compute.  A fixed cap of 20 avoids most runaway
   queries while covering realistic property graph depths.

3. **RDF-star edge identity in `pg:edgeBag`** — Should the edge bag store
   triple-level identity `<<s p o>>` rather than just the predicate IRI?  This
   matters when the same predicate appears multiple times between the same pair
   of nodes (parallel edges), which is valid in Cypher/LPG but unusual in RDF.

4. **Security** — The `pg_ext` schema should be read-only from the SPARQL
   evaluation layer; it must not be callable via user-supplied SPARQL literals
   to avoid injection into the recursive CTE.
