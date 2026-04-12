# Target Engine Analysis

This document evaluates SPARQL engine targets worth supporting in `rs-polygraph` beyond the built-in Oxigraph adapter.

The `TargetEngine` trait has three levers relevant to this analysis:

- `supports_rdf_star() -> bool` — chooses between RDF-star and reification edge encoding
- `supports_federation() -> bool` — controls whether `SERVICE` calls are emitted
- `finalize(query: String) -> Result<String, PolygraphError>` — dialect-specific post-processing

All engines listed here are accessed over HTTP SPARQL endpoints; none are embeddable like Oxigraph. Adapter structs in `target/` are purely capability descriptors and dialect rewriters — no HTTP client code belongs in this crate. Endpoint communication belongs in a future `rs-polygraph-client` crate or integration tests.

---

## Tier 1 — High Priority

### Apache Jena / Fuseki

| Capability | Value |
|---|---|
| `supports_rdf_star()` | `true` (Jena 4.x+) |
| `supports_federation()` | `true` |

ARQ (Jena's SPARQL engine) added SPARQL-star in Jena 4.x using `<< >>` syntax aligned with the W3C proposal. RDF4J-style property functions (`apf:textContains`, `list:member`) and custom aggregates are available but not relevant to transpiled output. Standard SPARQL 1.1 path expressions work without quirks.

`finalize()` is a no-op for modern Jena. Older deployments (< 4.0) should use the `GenericSparql11` adapter (reification fallback) until a version-gated variant is warranted.

**Planned module**: `target::jena::Jena`

**Rationale**: Largest Java SPARQL install base. Many enterprise triplestores and academic deployments run Fuseki. Low implementation cost — no dialect rewriting needed.

---

### Eclipse RDF4J

| Capability | Value |
|---|---|
| `supports_rdf_star()` | `true` (RDF4J 4.0+) |
| `supports_federation()` | `true` |

RDF4J 4.0 adopted the W3C RDF-star spec natively. Standard SPARQL 1.1 queries are clean; no quirks affect transpiler output. `finalize()` is a no-op.

**Planned module**: `target::rdf4j::Rdf4j`

**Rationale**: Widely deployed; also the foundation for GraphDB (see below). Implementing RDF4J first gives GraphDB support almost for free.

---

### GraphDB (Ontotext)

| Capability | Value |
|---|---|
| `supports_rdf_star()` | `true` |
| `supports_federation()` | `true` |

GraphDB is built on RDF4J and inherits its SPARQL-star support. It adds proprietary extensions: `RANK` scoring, `EXPLAIN` plan hints, and `USING NAMESPACE` prefix injection. Transpiled queries do not need RANK or EXPLAIN, but `USING NAMESPACE` must be injected when `base_iri()` returns a custom namespace, because GraphDB's default prefix resolution differs from other engines.

`finalize()`: inject `USING NAMESPACE <{base_iri}>` pragma when `base_iri()` is non-default.

**Planned module**: `target::graphdb::GraphDb`

**Rationale**: Dominant commercial RDF store in enterprise Knowledge Graph deployments (pharma, finance, publishing). Justifies a dedicated target rather than inheriting plain `Rdf4j`.

---

### Amazon Neptune

| Capability | Value |
|---|---|
| `supports_rdf_star()` | `false` |
| `supports_federation()` | `false` |

Neptune does not support RDF-star; reification fallback is mandatory. Neptune's SERVICE support is restricted to other Neptune clusters, so external federation must be suppressed. Known gaps in Neptune's SPARQL 1.1 conformance:

- `MINUS` is not supported; rewrite as `FILTER NOT EXISTS` in `finalize()`
- `FROM NAMED` in standard query form is rejected
- Blank node identity across requests is not preserved
- `VALUES` support is limited in certain join shapes

`finalize()` should validate for these patterns and either rewrite them or return a `PolygraphError::UnsupportedFeature`.

**Planned module**: `target::neptune::Neptune`

**Rationale**: Large cloud footprint. Many production knowledge graphs run on Neptune because of AWS ecosystem integration. The quirks are well-documented and bounded.

---

## Tier 2 — Worthwhile

### Virtuoso (OpenLink)

| Capability | Value |
|---|---|
| `supports_rdf_star()` | `false` |
| `supports_federation()` | `true` |

Virtuoso 8.x has partial RDF-star support but it diverges from the W3C spec; reification is the safe default. Virtuoso supports SERVICE and is commonly used as a federated hub (e.g. historic DBpedia deployments).

Virtuoso accepts pragma annotations at the top of queries that control query execution:

```sparql
define sql:big-data-const 0
define input:inference "..."
SELECT ...
```

`finalize()` can inject configurable pragmas. A `VirtuosoConfig` struct (passed to the `Virtuoso` constructor) could carry `inference_graph: Option<String>` and `big_data_const: bool`.

**Planned module**: `target::virtuoso::Virtuoso`

**Rationale**: DBpedia, many legacy Linked Open Data deployments, and several commercial installs. The pragma injection in `finalize()` is the main value-add over `GenericSparql11`.

---

### Stardog

| Capability | Value |
|---|---|
| `supports_rdf_star()` | `true` (Stardog 5+) |
| `supports_federation()` | `true` |

Stardog 5+ supports SPARQL-star using `<< >>` aligned with the W3C spec. Virtual Graphs extend SERVICE to relational databases and Elasticsearch, but transpiled queries target native RDF; those extensions are irrelevant here. SHACL reasoning can be activated via `USING` clauses but is not needed for transpiled output.

`finalize()` is a no-op for standard use.

**Planned module**: `target::stardog::Stardog`

**Rationale**: Significant commercial deployment in enterprise knowledge graph stacks. Well-behaved W3C compliance keeps the adapter cheap.

---

### AllegroGraph (Franz Inc.)

| Capability | Value |
|---|---|
| `supports_rdf_star()` | `true` |
| `supports_federation()` | `true` |

AllegroGraph predates the W3C RDF-star spec and coined the term "RDF*". Its `<< >>` embedded triple syntax is compatible with the W3C form for asserted triples. Non-asserted (quoted) triples have subtly different semantics — triples inside `<< >>` in a WHERE clause match only if they appear as subjects of other triples, not if they are also asserted as regular triples. This matters for edge property queries: `finalize()` may need to add an `UNION` branch depending on encoding choices.

**Planned module**: `target::allegrograph::AllegroGraph`

**Rationale**: Early RDF-star adopter; some organizations standardized on it before alternatives matured. Smaller market share than the Tier 1 engines but worth supporting given the non-trivial `finalize()` logic.

---

## Tier 3 — Specialized / Lower Priority

### Halyard (Apache HBase–backed)

| Capability | Value |
|---|---|
| `supports_rdf_star()` | partial |
| `supports_federation()` | limited |

Halyard is designed for billion-triple graphs on Hadoop/HBase infrastructure. Recent versions are adding RDF-star support but it is not mature. Query shapes that cause cross-partition scans (non-selective triple patterns, large cross-joins) can be extremely slow. A Halyard target would need `finalize()` to warn about — or refuse to emit — known pathological patterns.

**Verdict**: Niche. Worth implementing only if rs-polygraph explicitly targets big-data pipeline use cases. Defer until there is concrete demand.

---

### Blazegraph

| Capability | Value |
|---|---|
| `supports_rdf_star()` | `false` |
| `supports_federation()` | `true` |

Blazegraph is in maintenance mode. Wikidata is actively migrating off it. The base engine is clean SPARQL 1.1. The only strong motivation for a dedicated Blazegraph target would be Wikidata compatibility, where the `wikibase:label` service and Wikidata-specific extensions would need `finalize()` treatment. A `Wikidata` target (wrapping Blazegraph or its successor) could be considered separately.

**Verdict**: Declining ecosystem. Skip unless a Wikidata-specific target is explicitly requested.

---

### Corese (INRIA)

| Capability | Value |
|---|---|
| `supports_rdf_star()` | `true` |
| `supports_federation()` | `true` |

One of the earliest complete SPARQL-star implementations. Research/academic engine with a small install base.

**Verdict**: Low ROI. `GenericSparql11` or a plain `supports_rdf_star: true` generic adapter covers Corese adequately without a dedicated struct.

---

### MarkLogic

| Capability | Value |
|---|---|
| `supports_rdf_star()` | `false` |
| `supports_federation()` | limited |

SPARQL is a secondary interface in MarkLogic; the primary API is XQuery and document management. RDF triples are stored as managed triples within documents.

**Verdict**: Low ROI. Skip.

---

## Implementation Plan

Implement targets in this order, one module per engine:

| Order | Module | Struct | `rdf_star` | `finalize()` work |
|---|---|---|---|---|
| 1 | `target::jena` | `Jena` | `true` | none |
| 2 | `target::rdf4j` | `Rdf4j` | `true` | none |
| 3 | `target::graphdb` | `GraphDb` | `true` | namespace pragma injection |
| 4 | `target::neptune` | `Neptune` | `false` | MINUS→FILTER NOT EXISTS rewrite, unsupported-feature errors |
| 5 | `target::virtuoso` | `Virtuoso` | `false` | configurable pragma injection |
| 6 | `target::stardog` | `Stardog` | `true` | none |
| 7 | `target::allegrograph` | `AllegroGraph` | `true` | quoted-triple UNION handling |

Each adapter module should include:

1. A unit test asserting `supports_rdf_star()` and `supports_federation()` return the expected values.
2. An integration test (using a serialized SPARQL string, no live endpoint) asserting `finalize()` produces the expected output for any dialect-specific rewriting.

Engines with no `finalize()` work (Jena, RDF4J, Stardog) can share a blanket test asserting `finalize()` is a no-op (output equals input).
