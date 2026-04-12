# Implementation Plan

This document details the technical implementation of `rs-polygraph` — a Rust library that transpiles openCypher and ISO GQL queries into SPARQL 1.1 algebra.

---

## 1. Crate & Module Layout

```
rs-polygraph/
├── Cargo.toml
└── src/
    ├── lib.rs                  # Public API surface
    ├── ast/
    │   ├── mod.rs              # Re-exports
    │   ├── cypher.rs           # openCypher AST node types
    │   └── gql.rs              # ISO GQL AST node types
    ├── parser/
    │   ├── mod.rs
    │   ├── cypher.rs           # pest-based openCypher parser
    │   └── gql.rs              # ISO GQL parser
    ├── translator/
    │   ├── mod.rs
    │   ├── visitor.rs          # Visitor trait definition
    │   ├── cypher.rs           # openCypher → SPARQL algebra visitor
    │   └── gql.rs              # ISO GQL → SPARQL algebra visitor
    ├── target/
    │   ├── mod.rs
    │   ├── trait.rs            # TargetEngine trait
    │   └── oxigraph.rs         # Oxigraph adapter
    ├── rdf_mapping/
    │   ├── mod.rs
    │   ├── rdf_star.rs         # RDF-star edge property encoding
    │   └── reification.rs      # Standard reification fallback
    └── error.rs                # Unified error types
```

---

## 2. AST Design

The AST must faithfully represent the semantic structure of both query languages and form a stable intermediate representation (IR) that the translator consumes.

### openCypher AST (`ast/cypher.rs`)

Key node types:

```rust
pub enum CypherExpr {
    Query(CypherQuery),
    // ...
}

pub struct CypherQuery {
    pub clauses: Vec<Clause>,
}

pub enum Clause {
    Match(MatchClause),
    Where(WhereClause),
    Return(ReturnClause),
    With(WithClause),
    Merge(MergeClause),
    Create(CreateClause),
    Delete(DeleteClause),
    Set(SetClause),
    Unwind(UnwindClause),
}

pub struct MatchClause {
    pub optional: bool,
    pub pattern: PatternList,
    pub where_: Option<WhereClause>,
}

pub struct PatternList(pub Vec<Pattern>);

pub struct Pattern {
    pub variable: Option<Ident>,
    pub elements: Vec<PatternElement>,
}

pub enum PatternElement {
    Node(NodePattern),
    Relationship(RelationshipPattern),
}

pub struct NodePattern {
    pub variable: Option<Ident>,
    pub labels: Vec<Label>,
    pub properties: Option<MapLiteral>,
}

pub struct RelationshipPattern {
    pub variable: Option<Ident>,
    pub direction: Direction,
    pub rel_types: Vec<RelType>,
    pub properties: Option<MapLiteral>,
    pub range: Option<RangeQuantifier>,
}

pub enum Direction { Left, Right, Both, None }
```

### ISO GQL AST (`ast/gql.rs`)

Mirrors the GQL specification's structure, covering:
- `MATCH`, `FILTER`, `RETURN`, `LET`, `FOR`
- Graph pattern expressions including path patterns and quantifiers
- Element patterns and element variables

---

## 3. Parser Layer

### openCypher (`parser/cypher.rs`)

