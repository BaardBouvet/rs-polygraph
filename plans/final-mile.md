# Final Mile — Closing the Last TCK Gaps

**Status**: in progress
**Updated**: 2026-06-02

This plan supersedes the failure inventory in [remaining-work.md](remaining-work.md)
now that Tier A (harness plumbing) has landed (3558 → 3704 passes), and Tier FGH
partial work has pushed to 3734 / 3828 (97.5 %).

Phase 9 has since been delivered (UNWIND-first routing, pending_binds for IS NULL,
FILTER(BOUND) for null UNWIND aggregation), holding steady at 3757 / 3828 (98.2%).

---

## 1. Current Baseline (post-Phase-9)

| Bucket            | Count | % of 3828 |
|-------------------|------:|----------:|
| **Passing**       | 3757  | 98.2 %    |
| Failing           |   71  |  1.9 %    |
| Parse errors      |    3  |  0.1 %    |
| **Total**         | 3828  | 100 %     |

### What Phase 9 delivered (TCK-neutral, expanded LQA coverage)

- UNWIND-first clause_shape routing through LQA (first == "unwind" || "match")
- FILTER(BOUND(?var)) in GroupBy for UNWIND null vars (Aggregation2 [3]-[8] via LQA)
- pending_binds mechanism: IsNull/IsNotNull of complex exprs via BIND + BOUND check
  (Boolean5 [6] regression fixed)
- Removed with_orderby_out_of_scope guard (11 fewer legacy fallbacks)
- Removed incorrect hours≥24 normalization in tck_eval_duration

### What remains (71 failures, 3 parse errors)

**Non-L2 total: ~25 · True L2 total: ~46 · Parse errors: 3**

| Family                                          | Count | Tier | L2? |
|-------------------------------------------------|------:|------|-----|
| Duration arithmetic (Temporal8 [6,7])           |     2 | F    | No* |
| DST / IANA timezone (Temporal2/3/10)            |    10 | J    | No  |
| `relationships(p)` / `nodes(p)` post-projection |    14 | I    | Yes |
| UNWIND quantifier invariants (Q9-12)            |     8 | I    | Yes |
| List comprehension (List12)                     |     6 | I    | Yes |
| Pattern2 (pattern comprehension)                |     2 | I    | Yes |
| MERGE+WITH semantics (Merge1/5)                 |     7 | G    | No  |
| var-length misc (Match4/5/6)                    |     5 | G    | No  |
| collect+precedence (Precedence1, WithOrderBy1)  |     5 | I    | Yes |
| Mixed-type ORDER BY (ReturnOrderBy1/4)          |     3 | I    | Yes |
| properties(n) map (Graph9)                      |     2 | I    | Yes |
| path equality (Comparison1)                     |     1 | G    | No  |
| EXISTS+aggregation (ExistentialSubquery2)       |     1 | I    | Yes |
| Implicit grouping with path (With6 [4])         |     1 | I    | Yes |
| Hard / Tier-K (Set1, List11)                    |     2 | K    | —   |
| Parse errors (Gherkin bugs in TCK corpus)       |     3 | —    | —   |
| **Total**                                       |    71 |      |     |

*Duration arithmetic: SPARQL 1.1 does not support `xsd:duration + xsd:duration`.
Requires a custom implementation (string decomposition arithmetic in expressions
or a post-projection interpreter step).

---

**Tier F** (duration arithmetic) was designed in F.1–F.3 but *never implemented* —
all 17 Temporal8 failures are still open.

**Tier G** (translator bug fixes) was partially done. G.1 WITH→MERGE propagation
(7 failures), G.2 var-length property predicate (3 failures), and two additional
Match5 bugs (undirected overcount ×2, zero-hop ×5) remain.

**Tier H** remaining: Pattern1/ReturnOrderBy2 wrong-error-type (4 trivial).

---

## 2. Failure Inventory — Restated (94 remaining, post-FGH)

> KEY FINDING: the "Tier I (L2-runtime)" label was applied too broadly in the
> original plan. Full audit as of 2026-05-13 shows only **~37** failures genuinely
> need the post-projection interpreter. The remaining **~57** are unfinished
> Tier F/G/H/J work.

