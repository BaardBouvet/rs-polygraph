# Scenario-Debt Inventory

**Status**: in progress
**Updated**: 2026-05-04

This file tracks ad-hoc one-off probes accumulated under [examples/](../examples/)
during the TCK-driven development phase. Each entry is a candidate for
**deletion** (covered by an existing test) or **promotion** to a proper unit /
integration / regression test under [tests/](../tests/).

The file is created as a Phase 0 deliverable of [spec-first-pivot.md](spec-first-pivot.md).
It will be drained as part of Phase 4 (audit & delete scenario patches) and
Phase 5 (coverage expansion via differential fuzzing).

## Working agreement

- Do **not** add new files to `examples/`. Write a unit test instead, or a
  curated query under `polygraph-difftest/queries/` (Phase 1).
- When fixing a translator bug, before deleting the corresponding probe here,
  confirm the failure is covered by either a TCK scenario in the baseline or
  an explicit unit/integration test.
- When promoting a probe to a real test, prefer the smallest scope that still
  exercises the bug: unit test in the affected module > integration test in
  `tests/integration/` > differential test in `polygraph-difftest/`.

## Inventory

### Backup files (delete on sight)

- [examples/check_agg.rs.bak.ignore](../examples/check_agg.rs.bak.ignore)
- [grammars/cypher.pest.bak](../grammars/cypher.pest.bak)

### `check_*` probes — verify a single shape works

| File | Likely scope | Action |
|------|--------------|--------|
| examples/check_agg.rs | aggregation | promote → unit test in [src/translator/cypher/return_proj.rs](../src/translator/cypher/return_proj.rs) |
| examples/check_collect.rs | `collect()` aggregation | promote → unit test |
| examples/check_graph3.rs | graph fixture #3 | likely TCK-covered; verify and delete |
| examples/check_orderby.rs | ORDER BY lowering | promote → integration test |
| examples/check_pattern1.rs | pattern parsing | promote → unit test in [src/translator/cypher/patterns.rs](../src/translator/cypher/patterns.rs) |
| examples/check_q11.rs | TCK scenario Q11 | TCK-covered; delete after baseline confirms |
| examples/check_temporal7.rs | temporal #7 | promote → unit test in [src/translator/cypher/temporal.rs](../src/translator/cypher/temporal.rs) |
| examples/check_with2.rs | `WITH` chaining | promote → integration test |
| examples/check_with3.rs | `WITH` chaining | promote → integration test |
| examples/check_withwhere.rs | `WITH ... WHERE` | promote → integration test |

### `debug_*` probes — used to diagnose a specific failure

| File | Likely scope | Action |
|------|--------------|--------|
| examples/debug_collect.rs | collect() bug | TCK-covered; delete |
| examples/debug_comp4.rs | comprehension #4 | promote if not TCK-covered |
| examples/debug_graph6.rs | graph fixture #6 | TCK-covered; delete |
| examples/debug_merge_q.rs | MERGE diagnosis | promote → integration test |
| examples/debug_merge_skip.rs | MERGE skip path | promote → integration test |
| examples/debug_parse.rs | parser diagnosis | delete (replaced by parser unit tests) |
| examples/debug_prec.rs | operator precedence | TCK-covered; delete |
| examples/debug_q10.rs | TCK Q10 | TCK-covered; delete |
| examples/debug_remove3.rs | REMOVE #3 | promote → integration test |
| examples/debug_shadow.rs | scoping shadow | promote → unit test in semantics |
| examples/debug_shadow2.rs | scoping shadow #2 | promote → unit test in semantics |
| examples/debug_sparql.rs | sparql output diff | delete (replaced by integration tests) |
| examples/debug_t5.rs | TCK T5 | TCK-covered; delete |
| examples/debug_with7.rs | WITH #7 | TCK-covered; delete |
| examples/diagnose_q11.rs | TCK Q11 diagnosis | TCK-covered; delete (duplicate of check_q11) |

### `test_*` probes — close to real tests, just misplaced

| File | Likely scope | Action |
|------|--------------|--------|
| examples/test_agg5.rs | aggregation #5 | promote → integration test |
| examples/test_bound.rs | BOUND() function | promote → integration test |
| examples/test_create6.rs | CREATE #6 | promote → integration test |
| examples/test_distinct_collect.rs | DISTINCT + collect() | promote → integration test |
| examples/test_dur.rs | duration arithmetic | promote → unit test in temporal |
| examples/test_if_exec.rs | IF/EXEC | promote → integration test |
| examples/test_merge_ml.rs | MERGE multi-line | promote → integration test |
| examples/test_merge2.rs | MERGE #2 | promote → integration test |
| examples/test_merge5.rs | MERGE #5 | promote → integration test |
| examples/test_null_collect.rs | null + collect() | promote → integration test |
| examples/test_o1.rs | ORDER BY #1 | promote → integration test |
| examples/test_quant.rs | quantifier | promote → integration test (covered by TCK Quantifier1–12) |
| examples/test_remove.rs | REMOVE | promote → integration test |
| examples/test_t8.rs | TCK T8 | TCK-covered; delete |
| examples/test_tck_exact.rs | TCK exact match | TCK-covered; delete |
| examples/test_temporal_add.rs | temporal arithmetic | promote → unit test in temporal |
| examples/test_unwind_var.rs | UNWIND variable | promote → integration test |
| examples/test_unwind_var2.rs | UNWIND var #2 | promote → integration test |
| examples/test_unwind_var3.rs | UNWIND var #3 | promote → integration test |
| examples/test_week.rs | week() function | promote → unit test in temporal |
| examples/sparql_debug.rs | sparql diagnosis | delete |

## Cleanup pass

A bulk deletion pass is **explicitly out of scope for Phase 0**. The files are
preserved here so that, when Phase 4 audits each `// SCENARIO-PATCH(...)` tag
in the translator, the corresponding probe is available for context. Once a
patch is migrated into a normalization rule (and thus deleted), the matching
probe in this inventory should be deleted in the same change.
