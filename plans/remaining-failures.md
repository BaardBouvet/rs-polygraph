# Remaining TCK Failures — Triage and Fix Plan

**Status**: in progress
**Updated**: 2026-05-05
**Baseline**: 3437 / 3739 scenarios pass (95.8 %), 154 failed, 148 skipped.
**Current**: 3488 / 3789 scenarios pass (92.1 %), 153 failed, 148 skipped.

### Fixes applied since initial baseline

| Scenarios | What was fixed |
|----------:|----------------|
| +3 | String8/9/10 [8]: STARTS WITH/ENDS WITH/CONTAINS falsely matching list/map encoded strings. Added content guard `!STRSTARTS(x,"[") && !STRSTARTS(x,"{")` in StartsWith/EndsWith/Contains translator. |
| +1 | Temporal1 [11]: `datetime.fromepoch(s, ns)` and `datetime.fromepochmillis(ms)` not implemented. Added compile-time epoch→ISO-8601 conversion using proleptic Gregorian calendar in `temporal.rs`. |
| +2 | Set1 [6,7]: `SET a.prop = a.prop + [4, 5]` (list concat in SET) returned stale CREATE value. Fixed by pre-scanning CREATE properties before SET in skip_writes mode, then folding self-referential list concatenations statically. |

> Note: total scenario count increased by 50 (new TCK features scanned);
> Pattern1/2 appear as new pre-existing failures unrelated to these changes.

This plan triages the **154 remaining failures** into actionable groups and
proposes architecture improvements that would make further iteration faster.

---

## 1. Failure Distribution

By error category (from `/tmp/tck_out.txt`, parsed by feature):

| Count | Category                                              | Likely tractable? |
|------:|-------------------------------------------------------|------------------:|
|    51 | `Unsupported feature: complex return expression …`    | L2 (runtime/path operations) |
|    48 | `Result set mismatch` (translation succeeded, wrong result) | mostly L2 |
|    30 | `Unsupported feature: UNWIND of variable …`           | L2 deep design issue |
|    11 | `Row count mismatch`                                  | L2 mostly aggregation/temporal |
|     4 | `Row value mismatch`                                  | L2 list-ordering semantics |
|     1 | `Unsupported feature: list comprehension`             | L2 |
|     1 | other                                                 | — |

Top features (failures per file):

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
|        4 | ReturnOrderBy1| O1 — list/null ordering |
|        4 | WithOrderBy1  | O1 |
|        3 | Aggregation2  | A1 — `min`/`max` over heterogeneous types |
|        3 | Set1          | S1 — list-valued SET |
|        2 | Precedence1   | P1 — boolean 3VL precedence edge cases |
|        2 | Graph9        | G1 — `properties()` map projection |
|        2 | Path2         | PA1 — `relationships(p)` projection |

The top 4 buckets (T1 + Q1 + LC1 + O1) account for **123 of the 154 failures
(80 %)**. Fixing them would push pass rate to ≈ 99.2 %.

---

## 2. Fix Buckets

Each bucket lists the symptom, root cause, proposed fix, expected gain, and
estimated complexity (S = small / M = medium / L = large).

### Q1 — Quantifier predicates on literal lists  *(63 failures, complexity M)*

**Symptom**: `Unsupported feature: complex return expression (Phase 4+): list`

**Example** (Quantifier9 [1]):
```cypher
RETURN none(x IN [1, 2, 3] WHERE x = 4) AS result
```

**Root cause**: when a `none/any/single/all` quantifier is the top-level
expression in a `RETURN` item and its source is a *literal* list, the
translator falls through to the generic "complex expression" branch which
rejects unknown projection shapes. The compile-time list expansion path used
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

### O1 — Ordering of lists / nulls / mixed types  *(8 failures, complexity S)*

**Symptom** (ReturnOrderBy1 [9,10] / WithOrderBy1 [9,10]):
`Row 0 mismatch: got [Some("['a', 1]")], expected [Some("[]")]`

**Root cause**: SPARQL `ORDER BY` over our serialised string-list encoding
sorts lexicographically, so `"['a', 1]"` < `"[]"`. Cypher orders lists by
length first, then element-wise.

**Proposed fix**: when ORDER BY references a list-valued variable, emit an
ORDER BY sequence of `(STRLEN(?v_serialised), ?v_serialised)`. For mixed-type
ordering (ReturnOrderBy1 [11,12]), prepend a type-rank prefix during the
encoding (e.g. `"4|" + str` for lists, `"3|" + str` for strings, …) and strip
it during projection.

**Expected gain**: +8 passes.

---

### A1 — `min()` / `max()` over heterogeneous values  *(3 failures, complexity M)*

**Symptom** (Aggregation2 [9–12]): `Result set mismatch`.

**Root cause**: Cypher's `min`/`max` use a total order across types
(NULL < boolean < integer/float < string < list); SPARQL's MIN/MAX returns
type-erroring or platform-specific results when inputs mix types. Our
translation emits raw `MIN(?x)` which Oxigraph evaluates as undefined.

**Proposed fix**: emit a wrapper that maps each value to a sortable string
(`type_rank + lexical_form`) and `MIN(wrapped)` / `MAX(wrapped)`, then
unwrap in the projection. Reuse the type-rank ladder from O1.

**Expected gain**: +3.

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

1. **Q1** — quantifiers on literal lists (+63, M) → **98.5 %**
2. **LC1** — list comprehension projection (+7, M) → **98.7 %**
3. **O1** — list/null ordering (+8, S) → **99.0 %**
4. **G1 + PA1** — `properties()` and `relationships(p)` (+4, S after LC1) → **99.1 %**
5. **A1 + S1 + P1** — small targeted fixes (+8, S/M) → **99.3 %**
6. **T1 stage 1** — temporal nanos for literal-duration cases (+12, M) → **99.6 %**
7. **M1** — MERGE relationship matching (+5, M) → **99.7 %**
8. **T1 stage 2** — full nanosecond tracking (+21, L)
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