| Family                                          | Count | Tier | L2? |
|-------------------------------------------------|------:|------|-----|
| Duration arithmetic (Temporal8)                 |    17 | F    | No  |
| DST / IANA timezone (Temporal2/3/10)            |    10 | J    | No  |
| MERGE+WITH binding propagation (Merge1/5)       |     7 | G    | No  |
| `relationships(p)` / `nodes(p)` post-projection |    16 | I    | Yes |
| UNWIND of variable (Quantifier11)               |     6 | I    | Yes |
| var-length *0 zero-hop (Match5/6)               |     5 | G    | No  |
| UNWIND [node, rel, …] in harness                |     4 | H    | No  |
| translator singletons (Graph3/4, With6, etc.)   |     4 | G    | No  |
| collect+iterate / List12                        |     4 | I    | Yes |
| Wrong-error-type trivial (Delete1/Pattern1/ROB2)|     4 | H    | No  |
| var-length misc (Match4:111, Match6:273)        |     3 | G    | No  |
| collect+precedence (WithOrderBy1, Precedence1)  |     3 | I    | Yes |
| var-length undirected overcount (Match5:521/564)|     2 | G    | No  |
| properties(n) map (Graph9)                      |     2 | I    | Yes |
| pattern comprehension in WITH (Pattern2)        |     2 | I    | Yes |
| path equality (Comparison1)                     |     1 | G    | No  |
| EXISTS+non-MATCH (ExistentialSubquery2)         |     1 | I    | Yes |
| Hard / Tier-K (Match4:192, Set1, List11)        |     3 | K    | —   |
| **Total**                                       |  **94**|     |     |

**Non-L2 total: ~57 · True L2 total: ~34 · Tier-K: 3**

---

## 3. Failure Inventory by Feature File

```
17  expressions/temporal/Temporal8.feature        ← Tier F (duration normalisation)
 6  expressions/temporal/Temporal10.feature       ← Tier J (DST)
 6  expressions/quantifier/Quantifier11.feature   ← Tier I (L2-runtime quantifier)
 6  expressions/list/List12.feature               ← Tier I (list projections)
 5  clauses/merge/Merge5.feature                  ← Tier G (merge re-translation)
 3  expressions/pattern/Pattern2.feature          ← Tier G (pattern semantics)
 3  clauses/with-orderBy/WithOrderBy1.feature     ← Tier G (UNWIND var + sort key)
 2  expressions/temporal/Temporal2.feature        ← Tier J (IANA tz parser)
 2  expressions/temporal/Temporal3.feature        ← Tier J (DST round-trip)
 2  expressions/quantifier/Quantifier{1..4}       ← Tier I (relationships() ×8)
 2  expressions/precedence/Precedence1.feature    ← Tier H (UNWIND scenario-outline)
 2  expressions/pattern/Pattern1.feature          ← Tier H (1) + Tier G (1)
 2  expressions/path/Path2.feature                ← Tier I (relationships())
 2  expressions/graph/Graph9.feature              ← Tier I (properties())
 2  clauses/return-orderby/ReturnOrderBy1.feature ← Tier I (UNWIND var)
 2  clauses/merge/Merge1.feature                  ← Tier G (WITH→MERGE propagation)
 2  clauses/match/Match4.feature                  ← Tier G (var-length predicate)
 8  clauses/match/Match5.feature                  ← Tier G (*0 zero-hop ×5, overcount ×2, *..1 ×1)
 1  clauses/match/Match6.feature                  ← Tier G (named-path zero-hop)
 4  clauses/return-orderby/ReturnOrderBy1.feature ← Tier H (UNWIND [n,r,p])
 1  clauses/return-orderby/ReturnOrderBy2.feature ← Tier H (wrong error type)
 3  clauses/merge/Merge5.feature                  ← Tier G (WITH→MERGE ×3 more)
 1  clauses/delete/Delete1.feature                ← Tier H (wrong error type)
 1  clauses/set/Set1.feature                      ← Tier K (list-comp over prop)
 1  clauses/with/With6.feature                    ← Tier G (WITH * after CREATE)
 2  expressions/graph/Graph3-4.feature            ← Tier G (labels()/type() on list elem)
 2  expressions/precedence/Precedence1.feature    ← Tier I (collect+prec)
 1  expressions/list/List11.feature               ← Tier K (range()+ALL chain)
 1  expressions/comparison/Comparison1.feature    ← Tier G (path equality)
 1  clauses/return-orderby/ReturnOrderBy4.feature ← Tier G (singleton)
─────────────────────────────────────────────────
94  total failures (post-FGH)
```

