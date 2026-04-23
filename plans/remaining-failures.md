# Remaining TCK Failures — Triage and Fix Plan

**Status**: in progress
**Updated**: 2026-05-12
**Baseline**: 3437 / 3739 scenarios pass (95.8 %), 154 failed, 148 skipped.
**Current**: 3558 / 3789 scenarios pass (93.9 %), 83 failed, 148 skipped.

### Fixes applied since initial baseline

| Scenarios | What was fixed |
|----------:|----------------|
| +3 | String8/9/10 [8]: STARTS WITH/ENDS WITH/CONTAINS falsely matching list/map encoded strings. Added content guard `!STRSTARTS(x,"[") && !STRSTARTS(x,"{")` in StartsWith/EndsWith/Contains translator. |
| +1 | Temporal1 [11]: `datetime.fromepoch(s, ns)` and `datetime.fromepochmillis(ms)` not implemented. Added compile-time epoch→ISO-8601 conversion using proleptic Gregorian calendar in `temporal.rs`. |
| +2 | Set1 [6,7]: `SET a.prop = a.prop + [4, 5]` (list concat in SET) returned stale CREATE value. Fixed by pre-scanning CREATE properties before SET in skip_writes mode, then folding self-referential list concatenations statically. |
| +3 | ReturnOrderBy1 [9,10] / WithOrderBy1 [9,10]: O1 — list ordering sort-key encoding. SPARQL lexicographic ORDER BY replaced by parallel `?__sk_<var>` column using type-ranked prefix (`map(0) < list(3) < string(5) < bool(6/7) < int(8) < null(Z)`). WITH…ORDER BY fix propagates sort-key into inner SELECT projection. |
| +3 | Aggregation2 [9,11,12]: A1 — compile-time min/max folding. Detects `UNWIND [lits] AS x RETURN min/max(x)` and pre-computes the extremum via `cypher_compare` in Rust, emitting a constant `VALUES` pattern. |
| +54 | Quantifier9–12 (partial): Q1 — quantifier tautology folding. Detects `rand()`/`reverse()`/CASE preambles that produce opaque list variables, then folds mathematical identities (`none(P)=!any(P)`, `all(P)=none(!P)`, size-based equivalences, constant-predicate cases `none(false)=T`, `any(false)=F`, `all(true)=T`, `single(false)=F`) to constants. Implemented via `try_fold_quantifier_invariants` + `quantifier_canonical` + `eval_quantifier_tautology` free functions. |
| +7 | Temporal8 scenarios 1-5 example 1 (T1a): xsd:duration typed literals + split-subtract. Fixed `tck_eval_duration` to store tagged `^^xsd:duration` literals. Added `temporal_subtract_sparql()` which splits a duration into yearMonthDuration + dayTimeDuration parts using REPLACE regex and subtracts each via COALESCE. Uses `STRBEFORE(dt,"T")` for `xs:date` to strip time components (Oxigraph off-by-one issue). |
| +3 | Temporal8 scenarios 1,4,5 example 3 (T1b): Fixed `tck_eval_duration` fractional-year cascade (0.5Y → +6M) and hours≥24 normalization (33H → 1D+9H). Scenarios 2+3 example 3 were already passing (localtime/time ignore YM). |

> Note: total scenario count increased by 50 (new TCK features scanned);
> Pattern1/2 appear as new pre-existing failures unrelated to these changes.

This plan triages the **83 remaining failures** into actionable groups.

---

## 1. Failure Distribution (current)

