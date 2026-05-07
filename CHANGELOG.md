# Changelog

All notable changes to `polygraph` are documented here.  
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.7.0] — 2025-08-01 — Spec-anchored LQA + differential testing milestone

This release completes the spec-first pivot (Phase 8): the primary translation
path is now the LQA (Logical Query Algebra) pipeline, spec-anchored against the
openCypher TCK. The legacy direct-to-SPARQL translator is retained as a
fallback for constructs not yet covered by LQA.

### Added
- **250-query differential test suite** (`polygraph-difftest`): curated TOML
  fixtures covering arithmetic, aggregation, CASE, COLLECT, EXISTS, GQL label
  filters, named paths, XOR predicates, and write clauses.  All 250 pass
  end-to-end against Oxigraph.
- `rust-toolchain.toml` pinning to `stable` for reproducible CI builds.
- `keywords`, `categories`, `rust-version`, `readme`, and
  `[package.metadata.docs.rs]` fields in `Cargo.toml` for crates.io quality.

### Changed
- Bumped version from `0.1.0` to `0.7.0` to reflect the significant work
  accumulated since initial publication.
- `cargo clippy -- -D warnings` now passes with zero warnings across all
  source files.  Dead constants, methods, and functions were removed; intentional
  suppressions are annotated with `#[allow]`.
- `cargo fmt` is clean throughout.

### Fixed
- Inner-attribute placement in `translator/cypher/mod.rs` (inner attributes
  must precede outer doc comments).
- Orphaned doc-comment blocks in `lqa/sparql.rs` that triggered
  `clippy::empty_line_after_doc_comments`.
- Manual prefix-strip patterns in `parser/cypher.rs` replaced with
  `str::strip_prefix` chains.

### Known limitations
- 647 TCK scenarios still route through the legacy translator fallback; removing
  it requires the L2 runtime continuation API (planned for v0.8.1).
- `filter_is_null` GQL integration test is a known pre-existing failure
  (inline IS NULL filter not yet lowered by the GQL path).