---

## 4. Tier F — Duration Arithmetic Hot-Spot (+13)

**Single biggest cluster: 17 of 84 failures live in Temporal8.**

Of those, 4 are already passing (post-Tier-T1b normalisation). The remaining
13 fall into three sub-clusters:

### F.1  Hours-overflow regression  *(+1 to +9)*

`tck_eval_duration` currently normalises `33H → 1D + 9H` and similar. But
**`dur1 + dur2` and `dur1 - dur2` expect the un-normalised form** when both
operands carry hour fields:

```
got:      "P24Y10M29DT8H26M20.000000002S"   (we normalised)
expected: "P24Y10M28DT32H26M20.000000002S"  (TCK keeps 32H)
```

Root cause: normalisation happens in the *constructor* even when the value
will later participate in arithmetic. Fix:

1. Stop normalising the day↔hour boundary in `tck_eval_duration` for
   constructor calls — the TCK output is "as authored, mod sign".
2. Move the normalisation to display-only sites (`toString`) **only** when
   no arithmetic precedes it.
3. Add a unit test ladder: every Examples row of Temporal8 [6] becomes a
   table-driven case in `tck_eval.rs` against the expected literal.

Effort: small (~½ day). Files: [tests/tck/main.rs](tests/tck/main.rs)
helper `tck_eval_duration` + new `tck_eval_test.rs`.

### F.2  Mixed-sign duration construction *(+~3)*

`P12Y-4M-28DT-24M-0.000000001S` requires an explicit yearMonthDuration +
dayTimeDuration pair (xsd:duration normalises signs). Implement by:

1. Detect mixed signs in the input map at parse time.
2. Emit two parallel SPARQL string concatenations and join with `CONCAT`.
3. Wrap the result in an `xsd:duration` cast only when SPARQL serialises.

Effort: small (~½ day).

### F.3  duration × number / duration ÷ number  *(+~3, Temporal8 [7])*

Today `dur * n` and `dur / n` short-circuit to xsd:duration arithmetic
which loses the carry semantics. Replace with a small interpreter:

1. Decompose into (years, months, days, hours, minutes, seconds, nanos).
2. Multiply each field by `n` (or divide), then cascade overflow.
3. Re-emit as the canonical Cypher duration literal.

Effort: medium (1 d). Most of the cascade code already exists from F.1.

**Tier F yield: +13. Time: 2 d.**

---

## 5. Tier G — Translator Bug-Fix Cluster (+11)

Eleven row-count / set-mismatch failures share one of three root causes:
all addressable inside the translator without new architecture.

### G.1  WITH–MERGE constraint propagation  *(Merge1 +2, Merge5 +5, Merge6/7 — known good now)*

Scenarios:

- `Merge1[5]` (line 176): `WITH foo.x AS x …  MERGE (:N {x: x, y: y+1})` —
  expected 9 rows, got 0. We do not propagate `x` and `y` as bound through
  WITH into the MERGE pattern.
- `Merge1[8]` (line 273): MATCH after DELETE then MERGE — wrong row count.
- `Merge5[4,5,7,9]`: same WITH-binding loss.

Fix: extend `with_aliases` map in `tests/tck/main.rs::write_clauses_to_updates`
*and* in `src/translator/cypher/clauses.rs` `Clause::With` handling to
propagate inner-scope variables through to the *next* MERGE/CREATE.

Effort: medium (1–2 d). Files: harness + translator.