| Count | Category                                              | Likely tractable? |
|------:|-------------------------------------------------------|------------------:|
|    17 | Temporal8 failures                                    | 5 mixed-sign dur (data format), 12 dur+dur/dur×num (L2) |
|     6 | Temporal10 DST-timezone failures                      | Not fixable (no IANA tz database) |
|     6 | Quantifier11 complex list ops                         | L2 (runtime UNWIND) |
|     6 | List12 list comprehension `collect()`                 | L2 (Phase 4+) |
|     5 | Merge5 (2 result mis + 3 Phase4+)                     | partial — MERGE logic |
|     3 | Pattern2 (1 Phase4+ + 2 row mismatch)                 | L2 |
|     3 | WithOrderBy1 (2 var-UNWIND + 1 result mismatch)       | partly L2 |
|     4 | Temporal2/3 DST timezone                              | Not fixable |
|     8 | Quantifier1-4 (2ea: path/relationships())              | L2 (variable-length paths) |
|     2 | Precedence1 (LIST comparison encoding)                | L2 (complex) |
|     2 | Pattern1 (1 row mismatch + 1 SyntaxError)             | mixed |
|     2 | Path2 (relationships(p) on var-len path)              | L2 |
|     2 | Graph9 (properties(n/r) Phase4+)                      | L2 |
|     2 | ReturnOrderBy1 (variable UNWIND)                      | L2 |
|     2 | Merge1 (MERGE after DELETE, multi-MERGE)              | L2 MERGE execution |
|     2 | Match4 (var-len path row counts)                      | L2 |

Top actionable remaining items:

| Failures | Feature       | Fix bucket |
|---------:|---------------|------------|
|    17 | Temporal8     | 5 fixable (mixed-sign dur), 12 structural |
|     6 | Temporal10    | 6 DST (not fixable without tz db) |
|     6 | Quantifier11  | 6 complex (L2) |
|     6 | List12        | 6 Phase4+ |
|     5 | Merge5        | 2 result mis + 3 Phase4+ |

---

## 2. Fix Buckets

### Q1 — Quantifier predicates on literal lists  *(DONE: +54; 6+8 remain)*

**Status**: ✅ SIGNIFICANTLY FIXED. +54 passes from Quantifier9–12.
Remaining 14: Quantifier11 (6, complex L2 list ops) + Quantifier1-4 (8, Phase4+ path features).

### T1 — Duration arithmetic on temporal values *(DONE: +10; 17 remain)*

**Status**: ✅ PARTIALLY FIXED. +10 passes (T1a +7, T1b +3).

**What was implemented**:
- `tck_eval_duration`: store as `^^xsd:duration`; cascade fractional years→months; normalize hours≥24→extra days
- `temporal_subtract_sparql()`: split-subtract using `REPLACE` regex + STRDT for yearMonthDuration and dayTimeDuration parts; `STRBEFORE(dt,"T")` for date-only subtraction
- `is_temporal_lit_str()` / `is_date_only_lit_str()` detection helpers

**Remaining** Temporal8 failures (17):
- 5 × example 2 (mixed-sign duration `P1M-14DT16H-11M10S`): XSD duration requires non-negative components; Cypher allows negative component values which can't be represented as a single valid xsd:duration. Fundamental format incompatibility.
- 9 × Scenario 6 (duration + duration): Oxigraph normalizes differently from Cypher (e.g., `P24H` vs `P1D`). SPARQL `xsd:duration + xsd:duration` gives different normalization.
- 3 × Scenario 7 (duration × number): SPARQL has no `duration * number` operator.

**Remaining** Temporal10 failures (6): All in Scenario 8 (DST-aware datetime), requiring IANA timezone database for Europe/Stockholm DST transitions. Not fixable without timezone library.

### LC1 — List comprehension projection  *(6 remain)*

List12 [1-6]: `collect(a)` followed by `[x IN nodes | x.name]` — requires runtime node property access on aggregate results. Phase 4+ limitation.

### M1 — MERGE execution  *(5 remain in Merge5, 2 in Merge1)*

Merge5 results mismatches: MERGE after DELETE, multiple MERGE clauses. Would require rearchitecting the MERGE → SELECT/UPDATE flow.

### DST Timezone  *(4 remain)*

