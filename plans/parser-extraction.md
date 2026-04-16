# Parser Extraction — Standalone `cypher-parser` / `gql-parser` Crate

**Status**: planned  
**Updated**: 2026-04-15

> See also the discussion notes appended at the end of this document for context on the spargebra comparison.

---

## Summary

The parser and AST layers of `rs-polygraph` have **zero coupling** to SPARQL or any downstream translation logic. Extracting them into a standalone crate would let other projects (graph analytics, linters, migration tools, alternative backends) parse openCypher and GQL queries without pulling in `spargebra` or any SPARQL machinery.

**Verdict: Yes, extraction is a good idea.** The architecture already enforces a clean boundary; the work is mostly mechanical.

---

## Dependency Analysis

### What would move to the new crate

| Component | External deps | Internal deps | SPARQL coupled? |
|-----------|--------------|---------------|-----------------|
| `grammars/cypher.pest` | — | — | No |
| `grammars/gql.pest` | — | — | No |
| `src/ast/cypher.rs` | none | none | No |
| `src/ast/gql.rs` | none | `ast::cypher::Clause` | No |
| `src/ast/mod.rs` | none | re-exports | No |
| `src/parser/cypher.rs` | `pest`, `pest_derive` | `ast::cypher`, `error` | No |
| `src/parser/gql.rs` | `pest`, `pest_derive` | `ast::cypher`, `ast::gql`, `error` | No |
| `src/parser/mod.rs` | none | re-exports | No |
| `src/error.rs` (subset) | `thiserror` | none | No (see §Error Split) |

### What stays in `polygraph`

| Component | Key deps |
|-----------|----------|
| `translator/` | `spargebra`, AST (read-only) |
| `rdf_mapping/` | `spargebra` |
| `result_mapping/` | — |
| `sparql_engine/` | — |
| `lib.rs` (transpiler API) | new parser crate + `spargebra` |

### Dependency graph (post-extraction)

```
  ┌──────────────────────────────────────┐
  │  opencypher-parser  (new crate)      │
  │  ┌──────────┐  ┌──────────────────┐  │
  │  │ ast/     │  │ parser/          │  │
  │  │ cypher   │◄─┤ cypher.rs        │  │
  │  │ gql      │  │ gql.rs           │  │
  │  └──────────┘  └──────────────────┘  │
  │  deps: pest, pest_derive, thiserror  │
  └──────────────────┬───────────────────┘
                     │  (re-exported types)
  ┌──────────────────▼───────────────────┐
  │  polygraph  (existing crate)         │
  │  translator/ ─► spargebra            │
  │  rdf_mapping/                        │
  │  result_mapping/                     │
  │  sparql_engine/                      │
  │  deps: opencypher-parser, spargebra, ... │
  └──────────────────────────────────────┘
```

---

## Error Type Split

`PolygraphError` currently has four variants:

```rust
pub enum PolygraphError {
    Parse { span, message },         // parser-only
    UnsupportedFeature { feature },  // parser + translator
    Translation { message },         // translator-only
    ResultMapping { message },       // result-mapping-only
}
```

**Approach**: The new crate defines a `ParseError` enum with `Parse` and `UnsupportedFeature` variants. `polygraph` defines its own `PolygraphError` that wraps or re-exports `ParseError` plus the translator-specific variants. This keeps `From<ParseError> for PolygraphError` trivial and avoids breaking the public API.

```rust
// opencypher-parser/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum ParseError {
    #[error("Parse error at {span}: {message}")]
    Syntax { span: String, message: String },

    #[error("Unsupported feature: {feature}")]
    UnsupportedFeature { feature: String },
}
```

```rust
// polygraph/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum PolygraphError {
    #[error(transparent)]
    Parse(#[from] opencypher_parser::ParseError),

    #[error("Translation error: {message}")]
    Translation { message: String },

    #[error("Result mapping error: {message}")]
    ResultMapping { message: String },
}
```

---

## Naming and Discoverability

