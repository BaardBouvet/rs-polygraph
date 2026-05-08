# Test Harness Consolidation

**Status**: planned
**Updated**: 2026-05-08

Extract the shared Oxigraph execution infrastructure that is currently duplicated
between the TCK runner (`tests/tck/main.rs`) and the difftest harness
(`polygraph-difftest/src/runner.rs`) into a single location.

---

## Problem

Both test runners independently implement the same "transpile → execute in
Oxigraph → collect rows" pipeline.  The duplication is **not** visible in the
public API — it lives entirely in test/dev code — but it is a maintenance hazard:

| Duplicated item | TCK location | Difftest location |
|---|---|---|
| `make_evaluator()` — custom SPARQL function registrations | `tests/tck/main.rs:147` | `polygraph-difftest/src/runner.rs:106` |
| `OxigraphExecutor` / `DifftestOxExecutor` — `SparqlExecutor` wrappers | `main.rs:80` | `runner.rs:53` |
| `TckEngine` / `DifftestEngine` — `TargetEngine` impls | `main.rs:46` | `runner.rs:34` |
| `term_to_value()` and related term-conversion helpers | implicit in TCK hydration | `runner.rs:518` |

Every new custom SPARQL function (e.g. a new duration operator or list helper)
must be registered in **both** places.  This has already caused at least one
drift: the TCK `make_evaluator` adds `use oxigraph::model::Term as OxTerm`
inside each closure while the difftest version uses a top-level import, making
the functions look different at a glance even though they are semantically
identical.

---

## Proposed solution

Move the shared code into `polygraph-difftest` (or a thin sub-module of it),
which is already a `dev-dependency` of the workspace.  The TCK test crate then
imports from `polygraph-difftest`.

### New / changed modules

```
polygraph-difftest/
└── src/
    └── oxigraph_harness.rs    # NEW — shared Oxigraph execution primitives
```

`oxigraph_harness.rs` exports:

```rust
/// Build a `SparqlEvaluator` with all polygraph custom functions registered.
/// Single source of truth; import in both test crates.
pub fn make_evaluator() -> SparqlEvaluator { … }

/// A `TargetEngine` that can be constructed with a base IRI and rdf-star flag.
/// Replaces the hand-written `TckEngine` and `DifftestEngine` structs.
pub struct SimpleEngine {
    pub base_iri: Option<String>,
    pub rdf_star: bool,
}

impl TargetEngine for SimpleEngine { … }

/// Generic Oxigraph-backed `SparqlExecutor`.
/// The caller supplies a `term_to_value` converter so that the TCK and difftest
/// can keep their own `Value` types without a hard dependency.
pub struct OxExecutor<'a, F> {
    store: &'a Store,
    term_to_value: F,
}

impl<'a, F: Fn(&OxTerm) -> Option<String>> OxExecutor<'a, F> {
    pub fn new(store: &'a Store, term_to_value: F) -> Self { … }
}

impl<F: Fn(&OxTerm) -> Option<String>> polygraph::runtime::SparqlExecutor
    for OxExecutor<'_, F> { … }
```

### `tests/tck/main.rs` changes

- Delete local `make_evaluator()`, `TckEngine`, `OxigraphExecutor`.
- Add `polygraph-difftest` as a `dev-dependency` in the workspace `Cargo.toml`
  (it is already in the workspace; just needs to be declared as a dep of the
  `polygraph` package's `[dev-dependencies]` or the integration test's manifest
  if it gets its own).
- Replace all usages with `polygraph_difftest::oxigraph_harness::{make_evaluator,
  SimpleEngine, OxExecutor}`.

### `polygraph-difftest/src/runner.rs` changes

- Delete local `make_evaluator()`, `DifftestEngine`, `DifftestOxExecutor`.
- Replace with imports from `crate::oxigraph_harness`.

---

## Crate dependency implications

`polygraph-difftest` is `dev-only` in the workspace.  Pulling it into
`tests/tck/` does **not** affect the production build graph; both crates are
already compiled for `cargo test` only.

If the TCK test binary cannot take a dependency on the difftest crate
(Cucumber's test harness has constraints on the test binary shape), an
alternative is to extract the shared code into a **private workspace crate**
`polygraph-test-support` (no public API, not published) and have both
`polygraph-difftest` and the TCK runner depend on it.  This is the safer option
if the single-file size of `tests/tck/main.rs` (already > 2500 lines) makes
refactoring risky.

---

## Scope

This is **purely internal** test-infrastructure cleanup.  It:

- Does not change any public API.
- Does not affect TCK pass rate.
- Does not affect the difftest curated suite results.
- Must not be blocked on any other planned work.

It **should** be done before v0.10.0 or v0.11.0 adds more custom SPARQL
functions (each new function currently requires two identical edits).

---

## Exit criterion

- `make_evaluator()` exists in exactly **one** place in the codebase.
- `cargo test` and `cargo test -p polygraph-difftest` both pass without change.
- `grep -r "make_evaluator" --include="*.rs" | wc -l` returns 1 (definition) +
  N call sites (no second definition).