### G.2  Variable-length predicate with edge property  *(Match4[5] +1, Match4[8] +1, Match6[12] +1, Pattern1/2 ×3)*

```cypher
MATCH (a:Artist)-[:WORKED_WITH* {year: 1988}]->(b:Artist) RETURN *
```

Today the translator emits a SPARQL property path that ignores the inline
property predicate. The SPARQL path must filter via per-edge reification
(or RDF-star annotations) to keep `{year: 1988}` constraints alive across
the path.

Fix: enhance `translate_relationship_with_var_length` to wrap each path
edge in a sub-pattern with the property predicate; when path length > 1,
fall back to a recursive sub-query (engine-dependent).

Effort: medium-high (2–3 d). Reuses existing edge-property emit logic.

### G.3  WithOrderBy1[1137] — sort-key for collected lists  *(+1)*

ORDER BY of `collect(nodes(p))` returns no sort key for path lists.
Reuse the O1 list sort-key encoding (`?__sk_<var>`) already present for
RETURN ORDER BY.

Effort: trivial (~1 h).

### G.4  Graph3, Graph4, With6, Merge5[4]  *(+4)*

Each is a singleton. Triage in one batch:

| Scenario          | Likely cause                                         |
|-------------------|------------------------------------------------------|
| Graph3 line 122   | `head(labels(n))` over node with multi-label         |
| Graph4 line 112   | label-set comparison via `labels(n) = [...]`         |
| With6 line 110    | WITH * after CREATE in subquery                      |
| Merge5 line 96    | Anonymous merge with no rel var                      |

Effort: 2 d combined.

**Tier G yield: +11. Time: 5–6 d.**

---

## 6. Tier H — Quick Wins & Plumbing (+5 / +40 unlocks)

### H.1  Lone skip — `the result should be, in order (ignoring element order for lists)`  *(+1)*

[ReturnOrderBy2:259](tests/tck/features/clauses/return-orderby/ReturnOrderBy2.feature)
uses a step pattern not handled by the harness. Add a matching step:

```rust
#[then(regex = r"^the result should be, in order \(ignoring element order for lists\):$")]
async fn result_in_order_ignore_list_order(world: &mut TckWorld, step: &Step) { … }
```

This is a 1-line addition cloning `result_in_order` + the `sort_lists=true`
branch already present in `compare_results`.

Effort: trivial (~10 min).

### H.2  Pattern1[24] missing SyntaxError  *(+1)*

```cypher
MATCH (n) SET n.prop = head(nodes(head((n)-[:REL]->()))).foo
```

Should fail with `SyntaxError: UnexpectedSyntax` (path inside SET RHS).
Add a semantic-validator rule in
[src/translator/cypher/semantics.rs](src/translator/cypher/semantics.rs)
that walks the SetItem RHS and rejects any `Expression::Pattern(_)` /
`PathExpr` node.

Effort: small (~½ d).

### H.3  Precedence1[26,27] — list-membership scenario outline  *(+2)*

The scenario outline rotates `<comp>` over `=, <=, >=, <, >, <>`. Our
translator emits the wrong precedence for `b IN c` mixed with
chained comparisons. Audit
[src/translator/cypher/mod.rs](src/translator/cypher/mod.rs) precedence
table, ensure `IN` binds tighter than the comparison operators (per
openCypher grammar §5.5).

Effort: small (~½ d).

### H.4  Comparison2:123 + Quantifier7:80 example-expansion  *(+~10 unlocked)*

cucumber-rs misinterprets `<= <rhs>` in scenario-outline placeholders as a
nested placeholder reference. Workarounds:

- Patch the affected feature files to escape the literal `<=`
  (`\u003c=` or `&lt;=` per Gherkin escaping).
- Or upgrade cucumber-rs to a version that handles this correctly.
- Or pre-process feature files at load time in
  [tests/tck/main.rs](tests/tck/main.rs) to substitute the literal.

Effort: small (~½ d).

### H.5  Hard parse failures (×6)  *(+~30 unlocked)*

