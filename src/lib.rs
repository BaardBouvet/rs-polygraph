#![forbid(unsafe_code)]

//! `polygraph` — transpile openCypher and ISO GQL queries to SPARQL 1.1.
//!
//! Phases 1–4 are complete:
//! - Phase 1: openCypher parser + AST
//! - Phase 2: SPARQL algebra translator (MATCH/WHERE/RETURN/WITH/OPTIONAL)
//! - Phase 3: RDF-star and reification edge property encoding
//! - Phase 4: ORDER BY/SKIP/LIMIT, aggregation, UNWIND, variable-length paths,
//!   multi-type relationships, IN list literals, write clause stubs
//!
//! Use [`sparql_engine::RdfStar`] for engines that support SPARQL-star natively, or
//! [`sparql_engine::GenericSparql11`] for standard SPARQL 1.1.
//!
//! # Example
//!
//! ```rust
//! use polygraph::parser::parse_cypher;
//!
//! let ast = parse_cypher("MATCH (n:Person) WHERE n.age > 30 RETURN n.name").unwrap();
//! println!("{ast:#?}");
//! ```

pub mod ast;
pub mod error;
pub mod lqa;
pub mod parser;
pub mod rdf_mapping;
pub mod result_mapping;
pub mod sparql_engine;
pub mod translator;

pub use error::PolygraphError;
pub use result_mapping::{
    BindingRow, CypherRow, CypherValue, ProjectionSchema, RdfTerm, SparqlSolution, TranspileOutput,
};

/// The main entry point for transpilation operations.
///
/// Transpilation methods beyond parsing are planned for Phase 2 and later.
pub struct Transpiler;

impl Transpiler {
    /// Parse an openCypher query string and return a typed AST.
    ///
    /// This is the stable Phase 1 API. Transpilation to SPARQL is
    /// implemented in Phase 2 via [`Self::cypher_to_sparql`].
    pub fn parse_cypher(cypher: &str) -> Result<ast::CypherQuery, PolygraphError> {
        parser::parse_cypher(cypher)
    }

    /// Transpile an openCypher query to SPARQL.
    ///
    /// Returns a [`TranspileOutput`] containing the SPARQL string and a
    /// projection schema for result mapping.
    ///
    /// The `engine` is consulted for engine-specific capabilities (RDF-star,
    /// federation). The optional `base_iri` on the engine is used as the
    /// namespace for labels, relationship types and property names.
    ///
    /// # Example
    ///
    /// ```rust
    /// use polygraph::{Transpiler, sparql_engine::GenericSparql11};
    ///
    /// let engine = GenericSparql11;
    /// let output = Transpiler::cypher_to_sparql(
    ///     "MATCH (n:Person) WHERE n.age > 30 RETURN n.name",
    ///     &engine,
    /// ).unwrap();
    /// assert!(output.sparql.contains("SELECT"));
    /// ```
    pub fn cypher_to_sparql(
        cypher: &str,
        engine: &dyn sparql_engine::TargetEngine,
    ) -> Result<TranspileOutput, PolygraphError> {
        let ast = parser::parse_cypher(cypher)?;
        // Phase 4.5: try the LQA path first; fall back to legacy on Unsupported.
        if let Some(output) = try_lqa_path(&ast, engine)? {
            return Ok(output);
        }
        let result =
            translator::cypher::translate(&ast, engine.base_iri(), engine.supports_rdf_star())?;
        let sparql = engine.finalize(result.sparql)?;
        Ok(TranspileOutput::complete(sparql, result.schema))
    }

    /// Like `cypher_to_sparql` but silently skips write clauses (SET/REMOVE/MERGE/CREATE/DELETE).
    /// The caller is responsible for executing write operations separately.
    pub fn cypher_to_sparql_skip_writes(
        cypher: &str,
        engine: &dyn sparql_engine::TargetEngine,
    ) -> Result<TranspileOutput, PolygraphError> {
        let ast = parser::parse_cypher(cypher)?;
        let result = translator::cypher::translate_skip_writes(
            &ast,
            engine.base_iri(),
            engine.supports_rdf_star(),
        )?;
        let sparql = engine.finalize(result.sparql)?;
        Ok(TranspileOutput::complete(sparql, result.schema))
    }

    /// Transpile an ISO GQL query to SPARQL.
    ///
    /// Returns a [`TranspileOutput`] containing the SPARQL string and a
    /// projection schema for result mapping.
    ///
    /// GQL-specific syntax (`IS Label`, `FILTER`, `NEXT`) is lowered to
    /// Cypher-equivalent constructs during parsing, so translation reuses
    /// the Cypher algebra translator.
    ///
    /// # Example
    ///
    /// ```rust
    /// use polygraph::{Transpiler, sparql_engine::GenericSparql11};
    ///
    /// let engine = GenericSparql11;
    /// let output = Transpiler::gql_to_sparql(
    ///     "MATCH (n:Person) WHERE n.age > 30 RETURN n.name",
    ///     &engine,
    /// ).unwrap();
    /// assert!(output.sparql.contains("SELECT"));
    /// ```
    pub fn gql_to_sparql(
        gql: &str,
        engine: &dyn sparql_engine::TargetEngine,
    ) -> Result<TranspileOutput, PolygraphError> {
        let ast = parser::parse_gql(gql)?;
        let result =
            translator::gql::translate(&ast, engine.base_iri(), engine.supports_rdf_star())?;
        let sparql = engine.finalize(result.sparql)?;
        Ok(TranspileOutput::complete(sparql, result.schema))
    }
}

