# Legacy Translator Removal Plan

**Status**: planned  
**Updated**: 2026-05-09  
**Target release**: v1.0.0  
**Prerequisite**: 3820 / 3828 TCK scenarios passing (current baseline)

## Overview

The legacy translator lives in `crates/polygraph/src/translator/` (~19 k lines across
9 files). It was the original openCypher→SPARQL implementation, now used only as a
fallback when the LQA path returns `Err(Unsupported)`. The LQA path (`crates/polygraph/src/lqa/`)
was designed to replace it entirely.

There are currently **432 fallback events** during the full TCK run
(measure with `POLYGRAPH_TRACE_LEGACY=1 cargo test --test tck`). Until that number
reaches zero the legacy module cannot be deleted without regressions.

This plan eliminates the 432 events in four ordered phases, then deletes the dead code.

---

## Files involved

### Legacy (to be deleted at the end)

| File | Lines | Role |
|------|------:|------|
| `src/translator/cypher/mod.rs` | 5 162 | Main entry: `translate()`, `translate_skip_writes()` |
| `src/translator/cypher/clauses.rs` | 2 143 | MATCH / WITH / UNWIND / RETURN clause visitor |
| `src/translator/cypher/temporal.rs` | 3 983 | Temporal type constructors and arithmetic |
| `src/translator/cypher/functions.rs` | 1 660 | Built-in function translation |
| `src/translator/cypher/semantics.rs` | 1 617 | Null-handling, type-coercion rules |
| `src/translator/cypher/patterns.rs` | 1 605 | Graph pattern lowering |
| `src/translator/cypher/util.rs` | 1 173 | SPARQL string helpers |
| `src/translator/cypher/write_update.rs` | 1 121 | CREATE / MERGE / SET / DELETE translation |
| `src/translator/cypher/return_proj.rs` | 628 | RETURN/WITH projection builder |
| `src/translator/gql.rs` | 26 | GQL stub (already thin) |
| `src/translator/mod.rs` | 5 | Public re-export |
| `src/translator/visitor.rs` | 24 | `AstVisitor` trait definition |

**Total**: ~19 k lines to delete.

### LQA (to be extended)

| File | Role |
|------|------|
| `src/lqa/sparql.rs` | Read-path compiler — extend to cover L1 constructs |
| `src/lqa/write.rs` | Write-path compiler — extend to cover L1 write constructs |
| `src/lib.rs` | Fallback routing — remove legacy branches progressively |
| `src/lqa/op.rs` | Op enum — add ops for any new L1 constructs that need IR |
| `src/lqa/expr.rs` | Expr enum — add expressions currently only in AST |

---

## Fallback taxonomy (432 events, May 2026)

Events are classified into three categories depending on where the fallback fires:

- **Class A** — `lqa_compile=Unsupported` (LQA Op tree built but SPARQL lowering fails)
- **Class B** — `lqa_write_compile=Unsupported` (write Op tree built but write.rs rejects)  
- **Class C** — `is_lqa_safe=false` (safety pre-pass rejects, LQA never attempted)

| Class | Events | L-level mix |
|-------|-------:|-------------|
| A | 304 | L1: ~90, L2: ~214 |
| B | 97 | L1: ~60, L2: ~37 |
| C | 31 | L1: 31 |

---

## Phase 1 — Lift safety pre-pass restrictions  (31 events → 0)

**Goal**: Make `is_lqa_safe()` in `src/lib.rs` accept constructs it currently rejects,
so the LQA path is at least *attempted* before falling back.

Each restriction was added conservatively when LQA first launched. The LQA SPARQL
compiler has matured; these restrictions can now be lifted one at a time with
test-by-test verification.

### 1.1 `varlen_named_relvar` (13 events)

**Construct**: `MATCH (a)-[r*2..4]->(b) WHERE all(x IN r WHERE ...)` — a named
relationship variable on a varlen pattern.

**Current block**: The safety pass rejects these because the old LQA didn't support
iterating over the collected relationship list from a varlen match.

**Fix**: Remove the `varlen_named_relvar` guard in `is_lqa_safe()`. In `sparql.rs`,
handle `Op::Expand { range: Some(_), rel_var: Some(name), .. }` by emitting the
SPARQL property path and exposing `?name` as a sentinel marker (the same trick used for
single-hop relationships). If the query body uses `r` only as a type marker or in `count(r)`,
the existing marker encoding works without changes. If it uses `r` in `nodes(r)` or
`relationships(r)`, emit `Err(Unsupported)` so the specific failure becomes traceable.

**Files**: `src/lib.rs` (remove guard), `src/lqa/sparql.rs` (extend Expand lowering).

### 1.2 `relvar_after_with` (11 events)

**Construct**: `MATCH (a)-[r]->(b) WITH r RETURN type(r)` — relationship variable
carried past a WITH boundary.