| File                                         | First broken line | Likely cause                        |
|----------------------------------------------|------------------:|-------------------------------------|
| Match5.feature                               | TBD               | Investigate                         |
| ExistentialSubqueries1.feature               | TBD               | Probably `EXISTS { … }` subquery    |
| Literals6.feature                            | TBD               | Hex / Unicode escape sequence       |
| Pattern3.feature, Pattern4.feature, Pattern5.feature | TBD       | Pattern-comprehension `[ … \| … ]`  |

Action: open each file, identify the offending line, add a minimal
reduction to `examples/parse_failure_repro.rs`, then either patch the
file (escape) or extend the cucumber loader.

Effort: small per file (~½ d each, ~3 d total).

**Tier H yield: +4 + ~40 unlocked. Time: ~5 d.**

---

## 7. Tier I — L2 Runtime Layer (+~50)

This is the single biggest remaining unlock and corresponds to the
[L2 runtime support](l2-runtime-support.md) plan, focused on the
**post-projection** sub-component.

### I.1  Post-projection interpreter  *(+~26)*

For every `complex return expression (Phase 4+)` failure:

1. Translator emits SPARQL that returns the *raw* bound variables for the
   query plus a manifest of **deferred expressions** (the list/properties/
   keys/labels/relationships call) keyed by output column.
2. After SPARQL execution, walk each row:
   - For each deferred expression, evaluate the Cypher expression against
     the bound row using a tiny interpreter
     (`src/translator/cypher/return_proj_runtime.rs`).
   - The interpreter handles: `[x IN list WHERE p | e]`, `properties(n)`,
     `relationships(p)`, `nodes(p)`, `keys(m)`, `labels(n)`,
     `(n).prop` on computed `n`, list/map literals.
3. Result mapping converts each evaluated value back to a `CypherValue`.

Effort: large (2 weeks). Files:
- new `src/translator/cypher/deferred.rs` (deferred-expression registry)
- new `src/result_mapping/runtime.rs` (interpreter)
- patches to `src/translator/cypher/return_proj.rs` (emit the deferred
  manifest instead of erroring)

Recovers all 26 `Phase 4+` failures plus most of List12 and Set1.

### I.2  UNWIND of variable / non-literal  *(+~10)*

Same plan: when UNWIND encounters a non-literal expression, emit the
SPARQL up to the UNWIND point, materialise the rows, then *re-emit* the
suffix N times via the post-projection layer.

Effort: medium (1 week, depends on I.1).

### I.3  `relationships(p)` and `properties(n)`  *(+~8)*

Special-case implementations within I.1's interpreter:

- `properties(n)` → re-query the graph for `?n ?p ?v`, fold to a map.
- `relationships(p)` → walk the matched path, decode each edge ID back to
  `(srcVar, type, dstVar)`.

Both require a *back-reference* from the interpreter to oxigraph (already
held in `world.store`); design carefully so production users supply their
own graph store.

Effort: medium (3 d).

### I.4  Quantifier11 + List12 edge cases  *(+~6)*

Most resolve automatically once I.1 lands. The remaining 1–2 require
extending the interpreter with `single()`, `any()`, `all()`, `none()` over
deferred lists.

Effort: small (1 d).

**Tier I yield: +50. Time: 4 weeks.**

---

## 8. Tier J — DST / IANA Timezone (+10)

| Feature   | Failures | Cause                                                     |
|-----------|---------:|-----------------------------------------------------------|
| Temporal2 |        2 | Parse `[Europe/Stockholm]` named-zone suffix              |
| Temporal3 |        2 | Round-trip `datetime + duration` across DST boundary      |
| Temporal10|        6 | `duration.inSeconds(zoned, local)` across DST             |

Implementation:

1. Add `chrono-tz` as a `dev-dependency` (TCK harness only; the library
   itself stays tz-agnostic).
2. In `src/sparql_engine/`, register a custom SPARQL function
   `urn:polygraph:tz-resolve(zoneName, instant)` that returns the offset
   in minutes for the given (zoneName, instant) pair.
3. The translator emits this function when it sees a named zone literal.
4. For pure parsing (Temporal2), extend `tck_eval_temporal_fn` to accept
   the `[Region/City]` suffix and round-trip it through `chrono_tz::Tz`.

