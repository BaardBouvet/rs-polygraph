//! Result mapping: SPARQL bindings → Cypher-shaped rows.
//!
//! This module is the inverse of [`crate::rdf_mapping`]. The translator
//! produces a SPARQL query _and_ a [`ProjectionSchema`] describing the
//! shape of each RETURN column. After the caller executes the SPARQL
//! against their triplestore, [`map_results`] converts the raw bindings
//! back into Cypher values.

pub mod mapper;
pub mod schema;
pub mod types;
pub mod xsd;

pub use mapper::map_results;
pub use schema::{ColumnKind, ProjectedColumn, ProjectionSchema};
pub use types::{CypherNode, CypherRelationship, CypherRow, CypherValue, RdfTerm, SparqlSolution};

use crate::error::PolygraphError;

/// A single SPARQL variable binding (name → optional value string).
///
/// Used by [`TranspileOutput::Continuation`] to ferry phase-1 results to
/// the continuation closure without pulling in an Oxigraph dependency.
pub type BindingRow = Vec<(String, Option<String>)>;

/// The output of a Cypher/GQL → SPARQL transpilation.
///
/// This is an enum that represents either:
/// - [`TranspileOutput::Complete`]: a single SPARQL query ready to execute.
/// - [`TranspileOutput::Continuation`]: a two-phase (or N-phase) execution
///   pipeline where phase-1 results are fed back to produce phase-2 query.
///
/// For single-phase queries (the common case), callers access `.sparql`
/// and `.schema` via the accessor methods or `match`. The [`Transpiler`]
/// always emits `Complete` for queries fully expressible in static SPARQL.
///
/// [`Transpiler`]: crate::Transpiler
pub enum TranspileOutput {
    /// Single-phase output: one SPARQL string ready to execute.
    Complete {
        /// The SPARQL query string.
        sparql: String,
        /// Schema describing column types and SPARQL variable mappings.
        schema: ProjectionSchema,
    },

    /// Multi-phase output: execute `phase1`, pass every result row to
    /// `continue_fn` to obtain the next `TranspileOutput` (which itself
    /// may be a `Continuation`, supporting N-phase pipelines).
    Continuation {
        /// The first SPARQL query to run.
        phase1: Box<TranspileOutput>,
        /// Closure that takes the phase-1 result rows and returns the next
        /// `TranspileOutput` to execute. Rows are represented as a list of
        /// `(variable_name, Option<value_string>)` pairs.
        continue_fn:
            Box<dyn FnOnce(Vec<BindingRow>) -> Result<TranspileOutput, PolygraphError> + Send>,
    },
}

impl std::fmt::Debug for TranspileOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete { sparql, schema } => f
                .debug_struct("Complete")
                .field("sparql", sparql)
                .field("schema", schema)
                .finish(),
            Self::Continuation { phase1, .. } => f
                .debug_struct("Continuation")
                .field("phase1", phase1)
                .field("continue_fn", &"<closure>")
                .finish(),
        }
    }
}

impl TranspileOutput {
    /// Returns `true` if this is a single-phase `Complete` output.
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }

    /// Unwrap as a `Complete` output, panicking on `Continuation`.
    ///
    /// Prefer using the [`crate::runtime`] module to drive continuation
    /// chains automatically.
    pub fn unwrap_complete(self) -> (String, ProjectionSchema) {
        match self {
            Self::Complete { sparql, schema } => (sparql, schema),
            Self::Continuation { .. } => panic!(
                "called TranspileOutput::unwrap_complete() on a Continuation"
            ),
        }
    }

    /// Convenience: access the SPARQL string when this is `Complete`.
    ///
    /// Returns `None` for `Continuation` outputs.
    pub fn sparql(&self) -> Option<&str> {
        match self {
            Self::Complete { sparql, .. } => Some(sparql.as_str()),
            Self::Continuation { .. } => None,
        }
    }

    /// Convenience: access the schema when this is `Complete`.
    pub fn schema(&self) -> Option<&ProjectionSchema> {
        match self {
            Self::Complete { schema, .. } => Some(schema),
            Self::Continuation { .. } => None,
        }
    }

    /// Map SPARQL query results back into Cypher-shaped rows.
    ///
    /// The caller executes `self.sparql()` against their triplestore and
    /// passes the raw bindings here.  Only valid for `Complete` outputs;
    /// use the runtime driver for `Continuation` outputs.
    pub fn map_results(
        &self,
        solutions: &[SparqlSolution],
    ) -> Result<Vec<CypherRow>, PolygraphError> {
        match self {
            Self::Complete { schema, .. } => mapper::map_results(solutions, schema),
            Self::Continuation { .. } => Err(PolygraphError::UnsupportedFeature {
                feature: "map_results() called on a Continuation output; \
                          use the runtime driver instead"
                    .to_string(),
            }),
        }
    }
}

// ── Backward-compat helpers ───────────────────────────────────────────────────
// Allow construction of the common Complete variant with field-like syntax.

impl TranspileOutput {
    /// Construct a `Complete` output — convenience for internal use.
    #[doc(hidden)]
    pub fn complete(sparql: String, schema: ProjectionSchema) -> Self {
        Self::Complete { sparql, schema }
    }

    /// Consume self and return the SPARQL string; panics on `Continuation`.
    ///
    /// Useful in tests and examples where the caller knows only `Complete`
    /// outputs are expected.
    pub fn into_sparql(self) -> String {
        self.unwrap_complete().0
    }
}
