# Cleanup and Improvement Assessment

**Status**: reference
**Updated**: 2026-05-08

This document catalogues concrete cleanup and improvement opportunities that are
**not already covered** by the existing roadmap (spec-first-pivot, parser-extraction,
l2-runtime-support, iana-timezone, temporal-cleanup, test-harness-consolidation,
pg-extension-protocol).  Items are ordered roughly by risk/effort, lowest first.

---

## 1. Dead source files in `crates/polygraph/src/ast/`

**Files**: [crates/polygraph/src/ast/cypher.rs](../crates/polygraph/src/ast/cypher.rs),
[crates/polygraph/src/ast/gql.rs](../crates/polygraph/src/ast/gql.rs)

**Issue**: `ast/mod.rs` is a pure re-export shim over `opencypher_parser::ast`.
It contains no `mod cypher` or `mod gql` declaration, so the two `.rs` files are
never compiled.  They are dead filesystem artefacts — leftover originals from
before the parser-extraction work copied the definitions into
`crates/opencypher-parser/`.

**Why it matters**: Editors and `rust-analyzer` may surface confusing
hover-to-definition results pointing at the dead copies. Contributors editing the
AST in `opencypher-parser` may not notice the shadowing files.

**Fix**: Delete both files.  This is safe immediately; it does not need to wait
for v0.9.0.  The re-export in `ast/mod.rs` already provides all needed types.

---

## 2. Stale README / usage example

**Files**: [README.md](../README.md), [crates/polygraph/README.md](../crates/polygraph/README.md)

**Issue**: Both READMEs (root and crate) show the same stale usage example:
```rust
use polygraph::{Transpiler, TargetEngine};
let sparql = Transpiler::to_sparql(cypher, TargetEngine::Oxigraph)?;
```
Neither `Transpiler::to_sparql` nor `TargetEngine::Oxigraph` exist.  The correct
API since v0.7.0 is `Transpiler::cypher_to_sparql(cypher, &engine)` with an
engine from `sparql_engine::GenericSparql11` / `RdfStar`.  A user who copies the
example gets a compile error on their first try.

**Fix**: Update both READMEs to use the real API.  While there, update the Project
Structure section to reflect that `polygraph::target` no longer exists as a
top-level module (it is now `sparql_engine`).

---

## 3. `#[must_use]` missing on `TranspileOutput`

**File**: [crates/polygraph/src/result_mapping/mod.rs](../crates/polygraph/src/result_mapping/mod.rs)

**Issue**: `TranspileOutput` is the primary return value of every transpilation
call.  It is not annotated with `#[must_use]`, so the compiler and Clippy produce
no warning if a caller writes:
```rust
Transpiler::cypher_to_sparql(q, &engine);  // silently discarded
```
This is an especially bad footgun for `Continuation` variants, where discarding
the output silently skips all phases of a multi-phase query.

**Fix**: Add `#[must_use = "transpilation result must be executed"]` to the enum
declaration.  Also consider adding it to `drive()` in `runtime.rs` for the same
reason.

---

## 4. Truly dead constant `XSD_STRING` in `lqa/sparql.rs`

**File**: [crates/polygraph/src/lqa/sparql.rs](../crates/polygraph/src/lqa/sparql.rs) (line 79)

**Issue**: `const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string"` is
defined with `#[allow(dead_code)]` and never referenced anywhere.  The `#[allow]`
is a sign this was a known issue left unaddressed.

**Fix**: Delete the constant.  If it is ever needed for plain-literal typing, it
can be re-added at that point.  While here, check whether the `#[allow(dead_code)]`
on the adjacent `edge_types` struct field (line 127) is also stale — `edge_types`
is accessed on lines 1882, 5363, and 5413, so that suppression can be removed too.

---

## 5. `unwrap_complete()` panics in public API

**File**: [crates/polygraph/src/result_mapping/mod.rs](../crates/polygql/src/result_mapping/mod.rs) (lines 112–114)

**Issue**: `TranspileOutput::unwrap_complete()` is a `pub` method that calls
`panic!()` when the variant is `Continuation` or `Write`.  This violates the
project rule "never panic in library code".

**Fix**: Either:
- Make `unwrap_complete` `pub(crate)` (it is only used internally in tests), or
- Replace the `panic!` body with a proper `Result` return type so callers can
  handle unexpected variants without crashing.

---

## 6. `expect()` in parser code (`parser/gql.rs`)

**File**: [crates/polygraph/src/parser/gql.rs](../crates/polygraph/src/parser/gql.rs)

**Issue**: Twelve `.expect("grammar guarantees …")` calls assume that the PEG
grammar will always produce certain child nodes.  While this is almost always true
by grammar construction, any future grammar edit that violates the invariant will
produce a panic rather than a `PolygraphError`.