Use the [`pest`](https://crates.io/crates/pest) PEG parser driven by a `.pest` grammar file that conforms to the [openCypher grammar specification](https://s3.amazonaws.com/artifacts.opencypher.org/railroad/Cypher.html).

- Grammar file: `grammars/cypher.pest`
- Walk the `pest` parse tree and produce the `CypherExpr` AST
- Return `ParseError` with source span on failure

### ISO GQL (`parser/gql.rs`)

Use `pest` similarly, driven by a grammar derived from [ISO/IEC 39075:2024](https://www.iso.org/standard/76120.html).

- Grammar file: `grammars/gql.pest`
- Produce the `GqlQuery` AST

---

## 4. Translator / Visitor Layer

The translator is the core of the library. It walks the AST and emits [`spargebra`](https://crates.io/crates/spargebra) algebra types.

### Visitor Trait (`translator/visitor.rs`)

```rust
pub trait AstVisitor {
    type Output;
    type Error;

    fn visit_match(&mut self, clause: &MatchClause) -> Result<Self::Output, Self::Error>;
    fn visit_return(&mut self, clause: &ReturnClause) -> Result<Self::Output, Self::Error>;
    fn visit_where(&mut self, clause: &WhereClause) -> Result<Self::Output, Self::Error>;
    fn visit_node_pattern(&mut self, node: &NodePattern) -> Result<Self::Output, Self::Error>;
    fn visit_relationship_pattern(&mut self, rel: &RelationshipPattern) -> Result<Self::Output, Self::Error>;
    // ...
}
```

### openCypher Translator (`translator/cypher.rs`)

Maps Property Graph concepts to RDF/SPARQL:

| Property Graph | RDF Mapping |
|---|---|
| `(n:Label)` | `?n rdf:type :Label` |
| `(n {prop: val})` | `?n :prop val` |
| `(a)-[:REL]->(b)` | `?a :REL ?b` (or RDF-star triple) |
| `RETURN n.name` | `SELECT ?name WHERE { ?n :name ?name }` |
| `WHERE n.age > 30` | `FILTER (?age > 30)` |
| `OPTIONAL MATCH` | `OPTIONAL { ... }` |
| `WITH` | Sub-select or `BIND` |

The output is a `spargebra::algebra::GraphPattern`.

### ISO GQL Translator (`translator/gql.rs`)

Parallel implementation using the GQL AST. Shares mapping logic where the two languages overlap semantically.

---

## 5. RDF Mapping Layer

### RDF-star mode (`rdf_mapping/rdf_star.rs`)

Edge properties are encoded as annotations on the edge triple:

```
<<:alice :knows :bob>> :since "2020"^^xsd:date .
```

Translate to SPARQL-star:

```sparql
SELECT ?since WHERE {
  <<?alice :knows ?bob>> :since ?since .
}
```

### Reification fallback (`rdf_mapping/reification.rs`)

For engines that do not support RDF-star:

```turtle
:edge1 rdf:type rdf:Statement ;
       rdf:subject :alice ;
       rdf:predicate :knows ;
       rdf:object :bob ;
       :since "2020"^^xsd:date .
```

---

## 6. Target Trait & Adapters (`target/`)

```rust
pub trait TargetEngine {
    /// Whether this engine supports RDF-star/SPARQL-star.
    fn supports_rdf_star(&self) -> bool;

    /// Whether this engine supports SPARQL 1.1 federation (SERVICE).
    fn supports_federation(&self) -> bool;

    /// Post-process or optimize the algebra for this engine.
    fn finalize(&self, pattern: GraphPattern) -> Result<GraphPattern, PolygraphError>;
}
```

Adapter implementations:
- `Oxigraph` — RDF-star enabled, native Rust
- `GenericSparql11` — Standard-only fallback

---

## 7. Public API (`lib.rs`)

```rust
pub struct Transpiler;

impl Transpiler {
    /// Transpile an openCypher query to a SPARQL query string.
    pub fn cypher_to_sparql(
        cypher: &str,
        engine: &dyn TargetEngine,
    ) -> Result<String, PolygraphError>;

    /// Transpile an ISO GQL query to a SPARQL query string.
    pub fn gql_to_sparql(
        gql: &str,
        engine: &dyn TargetEngine,
    ) -> Result<String, PolygraphError>;

    /// Return the raw Spargebra algebra without serialization.
    pub fn cypher_to_algebra(
        cypher: &str,
        engine: &dyn TargetEngine,
    ) -> Result<GraphPattern, PolygraphError>;
}
```

---

## 8. Testing Strategy

### Unit tests
- AST construction tests per node type
- Parser round-trip tests (parse → re-emit → compare)
- Translator tests: given an AST node, assert the correct `GraphPattern`

### Integration tests
- Feed a Cypher query end-to-end and assert the SPARQL output string
- Run against a live Oxigraph instance for semantic correctness

### TCK compliance (`tests/tck/`)
- Load openCypher TCK Gherkin scenarios from the [opencypher/openCypher](https://github.com/opencypher/openCypher/tree/master/tck) repository
- Use `cucumber` crate to drive test execution
- Execute transpiled SPARQL against a reference RDF dataset
- Assert results match expected TCK outcomes

---

## 9. Key Dependencies

| Crate | Purpose |
|---|---|
| [`pest`](https://crates.io/crates/pest) | PEG parser for openCypher & GQL grammars |
| [`spargebra`](https://crates.io/crates/spargebra) | SPARQL algebra types and serialization |
| [`oxigraph`](https://crates.io/crates/oxigraph) | Reference RDF store for integration tests |
| [`thiserror`](https://crates.io/crates/thiserror) | Ergonomic error type definitions |
| [`cucumber`](https://crates.io/crates/cucumber) | Gherkin-driven TCK test runner |
| [`criterion`](https://crates.io/crates/criterion) | Benchmarking translation throughput |

---

## 10. Error Handling

All public APIs return `Result<T, PolygraphError>`. Error variants:

```rust
#[derive(thiserror::Error, Debug)]
pub enum PolygraphError {
    #[error("Parse error at {span}: {message}")]
    Parse { span: String, message: String },

    #[error("Unsupported feature: {feature}")]
    UnsupportedFeature { feature: String },

    #[error("Translation error: {0}")]
    Translation(String),

    #[error("Target engine error: {0}")]
    Engine(String),
}
```
