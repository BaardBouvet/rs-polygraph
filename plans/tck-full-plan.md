# Full openCypher TCK Suite Expansion Plan

**Status**: in progress  
**Updated**: 2026-04-23

**Current state**: 3431/3789 (90.6%) — all 36 planned categories vendored.  
**Target state (Phase 7)**: ≥ 80% across the full suite — **already exceeded**.  
**Remaining gap**: 163 failing + 195 skipped + 8 parse errors. The work below targets reducing this to < 100 failures while paying down translator debt.

## Assessment snapshot (2026-04-23)

Pareto of failing-step messages from the full TCK run:

| Count | Cause | Where to fix |
|------:|-------|--------------|
| 60 | `Unsupported feature: complex return expression` | `translate_return_item` (return projection of compound expressions) |
| 25 | `Unsupported feature: UNWIND of variable` | `translate_unwind_clause` (needs runtime list lowering or VALUES join) |
|  4 | `Unsupported feature: variable in UNWIND list literal` | same |
|  ~3 | `complex return / property access on non-variable / list comprehension` | expression engine recursion |
|  ~70 | semantic / row-count mismatches | scattered: Match4/Match6 (multigraph), Merge, ReturnOrderBy null lists, write side-effects |

A single fix for "complex return expression" likely converts ~60 failures at once. Combined with UNWIND-of-variable, this is **~90 wins from 2 changes** — by far the highest leverage on the board.

**Target SPARQL dialect**: SPARQL-star / SPARQL 1.2 (Oxigraph 0.4 supports RDF-star natively). SPARQL 1.2 adds `TRIPLE()`, `SUBJECT()`, `PREDICATE()`, `OBJECT()`, and `isTRIPLE()` functions, which directly enable `type(r)` extraction and relationship-as-value scenarios.

---

## Coverage Inventory

### What we have today (4 categories, 24 feature files)

| Category              | Scenarios | Pass | Fail | Notes                    |
|-----------------------|-----------|------|------|--------------------------|
| clauses/match         | 369       | 368  | 1    | Core read queries        |
| clauses/match-where   | 34        | 34   | 0    | WHERE predicates         |
| clauses/return        | 63        | 63   | 0    | Projections, aliases     |
| clauses/unwind        | 14        | 14   | 0    | Literal list UNWIND      |
| **Subtotal**          | **480**   | **461** | **2** | **99.6%**             |

### What the full TCK adds (33 new categories, 196 feature files)

| Category                            | Scenarios | Difficulty | Phase |
|-------------------------------------|-----------|------------|-------|
| **Clause categories**               |           |            |       |
| clauses/return-orderby              | 35        | Low        | A     |
| clauses/return-skip-limit           | 31        | Low        | A     |
| clauses/with                        | 29        | Low        | A     |
| clauses/with-skip-limit             | 9         | Low        | A     |
| clauses/with-where                  | 19        | Low        | A     |
| clauses/with-orderBy                | 237       | Low        | A     |
| clauses/union                       | 12        | Medium     | A     |
| clauses/call                        | 50        | Medium     | C     |
| clauses/create                      | 78        | High       | D     |
| clauses/delete                      | 41        | High       | D     |
| clauses/merge                       | 75        | High       | D     |
| clauses/remove                      | 33        | High       | D     |
| clauses/set                         | 53        | High       | D     |
| **Expression categories**           |           |            |       |
| expressions/literals                | 131       | Low        | A     |
| expressions/boolean                 | 150       | Low        | A     |
| expressions/comparison              | 65        | Low        | B     |
| expressions/null                    | 44        | Low        | B     |
| expressions/mathematical            | 6         | Low        | B     |
| expressions/precedence              | 93        | Medium     | B     |
| expressions/string                  | 32        | Medium     | B     |
| expressions/aggregation             | 31        | Medium     | B     |
| expressions/conditional             | 13        | Medium     | B     |
| expressions/typeConversion          | 47        | Medium     | B     |
| expressions/list                    | 177       | Medium     | B     |
| expressions/map                     | 40        | Medium     | B     |
| expressions/graph                   | 59        | Medium     | C     |
| expressions/pattern                 | 49        | Hard       | C     |
| expressions/existentialSubqueries   | 10        | Hard       | C     |
| expressions/path                    | 7         | Hard       | C     |
| expressions/quantifier              | 545       | Hard       | C     |
| expressions/temporal                | 939       | Very Hard  | D     |
| **Use case categories**             |           |            |       |
| useCases/countingSubgraphMatches    | 11        | Medium     | B     |
| useCases/triadicSelection           | 19        | Medium     | C     |
| **Subtotal (new)**                  | **3,170** |            |       |

