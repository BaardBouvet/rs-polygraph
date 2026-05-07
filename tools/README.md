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

---

## Fast iteration workflow

### Full-suite run (parallel shards)

Use `cargo nextest` instead of `cargo test`; it runs the 8 TCK shards in
parallel and is typically 3–5× faster end-to-end:

```sh
cargo nextest run -p polygraph
```

### Target a single feature directory

Set `POLYGRAPH_TCK_FEATURES_DIR` to the directory (or file) that contains the
failing scenario so only that subset is exercised. Use an absolute path when
overriding (`.cargo/config.toml` makes the default relative, but manual
overrides must be absolute):

```sh
# All quantifier scenarios
POLYGRAPH_TCK_FEATURES_DIR=/workspaces/rs-polygraph/tests/tck/features/expressions/quantifier \
  cargo test --test tck

# A single feature file
POLYGRAPH_TCK_FEATURES_DIR=/workspaces/rs-polygraph/tests/tck/features/expressions/quantifier/Quantifier1.feature \
  cargo test --test tck
```

### Target a single scenario by name (inner loop)

Set `POLYGRAPH_TCK_FILTER` to a substring of the scenario name. Only
scenarios whose name contains that string will run — all others are skipped
immediately without executing any steps:

```sh
POLYGRAPH_TCK_FILTER="None quantifier on list containing nodes" \
  POLYGRAPH_TCK_FEATURES_DIR=/workspaces/rs-polygraph/tests/tck/features/expressions/quantifier \
  cargo test --test tck
```

Combining both env vars gives the tightest possible loop: compile + run one
scenario in seconds.
