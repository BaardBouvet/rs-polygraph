//! Bag-equality oracle with Cypher-flavoured semantics.

use crate::value::Value;

/// How the comparator should treat row order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderMode {
    /// Order is irrelevant — compare as multisets (bags).
    Bag,
    /// Order is meaningful — compare as sequences. Used for queries with
    /// `ORDER BY`.
    Ordered,
}

/// Outcome of comparing an actual result bag to an expected one.
#[derive(Clone, Debug, PartialEq)]
pub enum ComparisonOutcome {
    Match,
    Mismatch {
        missing_from_actual: Vec<Vec<Value>>,
        unexpected_in_actual: Vec<Vec<Value>>,
        column_name_diff: Option<(Vec<String>, Vec<String>)>,
    },
}

/// Comparator handle.
pub struct Comparison;

impl Comparison {
    pub fn compare(
        expected_columns: &[String],
        expected_rows: &[Vec<Value>],
        actual_columns: &[String],
        actual_rows: &[Vec<Value>],
        order: OrderMode,
    ) -> ComparisonOutcome {
        // Column-name parity is required: a query that returns `n.name` should
        // also project under that name.
        if expected_columns != actual_columns {
            return ComparisonOutcome::Mismatch {
                missing_from_actual: expected_rows.to_vec(),
                unexpected_in_actual: actual_rows.to_vec(),
                column_name_diff: Some((expected_columns.to_vec(), actual_columns.to_vec())),
            };
        }

        match order {
            OrderMode::Ordered => {
                if expected_rows.len() != actual_rows.len()
                    || expected_rows
                        .iter()
                        .zip(actual_rows.iter())
                        .any(|(e, a)| !rows_eq(e, a))
                {
                    diff_rows(expected_rows, actual_rows)
                } else {
                    ComparisonOutcome::Match
                }
            }
            OrderMode::Bag => {
                let mut remaining: Vec<Vec<Value>> = actual_rows.to_vec();
                let mut missing = Vec::new();
                'outer: for e in expected_rows {
                    for i in 0..remaining.len() {
                        if rows_eq(e, &remaining[i]) {
                            remaining.swap_remove(i);
                            continue 'outer;
                        }
                    }
                    missing.push(e.clone());
                }
                if missing.is_empty() && remaining.is_empty() {
                    ComparisonOutcome::Match
                } else {
                    ComparisonOutcome::Mismatch {
                        missing_from_actual: missing,
                        unexpected_in_actual: remaining,
                        column_name_diff: None,
                    }
                }
            }
        }
    }
}

fn rows_eq(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(x, y)| x.cypher_structural_eq(y))
}

fn diff_rows(expected: &[Vec<Value>], actual: &[Vec<Value>]) -> ComparisonOutcome {
    // Cheap diff: rows in expected not present in actual, and vice versa.
    let missing: Vec<Vec<Value>> = expected
        .iter()
        .filter(|e| !actual.iter().any(|a| rows_eq(e, a)))
        .cloned()
        .collect();
    let unexpected: Vec<Vec<Value>> = actual
        .iter()
        .filter(|a| !expected.iter().any(|e| rows_eq(e, a)))
        .cloned()
        .collect();
    ComparisonOutcome::Mismatch {
        missing_from_actual: missing,
        unexpected_in_actual: unexpected,
        column_name_diff: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(s: &str) -> String {
        s.to_owned()
    }

    #[test]
    fn bag_match_ignores_order() {
        let cols = vec![col("x")];
        let exp = vec![vec![Value::Int(1)], vec![Value::Int(2)]];
        let act = vec![vec![Value::Int(2)], vec![Value::Int(1)]];
        assert_eq!(
            Comparison::compare(&cols, &exp, &cols, &act, OrderMode::Bag),
            ComparisonOutcome::Match
        );
    }

    #[test]
    fn ordered_detects_swap() {
        let cols = vec![col("x")];
        let exp = vec![vec![Value::Int(1)], vec![Value::Int(2)]];
        let act = vec![vec![Value::Int(2)], vec![Value::Int(1)]];
        let res = Comparison::compare(&cols, &exp, &cols, &act, OrderMode::Ordered);
        assert!(matches!(res, ComparisonOutcome::Mismatch { .. }));
    }

    #[test]
    fn column_name_diff_caught() {
        let res = Comparison::compare(
            &[col("name")],
            &[],
            &[col("n.name")],
            &[],
            OrderMode::Bag,
        );
        match res {
            ComparisonOutcome::Mismatch {
                column_name_diff: Some(_),
                ..
            } => {}
            _ => panic!("expected mismatch with column_name_diff"),
        }
    }
}