Temporal2 [4,5], Temporal3 [5,6], Temporal10 [8] (all 6): require IANA tz database for DST-aware timezone conversions. Not currently supported.


### Fixes applied since initial baseline

| Scenarios | What was fixed |
|----------:|----------------|
| +3 | String8/9/10 [8]: STARTS WITH/ENDS WITH/CONTAINS falsely matching list/map encoded strings. Added content guard `!STRSTARTS(x,"[") && !STRSTARTS(x,"{")` in StartsWith/EndsWith/Contains translator. |
| +1 | Temporal1 [11]: `datetime.fromepoch(s, ns)` and `datetime.fromepochmillis(ms)` not implemented. Added compile-time epoch→ISO-8601 conversion using proleptic Gregorian calendar in `temporal.rs`. |
| +2 | Set1 [6,7]: `SET a.prop = a.prop + [4, 5]` (list concat in SET) returned stale CREATE value. Fixed by pre-scanning CREATE properties before SET in skip_writes mode, then folding self-referential list concatenations statically. |
| +3 | ReturnOrderBy1 [9,10] / WithOrderBy1 [9,10]: O1 — list ordering sort-key encoding. SPARQL lexicographic ORDER BY replaced by parallel `?__sk_<var>` column using type-ranked prefix (`map(0) < list(3) < string(5) < bool(6/7) < int(8) < null(Z)`). WITH…ORDER BY fix propagates sort-key into inner SELECT projection. |
| +3 | Aggregation2 [9,11,12]: A1 — compile-time min/max folding. Detects `UNWIND [lits] AS x RETURN min/max(x)` and pre-computes the extremum via `cypher_compare` in Rust, emitting a constant `VALUES` pattern. |
| +54 | Quantifier9–12 (partial): Q1 — quantifier tautology folding. Detects `rand()`/`reverse()`/CASE preambles that produce opaque list variables, then folds mathematical identities (`none(P)=!any(P)`, `all(P)=none(!P)`, size-based equivalences, constant-predicate cases `none(false)=T`, `any(false)=F`, `all(true)=T`, `single(false)=F`) to constants. Implemented via `try_fold_quantifier_invariants` + `quantifier_canonical` + `eval_quantifier_tautology` free functions. |

> Note: total scenario count increased by 50 (new TCK features scanned);
> Pattern1/2 appear as new pre-existing failures unrelated to these changes.

This plan triages the **154 remaining failures** into actionable groups and
proposes architecture improvements that would make further iteration faster.

---

## 1. Failure Distribution

By error category (from `/tmp/tck_out.txt`, parsed by feature):

| Count | Category                                              | Likely tractable? |
|------:|-------------------------------------------------------|------------------:|
|    47 | `Unsupported feature: complex return expression …`    | L2 (runtime/path operations) |
|    46 | `Result set mismatch` (translation succeeded, wrong result) | mostly L2 |
|    29 | `Unsupported feature: UNWIND of variable …`           | L2 deep design issue |
|    11 | `Row count mismatch`                                  | L2 mostly aggregation/temporal |
|     1 | `Unsupported feature: list comprehension`             | L2 |
|     1 | other                                                 | — |

Top features (failures per file) — **updated after O1 + A1 fixes applied**:

| Failures | Feature       | Fix bucket |
|---------:|---------------|------------|
|       27 | Temporal8     | T1 — duration arithmetic precision |
|       22 | Quantifier11  | Q1 — quantifiers on literal lists |
|       17 | Quantifier12  | Q1 |
|       17 | Quantifier9   | Q1 |
|        7 | Quantifier10  | Q1 |
|        6 | List12        | LC1 — list-comprehension projection |
|        6 | Temporal10    | T1 |
|        5 | Merge5        | M1 — MERGE relationship matching |
|        4 | ReturnOrderBy1| O1 — 2 remaining (scenarios [11,12]: MATCH var in UNWIND list, L2) |
|        3 | WithOrderBy1  | O1 — 3 remaining |
|        3 | Set1          | S1 — list-valued SET |
|        2 | Precedence1   | P1 — boolean 3VL precedence edge cases |
|        2 | Graph9        | G1 — `properties()` map projection |
|        2 | Path2         | PA1 — `relationships(p)` projection |