Effort: medium (1 week). Production-target engines need their own tz
resolver — document the contract in
[plans/target-engines.md](target-engines.md).

---

## 9. Tier K — Hard Limits / Won't Fix (+0)

### K.1  Delete1[7] — ConstraintVerificationFailed at runtime

```cypher
CREATE (x:X), (x)-[:R]->()  // (×3 connected nodes)
MATCH (n:X) DELETE n
// Expected: ConstraintVerificationFailed at runtime: DeleteConnectedNode
```

A static transpiler cannot detect this; it would require materialising the
match and checking each candidate's degree at translation time.

Resolution: document in
[plans/fundamental-limitations.md](fundamental-limitations.md) as a
known limitation. Mark the scenario `@skip` upstream-style or add an
allowlist in the harness.

### K.2  Quantifier11 path-with-rand  *(0)*

Includes `rand()` inside the quantifier expression (no static fold
possible). Already documented as L2 in the original plan; if I.1 lands
they pass.

---

## 10. Recommended Order of Execution

```
Week 1
 ├─ Tier H.1  in-order list-aware step                 (+1)   <½ d
 ├─ Tier H.2  Pattern1[24] SyntaxError                 (+1)   ½ d
 ├─ Tier H.3  Precedence1[26,27]                       (+2)   ½ d
 ├─ Tier H.4  scenario-outline `<=` escape             (+~10) ½ d
 └─ Tier F    duration arithmetic (F.1, F.2, F.3)      (+13)  2 d
                                                      ─────────
                                                       +27   ≈ 3731/3789 (98.5 %)

Week 2
 ├─ Tier H.5  hard parse failures ×6                   (+~30) 3 d
 └─ Tier G.1  WITH–MERGE propagation                   (+7)   2 d
                                                      ─────────
                                                       +37   ≈ 3768 (99.4 %)

Week 3
 ├─ Tier G.2  var-length edge property                 (+5–6) 3 d
 ├─ Tier G.3  WithOrderBy1[1137]                       (+1)   <½ d
 └─ Tier G.4  Graph3/Graph4/With6/Merge5 singletons    (+4)   2 d
                                                      ─────────
                                                       +11   ≈ 3779 (99.7 %)

Weeks 4–7
 └─ Tier I    L2 post-projection runtime               (+50)  4 wks
                                                      ─────────
                                                       Final ceiling 3829? — but
                                                       cap is 3788 (excluding
                                                       Delete1[7] hard limit)

Week 8
 └─ Tier J    DST / IANA timezone                      (+10)  1 wk

Result: 3788 / 3789 (99.97 %), 1 documented hard limit.
```

---

## 11. Open Questions

1. **Tier H.5 effort estimate is per file, not investigative**. We do
   not yet know what cucumber-rs limitation each file hits; the first
   half-day on Pattern3.feature should reveal whether all six share a
   single root cause (shrinking the budget to ~1 d total).

2. **Tier I.1 scope vs L2 design**. The post-projection interpreter
   sketched here is a *subset* of the full L2 runtime described in
   [l2-runtime-support.md](l2-runtime-support.md). We should decide
   whether to ship I.1 standalone or wait for the full L2 surface.

3. **Tier J vs target-engine independence**. The custom SPARQL function
   `urn:polygraph:tz-resolve` works for oxigraph but not for arbitrary
   targets. Document the engine-capability flag
   `TargetEngine::supports_iana_timezones()` and gate the emission.

---

## 12. Cross-references

- [remaining-work.md](remaining-work.md) — the original Tier-A–D plan,
  superseded by this document.
- [l2-runtime-support.md](l2-runtime-support.md) — Tier I depends on
  the `Continuation` infrastructure landed there.
- [fundamental-limitations.md](fundamental-limitations.md) — gain
  Delete1[7] (Tier K.1) on completion.
- [target-engines.md](target-engines.md) — capability flags for Tier J.
- [pg-extension-protocol.md](pg-extension-protocol.md) — engine-side
  contract for Tier I.3 path decomposition on Postgres triplestores.
