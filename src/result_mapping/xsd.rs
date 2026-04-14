//! XSD datatype → CypherValue conversion.

use super::types::{CypherValue, RdfTerm};

const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_LONG: &str = "http://www.w3.org/2001/XMLSchema#long";
const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#int";
const XSD_SHORT: &str = "http://www.w3.org/2001/XMLSchema#short";
const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const XSD_FLOAT: &str = "http://www.w3.org/2001/XMLSchema#float";
const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// Convert an [`RdfTerm`] to a [`CypherValue`], using XSD datatypes for
/// type-accurate conversion of literals.
///
/// - IRIs → `CypherValue::String` (the full IRI string).
/// - Blank nodes → `CypherValue::String` (the blank-node label).
/// - Typed literals → parsed according to their XSD datatype.
/// - Plain / language-tagged literals → `CypherValue::String`.
pub fn rdf_term_to_cypher(term: &RdfTerm, base_iri: &str) -> CypherValue {
    match term {
        RdfTerm::Iri(iri) => {
            // Strip base IRI prefix for cleaner display.
            let local = iri.strip_prefix(base_iri).unwrap_or(iri);
            CypherValue::String(local.to_string())
        }
        RdfTerm::BlankNode(label) => CypherValue::String(label.clone()),
        RdfTerm::Literal {
            value,
            datatype,
            language: _,
        } => match datatype.as_deref() {
            Some(XSD_INTEGER) | Some(XSD_LONG) | Some(XSD_INT) | Some(XSD_SHORT) => value
                .parse::<i64>()
                .map(CypherValue::Integer)
                .unwrap_or_else(|_| CypherValue::String(value.clone())),
            Some(XSD_DOUBLE) | Some(XSD_FLOAT) | Some(XSD_DECIMAL) => value
                .parse::<f64>()
                .map(CypherValue::Float)
                .unwrap_or_else(|_| CypherValue::String(value.clone())),
            Some(XSD_BOOLEAN) => match value.as_str() {
                "true" | "1" => CypherValue::Boolean(true),
                "false" | "0" => CypherValue::Boolean(false),
                _ => CypherValue::String(value.clone()),
            },
            Some(XSD_STRING) | None => CypherValue::String(value.clone()),
            Some(_) => {
                // Unknown datatype: return as string.
                CypherValue::String(value.clone())
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "http://example.org/";

    #[test]
    fn integer_literal() {
        let term = RdfTerm::Literal {
            value: "42".into(),
            datatype: Some(XSD_INTEGER.into()),
            language: None,
        };
        assert_eq!(rdf_term_to_cypher(&term, BASE), CypherValue::Integer(42));
    }

    #[test]
    fn double_literal() {
        let term = RdfTerm::Literal {
            value: "3.14".into(),
            datatype: Some(XSD_DOUBLE.into()),
            language: None,
        };
        assert_eq!(rdf_term_to_cypher(&term, BASE), CypherValue::Float(std::f64::consts::PI));
    }

    #[test]
    fn boolean_literal() {
        let term = RdfTerm::Literal {
            value: "true".into(),
            datatype: Some(XSD_BOOLEAN.into()),
            language: None,
        };
        assert_eq!(rdf_term_to_cypher(&term, BASE), CypherValue::Boolean(true));
    }

    #[test]
    fn plain_string() {
        let term = RdfTerm::Literal {
            value: "hello".into(),
            datatype: None,
            language: None,
        };
        assert_eq!(
            rdf_term_to_cypher(&term, BASE),
            CypherValue::String("hello".into())
        );
    }

    #[test]
    fn iri_strips_base() {
        let term = RdfTerm::Iri("http://example.org/Person".into());
        assert_eq!(
            rdf_term_to_cypher(&term, BASE),
            CypherValue::String("Person".into())
        );
    }

    #[test]
    fn iri_without_base_preserved() {
        let term = RdfTerm::Iri("http://other.org/Foo".into());
        assert_eq!(
            rdf_term_to_cypher(&term, BASE),
            CypherValue::String("http://other.org/Foo".into())
        );
    }
}