*O1 fixed +3 (ReturnOrderBy1[9,10] + WithOrderBy1[?]) — 2 ReturnOrderBy1 and ~1 WithOrderBy1 remain, requiring L2 for DB variables in UNWIND lists.*  
*A1 fixed +3 (Aggregation2[9,11,12]) — fully resolved.*

The top 3 buckets (T1 + Q1 + LC1) account for **~117 of the 147 failures
(80 %)**. Fixing them would push pass rate to ≈ 99.1 %.

---

## 2. Fix Buckets

Each bucket lists the symptom, root cause, proposed fix, expected gain, and
estimated complexity (S = small / M = medium / L = large).

### Q1 — Quantifier predicates on literal lists  *(DONE: +54; 9 remain)*

**Status**: ✅ SIGNIFICANTLY FIXED. +54 passes from Quantifier9–12.

**What was implemented**: `try_fold_quantifier_invariants()` at the top of `translate_query`. Scans all WITH clauses to build `opaque_vars: HashSet<String>` — variables bound to list comprehensions containing `rand()`, CASE expressions mixing rand/reverse/opaque, or additions of opaque+scalar. Then:
1. Checks the final WITH clause for non-aggregate items → calls `eval_quantifier_tautology(expr, opaque_vars)`
2. `eval_quantifier_tautology` handles: constant-pred (`none(false)=T`, `any(false)=F`, `all(true)=T`, `single(false)=F`) and identity comparisons via `quantifier_canonical`
3. `quantifier_canonical` normalizes to `(list_var, kind:0=none|1=any|2=single, base_pred, negated)` using `all(P)=none(NOT P)` and `NOT none=any` / `NOT any=none`; also handles `size([P])=0/1/size(list)/> 0` patterns
4. Two canonical keys are identical iff `(list_var, kind, base_pred, negated)` match exactly (using `Expression::PartialEq`)

**Remaining** (9 scenarios, all require non-empty list guarantee):
- `none(x IN list WHERE true)` = false → needs non-empty V (Q9[2])
- `any(x IN list WHERE true)` = true → needs non-empty V (Q11[2])
- `all(x IN list WHERE false)` = false → needs non-empty V (Q12[1])
- `single(x IN list WHERE true)` = false → needs size > 1 (Q10[2])
- `any(P) = true WHEN single(P) OR all(P)` → conditional implication (Q11[3]: 5 examples)

These 4 (not 5) constant-pred scenarios where the result depends on list being non-empty/multi-element → require tracking that `list + x` guarantees non-empty. Q11[3] requires implication reasoning, beyond current scope.
by `WHERE … none(x IN list …)` is not reused for `RETURN`.

**Proposed fix**: extend `return_proj.rs` to recognise quantifier expressions
over literal/known-finite lists and lower them to a constant bool expression
(or a `BIND` of the boolean fold) — the same logic already exists in
[functions.rs](src/translator/cypher/functions.rs) for `WHERE` contexts; lift it
into a shared helper `eval_quantifier_const(expr) -> Option<bool>` and call
from both sites.

**Expected gain**: +63 passes (Quantifier9–12 in full).

---

### T1 — Duration arithmetic on temporal values  *(33 failures, complexity L)*

**Symptom** (Temporal8/10): `Result set mismatch`. Example outputs differ in
sub-millisecond components (`'04:44:24.000000003'` vs `'04:44:24.000'`).

**Root cause**: SPARQL's `xsd:dateTime` arithmetic is millisecond-bounded in
Oxigraph; nanosecond preservation requires us to (a) keep a separate
`__nano_remainder` integer alongside each temporal value and (b) reassemble
the lexical form during projection. `temporal.rs` already carries scaffolding
for component splits but does not propagate nanos through `+ duration`/
`- duration`.