**Current block**: The safety pass suspected the LQA WITH-boundary variable scoping
would lose track of `r`. In practice, LQA correctly threads `r` through WITH via the
CartesianProduct/Projection chain.

**Fix**: Remove the `relvar_after_with` guard. Run TCK; if any sub-cases produce wrong
results, add targeted `Err(Unsupported)` in the affected LQA code paths rather than
blocking the entire class.

**Files**: `src/lib.rs` (remove guard).

### 1.3 `unbounded_varlen_unlabeled` (9 events — partial)

**Construct**: `MATCH (a)-[*]->(b)` with no type bound on the relationship.

**Current block**: An unbounded, untyped varlen expansion was considered too risky
(potential explosion). However Oxigraph handles property-path queries efficiently.

**Fix**: Remove the guard and add a depth-safety cap instead: emit the SPARQL property
path `((<p1>|<p2>|…)*)` for the union of all relationship types discovered in the schema,
or fall back with a clear error if no schema is available. For engines that declare a
schema (via `TargetEngine::relationship_types()`), this is safe. For schema-less mode,
keep the guard only when no type bound is present AND no schema is supplied.

**Files**: `src/lib.rs` (conditionalise guard), `src/target.rs` (add optional `relationship_types()` to trait with empty default).

---

## Phase 2 — L1 expression and write-path extensions  (~95 events → 0)

These constructs are individually tractable; each can be implemented in a focused PR
without architectural changes.

### 2.1 `write_delete_with_return` (23 events)

**Construct**: `MATCH (n) DELETE n RETURN count(*)` — a write query with a RETURN clause.

**Current block**: `compile_write()` succeeds (emits the DELETE), but `compile_output()`
on the stripped read-only op fails because the SELECT after DELETE reads a stale row
count.

**Fix**: Emit the DELETE UPDATE, then emit a separate COUNT/SELECT that reads the
post-delete graph. For simple `RETURN count(*)` / `RETURN count(n)` patterns this is
a trivially correct two-step: (1) capture the pre-delete match count via a SELECT
before executing the DELETE, or (2) emit `RETURN count(*)` as `0` when all matched
nodes are deleted. The exact strategy depends on whether the RETURN expression references
deleted variables; handle the common cases, emit `Err(Unsupported)` for anything more
complex.

**Files**: `src/lqa/write.rs` (`compile_write_recursive(Op::Delete)` branch), `src/lib.rs`.

### 2.2 `write_merge_with_outer_match` (15 events)

**Construct**: `MATCH (a:Person) MERGE (a)-[:KNOWS]->(b:Person) RETURN b`.

**Current block**: The outer MATCH provides a binding for `a`; the MERGE write path
requires `a` to be in the WHERE clause of its INSERT. The LQA write path currently
rejects MERGEs whose WHERE is non-trivial.

**Fix**: Extend `op_to_where_parts_with_bnodes()` to traverse the full Op tree up to the
MERGE's inner, not just the immediate inner. The MATCH context (below the MERGE) supplies
the WHERE parts; the bnode_map from outer CREATEs is already passed through. Ensure
`from_is_constrained` / `to_is_constrained` detection correctly finds the MATCH-bound
variable in the accumulated WHERE parts.

**Files**: `src/lqa/write.rs` (`compile_merge_rel_with_props`, `op_to_where_parts_with_bnodes`).

### 2.3 `write_set_replace_or_merge_map` (12 events)

**Construct**: `SET n = {name: 'x', age: 42}` (replace-map) and `SET n += {age: 43}` (merge-map).

**Current block**: `compile_set_items()` does not implement map-assignment `SetItem::Map`.

**Fix**: 
- **Replace** (`SET n = map`): DELETE all current properties of `n`, then INSERT the
  map entries. Emit `DELETE { ?n ?p ?v } WHERE { ?n ?p ?v . FILTER(?p != <__node> && ?p != rdf:type) }` followed by individual `INSERT DATA` for each map key.
- **Merge** (`SET n += map`): Only update/insert the specified keys. For each key `k`
  with value `v`, emit `DELETE { ?n <:k> ?old } WHERE { ?n <:k> ?old } INSERT { ?n <:k> v }`.
  If the map is a literal, this can be folded into a single multi-DELETE-INSERT.

**Files**: `src/lqa/write.rs` (add `compile_set_map_replace()` and `compile_set_map_merge()`).

### 2.4 `Exists` expression (9 events)

**Construct**: `WHERE EXISTS { (a)-[:KNOWS]->(b) }` — a full subpattern existential.

**Current block**: `sparql.rs` has no case for `Expr::Exists { .. }`.

**Fix**: Lower `Expr::Exists { pattern }` to a SPARQL `FILTER EXISTS { ... }` inline
expression. The subpattern is itself an LQA op subtree; call `op_to_where_parts()` on
it and wrap the result. This is pure L1 — no runtime evaluation needed.