---

## Phase A — Low-Hanging Fruit (572 scenarios)

**Status**: not started

**Scope**: Categories that test features already implemented in Phases 2–4 (WITH, ORDER BY, SKIP/LIMIT, UNION, literals, boolean expressions). Primarily tests the existing translator against new inputs.

**Categories**: return-orderby (35), return-skip-limit (31), with (29), with-skip-limit (9), with-where (19), with-orderBy (237), union (12), expressions/literals (131), expressions/boolean (150)

### Work required

1. **Vendor feature files**: Write `scripts/vendor-tck.sh` to clone opencypher/openCypher at a pinned ref and copy all 220 feature files into `tests/tck/features/`. Commit vendored files separately.
2. **Step definition gaps**: None expected — existing `Given/When/Then` steps cover these patterns.
3. **Grammar fixes** (likely handful of parse failures):
   - `CASE WHEN … THEN … ELSE … END` expressions
   - `IS NULL` / `IS NOT NULL` null checks
   - `NOT` prefix operator in boolean expressions
   - `UNION` / `UNION ALL` clause (grammar exists; verify translation round-trip)
   - `WITH count(*) AS c` aggregation combos
4. **Harness additions**:
   - Handle `Scenario Outline:` + `Examples:` tables (cucumber crate handles natively; verify step routing)
   - Add step for `And having executed:` with multiple comma-separated `CREATE` statements

**Target**: ≥ 90% pass rate on phase A categories after fixes.

---

## Phase B — Expression Engine (558 scenarios)

**Status**: not started

**Scope**: Expression types needing grammar additions and translator mappings, all with direct SPARQL 1.1 equivalents.

**Categories**: comparison (65), null (44), mathematical (6), precedence (93), string (32), aggregation (31), conditional (13), typeConversion (47), list (177), map (40), countingSubgraphMatches (11)

### Work required

1. **Grammar additions** to `grammars/cypher.pest`:
   ```pest
   function_call       = { ident ~ "(" ~ (kw_DISTINCT? ~ expression ~ ("," ~ expression)*)? ~ ")" }
   case_expression     = { kw_CASE ~ expression? ~ (kw_WHEN ~ expression ~ kw_THEN ~ expression)+ ~ (kw_ELSE ~ expression)? ~ kw_END }
   list_comprehension  = { "[" ~ ident ~ kw_IN ~ expression ~ (kw_WHERE ~ expression)? ~ "|" ~ expression ~ "]" }
   map_literal         = { "{" ~ (ident ~ ":" ~ expression ~ ("," ~ ident ~ ":" ~ expression)*)? ~ "}" }
   ```

2. **New AST nodes** in `ast/cypher.rs`:
   - `Expression::FunctionCall { name, distinct, args }`
   - `Expression::CaseExpression { operand, whens, else_expr }`
   - `Expression::ListComprehension { var, source, filter, projection }`
   - `Expression::MapLiteral { entries }`