**Proposed fix** (two-stage):

1. **Quick win** (≈ 12 of 33): add nanosecond pass-through for the cases
   `time + duration` and `localtime + duration` where the duration is a
   compile-time literal map — the nanos add can be folded statically.
2. **Full fix**: emit a parallel `?_<var>_nanos` companion variable for every
   `time` / `localtime` / `datetime` binding; teach `+`/`-` of duration to
   carry-propagate into seconds; rebuild the output string via `CONCAT`.

**Expected gain**: +12 (stage 1), +33 (full).

---

### LC1 — List comprehension in RETURN/WITH  *(7 failures, complexity M)*

**Symptom**: `Unsupported feature: complex return expression (Phase 4+): <alias>`

**Example** (List12 [1]):
```cypher
RETURN [n IN nodes(p) | n.name] AS oldNames
```

**Root cause**: Phase C list-comprehension support was added for `WHERE`
contexts only. `RETURN`/`WITH` projection of a comprehension is rejected at
the "complex return expression" gate.

**Proposed fix**: in `return_proj.rs::translate_return_item`, detect
`Expression::ListComprehension { … }` and:

* If the source list is statically bounded (literal, `range(c1,c2)`, or a
  named path with known hop count) → unroll to `CONCAT('[', e1, ', ', …, ']')`
  using the existing list-encoding helper.
* Otherwise → emit a sub-`SELECT … (GROUP_CONCAT(IF(pred, expr, UNDEF)) AS
  ?out)` pattern and bind `?out` as the projected variable.

**Expected gain**: +7 passes (List12 full + Path2 [1,2] which use the same
projection on `relationships(p)`).

---

### O1 — Ordering of lists / nulls / mixed types  *(DONE: +3; 5 remain)*

**Symptom** (ReturnOrderBy1 [9,10] / WithOrderBy1 [9,10]):
`Row 0 mismatch: got [Some("['a', 1]")], expected [Some("[]")]`

**Status**: ✅ FIXED for literal-list UNWIND variables (ReturnOrderBy1[9,10] + WithOrderBy1 partial).

**What was implemented**: parallel `?__sk_<var>` column emitted alongside UNWIND list variables, using type-ranked prefix encoding (`map(0) < list(3) < string(5) < bool(6/7) < int(8) < null(Z)`). ORDER BY redirected to sort-key variable. WITH…ORDER BY fix propagates sort-key into inner SELECT via `list_sort_key_vars`. Sort key tracked in `TranslationState::list_sort_key_vars`.

**Remaining** (ReturnOrderBy1 [11,12] + some WithOrderBy1): UNWIND list contains DB variables (`x` bound by `MATCH`). Requires L2 continuation — phase1 fetches values, continuation sorts them in Rust.

**Original expected gain**: +8 passes. **Achieved**: +3. **Remaining**: +5 (L2).

---

### A1 — `min()` / `max()` over heterogeneous values  *(DONE: +3; fully resolved)*

**Symptom** (Aggregation2 [9–12]): `Result set mismatch`.

**Status**: ✅ FULLY FIXED. Aggregation2 [9,11,12] now pass.

**What was implemented**: `try_fold_minmax_aggregate()` method on `TranslationState`. Detects the pattern `UNWIND [lits] AS x RETURN min(x)` / `RETURN max(x)` (exactly 2 clauses, all RETURN items are aggregates over the UNWIND variable). If the list contains only literals (including mixed types, nulls, nested lists), the extremum is computed at translation time via `cypher_compare()` (a pure Rust Cypher total-order comparator) and the result is emitted as a constant `VALUES (?col) { (result) }` pattern. `literal_expr_to_ground_term()` converts AST literals to `spargebra::GroundTerm` for VALUES emission.

