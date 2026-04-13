# Full openCypher TCK Integration Plan

**Current state**: 480 scenarios from 4 clause categories (12.7% of full TCK)
**Target state**: 3,650 scenarios from all 37 categories (100% of full TCK)
**Gap**: 3,170 new scenarios across 33 missing categories + 196 feature files

**Target SPARQL dialect**: SPARQL-star / SPARQL 1.2 (not limited to SPARQL 1.1).
Oxigraph 0.4 supports RDF-star natively. The `TargetEngine` trait and `rdf_mapping::rdf_star`
module are already wired for annotated triples. SPARQL 1.2 adds `TRIPLE()`, `SUBJECT()`,
`PREDICATE()`, `OBJECT()`, and `isTRIPLE()` functions on triple terms, which directly
enable `type(r)` extraction and relationship-as-value scenarios.

---

## Coverage Inventory

### What we have today (4 categories, 24 feature files)

| Category              | Scenarios | Pass | Fail | Notes                   |
|-----------------------|-----------|------|------|-------------------------|
| clauses/match         | 369       | ~280 | ~89  | Core read queries       |
| clauses/match-where   | 34        | ~30  | ~4   | WHERE predicates         |
| clauses/return        | 63        | ~42  | ~21  | Projections, aliases     |
| clauses/unwind        | 14        | ~10  | ~4   | Literal list UNWIND      |
| **Subtotal**          | **480**   | **362** | **118** | **75.4%**           |

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
| expressions/graph                   | 59        | Medium       | C     |
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

## Phased Delivery

### Phase A — Low-Hanging Fruit (572 scenarios)

**Scope**: Categories that test features already implemented in Phases 2–4 (WITH, ORDER BY, SKIP/LIMIT, UNION, literals, boolean expressions). Primarily tests the existing translator against new inputs.

**Categories**: return-orderby (35), return-skip-limit (31), with (29), with-skip-limit (9), with-where (19), with-orderBy (237), union (12), expressions/literals (131), expressions/boolean (150)

**Work required**:
1. **Vendorize feature files**: Copy 42 `.feature` files from upstream into `tests/tck/features/`
2. **Step definition gaps**: None expected — existing Given/When/Then steps cover these
3. **Grammar fixes**: Likely a handful of parse failures for edge-case syntax:
   - `CASE WHEN ... THEN ... ELSE ... END` expressions (may parse as unknown)
   - Null literal checks (`IS NULL`, `IS NOT NULL`)
   - `NOT` prefix operator in boolean expressions
   - `UNION` / `UNION ALL` clause (grammar exists but verify translation)
   - WITH + aggregation combos (`WITH count(*) AS c`)
4. **Translator fixes**: Mostly covered; fix any edge cases surfaced by new scenarios
5. **Harness additions**:
   - Add step for `And having executed:` that contains multiple CREATE statements separated by commas
   - Handle `Scenario Outline:` + `Examples:` tables (cucumber crate handles this natively)

**Expected pass rate**: ~80% immediately on copy, ~90% after grammar/translator fixes

**Effort**: Small — primarily vendorize + triage failures

---

### Phase B — Expression Engine (558 scenarios)

**Scope**: Expression types that need grammar additions and translator mappings but are conceptually straightforward SPARQL 1.1 equivalents.

**Categories**: comparison (65), null (44), mathematical (6), precedence (93), string (32), aggregation (31), conditional (13), typeConversion (47), list (177), map (40), countingSubgraphMatches (11)

**Work required**:

1. **Grammar**: Add missing expression atoms to `grammars/cypher.pest`:
   ```pest
   function_call = { ident ~ "(" ~ (kw_DISTINCT? ~ expression ~ ("," ~ expression)*)? ~ ")" }
   case_expression = { kw_CASE ~ (expression)? ~ (kw_WHEN ~ expression ~ kw_THEN ~ expression)+ ~ (kw_ELSE ~ expression)? ~ kw_END }
   list_comprehension = { "[" ~ ident ~ kw_IN ~ expression ~ (kw_WHERE ~ expression)? ~ "|" ~ expression ~ "]" }
   map_literal = { "{" ~ (ident ~ ":" ~ expression ~ ("," ~ ident ~ ":" ~ expression)*)? ~ "}" }
   ```