/// Attempt to transpile `ast` via the LQA IR path.
///
/// Returns `Ok(Some(output))` on success, `Ok(None)` when the query contains
/// constructs not yet handled by the LQA path (triggering legacy fallback),
/// and `Err(e)` for unexpected errors (parse failures, etc.).
fn try_lqa_path(
    ast: &ast::CypherQuery,
    engine: &dyn sparql_engine::TargetEngine,
) -> Result<Option<TranspileOutput>, PolygraphError> {
    // Safety guard: only route queries through LQA that are known to be
    // handled correctly. Anything else falls back to the legacy translator.
    //
    // Current safe subset: read-only queries with a single MATCH+RETURN,
    // all nodes labeled, no named relationship variables, no varlen paths,
    // no OPTIONAL MATCH, no WITH clauses. Subsequent phases will extend this.
    if !is_lqa_safe(ast) {
        return Ok(None);
    }

    let mut lowerer = lqa::lower::AstLowerer::new();
    let op = match lowerer.lower_query(ast) {
        Ok(op) => op,
        Err(PolygraphError::Unsupported { .. })
        | Err(PolygraphError::UnsupportedFeature { .. }) => {
            return Ok(None);
        }
        Err(e) => return Err(e),
    };

    let compiled = match lqa::sparql::compile(&op, engine.base_iri().as_deref()) {
        Ok(c) => c,
        Err(PolygraphError::Unsupported { .. })
        | Err(PolygraphError::UnsupportedFeature { .. }) => {
            return Ok(None);
        }
        Err(e) => return Err(e),
    };

    let sparql = match engine.finalize(compiled.sparql) {
        Ok(s) => s,
        Err(PolygraphError::Unsupported { .. })
        | Err(PolygraphError::UnsupportedFeature { .. }) => {
            return Ok(None);
        }
        Err(e) => return Err(e),
    };

    Ok(Some(TranspileOutput::complete(sparql, compiled.schema)))
}

/// Return `true` only if the query can be safely transpiled through the LQA
/// path without risk of producing semantically wrong SPARQL.
///
/// This is a conservative allow-list: queries NOT matching all criteria are
/// routed to the legacy translator.  The allow-list is expanded as the LQA
/// path matures.
fn is_lqa_safe(ast: &ast::CypherQuery) -> bool {
    use ast::cypher::{Clause, PatternElement};
    use std::collections::HashSet;

    // Require exactly one MATCH clause followed by exactly one RETURN clause.
    let mut clause_kinds: Vec<&str> = Vec::new();
    for c in &ast.clauses {
        match c {
            Clause::Match(m) => {
                clause_kinds.push("match");
                // Check for unsupported rel-var and varlen in OPTIONAL MATCH too
                if m.optional {
                    clause_kinds.push("optional_match");
                }
            }
            Clause::Return(r) => {
                clause_kinds.push("return");
                let _ = r; // ORDER BY is handled correctly in the LQA SPARQL lowerer
            }
            Clause::With(_w) => {
                // WITH generates mid-pipeline Projection ops that can create
                // variable scoping issues in SPARQL. Defer to legacy for now.
                return false;
            }
            Clause::Unwind(_) => clause_kinds.push("unwind"),
            // Any write clause → legacy
            Clause::Create(_)
            | Clause::Set(_)
            | Clause::Remove(_)
            | Clause::Delete(_)
            | Clause::Merge(_)
            | Clause::Call(_) => return false,
            Clause::Union { .. } => return false,
        }
    }

    // Must start with a MATCH and end with RETURN.
    if clause_kinds.first() != Some(&"match") || clause_kinds.last() != Some(&"return") {
        return false;
    }

    // Every MATCH pattern element must:
    // - Have labeled nodes UNLESS the variable is already bound from a prior MATCH
    // - Have no named relationship variables
    // - Have no variable-length paths
    //
    // Track bound node variables across clauses so re-used vars in OPTIONAL MATCH
    // (e.g. `MATCH (n:P) OPTIONAL MATCH (n)-[:K]->(m:P)`) are not rejected.
    let mut bound_vars: HashSet<&str> = HashSet::new();
    for c in &ast.clauses {
        if let Clause::Match(m) = c {
            for pattern in &m.pattern.0 {
                for elem in &pattern.elements {
                    match elem {
                        PatternElement::Node(n) => {
                            let already_bound = n
                                .variable
                                .as_deref()
                                .map(|v| bound_vars.contains(v))
                                .unwrap_or(false);
                            if n.labels.is_empty() && !already_bound {
                                return false; // Unlabeled unbound node — complex to scan
                            }
                            // Mark this variable as bound for subsequent clauses.
                            if let Some(v) = n.variable.as_deref() {
                                bound_vars.insert(v);
                            }
                        }
                        PatternElement::Relationship(r) => {
                            if r.variable.is_some() {
                                return false; // Named rel var — can't bind in std SPARQL
                            }
                            if r.range.is_some() {
                                return false; // Variable-length path
                            }
                        }
                    }
                }
            }
        }
    }

    true
}