**Why this category is lower priority**: The grammar is not user-modifiable at
runtime, so these panics can only be triggered by a developer mistake, not by a
library consumer.  Still, a panic is a worse failure mode than an `Err`.

**Fix**: Progressively replace with `ok_or_else(|| PolygraphError::Internal { … })`.
Create an `InternalError` variant in `PolygraphError` for "grammar invariant
violation" — this is both honest and searchable in logs.

---

## 7. Missing workspace-level `[lints]` table

**File**: [Cargo.toml](../Cargo.toml) (workspace root)

**Issue**: The workspace `Cargo.toml` has no `[workspace.lints]` section.
Individual crates carry scattered `#[allow(clippy::…)]` attributes with no central
policy:
- `#[allow(clippy::type_complexity)]` in `parser/cypher.rs`, `lqa/lower.rs`,
  `translator/cypher/return_proj.rs`
- `#[allow(clippy::too_many_arguments)]` in `lqa/write.rs` (×2),
  `translator/cypher/patterns.rs`
- `#[allow(clippy::should_implement_trait)]` in `lqa/expr.rs`, `lqa/normalize.rs`

The CI `clippy` job runs with `-D warnings`, which is correct, but there is no
way to see the project-wide lint policy without grepping every file.

**Fix**: Add a `[workspace.lints.clippy]` table to the root `Cargo.toml` that
documents the intentional blanket allows.  Then inherit it with
`[lints] workspace = true` in each member `Cargo.toml`.  Remove the now-redundant
per-site `#[allow]` attributes.

---

## 8. Duplicate grammar files

**Directories**: [grammars/](../grammars/), [crates/opencypher-parser/grammars/](../crates/opencypher-parser/grammars/)

**Issue**: Both directories contain full copies of `cypher.pest` and `gql.pest`.
`crates/polygraph/src/parser/cypher.rs` references `grammars/cypher.pest` (the
root-level copy) via the `#[grammar = "…"]` pest attribute.
`crates/opencypher-parser/` uses its own copy.

This was the correct interim state during the parser-extraction work, but it means
any grammar bugfix must be applied twice.

**Fix**: This is already targeted by the v0.9.0 parser-extraction plan, but the
specific migration step (update `#[grammar]` path in the polygraph parser to
reference the opencypher-parser copy, then delete the root `grammars/`) should be
filed as an explicit checklist item in [parser-extraction.md](parser-extraction.md)
to prevent it from being overlooked.

---

## 9. CI: no MSRV check

**File**: [.github/workflows/ci.yml](../.github/workflows/ci.yml)

**Issue**: `Cargo.toml` declares `rust-version = "1.80"` for both published
crates, but the CI matrix only tests `[stable, beta]`.  If a contributor
accidentally uses a Rust 1.81+ feature (e.g., a new standard library API), CI
passes but consumers on the declared MSRV break.

**Fix**: Add a third matrix entry `msrv` that pins to `1.80`:
```yaml
matrix:
  toolchain: [stable, beta, "1.80"]
```
No separate job is needed; the existing `test` job already runs unit and integration
tests.  The difftest and TCK jobs do not need to run on MSRV (they are development
tools, not public API).

---

## 10. CI: benchmarks not run on CI

**File**: [benches/transpilation.rs](../benches/transpilation.rs)

**Issue**: The single benchmark (`parse_simple_match_return`) is never compiled or
run in CI.  A refactor that breaks the benchmark signature will only be discovered
locally.  More importantly, the benchmark only measures parsing, not
transpilation — so the cost of the LQA lowering + SPARQL serialization pipeline is
unmeasured.

**Fixes**:
- Add `cargo bench --no-run` to CI to catch compilation failures without the
  runtime overhead of actually running benchmarks.
- Add benchmarks for end-to-end transpilation: a simple `MATCH/RETURN`, an
  aggregate (`GROUP BY`), a variable-length path, and an RDF-star vs. reification
  comparison.  These give a regression floor when the LQA pipeline is restructured
  for v0.10.0.

---

## 11. Duplicated TCK test binary declarations

**File**: [crates/polygraph/Cargo.toml](../crates/polygraph/Cargo.toml)

**Issue**: The `[[test]]` section repeats the same `path = "../../tests/tck/main.rs"`
eight times:
```
tck, tck_clauses, tck_write_clauses, tck_expressions_agg,
tck_expressions_heavy, tck_expressions_misc, tck_expressions_temporal, tck_usecases
```
Each entry compiles the same source file as a distinct binary, presumably to
parallelise the TCK suite by feature-file filter.  The mechanism works but there
is no comment explaining the filter logic, and the naming scheme diverges from the
actual feature directory structure.