3. **Translator mappings** in `translator/cypher.rs`:

   | Cypher | SPARQL |
   |--------|--------|
   | `toString(x)` | `STR(x)` |
   | `toInteger(x)` | `xsd:integer(x)` |
   | `toFloat(x)` | `xsd:double(x)` |
   | `toBoolean(x)` | `xsd:boolean(x)` |
   | `abs(x)` | `ABS(x)` |
   | `ceil(x)` | `CEIL(x)` |
   | `floor(x)` | `FLOOR(x)` |
   | `round(x)` | `ROUND(x)` |
   | `rand()` | `RAND()` |
   | `sign(x)` | `IF(x > 0, 1, IF(x < 0, -1, 0))` |
   | `sqrt(x)` | unsupported — no SPARQL equivalent |
   | `left(s, n)` | `SUBSTR(s, 1, n)` |
   | `right(s, n)` | `SUBSTR(s, STRLEN(s) - n + 1, n)` |
   | `trim(s)` | `REPLACE(s, "^\\s+\|\\s+$", "")` |
   | `ltrim(s)` / `rtrim(s)` | `REPLACE` with regex |
   | `toUpper(s)` | `UCASE(s)` |
   | `toLower(s)` | `LCASE(s)` |
   | `replace(s, f, t)` | `REPLACE(s, f, t)` |
   | `substring(s, start, len?)` | `SUBSTR(s, start+1, len)` (0-indexed → 1-indexed) |
   | `STARTS WITH` | `STRSTARTS(s, prefix)` |
   | `ENDS WITH` | `STRENDS(s, suffix)` |
   | `CONTAINS` | `CONTAINS(s, substr)` |
   | `=~` | `REGEX(s, pattern)` |
   | `x IS NULL` | `!BOUND(x)` |
   | `x IS NOT NULL` | `BOUND(x)` |
   | `CASE WHEN` | nested `IF()` |
   | `coalesce(a, b)` | `COALESCE(a, b)` |
   | `x ^ y` | unsupported — no SPARQL equivalent |
   | `x % y` | `?x - FLOOR(?x / ?y) * ?y` |
   | `size(string)` | `STRLEN(s)` |
   | `reverse(s)` | unsupported |
   | `split(s, delim)` | unsupported |

4. **Known structural limitations**:
   - List indexing `list[0]` / slicing `list[1..3]` — no SPARQL equivalent for runtime lists
   - Map property access `map.key` — rewrite to projected variable where possible

**Target**: ≥ 75% pass rate on phase B categories.

---

## Phase C — Advanced Features (670 scenarios)

**Status**: not started

**Scope**: Features requiring significant new translator capabilities or SPARQL 1.2.

**Categories**: call (50), graph (59), pattern (49), existentialSubqueries (10), path (7), quantifier (545), triadicSelection (19)

### Work required

1. **Graph functions** (59 scenarios):
   - `type(r)` → `PREDICATE(?r_triple)` then `STRAFTER(STR(?pred), BASE)` (SPARQL 1.2). **Feasible.**
   - `labels(n)` → `SELECT ?label WHERE { ?n a ?label }` subquery
   - `id(n)` → IRI or blank node identifier
   - `properties(n)` / `keys(n)` → property enumeration subquery
   - `nodes(p)` / `relationships(p)` → available for bounded (unrolled) paths only

2. **Pattern predicates** (49 scenarios):
   - `EXISTS { (a)-[:REL]->(b) }` → `FILTER EXISTS { … }`
   - `NOT EXISTS { … }` → `FILTER NOT EXISTS { … }`

3. **Existential subqueries** (10 scenarios):
   - `EXISTS { MATCH … WHERE … }` → SPARQL `EXISTS` block

4. **Quantifier expressions** (545 scenarios — 14.9% of full TCK):
   - `all(x IN list WHERE pred)`, `any`, `none`, `single`
   - For **compile-time literal lists**: unroll at translation time
   - For **runtime lists**: fundamentally unsupported as a static transpiler — emit `UnsupportedFeature`

5. **Path functions** (7 scenarios):
   - `length(p)` → hop count for bounded paths
   - `shortestPath()` → no SPARQL 1.1 equivalent; emit `UnsupportedFeature`

6. **Procedure calls** (50 scenarios):
   - Parse `CALL db.labels()` etc.; emit `UnsupportedFeature` for unknown procedures (counts as correctly rejected)

7. **Triadic selection** (19 scenarios):
   - Complex multi-hop patterns; should work with existing translator once graph functions land

**Target**: ≥ 40% pass rate on phase C categories (runtime-list quantifiers are a fundamental limit).

---

