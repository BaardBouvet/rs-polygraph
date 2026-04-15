# TCK Parallelism Plan

**Status**: planned  
**Updated**: 2026-04-15

## Problem

`cargo test --test tck` saturates only one CPU core despite `max_concurrent_scenarios(None)`.
With 985 scenarios across 144 feature files this makes the suite slow on multi-core machines.

### Root causes

| Layer | Issue |
|---|---|
| cucumber 0.22 runner | Returns `.boxed_local()` — the entire runner is a `!Send` future pinned to the calling thread |
| `FuturesUnordered` | Provides *cooperative* interleaving, not OS-level parallelism; all scenario futures poll on a single thread |
| Step functions | `having_executed` and `executing_query` call `store.0.update()` / `store.0.query_opt()` synchronously inside `async fn` with no `.await` yield — they never release the executor while doing CPU work |
| `#[tokio::main]` | Spawns a multi-thread runtime but `block_on` with a `!Send` future drives everything from the main thread; the worker pool sits idle |

Raising `max_concurrent_scenarios` to a large number only schedules more scenario futures into `FuturesUnordered` — all still polled from thread 0.

---

## Option A — Feature-directory sharding via multiple `[[test]]` targets (recommended first step)

**Effort**: ~2 hours  
**Gain**: near-linear with number of shards (currently 3 natural top-level groups)

The feature tree has three independent sub-trees:

```
tests/tck/features/
  clauses/       (~9 sub-dirs, ~400 scenarios)
  expressions/   (~11 sub-dirs, ~450 scenarios)
  useCases/      (~1 sub-dir,  ~135 scenarios)
```

Each can become its own `[[test]]` binary that points at the same `main.rs` but reads a different features root via an env var or compile-time constant.

### Changes required

1. **`tests/tck/main.rs`**: read the feature path from env var `POLYGRAPH_TCK_FEATURES_DIR`, defaulting to `"tests/tck/features"`.

   ```rust
   let features_dir = std::env::var("POLYGRAPH_TCK_FEATURES_DIR")
       .unwrap_or_else(|_| "tests/tck/features".to_owned());
   TckWorld::cucumber()
       .max_concurrent_scenarios(None)
       .run(&features_dir)
       .await;
   ```

2. **`Cargo.toml`**: replace the single `[[test]]` entry with four targets:

   ```toml
   [[test]]
   name = "tck"                      # keeps existing default (all features)
   path = "tests/tck/main.rs"
   harness = false

   [[test]]
   name = "tck_clauses"
   path = "tests/tck/main.rs"
   harness = false

   [[test]]
   name = "tck_expressions"
   path = "tests/tck/main.rs"
   harness = false

   [[test]]
   name = "tck_usecases"
   path = "tests/tck/main.rs"
   harness = false
   ```

3. **`.config/nextest.toml`** (new file): configure `cargo-nextest` to inject per-target env vars:

   ```toml
   [[profile.default.overrides]]
   filter = "test(tck_clauses)"
   [profile.default.overrides.env]
   POLYGRAPH_TCK_FEATURES_DIR = "tests/tck/features/clauses"

   [[profile.default.overrides]]
   filter = "test(tck_expressions)"
   [profile.default.overrides.env]
   POLYGRAPH_TCK_FEATURES_DIR = "tests/tck/features/expressions"

   [[profile.default.overrides]]
   filter = "test(tck_usecases)"
   [profile.default.overrides.env]
   POLYGRAPH_TCK_FEATURES_DIR = "tests/tck/features/useCases"
   ```

4. Run with `cargo nextest run --test tck_clauses --test tck_expressions --test tck_usecases`.  
   Nextest runs each binary in its own process — true OS parallelism with no code changes to the test logic.

### Trade-offs

- The top-level `tck` target still works unchanged (`cargo test --test tck`) for CI consistency.
- Adding more shards (e.g., per sub-directory) multiplies the parallelism further at the cost of more `[[test]]` entries; could be scripted.
- Each shard binary is compiled separately but shares the same `main.rs` — zero logic duplication.
- Requires `cargo-nextest`: `cargo install cargo-nextest`.