**Fix**: Add a comment block above the `[[test]]` entries explaining how the
feature-file filtering works (environment variables? cargo features? something in
`main.rs`?).  Consider whether some of the eight binaries could be merged now that
the TCK pass rate is high and CI is fast.

---

## 12. Missing top-level re-exports for `TargetEngine` / `GenericSparql11` / `RdfStar`

**File**: [crates/polygraph/src/lib.rs](../crates/polygraph/src/lib.rs)

**Issue**: The Rust API Guidelines recommend that types used by most consumers be
accessible directly from the crate root.  Currently, users must write
`polygraph::sparql_engine::GenericSparql11` and
`polygraph::sparql_engine::TargetEngine` — both of which appear in nearly every
`Transpiler::cypher_to_sparql` call site.  There are no re-exports for these at
the crate root.  `SparqlExecutor` (for `runtime::drive`) and `ProjectionSchema`
(for column inspection) are also absent.

**Fix**: Add to `lib.rs`:
```rust
pub use sparql_engine::{GenericSparql11, RdfStar, TargetEngine};
pub use runtime::SparqlExecutor;
```
(`ProjectionSchema` is already re-exported via `result_mapping`.)

---

## 13. `is_localdatetime` field — stale `#[allow(dead_code)]`

**File**: [crates/polygraph/src/translator/cypher/temporal.rs](../crates/polygraph/src/translator/cypher/temporal.rs) (lines 19–20)

**Issue**: `TcComponents::is_localdatetime` has `#[allow(dead_code)]` but the
field is set in two branches (lines 157, 199) and never read.  The field was
presumably intended for a timezone-adjustment rule that was not yet implemented.

**Fix**: This is in the legacy translator that v0.9.0 will delete; however since
the deletion is still weeks away, the attribute should be replaced with a comment:
```rust
/// Set but not yet read; reserved for the localtime → UTC-Z conversion
/// rule that will be implemented when IANA timezone support lands.
pub(crate) is_localdatetime: bool,
```
This makes the intent explicit and avoids misleading future readers.

---

## 14. `opencypher-parser` crate missing `[package.metadata.docs.rs]`

**File**: [crates/opencypher-parser/Cargo.toml](../crates/opencypher-parser/Cargo.toml)

**Issue**: The `opencypher-parser` crate is missing the docs.rs configuration
present in the main crate.  When published to crates.io, docs.rs will use
its default settings and may not build all features.

**Fix**: Add:
```toml
[package.metadata.docs.rs]
all-features = true
```

---

## 15. `lqa/write.rs` — `too_many_arguments` suppressions indicate missing struct abstraction

**File**: [crates/polygraph/src/lqa/write.rs](../crates/polygraph/src/lqa/write.rs) (lines 1238, 1358)

**Issue**: Two functions suppress `clippy::too_many_arguments` rather than
grouping their parameters.  This is not urgent while those functions work, but is
a maintenance risk: callers can pass arguments in the wrong positional order and
the compiler cannot detect it.

**Fix**: Group the shared write-context fields (`base_iri`, `rdf_star` flag, counter,
etc.) into a `WriteCtx` struct and thread it through, replacing the individual
arguments.  This is a refactor best done as part of the LQA write-path work in
v0.10.0.

---

## Priority summary

| # | Item | Effort | Risk | When |
|---|------|--------|------|------|
| 1 | Delete dead `ast/cypher.rs` + `ast/gql.rs` | trivial | none | now |
| 2 | Fix stale README usage example | trivial | none | now |
| 3 | `#[must_use]` on `TranspileOutput` | trivial | none | now |
| 4 | Delete `XSD_STRING`; remove stale `#[allow(dead_code)]` on `edge_types` | trivial | none | now |
| 5 | `unwrap_complete()` panic → `pub(crate)` or `Result` | small | low | now |
| 12 | Add `TargetEngine`/`GenericSparql11` re-exports to crate root | small | low | now |
| 9 | MSRV check in CI | small | none | now |
| 10 | `cargo bench --no-run` in CI + expand benchmarks | small | none | now |
| 7 | Workspace `[lints]` table | medium | none | before v0.9.0 |
| 6 | Replace parser `expect()` with `PolygraphError::Internal` | medium | low | v0.9.0 |
| 11 | Document / clean up TCK binary declarations | small | none | v0.9.0 |
| 13 | Document `is_localdatetime` intent | trivial | none | before legacy deletion |
| 14 | docs.rs metadata for `opencypher-parser` | trivial | none | before v0.9.0 publish |
| 8 | Grammar dedup migration step in parser-extraction plan | small | none | v0.9.0 |
| 15 | `WriteCtx` struct for `lqa/write.rs` | medium | low | v0.10.0 |