2. **AST node types**: Add to `ast/cypher.rs`:
   - `Expression::FunctionCall { name, distinct, args }`
   - `Expression::CaseExpression { operand, whens, else_expr }`
   - `Expression::ListComprehension { var, source, filter, projection }`
   - `Expression::MapLiteral { entries }`
   - `Expression::IsNull { expr, negated }`

3. **Translator mappings** in `translator/cypher.rs`:

   | Cypher | SPARQL 1.1 |
   |--------|-----------|
   | `toString(x)` | `STR(x)` |
   | `toInteger(x)` | `xsd:integer(x)` |
   | `toFloat(x)` | `xsd:double(x)` |
   | `toBoolean(x)` | `xsd:boolean(x)` |
   | `abs(x)` | `ABS(x)` |
   | `ceil(x)` | `CEIL(x)` |
   | `floor(x)` | `FLOOR(x)` |
   | `round(x)` | `ROUND(x)` |
   | `sqrt(x)` | — (no SPARQL equivalent; compute or error) |
   | `sign(x)` | `IF(x > 0, 1, IF(x < 0, -1, 0))` |
   | `rand()` | `RAND()` |
   | `left(s, n)` | `SUBSTR(s, 1, n)` |
   | `right(s, n)` | `SUBSTR(s, STRLEN(s) - n + 1, n)` |
   | `trim(s)` | — (concat of REPLACE patterns or custom) |
   | `ltrim(s)` / `rtrim(s)` | REPLACE with regex |
   | `toUpper(s)` | `UCASE(s)` |
   | `toLower(s)` | `LCASE(s)` |
   | `replace(s, from, to)` | `REPLACE(s, from, to)` |
   | `substring(s, start, len?)` | `SUBSTR(s, start+1, len)` (0-indexed → 1-indexed) |
   | `size(list)` | — (count members if literal, or use subquery) |
   | `size(string)` | `STRLEN(s)` |
   | `reverse(s)` | — (no SPARQL equivalent) |
   | `split(s, delim)` | — (no SPARQL equivalent) |
   | `STARTS WITH` | `STRSTARTS(s, prefix)` |
   | `ENDS WITH` | `STRENDS(s, suffix)` |
   | `CONTAINS` | `CONTAINS(s, substr)` |
   | `=~` (regex match) | `REGEX(s, pattern)` |
   | `x IS NULL` | `!BOUND(x)` |
   | `x IS NOT NULL` | `BOUND(x)` |
   | `CASE WHEN` | nested `IF()` expressions |
   | `x IN [list]` | `x IN (a, b, c)` (already done for literals) |
   | `coalesce(a, b)` | `COALESCE(a, b)` |
   | `x ^ y` (power) | — (no standard SPARQL equivalent; could use nested multiply for integer powers, or mark unsupported) |
   | `x % y` (modulo) | `?x - FLOOR(?x / ?y) * ?y` (derived from arithmetic) |

4. **List & map structural operations**: These are the hardest in this phase
   - List indexing `list[0]` — no SPARQL equivalent for runtime lists
   - List slicing `list[1..3]` — similar limitation
   - Map property access `map.key` — rewrite to projected variable
   - Known limitation: lists are not first-class values in SPARQL (unaffected by SPARQL-star)

5. **Vendorize**: Copy 64 `.feature` files

**Expected pass rate**: ~60% immediately, ~75% after expression engine work

**Effort**: Medium — 2-3 weeks of grammar + translator work

---

### Phase C — Advanced Features (670 scenarios)

**Scope**: Features that require significant new translator capabilities or have no direct SPARQL 1.1 equivalent.

**Categories**: call (50), graph (59), pattern (49), existentialSubqueries (10), path (7), quantifier (545), triadicSelection (19)

**Work required**:

1. **Graph functions** (59 scenarios):
   - `type(r)` → With SPARQL-star: if `r` is bound to annotated triple `<< ?s ?pred ?o >>`, extract `?pred` via pattern matching or `PREDICATE(?r_triple)` (SPARQL 1.2). Then `STRAFTER(STR(?pred), BASE)` to get the local name. **Now feasible.**
   - `labels(n)` → `SELECT ?label WHERE { ?n a ?label }` subquery
   - `id(n)` → use IRI or blank node identifier as the id
   - `properties(n)` → With SPARQL-star: `SELECT ?prop ?val WHERE { << ?n ?_ ?__ >> ?prop ?val }` to find all annotated properties on edges involving `n`. For node properties: `?n ?prop ?val`. **Partially feasible.**
   - `keys(n)` → similar to properties, project `?prop` only
   - `nodes(p)`, `relationships(p)` → For bounded paths (unrolled to explicit chains): intermediate variables are available. For unbounded `*`/`+` paths: still no SPARQL mechanism to enumerate intermediate path nodes. **Partially feasible for bounded paths only.**

2. **Pattern expressions** (49 scenarios):
   - `EXISTS { (a)-[:REL]->(b) }` → SPARQL `EXISTS { }` or `FILTER EXISTS { }`
   - Pattern predicates in WHERE → `FILTER EXISTS { ... }`
   - `NOT EXISTS` → `FILTER NOT EXISTS { }`

3. **Existential subqueries** (10 scenarios):
   - `EXISTS { MATCH (n)-->(m) WHERE m.age > 25 }` → SPARQL `EXISTS` block

4. **Quantifier expressions** (545 scenarios — 14.9% of full TCK!):
   - `all(x IN list WHERE predicate)` — requires universal check
   - `any(x IN list WHERE predicate)` — requires existential check
   - `none(x IN list WHERE predicate)` — requires negated existential
   - `single(x IN list WHERE predicate)` — requires count == 1
   - **SPARQL mapping**: For literal lists, can unroll at compile time. For runtime lists from query results, fundamentally limited
   - **Strategy**: Support compile-time unrolling for literal lists; emit `FILTER NOT EXISTS` / `FILTER EXISTS` patterns for bound variables where possible; mark dynamic list quantifiers as unsupported

5. **Path functions** (7 scenarios):
   - `length(p)` → count hops in property path result
   - `shortestPath()` → no SPARQL 1.1 equivalent

6. **Procedure calls** (50 scenarios):
   - All scenarios use procedures that don't exist in our SPARQL backend
   - Strategy: Parse and recognize `CALL db.labels()` etc., return UnsupportedFeature for unknown procedures
   - Count these as "correctly rejected"

7. **Triadic selection** (19 scenarios):
   - Complex multi-hop patterns — should be testable with existing translator if graph functions work

**Expected pass rate**: ~40% after implementation (quantifiers and path decomposition are fundamentally hard)

**Effort**: Large — 4-6 weeks

---

### Phase D — Write Operations & Temporal (1,370 scenarios)

**Scope**: Features that either require SPARQL Update or temporal type support, both beyond current project scope.

**Categories**: create (78), delete (41), merge (75), remove (33), set (53), temporal (939)

**Work required**:

1. **Write clauses** (280 scenarios):
   - `CREATE` → `INSERT DATA { }` (basic form already works for TCK setup)
   - `DELETE` → `DELETE DATA { }` or `DELETE WHERE { }`
   - `SET` → `DELETE { old } INSERT { new } WHERE { }`
   - `REMOVE` → `DELETE DATA { }` for labels/properties
   - `MERGE` → `INSERT { } WHERE { NOT EXISTS { } }` pattern
   - **Blocker**: The translator currently returns `UnsupportedFeature` for write clauses. Must implement `cypher_to_sparql_update()` API
   - **Step definition change**: Write scenarios use `When executing query:` + `Then the side effects should be:` — must validate graph mutations against the Oxigraph store

