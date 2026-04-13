//! Core result mapping algorithm: SPARQL bindings → Cypher rows.

use crate::error::PolygraphError;

use super::schema::{ColumnKind, ProjectionSchema};
use super::types::{CypherRow, CypherValue, SparqlSolution};
use super::xsd::rdf_term_to_cypher;

/// Map SPARQL query results back into Cypher-shaped rows.
///
/// For scalar-only projections (Phase R1-R2), this is a direct 1:1 mapping
/// of SPARQL bindings to Cypher values. Entity hydration (nodes,
/// relationships) is added in Phase R3+.
pub fn map_results(
    solutions: &[SparqlSolution],
    schema: &ProjectionSchema,
) -> Result<Vec<CypherRow>, PolygraphError> {
    let mut rows = Vec::with_capacity(solutions.len());

    for solution in solutions {
        let mut row = CypherRow::new();

        for col in &schema.columns {
            let value = map_column(col, solution, &schema.base_iri)?;
            row.insert(col.name.clone(), value);
        }

        rows.push(row);
    }

    Ok(rows)
}

/// Map a single column from a SPARQL solution to a CypherValue.
fn map_column(
    col: &super::schema::ProjectedColumn,
    solution: &SparqlSolution,
    base_iri: &str,
) -> Result<CypherValue, PolygraphError> {
    match &col.kind {
        ColumnKind::Scalar { var } => match solution.bindings.get(var.as_str()) {
            Some(Some(term)) => Ok(rdf_term_to_cypher(term, base_iri)),
            Some(None) | None => Ok(CypherValue::Null),
        },
        ColumnKind::Node { iri_var } => {
            // Phase R1-R2: return node IRI as a string placeholder.
            // Phase R3 will hydrate labels and properties.
            match solution.bindings.get(iri_var.as_str()) {
                Some(Some(term)) => Ok(rdf_term_to_cypher(term, base_iri)),
                Some(None) | None => Ok(CypherValue::Null),
            }
        }
        ColumnKind::Relationship {
            src_var,
            dst_var,
            type_info,
        } => {
            // Phase R1-R2: return relationship type as a string placeholder.
            // Phase R4 will fully hydrate.
            let _ = (src_var, dst_var);
            Ok(CypherValue::String(type_info.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result_mapping::schema::{ProjectedColumn, ProjectionSchema};
    use crate::result_mapping::types::{RdfTerm, SparqlSolution};

    fn scalar_schema(columns: Vec<(&str, &str)>) -> ProjectionSchema {
        ProjectionSchema {
            columns: columns
                .into_iter()
                .map(|(name, var)| ProjectedColumn {
                    name: name.to_string(),
                    kind: ColumnKind::Scalar {
                        var: var.to_string(),
                    },
                })
                .collect(),
            distinct: false,
            base_iri: "http://example.org/".to_string(),
            rdf_star: false,
        }
    }

    fn solution(bindings: Vec<(&str, Option<RdfTerm>)>) -> SparqlSolution {
        SparqlSolution {
            bindings: bindings
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    #[test]
    fn scalar_integer_mapping() {
        let schema = scalar_schema(vec![("count", "__agg_1")]);
        let solutions = vec![solution(vec![(
            "__agg_1",
            Some(RdfTerm::Literal {
                value: "42".into(),
                datatype: Some("http://www.w3.org/2001/XMLSchema#integer".into()),
                language: None,
            }),
        )])];
        let rows = map_results(&solutions, &schema).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["count"], CypherValue::Integer(42));
    }

    #[test]
    fn null_for_unbound_variable() {
        let schema = scalar_schema(vec![("x", "x")]);
        let solutions = vec![solution(vec![("x", None)])];
        let rows = map_results(&solutions, &schema).unwrap();
        assert_eq!(rows[0]["x"], CypherValue::Null);
    }

    #[test]
    fn multiple_columns_and_rows() {
        let schema = scalar_schema(vec![("name", "__n_name_0"), ("age", "__n_age_1")]);
        let solutions = vec![
            solution(vec![
                (
                    "__n_name_0",
                    Some(RdfTerm::Literal {
                        value: "Alice".into(),
                        datatype: None,
                        language: None,
                    }),
                ),
                (
                    "__n_age_1",
                    Some(RdfTerm::Literal {
                        value: "30".into(),
                        datatype: Some("http://www.w3.org/2001/XMLSchema#integer".into()),
                        language: None,
                    }),
                ),
            ]),
            solution(vec![
                (
                    "__n_name_0",
                    Some(RdfTerm::Literal {
                        value: "Bob".into(),
                        datatype: None,
                        language: None,
                    }),
                ),
                ("__n_age_1", None),
            ]),
        ];
        let rows = map_results(&solutions, &schema).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], CypherValue::String("Alice".into()));
        assert_eq!(rows[0]["age"], CypherValue::Integer(30));
        assert_eq!(rows[1]["name"], CypherValue::String("Bob".into()));
        assert_eq!(rows[1]["age"], CypherValue::Null);
    }

    #[test]
    fn empty_solutions() {
        let schema = scalar_schema(vec![("x", "x")]);
        let rows = map_results(&[], &schema).unwrap();
        assert!(rows.is_empty());
    }
}