**Expected gain**: +3. **Achieved**: +3. **Remaining**: 0.

---

### S1 — `SET n.prop = list_expression`  *(3 failures, complexity S)*

**Symptom**: `complex return expression (Phase 4+): x` after a `SET` clause
that assigns a list literal.

**Root cause**: `SET` skip_writes mode does not register the list value into
`node_props_from_create`, so subsequent reads fall to the generic "complex
expression" reject.

**Proposed fix**: in [src/translator/cypher/clauses.rs](src/translator/cypher/clauses.rs)
`Set` handling, propagate `Expression::List` values through
`set_tracked_vars` analogously to scalar assignments; teach the projection
path to re-serialise via `serialize_list_literal`.

**Expected gain**: +3.

---

### M1 — MERGE relationship matching  *(5 failures, complexity M)*

Mixed: 4 are `Result set mismatch` (the MERGE side-effect prediction differs
from actual), 1 is `property access on non-variable base expression`.

**Proposed approach**: triage individually after Q1+T1+LC1 land — these
benefit from improved diagnostics added in those buckets.

---

### G1, PA1, P1, UV (UNWIND of variable, 30 failures)

* **G1** (`properties()` returning a map literal): same projection path as
  LC1; `RETURN properties(n)` should emit `CONCAT('{', key1, ': ', val1, …)`.
  +2 passes once LC1's encoding helper is generalised.
* **PA1**: subset of LC1 once `relationships(p)` projection works.
* **P1**: 3VL precedence edge cases; small targeted fix in
  [src/translator/cypher/mod.rs](src/translator/cypher/mod.rs) boolean
  rewrites.
* **UV** (UNWIND of a non-literal variable): documented in
  [plans/fundamental-limitations.md](plans/fundamental-limitations.md) as L2/L3
  territory — requires runtime round-trip. Mark as out-of-scope for the
  static transpiler unless engine extensions land.

---

## 3. Suggested Order of Work

Sequence chosen to maximise pass-rate gain per unit effort:

**Completed:**
- ✅ **O1** — list/null ordering (+3 of +8; started with literal UNWIND lists). See commit `3a84db5`.
- ✅ **A1** — `min`/`max` over heterogeneous types (+3 of +3; fully resolved). See commit `17fe831`.

**Remaining:**
1. **Q1** — quantifiers on literal lists (+63, M) → **~98.3 %**
2. **LC1** — list comprehension projection (+7, M) → **~98.5 %**
3. **G1 + PA1** — `properties()` and `relationships(p)` (+4, S after LC1) → **~98.6 %**
4. **S1 + P1** — list SET + 3VL precedence (+5, S/M) → **~98.8 %**
5. **T1 stage 1** — temporal nanos for literal-duration cases (+12, M) → **~99.1 %**
6. **M1** — MERGE relationship matching (+5, M) → **~99.2 %**
7. **T1 stage 2** — full nanosecond tracking (+21, L) → **~99.8 %**
8. **O1 remainder** — UNWIND of DB variable in ORDER BY (+5, requires L2) → full O1
9. **UV** — UNWIND of variable: defer; requires L2/L3 mitigation
   ([plans/fundamental-limitations.md](plans/fundamental-limitations.md)).

---

## 4. Architecture & Tooling Improvements

The investigation revealed several friction points that slow down each
fix-and-test cycle. Addressing these will materially speed up the work above.

### 4.1 Split `mod.rs` (4059 lines) and `temporal.rs` (3353 lines)

Both files are now over 3 KLOC and dominate compile times in the translator
crate. Phase F already split clauses/patterns/etc. out; the remaining
monoliths should follow the same recipe:

* `mod.rs` → split into `mod.rs` (≤ 600 L: public API + state struct), plus
  `expr.rs` (~ 1500 L: `translate_expr` and helpers), `consts.rs` (IRIs),
  `eval.rs` (compile-time evaluators `try_eval_to_*`).
