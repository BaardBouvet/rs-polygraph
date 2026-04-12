/// RDF-star edge-property encoding.
///
/// Relationship properties are encoded as annotated triples (SPARQL-star):
///
/// ```sparql
/// <<?a <base:KNOWS> ?b>> <base:since> ?val .
/// ```
///
/// The `rdf-star` feature on `spargebra` must be enabled (it is, via
/// `Cargo.toml`).

use spargebra::term::{NamedNode, TermPattern, TriplePattern};

/// Produce the annotated triple pattern `<<?src <pred> ?dst>> <prop_iri> ?prop_var`
/// for a single edge property key/value.
///
/// # Arguments
/// * `src` – SPARQL term for the source node (subject of the edge)
/// * `pred` – edge predicate IRI
/// * `dst` – SPARQL term for the destination node (object of the edge)
/// * `prop_iri` – property predicate IRI
/// * `prop_var` – SPARQL term bound to the property value (variable or literal)
pub fn annotated_triple(
    src: TermPattern,
    pred: NamedNode,
    dst: TermPattern,
    prop_iri: NamedNode,
    prop_val: TermPattern,
) -> TriplePattern {
    let edge_triple = TriplePattern {
        subject: src,
        predicate: pred.into(),
        object: dst,
    };
    TriplePattern {
        subject: TermPattern::Triple(Box::new(edge_triple)),
        predicate: prop_iri.into(),
        object: prop_val,
    }
}

/// Build a `TermPattern::Triple` (the `<<…>>` subject) reusable when the
/// same annotated-triple subject is needed for multiple property triples.
pub fn edge_subject(src: TermPattern, pred: NamedNode, dst: TermPattern) -> TermPattern {
    let edge_triple = TriplePattern {
        subject: src,
        predicate: pred.into(),
        object: dst,
    };
    TermPattern::Triple(Box::new(edge_triple))
}

/// Produce all annotation triples for a set of property `(iri, term)` pairs
/// sharing the same underlying edge.
pub fn all_property_triples(
    src: TermPattern,
    pred: NamedNode,
    dst: TermPattern,
    props: &[(NamedNode, TermPattern)],
) -> Vec<TriplePattern> {
    let edge_subj = edge_subject(src, pred, dst);
    props
        .iter()
        .map(|(prop_iri, prop_val)| TriplePattern {
            subject: edge_subj.clone(),
            predicate: prop_iri.clone().into(),
            object: prop_val.clone(),
        })
        .collect()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use spargebra::term::{Literal, Variable};

    fn var(n: &str) -> TermPattern {
        Variable::new_unchecked(n).into()
    }
    fn iri(s: &str) -> NamedNode {
        NamedNode::new_unchecked(s)
    }

    #[test]
    fn annotated_triple_subject_is_triple_pattern() {
        let tp = annotated_triple(
            var("a"), iri("http://ex/KNOWS"), var("b"),
            iri("http://ex/since"), var("since"),
        );
        assert!(matches!(tp.subject, TermPattern::Triple(_)));
    }

    #[test]
    fn annotated_triple_object_is_variable() {
        let tp = annotated_triple(
            var("a"), iri("http://ex/KNOWS"), var("b"),
            iri("http://ex/since"), var("since"),
        );
        assert!(matches!(tp.object, TermPattern::Variable(_)));
    }

    #[test]
    fn annotated_triple_object_is_literal() {
        let tp = annotated_triple(
            var("a"), iri("http://ex/KNOWS"), var("b"),
            iri("http://ex/since"),
            TermPattern::Literal(Literal::new_simple_literal("2020")),
        );
        assert!(matches!(tp.object, TermPattern::Literal(_)));
    }

    #[test]
    fn all_property_triples_count() {
        let props = vec![
            (iri("http://ex/since"),  var("since")),
            (iri("http://ex/weight"), var("weight")),
        ];
        let result = all_property_triples(var("a"), iri("http://ex/KNOWS"), var("b"), &props);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn all_property_triples_subjects_are_annotated() {
        let props = vec![
            (iri("http://ex/p1"), var("v1")),
            (iri("http://ex/p2"), var("v2")),
        ];
        let result = all_property_triples(var("a"), iri("http://ex/KNOWS"), var("b"), &props);
        assert!(result.iter().all(|tp| matches!(tp.subject, TermPattern::Triple(_))));
        // Both annotation subjects should display identically.
        assert_eq!(result[0].subject.to_string(), result[1].subject.to_string());
    }

    #[test]
    fn display_contains_rdf_star_syntax() {
        let tp = annotated_triple(
            var("a"), iri("http://ex/KNOWS"), var("b"),
            iri("http://ex/since"), var("since"),
        );
        let s = tp.to_string();
        assert!(s.contains("<<") && s.contains(">>"), "got: {s}");
    }
}