## Phase D — Write Operations & Temporal (1,370 scenarios)

**Status**: not started

**Scope**: Features requiring SPARQL Update or temporal type support.

**Categories**: create (78), delete (41), merge (75), remove (33), set (53), temporal (939)

### Work required

1. **Write clauses** (280 scenarios):
   - New public API: `cypher_to_sparql_update() -> Result<TranspileOutput, PolygraphError>`
   - `CREATE` → `INSERT DATA { }` with RDF-star annotated triples for relationship properties
   - `DELETE` → `DELETE DATA { }` / `DELETE WHERE { }`
   - `SET` / `REMOVE` → `DELETE { old } INSERT { new } WHERE { }`
   - `MERGE` → `INSERT { } WHERE { NOT EXISTS { } }` pattern
   - TCK harness: add `Then the side effects should be:` step validating graph mutations against Oxigraph store

2. **Temporal** (939 scenarios — 25.7% of full TCK):
   - Map constructors to `xsd:date`, `xsd:dateTime`, `xsd:time`, `xsd:duration`
   - Accessor functions: `YEAR()`, `MONTH()`, `DAY()`, `HOURS()`, `MINUTES()`, `SECONDS()`
   - **Known gaps**: `LocalTime` / `LocalDateTime` (no xsd equivalent), duration arithmetic, `date.truncate('month')`

**Target**: ≥ 50% for write ops, ≥ 30% for temporal.

---

## Compliance Tracker

See ROADMAP.md Phase 7 for the full per-release table. Latest: **3431/3789 (90.6%)**, 163 failed, 195 skipped, 8 parse errors.

---

## Phase E — Pareto Cleanup (next, ~90 wins from 2 fixes)

**Status**: planned

These are the highest-leverage gaps still open, ordered by impact.

1. **Complex return expression (60 failures)** — `translate_return_item` rejects compound expressions in `RETURN`. Trace the rejection (see `Unsupported feature: complex return expression (Phase 4+)`), determine which expression shapes are bailing, and route them through `translate_expr` instead of bailing. Expected: ~60 scenarios pass.
2. **`UNWIND` of variable / non-literal expression (29 failures combined)** — `translate_unwind_clause` only handles compile-time literal lists. For runtime lists known to be a list-typed variable bound by an upstream `WITH … AS` we can:
   - For literal lists tracked through `WITH`, propagate the list into the UNWIND site (extend the existing `try_resolve_to_items`).
   - For genuinely runtime lists, lower to a SPARQL subquery that joins on a list-element triple pattern (only works under specific upstream shapes; mark the rest as fundamental).
3. **List comprehension (1 failure)**, **property access on non-variable base** (1), and the lone non-literal UNWIND case — fix opportunistically while in the file.

After this phase, the failure mix is dominated by:
- Match4/Match6 multigraph & `[rs*]` runtime-path constraint (already documented as fundamental in `plans/fundamental-limitations.md`)
- Merge interleaving (write-side ordering)
- ReturnOrderBy null-list ordering edge cases
- Skipped Procedure/Call scenarios (intentional)

**Target**: ≤ 100 failing scenarios; ≥ 92% pass rate.

---

## Phase F — Translator Code-Health Refactor (parallel track)

**Status**: in progress  
**Updated**: 2026-04-23

### Current size breakdown

| Region | Lines | Lines (kept inline) |
|--------|------:|------:|
| Top-level free helpers (numeric/agg classification, semantics) | 64–1999 | ~1,940 |
| `impl TranslationState` (one block) | 2001–~11050 | ~9,050 |
| Free helpers — expression rewriting / const folding | ~11050–11900 | ~850 |
| Free helpers — temporal (constructors, JDN, durations, fmt) | ~11900–end | **~4,300** |

The five hottest methods inside the impl:

| Method | Approx lines | Action |
|--------|-------:|--------|
| `translate_clause_sequence` | ~1,755 | Split per-clause arms into private fns (`translate_with`, `translate_create_clause`, …) in sibling files. |
| `translate_function_call` | ~1,524 | Move to `translator/cypher/functions.rs`; dispatch table by family (string, numeric, list, map, temporal, predicate). |
| `temporal_prop_binds` | ~1,280 | Move to `translator/cypher/temporal.rs` with the existing temporal free helpers. |
| `translate_expr` | ~1,380 | Stays in core, but extract per-variant arms (case/list/map/coalesce). |
| `translate_relationship_pattern` | ~650 | Move to `translator/cypher/patterns.rs` alongside `emit_edge_triple` & path unrolling. |

### Actual module layout (delivered 2026-04-23)

**Status**: complete — all files are ≤ 1,753 lines, TCK unchanged at 3431/3789.

```
src/translator/cypher/
  mod.rs         (4,059 lines)  — TranslationState struct, translate_query/union,
                                   translate_expr, translate_unwind, translate_aggregate,
                                   apply_order_skip_limit, small helpers, include! stitching
  clauses.rs     (1,753 lines)  — translate_clause_sequence (main dispatch loop)
  patterns.rs    (1,549 lines)  — translate_match_clause, translate_pattern,
                                   translate_node_pattern_with_term,
                                   translate_relationship_pattern,
                                   emit_edge_triple, emit_bounded_path_union*
  functions.rs   (1,528 lines)  — translate_function_call (150+ function mappings)
  semantics.rs   (1,554 lines)  — validate_semantics, segment_columns, classification helpers
  temporal.rs    (3,343 lines)  — TcComponents, all temporal constructors, JDN math,
                                   date/time/duration arithmetic, parse/format utils
  rewrite.rs       (826 lines)  — eliminate_collect_unwind, substitute_var_in_expr,
                                   const folding, SPARQL utility helpers
  return_proj.rs   (593 lines)  — translate_return_clause, translate_return_item
```

**Technique**: inner `impl TranslationState { }` blocks extracted as `include!("xxx.rs")` at module level — zero visibility overhead, zero API change, zero test churn. Free-function blocks use the same `include!` technique.

### Pre-refactor cleanups (completed 2026-04-23)

- ✅ Deleted dead helpers: `make_bool_op_is_null`, `extract_tz_offset_s`, `temporal_prop_expr` (−1,038 lines)
- ✅ Removed unreachable `Clause::Set(_)` arm in validate_semantics
- ✅ Suppressed all 35 warnings: `#[allow(non_snake_case)]` on temporal_prop_binds, prefix unused locals with `_`
- ✅ 0 lib warnings, 0 errors

### Acceptance criteria — status

- ✅ No file exceeds 2,000 lines (largest: clauses.rs 1,753 lines)
- ✅ TCK pass count 3431/3789 unchanged across all refactor commits
- ✅ `cargo build --lib` produces zero warnings
- `translate_function_call` dispatch table reorganization: deferred to future PR

### Remaining refactor opportunities (not blocking)

- `temporal.rs` (3,343 lines) can be split further: `temporal/construct.rs` (constructors), `temporal/arithmetic.rs` (Temporal8 duration math), `temporal/parse.rs` (string parsing). Same `include!` technique.
- `mod.rs` (4,059 lines) still contains `translate_expr` (~1,380 lines) + `temporal_prop_binds` + all impl machinery. Could extract to `expr.rs`.
- Long-term: replace `include!` with proper Rust modules once the team is ready to annotate all cross-module types with `pub(super)`.

---

## Vendorization Script

```bash
#!/usr/bin/env bash
# scripts/vendor-tck.sh
set -euo pipefail
TCK_REF="${1:-master}"
TCK_REPO="https://github.com/opencypher/openCypher"
TARGET="tests/tck/features"

rm -rf "$TARGET"
mkdir -p "$TARGET"

TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT
git clone --depth 1 --branch "$TCK_REF" "$TCK_REPO" "$TMPDIR/oc"
cp -r "$TMPDIR/oc/tck/features/"* "$TARGET/"

echo "Vendorized $(find "$TARGET" -name '*.feature' | wc -l) feature files"
echo "$(grep -r 'Scenario:' "$TARGET" --include='*.feature' | wc -l) scenario definitions"
```