2. **Temporal** (939 scenarios — 25.7% of full TCK!):
   - Date, Time, DateTime, Duration, LocalTime, LocalDateTime
   - `date()`, `time()`, `datetime()`, `duration()` constructors
   - Temporal arithmetic: `date + duration`, comparison
   - Temporal accessors: `.year`, `.month`, `.day`, `.hour`, etc.
   - **SPARQL mapping**: `xsd:date`, `xsd:dateTime`, `xsd:time`, `xsd:duration` exist but accessors (`YEAR()`, `MONTH()`, `DAY()`, `HOURS()`, `MINUTES()`, `SECONDS()`) only partially cover Cypher's temporal model
   - **Biggest gap**: Duration arithmetic, LocalTime/LocalDateTime (no xsd equivalent), temporal truncation (`date.truncate('month')`)

**Expected pass rate**: ~50% for write ops (basic CRUD), ~30% for temporal (type mapping mismatch)

**Effort**: Very large — 6-10 weeks

---

## Implementation Steps

### Step 0 — Automate TCK Vendorization

Replace manual file copying with a reproducible script.

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

### Step 1 — Vendorize All Feature Files & Establish Baseline

1. Run `scripts/vendor-tck.sh` to pull all 220 feature files
2. Run `cargo test --test tck 2>&1 | tail -5` — count pass/fail/skip
3. Record baseline numbers before any code changes
4. Commit feature files with counts in the commit message

### Step 2 — Harden the Test Harness

The current harness must handle new step patterns from the full TCK:

| New step pattern | Frequency | Action |
|-----------------|-----------|--------|
| `Given there exists a procedure ...` | ~50 | Return UnsupportedFeature or mock |
| `And no side effects` | Many | No-op (already handled) |
| `And the side effects should be:` | Write scenarios | Validate graph mutations |
| `And having executed: CREATE ...` with relationships having properties | Many | Extend `create_to_insert_data()` to emit RDF-star annotated triples for relationship properties |
| `Then a TypeError should be raised at ...` | Expression tests | Assert `query_error` is set |
| `Then a ArgumentError should be raised ...` | Expression tests | Assert `query_error` is set |

**Key harness changes**:
- Extend error assertion regex to match more error types (`TypeError`, `ArgumentError`, `SemanticError`)
- Add `Given there exists a procedure` step that registers mock procedures or sets skip flag
- Improve `create_to_insert_data()` to handle relationship properties (needed for many expression tests)
- Add result comparison for boolean/null/numeric values with type-aware equality

### Step 3 — Phase A Implementation

1. Verify existing pass rate on new feature files
2. Fix parse failures one grammar rule at a time
3. Run `cargo test --test tck` after each fix to measure delta
4. Target: 480 + 572 = 1,052 scenarios, ≥85% pass rate

### Step 4 — Phase B Implementation

1. Implement function call grammar + AST + translator
2. Implement CASE, list comprehension, map literal
3. Implement string/type conversion function mappings
4. Target: 1,610 scenarios, ≥70% pass rate

### Step 5 — Phase C Implementation

1. Implement graph functions (type, labels, id)
2. Implement pattern expressions → EXISTS
3. Implement quantifier compile-time unrolling
4. Target: 2,280 scenarios, ≥55% pass rate

### Step 6 — Phase D Implementation

1. Implement SPARQL Update translation for write clauses
2. Implement temporal type mappings
3. Target: 3,650 scenarios, ≥45% pass rate

---

## Pass Rate Projections

| Milestone | Scenarios | Projected Pass | Projected % | Cumulative Pass |
|-----------|-----------|---------------|-------------|-----------------|
| Baseline (today) | 480 | 362 | 75.4% | 362 |
| After Step 1 (vendorize only) | 3,650 | ~500 | ~13.7% | ~500 |
| After Phase A | 3,650 | ~850 | ~23.3% | ~850 |
| After Phase B | 3,650 | ~1,300 | ~35.6% | ~1,300 |
| After Phase C | 3,650 | ~1,650 | ~45.2% | ~1,650 |
| After Phase D | 3,650 | ~2,200 | ~60.3% | ~2,200 |

### Theoretical ceiling with SPARQL-star / SPARQL 1.2

