# Full openCypher TCK Suite Expansion Plan

**Status**: in progress  
**Updated**: 2026-04-14

**Current state**: 461/463 (99.6%) across 4 clause categories — 12.7% of the full TCK  
**Target state**: ≥ 80% pass rate across all 3,650 scenarios from 37 categories  
**Gap**: 3,170 new scenarios across 33 missing categories + 196 feature files

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

---

## Compliance Tracker

| Release | Pass | Fail | Total | % | Notes |
|---------|------|------|-------|---|-------|
| dev     | 461  | 2    | 463   | 99.6% | 4-category subset only |
| Phase A | —    | —    | 1,035 | target ≥ 80% | after A categories vendorized |
| Phase B | —    | —    | 1,593 | target ≥ 75% | after B categories vendorized |
| Phase C | —    | —    | 2,263 | target ≥ 60% | after C categories vendorized |
| Phase D | —    | —    | 3,650 | target ≥ 80% | full suite |