### Crate name options

| Crate name | Scope | Searchable for "cypher"? | Notes |
|------------|-------|--------------------------|-------|
| `cypher-parser` | openCypher only | ✅ Yes | Cleanest signal; most likely search term |
| `opencypher-parser` | openCypher only | ✅ Yes | Mirrors the spec name exactly |
| `graph-query-parser` | Both languages | ❌ No | Too generic; won't surface in Cypher searches |
| `polygraph-parser` | Both, branded | ❌ No | Project identity, but invisible to crates.io searches for "cypher" or "gql" |

**Key finding from ecosystem research**: A search for "cypher parser" on crates.io returns 42 results; a search for "opencypher" returns 30. The most-downloaded active competitor (`drasi-query-cypher`, 3k recent downloads) is tied to Microsoft's Drasi execution engine. No standalone, backend-agnostic openCypher parser with a typed AST exists. **The name must contain "cypher" to be discovered.**

**Recommendation**: `opencypher-parser`

- The official spec name is "openCypher" — matches searches for both "cypher parser" and "opencypher"
- Clearly scoped (GQL support is implicit since GQL's AST reuses Cypher types)
- Not tied to any specific backend or project branding
- `polygraph` can still depend on it: `opencypher-parser = { path = "../opencypher-parser" }`

### crates.io metadata (Cargo.toml)

Discoverability on crates.io comes from three sources: crate name, `keywords`, and `categories`. The spec allows up to 5 keywords and maps to a fixed set of categories.

```toml
[package]
name = "opencypher-parser"
description = "Standalone openCypher and ISO GQL parser producing a typed AST. No execution engine required."

keywords = ["cypher", "opencypher", "gql", "graph", "parser"]

categories = ["parser-implementations", "database"]
```

**Why these keywords**:
- `cypher` — matches "cypher parser" search
- `opencypher` — matches "opencypher" search
- `gql` — matches ISO GQL interest
- `graph` — matches graph database tooling searches
- `parser` — matches parser-implementations category browsing

**Comparison with top results on crates.io**:

| Crate | Keywords | Why it surfaces |
|---|---|---|
| `open-cypher` (21 SLoC, abandoned) | `cypher, graph, parser, sql` | Name + keywords |
| `drasi-query-cypher` (active) | `drasi` only | Name contains "cypher" |
| `gdl` (25k downloads) | `graph, gdl, cypher` | High downloads + keywords |

`opencypher-parser` would outrank all of these on relevance for "cypher parser" due to name + full keyword coverage + active maintenance.

---

## Workspace Layout (post-extraction)

Convert the repo to a Cargo workspace:

```
rs-polygraph/
├── Cargo.toml              # [workspace] members = ["crates/*"]
├── crates/
│   ├── opencypher-parser/
│   │   ├── Cargo.toml      # pest, pest_derive, thiserror
│   │   ├── grammars/
│   │   │   ├── cypher.pest
│   │   │   └── gql.pest
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs
│   │       ├── ast/
│   │       │   ├── mod.rs
│   │       │   ├── cypher.rs
│   │       │   └── gql.rs
│   │       └── parser/
│   │           ├── mod.rs
│   │           ├── cypher.rs
│   │           └── gql.rs
│   └── polygraph/
│       ├── Cargo.toml      # opencypher-parser, spargebra, thiserror
│       └── src/
│           ├── lib.rs
│           ├── error.rs
│           ├── translator/
│           ├── rdf_mapping/
│           ├── result_mapping/
│           └── sparql_engine/
├── tests/                  # stays at workspace root
├── benches/
└── examples/
```

---

## Migration Steps

### Step 1 — Convert to Cargo workspace (non-breaking)

1. Create `crates/opencypher-parser/` directory structure.
2. Move `ast/`, `parser/`, grammar files, and the `Parse`/`UnsupportedFeature` error variants.
3. Add a root `Cargo.toml` `[workspace]` section listing both crates.
4. In `opencypher-parser/Cargo.toml`, depend on `pest`, `pest_derive`, `thiserror`.
5. In `polygraph/Cargo.toml`, add `opencypher-parser = { path = "../opencypher-parser" }`.
6. Re-export `opencypher_parser::*` from `polygraph::ast` and `polygraph::parser` so that all existing public types remain accessible at their current paths.

### Step 2 — Update imports in `polygraph`

1. Replace `crate::ast::*` → `opencypher_parser::ast::*` in `translator/`, `rdf_mapping/`, etc.
2. Replace `crate::parser::*` → `opencypher_parser::parser::*` in `lib.rs`.
3. Wrap `ParseError` into `PolygraphError` via `From` impl.

### Step 3 — Update tests

1. Parser unit tests move into `opencypher-parser`.
2. Integration / TCK tests stay in the workspace root, depending on `polygraph`.
3. Verify `cargo test --workspace` passes with zero regressions.

### Step 4 — Grammar path adjustment

`pest_derive` uses `#[grammar = "..."]` relative to `Cargo.toml`. Update the path attribute in the parser files to point to `grammars/cypher.pest` and `grammars/gql.pest` relative to the new crate root.

### Step 5 — Publish

1. Publish `opencypher-parser` to crates.io (it has no path dependencies).
2. `polygraph` depends on the published version.
3. Third-party projects can now `cargo add opencypher-parser` without pulling in `spargebra`.

---

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Breaking public API paths (`polygraph::ast::CypherQuery`) | Medium | Re-export from `polygraph` so old paths still work |
| Grammar `#[grammar = "..."]` path breaks | Low | Test immediately after move; one-line fix |
| GQL's dependency on Cypher AST types forces both into one crate | N/A | Already the plan — both live in `opencypher-parser` |
| Workspace complicates CI / release process | Low | Standard Rust workspace pattern; cargo-release handles it |
| Divergent error types confuse downstream users | Low | `PolygraphError` wraps `ParseError` transparently via `#[from]` |

---

## Who Benefits

| Consumer | Use case | Needs translator? |
|----------|----------|-------------------|
| Query linters / formatters | Parse → AST → validate / pretty-print | No |
| Graph analytics tools | Parse Cypher → custom execution engine | No |
| Migration tools | Parse Cypher → emit SQL / Gremlin / other | No |
| IDE / language server | Parse for syntax highlighting, completion | No |
| This project (`polygraph`) | Parse → translate → SPARQL | Yes (uses both crates) |

---

## Effort Estimate

The refactoring is **mechanical** — no logic changes, no new code:

- **~0 lines of new logic** — only file moves and import rewrites
- **Key files to touch**: `Cargo.toml` (×3), `lib.rs` (×2), `error.rs` (×2), every `use crate::ast` / `use crate::parser` in `translator/`
- **Risk of regressions**: Low — `cargo test --workspace` catches everything

---

## Value-Add APIs for `opencypher-parser`

To be genuinely useful as a standalone crate — comparable to what `spargebra` offers for SPARQL — the extracted crate should include these additions beyond the bare parser/AST.

### Comparison with `spargebra`

`spargebra` is the closest analogue: it parses SPARQL and returns a typed algebra tree with `Display`, `Eq`, `Hash`, and an `on_in_scope_variable` visitor. It does **no constant folding or normalization** — `FILTER (1 + 1 = 2)` is stored as-is. The reason it feels "normalized" is that the SPARQL spec defines its query language directly in terms of algebra operators (`Join`, `LeftJoin`, `Filter`, `Project`…), so there is almost no distance between surface syntax and the algebra. Cypher is the opposite: its high-level syntax implies a rich structure that the translator has to unpack. That gap is what makes transformation passes valuable here.

The key difference in design philosophy: spargebra's AST is an **algebra** (semantically reduced). `opencypher-parser`'s AST is intentionally a **syntax tree** (structurally faithful to what was written). This is a feature — formatters and linters need source fidelity that a pre-normalized tree cannot provide.

### Priority-ordered additions

| Addition | Effort | Value | Notes |
|---|---|---|---|
| `Display` / pretty-printer | Medium | Highest | Enables formatters, round-trip tests, query rewriting |
| Complete `CypherVisitor` / `CypherVisitorMut` traits | Low | High | Every consumer needs tree walks; move out of translator module |
| `variables()` / `bound_variables()` / `projected_variables()` | Low | High | Linters, IDEs, query planners |
| `Eq + Hash` on all AST types | Low | Medium | Caching, deduplication; blocked by `Literal::Float` (use `OrderedFloat`) |
| Serde support (feature-gated) | Low | Medium | Already scaffolded; drop-in derives |
| Semantic validation pass | High | Medium | Variable scope check, aggregate mixing, empty patterns |
| Constant folding pass | Medium | Medium | See §Normalization below |

### Normalization and constant folding

`spargebra` does zero constant folding — this is correct behaviour for a parser crate. But that does **not** mean we should skip it. It means we should implement it as **explicit, opt-in transformation passes** rather than wiring it into `parse_cypher()`.

The model is Rust's `syn` crate: faithful syntax tree by default, separate `visit_mut` passes for rewrites. A formatter must preserve `NOT NOT x` as written; a linter might want to flag `1 + 1` as a constant. Silent eager folding in the parser would destroy source fidelity.

Passes that are **backend-agnostic** (belong in `opencypher-parser`):

```rust
// Implement as CypherVisitorMut — consumer calls explicitly
pub struct ConstantFolder;      // 1 + 1 → 2, true AND x → x
pub struct NegationNormaliser;  // NOT NOT x → x, !(a AND b) → !a OR !b
pub struct AndFlattener;        // (a AND (b AND c)) → (a AND b AND c)
```

Passes that are **backend-specific** (stay in `polygraph` / translator):
- Predicate pushdown — depends on index layout
- Join ordering — depends on cardinality estimates
- Property path merging — depends on SPARQL engine capabilities

### Visitor trait placement

The current `AstVisitor` in `src/translator/visitor.rs` covers only 5 node types and is defined inside the translator module — making it inaccessible to `polygraph-parser` consumers. On extraction:

1. Move a complete read-only `CypherVisitor<Output>` (default no-op for every node) into `opencypher-parser`.
2. Add `CypherVisitorMut` for in-place rewrites.
3. The translator's internal visitor becomes an `opencypher_parser::CypherVisitor` implementor.

---

## Decision

Extraction is recommended when any of these triggers occur:

1. Another project wants to use the parser independently.
2. `spargebra` or other heavy deps slow down compile times for parser-only consumers.
3. The project moves toward a crates.io publish (Phase 8 in ROADMAP).

Until then, the existing module boundary is clean enough that extraction can be deferred without accumulating technical debt. The key invariant to maintain: **parser and AST must never import from `translator`, `rdf_mapping`, or `spargebra`**.

When extraction happens, the value-add APIs above should be included in the same PR — a bare parser/AST with no `Display`, no visitor, and no `Eq+Hash` is a significantly less useful crate than one that ships all of those on day one.

### crates.io ecosystem summary (researched 2026-04-15)

Of the 42 "cypher parser" results on crates.io:
- **`open-cypher`** (3.3k downloads): abandoned 3+ years ago, 21 SLoC, no typed AST — just exposes the raw pest grammar.
- **`drasi-query-cypher`** (4.6k total, 3k recent): actively maintained by Microsoft/Drasi but inseparable from the Drasi continuous-query execution engine.
- **`sparrowdb-cypher`**, **`cypherlite-query`**, **`plexus-parser`**: all brand-new (days/weeks old) and tightly coupled to their own backends.
- **GQL (ISO/IEC 39075)**: no parser crates exist at all.

`opencypher-parser` would be the only standalone, backend-agnostic openCypher+GQL parser with a typed AST on crates.io.