---

## Option B — `spawn_blocking` inside heavy step functions (complementary to A)

**Effort**: ~1 hour  
**Gain**: within each shard, the oxigraph operations are offloaded to tokio's blocking thread pool, allowing the async event loop to interleave I/O for other scenarios concurrently

The two expensive steps are `having_executed` and `executing_query`. Both call synchronous oxigraph APIs while holding `&mut TckWorld`.

Pattern using `std::mem::take`:

```rust
#[when(regex = r"^executing query:$")]
async fn executing_query(world: &mut TckWorld, step: &Step) {
    if world.skip { return; }
    let cypher = step.docstring.as_deref().unwrap_or("").trim().to_owned();
    let mut store = world.store.take()
        .unwrap_or_else(|| OxStore(Store::new().unwrap()));

    // Move store + cypher into blocking thread; get both back.
    let (store, outcome) = tokio::task::spawn_blocking(move || {
        let sparql = Transpiler::cypher_to_sparql(&cypher, &ENGINE);
        (store, sparql)
    })
    .await
    .expect("spawn_blocking panicked");

    world.store = Some(store);
    match outcome {
        Err(e) => { world.query_error = Some(e.to_string()); }
        Ok(output) => { /* run query against world.store, same as today */ }
    }
}
```

`oxigraph::store::Store` (in-memory, no rocksdb feature) is `Send + Sync`. The `OxStore` wrapper is too. No changes to `TckWorld`'s structure are needed.

When combined with Option A, each shard's `FuturesUnordered` can poll N scenario futures simultaneously, each of which is waiting on a `spawn_blocking` worker — giving real multi-core work within a single shard process.

### Trade-offs

- Requires careful `mem::take` / put-back pattern in two step functions.
- Adds a small overhead per blocking dispatch (~microseconds) — irrelevant given oxigraph costs.
- Does not help if cucumber's event loop is the bottleneck (it isn't — the blocking calls are).

---

## Option C — Custom parallel runner replacing cucumber (maximum parallelism)

**Effort**: ~2–3 days  
**Gain**: true `rayon::par_iter` across all 985 scenarios simultaneously; linear with core count

Replace `tests/tck/main.rs` entirely:

1. Use the `gherkin` crate (already a transitive dependency via cucumber) to parse `.feature` files.
2. Collect all `(Feature, Scenario)` pairs into a `Vec`.
3. Run via `rayon::par_iter()` — each scenario executes on its own rayon worker:
   - Fresh `oxigraph::store::Store::new()` per scenario (cheap, in-memory)
   - Inline the same step-matching logic currently in the cucumber step functions
   - Collect `ScenarioResult { passed | failed(msg) | skipped }` 
4. Print a summary and set process exit code to 1 if any scenario failed.

This removes the cucumber framework entirely and with it the `LocalBoxFuture` / `!Send` constraint.

```rust
let results: Vec<_> = all_scenarios
    .par_iter()
    .map(|(feature, scenario)| run_scenario(feature, scenario))
    .collect();
```

### Trade-offs

- Loses cucumber's step-registration DSL (`#[given]`, `#[when]`, `#[then]`) — step matching becomes a plain `match` or function dispatch table.
- Loses cucumber's retry logic, before/after hooks, and JUnit XML output (though these aren't used today).
- Full rewrite is ~800 lines but the logic is straightforward since there are only ~10 distinct step patterns.
- No runtime dependency on tokio at all for the test binary.

---

## Recommended execution order

| Step | Option | Why |
|---|---|---|
| 1 | A (sharding) | Zero logic risk; immediate 3× speedup with 2 hours of work |
| 2 | B (spawn_blocking) | Complementary; helps within each shard; low risk |
| 3 | C (rayon runner) | Do when the suite grows beyond ~3,000 scenarios and shard count management becomes unwieldy |

Options A and B together should bring wall-clock time from ~N minutes to roughly N/( cores × 0.6 ) minutes on a laptop with 4+ cores, using only conservative estimates.