* `temporal.rs` → group by feature: `parse.rs`, `arithmetic.rs`,
  `extract.rs`, `format.rs` (~ 800 L each).

Benefit: parallel compilation of independent sub-modules; easier to test
individual lowering passes.

### 4.2 Replace `include!()` with proper `mod` declarations

`mod.rs` uses `include!("clauses.rs")` etc. — this defeats incremental
compilation (any change to `clauses.rs` recompiles all of `mod.rs`'s
4059 lines because they share a single translation unit). Convert each
included file into a real submodule with `pub(super)` items. This alone
should cut translator rebuild time by 50–70 %.

### 4.3 TCK runner: replace 64 MB-stack workaround with iterative algorithm

[tests/tck/main.rs](tests/tck/main.rs) currently spawns a 64 MB stack thread
to avoid stack overflow in deep async/cucumber recursion. The real fix is to
ensure the cucumber `World` impl does not transitively recurse on every step;
investigate whether the 27 KB SPARQL strings cause `Display` to recurse
(they do — `spargebra::algebra::Expression` is recursive via `Box<Expression>`
and serialises by recursive `Display`). Convert the SPARQL string formatter
to an iterative writer; remove the 64 MB workaround.

### 4.4 Snapshot-based regression detection

Today, identifying *which* scenarios regressed between two runs requires
manual diffing of `/tmp/tck_out.txt`. Add a small helper:

* Write a JSON manifest `tests/tck/snapshot.json` listing every
  `feature::scenario` with its current pass/fail/skip status.
* Add `cargo xtask tck-snapshot` that compares current run against the
  manifest and prints `+passes`, `-passes`, and `still-failing` lists.
* CI check: PRs must not introduce `-passes`.

This makes the cost of a regression visible immediately and is a prerequisite
for landing the larger T1 / Q1 fixes safely.

### 4.5 Failure clustering tool

The triage above was done by ad-hoc Python over `/tmp/tck_out.txt`. Promote
that script into `tests/tck/cluster.py` (committed) so the same breakdown can
be reproduced after every run. This is what the table in §1 was generated
from.

### 4.6 Make `Result set mismatch` errors actionable

Today the panic message just says `Result set mismatch (sorted):` followed by
`assertion left == right`. Augment the assertion to:

* Print the **generated SPARQL** (already in `world` state).
* Print the **input Cypher**.
* Print a **unified diff** of expected vs actual rows, not the raw `Vec<Row>`
  Debug output.

A 30-line change in [tests/tck/main.rs](tests/tck/main.rs) `then_result_should_be`
pays back on every failed scenario.

### 4.7 Examples directory hygiene

[examples/](examples/) contains 30+ ad-hoc `debug_*.rs` and `test_*.rs` files
left over from past investigations. They compile every time `cargo build`
runs. Move them under `examples/legacy/` with `publish = false`, or delete
those whose corresponding test now passes.

### 4.8 Parser extraction (already planned)

[plans/parser-extraction.md](plans/parser-extraction.md) is still `planned`.
Splitting the 1994-line `parser/cypher.rs` into its own crate would let TCK
runs reuse a single parser build artefact across all 7 test binaries. Worth
revisiting once Q1+LC1 are landed, since those changes touch the AST.

---

## 5. Definition of Done for this Plan

* Q1, LC1, O1, G1+PA1, A1+S1+P1, T1-stage-1, M1 all landed → ≥ 99 % pass rate.
* `mod.rs` < 1000 lines, `include!()` replaced with `mod` declarations.
* TCK runner uses default thread stack (64 MB workaround removed).
* `cargo xtask tck-snapshot` exists and is wired into CI.
* T1-stage-2 and UV documented as accepted limitations or scheduled into a
  separate plan.

When the bucketed work above lands, update
[ROADMAP.md](ROADMAP.md) tracker rows accordingly and bump this plan's status
to `complete`.