The project targets SPARQL-star (widely supported: Oxigraph, Jena, RDF4J, GraphDB)
and can leverage SPARQL 1.2 triple-term functions where available. This resolves
relationship property and `type(r)` scenarios that were previously blocked under
SPARQL 1.1, but does **not** help with the two largest blocker categories
(temporal and quantifier).

| Category | Scenarios | Status with SPARQL-star |
|----------|-----------|------------------------|
| Temporal arithmetic/accessors | ~600 | **Still blocked** — Cypher `Duration`, `LocalDateTime`, `date.truncate()` have no RDF/SPARQL equivalent regardless of version |
| Runtime list operations | ~200 | **Still blocked** — lists aren't first-class in SPARQL; `list[0]`, slicing, `size(runtime_list)` remain impossible |
| `type(r)` / relationship properties | ~0 | **Resolved** — annotated triples + `PREDICATE()` or pattern matching |
| Path decomposition (unbounded) | ~30 | **Still blocked** — `nodes(p)` on `*`/`+` paths; SPARQL property paths don't expose intermediates |
| Path decomposition (bounded) | ~20 | **Resolved** — unrolled chains expose intermediate variables |
| Dynamic property access (`n[expr]`) | ~30 | **Still blocked** — requires computed predicate at runtime |
| Procedure call side effects | ~40 | **Still blocked** — no SPARQL procedure framework |
| Variable UNWIND from collect() | ~10 | **Still blocked** — requires runtime list iteration |
| **Total untranslatable** | **~910** | |
| **Theoretical ceiling** | **~2,740** | **~75.1%** |

The delta vs. SPARQL 1.1-only is modest (~+80 scenarios) because **temporal (939)**
and **quantifier (545)** together account for 40.6% of the full TCK and are
completely unaffected by SPARQL-star.

To exceed ~75%, the project would need:
- Engine-specific temporal function extensions (e.g., Jena ARQ's `afn:` functions)
- An RDF-based list encoding with custom unrolling functions
- A hybrid execution model (partial Cypher evaluation + SPARQL)

---

## Tracking & Reporting

### Automated compliance report

Add a CI step that produces a per-category breakdown:

```bash
# scripts/tck-report.sh
cargo test --test tck 2>&1 | python3 scripts/parse_tck_results.py
```

Output format:
```
Category                          Total  Pass  Fail  Skip    %
clauses/match                       369   280    62    27  75.9%
clauses/match-where                  34    30     4     0  88.2%
expressions/boolean                 150    --    --    --    NEW
...
TOTAL                              3650  1300  1420   930  35.6%
```

### ROADMAP.md update format

Update the compliance tracker table after each phase:

```markdown
| Release | Pass | Fail | Skip | Total | % |
|---------|------|------|------|-------|---|
| 0.1.0   | 362  | 118  | 0    | 480   | 75.4% (4/37 categories) |
| 0.2.0   | 850  | 600  | 2200 | 3650  | 23.3% (full TCK) |
```

### Per-category skip annotations

For categories with known SPARQL 1.1 limitations, add a `@skip` or `@known-limitation` tag in the feature files (or use cucumber tag filtering) so that untranslatable scenarios are tracked separately from bugs.

---

## Summary

We currently include **4 out of 37** TCK categories (480 / 3,650 scenarios = 12.7%). The selection covered only core read clauses: MATCH, MATCH-WHERE, RETURN, and UNWIND.

**Missing entirely**:
- 13 clause categories (WITH variants, write ops, UNION, CALL)
- 18 expression categories (all of them — boolean, string, list, temporal, etc.)
- 2 use-case categories

The recommended approach is a 4-phase expansion (A→D) ordered by implementation difficulty, starting with vendorizing all feature files immediately to establish a full-TCK baseline. The theoretical ceiling with SPARQL-star is ~75.1% (~2,740 / 3,650) — the dominant blockers are temporal (939 scenarios, 25.7%) and quantifier expressions (545, 14.9%), neither of which benefit from RDF-star or SPARQL 1.2.
