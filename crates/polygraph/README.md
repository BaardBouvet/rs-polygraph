# rs-polygraph

**rs-polygraph** is a high-performance Rust library that transpiles graph query languages to SPARQL. It transpiles **openCypher** and **ISO GQL** queries into **SPARQL 1.1** (and SPARQL-star) algebra.

By providing a plug-and-play translation layer, `rs-polygraph` enables any SPARQL-compliant engine—such as [Oxigraph](https://docs.rs/oxigraph), Apache Jena, or Ontotext GraphDB—to support modern property graph query standards without re-implementing their core execution logic.

## Key Features

- **Multi-Language Support**: Support for both the widely-used [openCypher](http://opencypher.org) and the emerging [ISO GQL](https://www.iso.org/standard/76120.html) standards.
- **Engine Agnostic**: Emits standard [SPARQL 1.1 Query Algebra](https://www.w3.org/TR/sparql11-query/) using [Spargebra](https://crates.io/crates/spargebra).
- **RDF-star Ready**: Supports high-efficiency edge property mapping via [RDF-star/SPARQL-star](https://w3c.github.io/rdf-star/cg-spec/), with fallback to standard reification for legacy engines.
- **Rust Native**: Zero-cost abstractions and memory safety, perfect for embedding directly into database kernels.

## Project Structure

The library is modularized to separate parsing from translation logic:

- `polygraph::parser`: Leverages `pest` to generate a clean Abstract Syntax Tree (AST).
- `polygraph::translator`: The core visitor-based logic that transforms Property Graph patterns into RDF triple patterns.
- `polygraph::lqa`: Logical Query Algebra — a typed intermediate representation (Phase 3+) that factors openCypher semantics independently of SPARQL.
- `polygraph::target`: Trait-based interface for easy integration with different SPARQL backends.

### ISO GQL Support

GQL queries are handled via **parse-time normalization** to openCypher equivalents:

- `(n IS Label)` → `(n:Label)` (IS predicate)
- `(n IS Label1 & Label2)` → `(n:Label1:Label2)` (multi-label)
- `-[r IS KNOWS]->` → `-[r:KNOWS]->` (IS edge type)
- `FILTER expr` → `WHERE expr`
- `NEXT` → `WITH *`

This design ensures GQL queries automatically benefit from the same semantic rules and correctness guarantees as openCypher without duplicating translation logic. At present, GQL support covers the ISO 39075 read-query subset that maps to openCypher semantics; advanced GQL constructs (quantified path patterns, full label expression trees) are future work.

## Compliance

The project aims for full compliance with the [openCypher TCK (Technology Compatibility Kit)](https://github.com/opencypher/openCypher/tree/master/tck). We verify our transpilation by running TCK Gherkin scenarios against a reference SPARQL implementation to ensure semantic equivalence.

## Usage

```rust
use polygraph::{Transpiler, TargetEngine};

let cypher = "MATCH (p:Person {name: 'Alice'})-[:KNOWS]->(friend) RETURN friend.name";

// Transpile to a SPARQL string or Algebra object
let sparql = Transpiler::to_sparql(cypher, TargetEngine::Oxigraph)?;

println!("Transpiled SPARQL:\n{}", sparql);
