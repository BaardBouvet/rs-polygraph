# tools/

Scripts that support the [spec-first-pivot.md](../plans/spec-first-pivot.md)
methodology. They operate on the JSONL result file emitted by the TCK harness
when `POLYGRAPH_TCK_RESULTS_PATH` is set (see [tests/tck/main.rs](../tests/tck/main.rs)
for the writer).

## tck_diff.sh

Compare a fresh TCK run against the frozen baseline at
[tests/tck/baseline/scenarios.jsonl](../tests/tck/baseline/scenarios.jsonl).

```sh
tools/tck_diff.sh                # run TCK, diff against baseline; exit 1 on regression
tools/tck_diff.sh --freeze       # overwrite the baseline with the current run
tools/tck_diff.sh --against FILE # diff a previously-captured JSONL
```

Output classifies every per-scenario change as one of:

- **REGRESSIONS** — was-pass, now-fail/skip. Exit code 1.
- **Improvements** — was-fail/skip, now-pass.
- **Added/removed scenarios** — TCK feature corpus changed (uncommon).

The baseline file is committed; updating it requires `--freeze` and a deliberate
PR.