**Files**: `src/lqa/sparql.rs` (`lower_expr` match arm for `Expr::Exists`).

### 2.5 `write_set_complex_expr` and `write_delete_complex_expr` (10 events)

**Construct**: `SET n.score = n.score * 1.1 + bonus` or `DELETE CASE … END`.

**Current block**: `expr_to_sparql_lit()` in `write.rs` only handles simple literals and
variable references; arithmetic expressions are rejected.

**Fix**: Call `lower_expr()` from `sparql.rs` to produce a SPARQL expression string, then
use it inline in the SET INSERT triple or DELETE WHERE filter. The expression evaluator
in `sparql.rs` already handles arithmetic; it just needs to be reachable from `write.rs`.
Extract a shared `expr_to_sparql_string(expr, base) -> Option<String>` helper usable in
both paths.

**Files**: `src/lqa/write.rs` (call shared helper), `src/lqa/sparql.rs` (expose helper).

### 2.6 `varlen_named_relvar` expressions: `Subscript`, `head()`, `list ordering` (10 events)

**Construct**: `list[0]`, `head(list)`, `ORDER BY list_column`.

**Fix**:
- `Subscript`: In `lower_expr`, emit `SPARQL_LIST_SUBSCRIPT` custom function call, or
  if the list is a serialised JSON-array string (our encoding), use `STRBEFORE`/`STRAFTER`
  string splitting. More robustly: add `Op::Subscript { list, index }` to the Op enum
  and emit a SPARQL inline expression using regex-based element extraction on the
  serialised list string.
- `head()`: `head(list)` = `list[0]`. Same implementation as Subscript with index 0.
- List ordering comparison: Extend `lower_expr` for `CmpOp::Lt` etc. when both
  operands are lists, using the `cypher_compare` type-rank encoding already present
  in `sparql.rs`.

**Files**: `src/lqa/sparql.rs`.

### 2.7 Remaining L1 write constructs (12 events)

- **`write_merge_rel_unbound_nodes`** (2): Handle MERGE relationships where one endpoint
  is not in any WHERE clause — require both endpoints to be in scope or emit a targeted
  error with a clear message.
- **`write_delete_rel_undirected_untyped`** (2): `DELETE (a)--(b)` — emit both
  directions: `DELETE { ?a ?p ?b } WHERE { ?a ?p ?b } UNION { ?b ?p ?a }`.
- **Aggregate in non-standard position** (3): `count()` / `collect()` outside RETURN
  in WITH — route to a GROUP BY subquery in LQA.
- **List/map equality with null** (3): Three remaining dynamic cases — extend
  `tri_bool_eq` to handle variable list operands by emitting `IF(BOUND(?x), ..., false)`.
- **`head()`** (2): covered above (2.6).

---

## Phase 3 — L2 Continuation extensions  (~276 events → 0 or Continuation)

These constructs require the `TranspileOutput::Continuation` multi-phase API (already
defined in `src/runtime.rs` and `src/lib.rs`). Each emitter is a separate PR.

The strategy is: for each construct below, if a `Continuation` emitter exists it
produces the correct multi-phase result. If no emitter exists yet, return
`Err(Unsupported)` from the LQA path — **do not fall through to legacy**. Once all
L2 emitters are implemented or all remaining L2 constructs return proper errors, the
legacy fallback can be removed even if some constructs still return `Err(Unsupported)`
to the caller.

### 3.1 Quantifier on runtime list (97 events) — `none/single/any/all` on `nodes(p)` / `relationships(p)`

**Continuation shape**: The varlen path is evaluated in Phase 1, returning the sequence
of intermediate nodes/relationships. Phase 2 evaluates the quantifier predicate over that
sequence in Rust and returns a boolean `VALUES` block for the outer query.

This is the highest-value single target (97 events, all currently failing in TCK).

**Design**: The path decomposition L2 emitter (already planned in
`plans/l2-runtime-support.md §3.1`) drives this. Once path decomposition materialises
`nodes(p)` / `relationships(p)` as a Rust `Vec<Term>`, the quantifier loop is trivial.

### 3.2 List comprehension on variable list (94 events)

**Continuation shape**: Phase 1 evaluates the list source (`collect()`, named path, etc.)
and materialises it. Phase 2 maps the comprehension expression over each element in Rust,
building the result list. Phase 3 returns the result via a `VALUES` block.

The existing `try_list_comp_projection_continuation()` emitter handles the simple
`[x IN literal_list | expr]` case; extend it to variable-list sources.

### 3.3 UNWIND with variable/expression list (58 events combined)

**Continuation shape**: Phase 1 materialises the list variable (from `collect()`, runtime
property access, or list expression). Phase 2 UNWINDs over it using a `VALUES` block,
one row per element.

