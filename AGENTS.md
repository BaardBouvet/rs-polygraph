# rs-polygraph — Agent Guidelines

## Project Context

`rs-polygraph` is a Rust library that transpiles **openCypher** and **ISO GQL** property graph queries into **SPARQL 1.1** (and SPARQL-star) algebra. The output targets any SPARQL-compliant engine without modifying those engines.

See [plans/implementation-plan.md](plans/implementation-plan.md) for detailed design decisions and [ROADMAP.md](ROADMAP.md) for phased delivery.

## Architecture

```
Input query (Cypher / GQL)
       │
   [parser]        pest PEG grammars → typed AST
       │
   [translator]    visitor pattern → spargebra GraphPattern
       │
   [rdf_mapping]   RDF-star or reification encoding for edge properties
       │
   [target]        TargetEngine trait — engine-specific finalization
       │
  SPARQL string / algebra
```

Key modules: `ast`, `parser`, `translator`, `rdf_mapping`, `target`. See `src/` layout in the implementation plan.

## Build and Test

```sh
cargo build
cargo test
cargo test --test tck        # TCK compliance suite
cargo bench                  # criterion benchmarks
```

The TCK test suite (`tests/tck/`) uses the `cucumber` crate and requires the openCypher TCK Gherkin files at `tests/tck/features/`.

## Code Conventions

- **Errors**: All public APIs return `Result<T, PolygraphError>` via `thiserror`. Never panic in library code.
- **No unsafe**: This crate must be `#![forbid(unsafe_code)]`.
- **Visitor pattern**: Translators implement the `AstVisitor` trait. Do not add ad-hoc match arms outside visitor impls.
- **Engine capabilities**: Always consult `TargetEngine::supports_rdf_star()` before emitting SPARQL-star syntax. Fall back to reification when false.
- **Span preservation**: Parser errors must include source spans from `pest`. Do not discard span info during AST construction.
- **Grammar files**: openCypher grammar lives in `grammars/cypher.pest`, GQL in `grammars/gql.pest`. Edits to grammars require regenerating parser tests.

## Dependencies

Prefer existing dependencies over adding new ones. Core crates: `pest`, `spargebra`, `oxigraph` (dev/integration only), `thiserror`, `cucumber` (dev), `criterion` (dev). Any new dependency requires justification in the PR.

## Testing Requirements

- Every new AST node type needs a unit test in its module.
- Every new translator mapping needs an integration test asserting the SPARQL output.
- TCK pass rate must not regress. Track compliance percentage in `ROADMAP.md`.
