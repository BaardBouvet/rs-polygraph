# Remaining TCK Work — Phased Plan

**Status**: in progress
**Updated**: 2025-11-19

This plan enumerates every remaining gap in the openCypher TCK suite and groups
them into ordered tiers by ROI (scenarios recovered ÷ engineering effort).

---

## 1. Baseline

### Original baseline (pre-Tier-A)

| Bucket            | Count | % of 3789 |
|-------------------|------:|----------:|
| **Passing**       | 3558  | 93.9 %    |
| Failing           |   83  |  2.2 %    |
| Skipped           |  148  |  3.9 %    |
| Parse errors      |    8  |  0.2 %    |

### Current baseline (after Tier-A)

| Bucket            | Count | % of 3789 |
|-------------------|------:|----------:|
| **Passing**       | 3704  | 97.8 %    |
| Failing           |   84  |  2.2 %    |
| Skipped           |    1  |  0.0 %    |
| Parse errors      |    8  |  0.2 %    |

**Net change**: +146 passes, −147 skips, +1 failure (Delete1[7] correctly fails — can't statically detect ConstraintVerificationFailed for connected-node deletion)

### Theoretical ceilings

| If we land …                          | Pass count | Pass rate |
|---------------------------------------|-----------:|----------:|
| Tier A only (skip-step plumbing)      | ~3700      | 97.7 %    |
| Tier A + B (translator wins)          | ~3720      | 98.2 %    |
| Tier A + B + C (Phase 4+ runtime)     | ~3760      | 99.2 %    |
| Tier A + B + C + D (DST/IANA tz, parse) | ~3780    | 99.7 %    |

The remaining ~10 scenarios (Tier E) are fundamental SPARQL limitations and
require the [L2 runtime](l2-runtime-support.md) approach.

---

## 2. Failure Inventory (83)

### 2.1 Bucket counts (by error message family)

| # | Bucket                                                    | Count |
|--:|-----------------------------------------------------------|------:|
| 1 | `Result set mismatch (sorted)` — wrong values              |    34 |
| 2 | `Phase 4+ complex return expression: <name>`               |    26 |
| 3 | `Row count mismatch …`                                     |    11 |
| 4 | `UNWIND of variable / non-literal / list literal`          |    10 |
| 5 | `list comprehension [x IN list WHERE pred \| expr]`        |     1 |
| 6 | `property access on non-variable base expression`          |     1 |
| 7 | `Expected a SyntaxError at compile time but translation succeeded` | 1 |
|   | **Total**                                                  | **83**|

### 2.2 By feature (top concentrations)

| Feature              | Failures | Root cause                                      |
|----------------------|---------:|-------------------------------------------------|
| Temporal8            |       17 | duration arithmetic: dur+dur, dur×num, mixed-sign|
| Temporal10           |        6 | DST timezone (no IANA tz database)              |
| Quantifier11         |        6 | complex L2 quantifier with path returns         |
| List12               |        6 | Phase 4+ complex list/comprehension projections |
| Merge5               |        5 | merge result-set discrepancies                  |
| WithOrderBy1, Pattern2 |   3 each | order-by stability, pattern semantics         |
| Quantifier1–4        |    2 each (8)| `relationships(p)` on var-length path        |
| Temporal2/3          |    2 each (4)| DST timezone                                 |
| Path2, Graph9, Pattern1, Match4, Merge1, ReturnOrderBy1, Precedence1 | 2 each (14) | path/properties/order edge cases |
| singletons           |        9 | misc                                            |

---

## 3. Skip Inventory (148)

### 3.1 By step type (root cause)

| Step text (truncated)                                         | Count | Cause                                                |
|---------------------------------------------------------------|------:|------------------------------------------------------|
| `Then the result should be empty`                              |    95 | After a write query (CREATE/MERGE/DELETE/SET) we run `update()`, which returns no rows; the cucumber step is unimplemented so the whole scenario short-circuits to *skip*. |
| `Then a TypeError should be raised at compile time: InvalidArgumentType` | 12 | No step definition for compile-time error assertions |
| `Then a TypeError should be raised at any time: <code>`        |    14 | No step definition for runtime type-error assertions |
| `Then a TypeError should be raised at any time: InvalidArgumentType` | 4 | Same as above (specific code)                        |
| `Given the binary-tree-1 graph`                                |    10 | Fixture not loaded                                   |
| `Given the binary-tree-2 graph`                                |     9 | Fixture not loaded                                   |
| `there exists a procedure …` (procedure stubs)                 |     5 | `procedure_stub_given` flips `world.skip = true`     |
| `Then a ProcedureError should be raised at compile time: ProcedureNotFound` | 2 | No step definition                              |
| `Then a ParameterMissing should be raised at compile time: MissingParameter` | 1 | No step definition + parameter passing             |
|                                                                | **152** | (some scenarios contribute to multiple buckets)  |

### 3.2 By feature (top concentrations)

| Feature             | Skips | Notes                                          |
|---------------------|------:|------------------------------------------------|
| TriadicSelection1   |    19 | binary-tree-1/2 fixtures                       |
| Temporal4           |    18 | write→empty-result assertions                  |
| List1               |    18 | TypeError assertions                           |
| Create2             |    14 | write→empty-result                             |
| Create3             |     9 | write→empty-result                             |
| Create1             |     8 | write→empty-result                             |
| Delete5             |     7 | write→empty-result                             |
| Map1, Graph6        |  6 ea | TypeError / write-empty                        |
| Merge6, Delete1, Create5, Call1 | 5 ea | write-empty / procedures                |

---

## 4. Parsing Errors (8)

| Feature                  | Cause                                        | Fixability |
|--------------------------|----------------------------------------------|------------|
| Match5.feature           | Gherkin parse failure (likely backtick / pipe in body)| Investigate cucumber-rs version |
| ExistentialSubqueries1   | Gherkin parse failure                        | Same       |
| Literals6                | Gherkin parse failure (escapes?)             | Same       |
| Pattern3 / Pattern4 / Pattern5 | Gherkin parse failure (3 files)        | Same       |
| Comparison2:123          | `Failed to resolve <= <rhs>` in scenario outline | Examples table needs `\<= ...` escape, or our handler |
| Quantifier7:80           | `Failed to resolve <= any(<operands>`        | Same as above |

The 6 hard parse failures need investigation of *which* cucumber-rs limitation
is being hit. The 2 `<= …>` resolution failures look like a literal-versus-
placeholder confusion in scenario-outline expansion — a bugfix in
`tests/tck/main.rs` glue (or a cucumber-rs upgrade) should unblock them.

Estimated yield: **~40 scenarios** unlocked across the 8 features.

---

## 5. Tier A — Skip-step plumbing (highest ROI, ~150 scenarios)

These are pure test-harness work. The translator already does the right thing;
we just lack step definitions. Each item is independent.

### A.1  Implement "Then a TypeError should be raised …" assertions  *(+30)*

Add steps matching:

```text
Then a TypeError should be raised at any time: <code>
Then a TypeError should be raised at compile time: <code>
Then a SyntaxError should be raised at compile time: <code>
Then a SemanticError should be raised at compile time: <code>
```

Implementation:
- Defer the query translation+execution until this step instead of at
  "When executing query".
- Translate, then either:
  - At *compile time*: assert `Err(PolygraphError::Translation { … })` with a
    matching error category.
  - At *any time*: try to execute against oxigraph; allow failure either at
    translate or execute.
- Map TCK error codes (`InvalidArgumentType`, `InvalidArgumentValue`, …) to a
  small `enum TckErrorCode` and tag our `PolygraphError` variants accordingly.

Effort: medium (1–2 days). Files:
[tests/tck/main.rs](tests/tck/main.rs), new `tests/tck/error_codes.rs`.

### A.2  Implement "Then the result should be empty" for write queries  *(+95)*

Currently `having_executed`/`executing_query` short-circuits write queries by
calling `update()` which returns `()`. The cucumber framework then sees no
matching step for "Then the result should be empty" and skips the scenario.

Implementation:
- When the parsed query is a write (CREATE/MERGE/DELETE/SET/REMOVE/FOREACH
  with no terminal RETURN), still record an empty `QueryResults::Solutions(0)`
  on `world`.
- Add a `then(regex = r"^the result should be empty$")` step asserting that
  recorded result has zero rows.
- Side-effect assertions (`+nodes`, `-relationships`, etc.) already work
  because we run the UPDATE for real on oxigraph; just need to compute a
  diff snapshot before/after.

Effort: medium (2–3 days, side-effect diff is non-trivial).
Files: [tests/tck/main.rs](tests/tck/main.rs).

### A.3  Add the `binary-tree-1` and `binary-tree-2` graph fixtures  *(+19)*

The TriadicSelection1 feature uses two pre-canned fixture graphs. Their
shapes are documented in
[the openCypher TCK README](https://github.com/opencypher/openCypher/blob/master/tools/tck-api/src/main/resources/db/binary-tree-1.cypher).

Implementation:
- Convert the two .cypher fixtures to literal Turtle (RDF-star) snapshots
  living under `tests/tck/fixtures/`.
- Extend the `empty_graph` / `any_graph` step matcher with a "Given the
  binary-tree-N graph" branch that loads the fixture into oxigraph instead
  of resetting to empty.

Effort: small (~1 day).

### A.4  Procedure CALL stubs  *(+5 to +8)*

Five Call1 scenarios + 2 ProcedureError + 1 ParameterMissing all involve
either a no-op procedure (`test.doNothing()`) or an introspection procedure
(`test.labels()`). Implement two strategies:

- **Stub procedures**: When the parser sees `CALL test.<name>(...)`, route
  to a tiny built-in registry that produces a fixed result-set, no SPARQL
  emitted.
- **Procedure-not-found error**: If the called name is unknown, emit
  `PolygraphError::Translation { message: "ProcedureNotFound: …" }`
  tagged with `TckErrorCode::ProcedureNotFound`.
- **Parameter-missing error**: The single `ParameterMissing` test exercises
  `$paramName` substitution; surface as
  `TckErrorCode::MissingParameter` from the parameter binder.

Effort: small (~1 day) once A.1 lands (reuses error-code mapping).

**Tier A subtotal: ~150 scenarios moved skip → pass.**

---

## 6. Tier B — Translator wins (~20 scenarios)

Bounded code changes inside the existing translator + rdf_mapping layers.

### B.1  Mixed-sign duration construction *(Temporal8, ~5)*

Today `tck_eval_duration` collapses everything into a single `xsd:duration`
literal then uses regex+STRDT to split into yearMonth/dayTime. The mixed-sign
cases (e.g. `P1Y-1M`) lose precision because xsd:duration normalises signs.

Fix: detect mixed signs at parse time and emit a *pair* of literals
(yearMonthDuration, dayTimeDuration) bound through a small helper, using
SPARQL `CONCAT`/`STRDT` only when all signs match.

Effort: small.

### B.2  `properties()` on nodes/relationships  *(Graph9 +2, partial others)*

`properties(n)` currently fails as a Phase 4+ complex return. Implement it
by:

- During translation, emit a SPARQL subquery that gathers all
  `?n :prop ?val` triples, GROUP_CONCAT them as JSON, and bind the result
  to a single column.
- During result-mapping, parse the JSON back into an openCypher map.

Effort: medium (touches both translator and result_mapping).

### B.3  `relationships(p)` on var-length paths  *(Path2 +2, Quantifier1–4 partial, ~6)*

When `p = (a)-[*]->(b)`, exposing `relationships(p)` requires reifying each
matched edge as a list. Implement by:

- Emitting SPARQL property paths *plus* an auxiliary GROUP_CONCAT subquery
  that re-walks the path through a recursive query.
- Engines without recursion fall back to L2-runtime path decomposition (see
  [pg-extension-protocol.md](pg-extension-protocol.md)).

Effort: medium-high. May be deferred to L2.

### B.4  Missing SyntaxError check  *(+1)*

One scenario asserts that a particular grammar production *should* fail at
compile time but our parser/translator accepts it. Identify the input and
add a semantic-validator rule.

Effort: trivial once A.1 lands.

**Tier B subtotal: ~20 scenarios.**

---

## 7. Tier C — Phase 4+ runtime (~30 scenarios)

These all share one root cause: openCypher allows arbitrary expressions in
the RETURN list, but SPARQL only allows variables and a fixed set of
built-ins. Concretely failing:

- 26 × `Phase 4+ complex return expression: <name>` (lists, `properties()`,
  `relationships()`, `nodes()`, `keys()`, `labels()`, etc.)
- 6 × List12 list-projection edge cases
- 1 × list comprehension `[x IN list WHERE pred | expr]`
- 1 × property on non-variable base (`(n).prop` after computed expression)

### Recommended approach

Adopt the [L2 runtime support](l2-runtime-support.md) plan:

1. The translator emits SPARQL that returns the *raw* bound variables.
2. A *post-projection* layer walks each row and evaluates the original
   Cypher RETURN expressions against the bound values + a small interpreter
   for `properties`, `relationships`, list comprehensions, etc.
3. Map back to `Vec<CypherRow>` via the existing result_mapping layer.

This keeps the SPARQL surface area small while unlocking the full Cypher
return language. Effort: large (multi-week), but the single biggest
unlock remaining.

**Tier C subtotal: ~30 scenarios.**

---

## 8. Tier D — DST / IANA timezone (~10 scenarios)

Temporal2 (×2), Temporal3 (×2), Temporal10 (×6) all assert behaviour that
crosses a daylight-savings boundary in named-zone arithmetic. SPARQL's
`xsd:dateTime` carries only fixed offsets, so this is unreachable without a
tz database.

Implementation: add a `chrono-tz` dev-dependency *and* register a small
`CUSTOM_FUNCTIONS` set with oxigraph that resolves IANA zone names. Already
prototyped under `src/sparql_engine/`.

Effort: medium. Gated on engine support (oxigraph is fine; PG extension is
covered in [pg-extension-protocol.md](pg-extension-protocol.md)).

---

## 9. Tier E — Hard limits (~10–15 scenarios)

These cannot be moved without architectural shifts:

- **UNWIND of a runtime variable** (10): SPARQL `VALUES` requires literal
  lists. Workarounds either re-execute the outer query (correctness hazard)
  or move into the L2 runtime.
- **dur+dur, dur×num normalisation** (Temporal8 long tail, ~5): would
  require a custom duration interpreter; SPARQL's date-arithmetic functions
  do not normalise carries.
- **Quantifier11 edge cases** (~3): inherently L2.

Document these as *won't fix* in `plans/fundamental-limitations.md`.

---

## 10. Recommended order of execution

```
1. Tier A.1  TypeError step definitions          (+30)   1–2d
2. Tier A.2  Empty-result for write queries      (+95)   2–3d
3. Tier A.3  Binary-tree fixtures                (+19)   1d
4. Tier A.4  Procedure CALL stubs                (+8)    1d
5. Parsing errors triage                          (+40)  1–2d
   ─────────────────────────────────────────────────────
   Sub-total after Tier A + parse                ≈ +192  (≈ 99 % pass)

6. Tier B.1  Mixed-sign durations                (+5)    1d
7. Tier B.2  properties() return                 (+4)    2–3d
8. Tier B.4  Missing SyntaxError                 (+1)    trivial
9. Tier B.3  relationships() on var-length       (+6)    1w (or defer)
   ─────────────────────────────────────────────────────
   Sub-total after Tier A + B + parse            ≈ +208

10. Tier D   DST/IANA timezone                   (+10)   3–5d
11. Tier C   Phase 4+ runtime (L2)               (+30)   multi-week
   ─────────────────────────────────────────────────────
   Final ceiling                                 ≈ 3779/3789 (99.7 %)
```

After Tier A + parsing-error fixes the suite reaches **≈ 99 % pass rate**
with only a few weeks of work, almost all of which is test-harness plumbing
and known patterns rather than new translator capability.

---

## 11. Cross-references

- [remaining-failures.md](remaining-failures.md) — per-feature failure
  triage (origin of the per-feature counts in §2.2).
- [l2-runtime-support.md](l2-runtime-support.md) — design for the runtime
  layer that closes Tier C and Tier E.
- [fundamental-limitations.md](fundamental-limitations.md) — the "won't
  fix" list to be expanded with Tier E items.
- [target-engines.md](target-engines.md) — engine capability matrix
  underlying Tier B.3 and Tier D.
- [tck-full-plan.md](tck-full-plan.md) — original phase-A→D rollout that
  brought us to 93.9 %; this document supersedes its open sections.