### 3.4 Path value in projection (17 events)

**Continuation shape**: Named path variables (`p` in `MATCH p = (a)-->(b)`) must be
serialised into our `<path-string>` encoding. Phase 1 extracts the path components (nodes
and edges) from the SPARQL result using a CONSTRUCT-like query or by decomposing the
property-path result. Phase 2 assembles the string encoding.

### 3.5 PatternComprehension (13 events)

**Continuation shape**: `[(a)-[:KNOWS]->(b) | b.name]` — evaluate the pattern as a
subquery in Phase 1, collect matching `b.name` values, return as a list literal in
Phase 2.

### 3.6 Dynamic list concatenation (8 events), `collect()` in WITH (6 events), `labels()` (2 events)

Lower-priority L2 items. Each follows the materialise-then-fold Continuation shape.

---

## Phase 4 — Final cleanup and deletion

Once Phases 1–3 drive all legacy fallback events to zero (verified by
`POLYGRAPH_TRACE_LEGACY=1 cargo test --test tck` producing no `[LEGACY]` lines):

### 4.1 Remove fallback branches in `src/lib.rs`

The try-LQA-then-legacy routing in `cypher_to_sparql()` (`src/lib.rs` lines ~59–76) and
`cypher_to_sparql_update()` (lines ~120+) currently falls back on `Err(Unsupported)`.
Change the fallback arm to return the error to the caller instead:

```rust
// Before
Err(PolygraphError::Unsupported { .. }) => {
    translator::cypher::translate(&ast, ...) // legacy
}

// After
Err(e) => return Err(e),
```

**Do this one function at a time, running the full TCK after each change.**

### 4.2 Remove `translate_skip_writes` call in write path

The SELECT-after-write path in `cypher_to_sparql_update()` currently calls
`translator::cypher::translate_skip_writes()`. Replace with the LQA
`strip_writes_with_bnodes` / `compile_output` path unconditionally.

### 4.3 Delete `src/translator/`

```bash
git rm -r crates/polygraph/src/translator/
```

Remove the `mod translator;` declaration in `src/lib.rs`. Remove the `translator::`
imports. Run `cargo build` — fix any remaining compile errors (there should be none
if Phases 1–4.2 are complete).

### 4.4 Clean up `src/lib.rs`

- Remove `POLYGRAPH_TRACE_LEGACY` env-var branches.
- Remove the `is_lqa_safe()` pre-flight function entirely (or keep it as a strict
  validator that returns errors instead of triggering fallback).
- Remove the `has_return` / `use_lqa_select` split if no longer needed.

### 4.5 Update public API docs and CHANGELOG

Bump to `v1.0.0-rc.1`. Update `README.md` to remove the legacy-fallback disclaimer.
Add a `CHANGELOG.md` entry noting the deletion of the legacy translator.

---

## Milestone schedule (estimate)

| Phase | Effort | Unlocks | Validation |
|-------|--------|---------|------------|
| **1** — Safety pre-pass | 2–3 days | 31 fallback events → LQA | `cargo test --test tck` no regression |
| **2** — L1 expressions/writes | 1–2 weeks | ~95 fallback events eliminated | Same |
| **3.1** — Quantifier L2 | 1–2 weeks | 97 events → Continuation | Same + quantifier scenarios pass |
| **3.2** — List comprehension L2 | 1–2 weeks | 94 events → Continuation | List12[1,2] pass |
| **3.3** — UNWIND L2 | 1 week | 58 events → Continuation | |
| **3.4–3.6** — Path/Pattern/misc L2 | 2–3 weeks | 38 events → Continuation | |
| **4** — Delete legacy | 1–2 days | 19 k lines deleted | Full TCK green, no `[LEGACY]` |

**Total**: approximately 8–12 weeks of focused engineering. Each phase is independently
shippable and does not require all of the next phase to be complete before merging.

---

## Invariants during migration

1. **No TCK regression** — every PR must leave `cargo test --test tck` at ≥ 3820
   passing scenarios.
2. **`POLYGRAPH_TRACE_LEGACY=1`** must show a non-increasing event count after each
   merged phase.
3. **No new `panic!` in library code** — `Err(Unsupported)` must be returned cleanly
   for any construct that would previously have fallen back to legacy.
4. **L3 constructs return `Err`**, not wrong results — the 3 permanent multigraph
   failures (Match5[27], Match6[14], Merge5[3]) must produce
   `Err(PolygraphError::Unsupported { .. })` once legacy is gone, not incorrect SPARQL.
5. **GQL path** (`gql.rs`, 26 lines) is deleted at the same time as the Cypher legacy.
   The GQL surface already routes through LQA; the legacy GQL file contains only a thin
   wrapper that can be removed or replaced with a direct LQA call.
