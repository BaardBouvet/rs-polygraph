/// Unified error type for all `polygraph` operations.
#[derive(thiserror::Error, Debug)]
pub enum PolygraphError {
    /// A syntax or structural error encountered while parsing an input query.
    #[error("Parse error at {span}: {message}")]
    Parse { span: String, message: String },

    /// The query uses a language feature not yet supported by this library.
    #[error("Unsupported feature: {feature}")]
    UnsupportedFeature { feature: String },

    /// An error occurred during translation from the AST to SPARQL algebra.
    #[error("Translation error: {message}")]
    Translation { message: String },

    /// An error occurred while mapping SPARQL results back to Cypher values.
    #[error("Result mapping error: {message}")]
    ResultMapping { message: String },
}
