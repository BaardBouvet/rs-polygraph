/// RDF-star / RDF 1.2 reification-based edge-property encoding.
///
/// Relationship properties are encoded using the RDF 1.2 reification approach:
///
/// ```sparql
/// ?reifier <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies>
///     <<( ?a <base:KNOWS> ?b )>> .
/// ?reifier <base:since> ?val .
/// ```
///
/// The `sparql-12` feature on `spargebra` must be enabled (it is, via
/// `Cargo.toml`).  This approach works with both INSERT DATA (which uses the
/// `<< s p o >> prop val .` reification syntax to produce a reifier) and
/// SPARQL 1.2 SELECT queries.
use spargebra::term::{NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable};

/// The `rdf:reifies` predicate IRI (RDF 1.2).
const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

/// Produce the RDF 1.2 reification-based patterns for a single edge property.
///
/// Emits:
/// ```sparql
/// ?reifier rdf:reifies <<( src pred dst )>> .
/// ?reifier <prop_iri> prop_val .
/// ```
///
/// # Arguments
/// * `src` – SPARQL term for the source node (subject of the edge triple)
/// * `pred` – edge predicate IRI or variable
/// * `dst` – SPARQL term for the destination node (object of the edge triple)
/// * `prop_iri` – property predicate IRI
/// * `prop_val` – SPARQL term bound to the property value (variable or literal)
/// * `reifier_var` – fresh variable for the reifier blank node
pub fn annotated_triple(
    src: TermPattern,
    pred: NamedNodePattern,
    dst: TermPattern,
    prop_iri: NamedNode,
    prop_val: TermPattern,
    reifier_var: Variable,
) -> Vec<TriplePattern> {
    all_property_triples(src, pred, dst, &[(prop_iri, prop_val)], reifier_var)
}

/// Build a `TermPattern::Triple` (the `<<(…)>>` triple term) for use as the
/// object of a `rdf:reifies` triple pattern.
pub fn edge_triple_term(
    src: TermPattern,
    pred: NamedNodePattern,
    dst: TermPattern,
) -> TermPattern {
    let edge_triple = TriplePattern {
        subject: src,
        predicate: pred,
        object: dst,
    };
    TermPattern::Triple(Box::new(edge_triple))
}

/// Produce all reification-based annotation triples for a set of property
/// `(iri, term)` pairs sharing the same underlying edge.
///
/// Emits `1 + props.len()` triple patterns:
/// ```sparql
/// ?reifier rdf:reifies <<( src pred dst )>> .
/// ?reifier <prop1> val1 .
/// ?reifier <prop2> val2 .
/// ```
pub fn all_property_triples(
    src: TermPattern,
    pred: NamedNodePattern,
    dst: TermPattern,
    props: &[(NamedNode, TermPattern)],
    reifier_var: Variable,
) -> Vec<TriplePattern> {
    if props.is_empty() {
        return Vec::new();
    }
    let edge_term = edge_triple_term(src, pred, dst);
    let reifier_tp: TermPattern = reifier_var.into();
    let rdf_reifies = NamedNode::new_unchecked(RDF_REIFIES);

    let mut result = vec![TriplePattern {
        subject: reifier_tp.clone(),
        predicate: NamedNodePattern::NamedNode(rdf_reifies),
        object: edge_term,
    }];
    result.extend(props.iter().map(|(prop_iri, prop_val)| TriplePattern {
        subject: reifier_tp.clone(),
        predicate: NamedNodePattern::NamedNode(prop_iri.clone()),
        object: prop_val.clone(),
    }));
    result
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use spargebra::term::{Literal, Variable};

    fn var(n: &str) -> TermPattern {
        Variable::new_unchecked(n).into()
    }
    fn svar(n: &str) -> Variable {
        Variable::new_unchecked(n)
    }
    fn iri(s: &str) -> NamedNode {
        NamedNode::new_unchecked(s)
    }
    fn pred(s: &str) -> NamedNodePattern {
        NamedNodePattern::NamedNode(NamedNode::new_unchecked(s))
    }

    #[test]
    fn annotated_triple_emits_reifies_plus_property() {
        let tps = annotated_triple(
            var("a"),
            pred("http://ex/KNOWS"),
            var("b"),
            iri("http://ex/since"),
            var("since"),
            svar("__reif"),
        );
        // 2 triples: rdf:reifies + 1 property
        assert_eq!(tps.len(), 2);
        // First triple: ?__reif rdf:reifies <<(...)>>
        assert!(matches!(tps[0].object, TermPattern::Triple(_)));
        // Second triple: ?__reif <since> ?since
        assert!(matches!(tps[1].subject, TermPattern::Variable(_)));
    }

    #[test]
    fn annotated_triple_object_is_variable() {
        let tps = annotated_triple(
            var("a"),
            pred("http://ex/KNOWS"),
            var("b"),
            iri("http://ex/since"),
            var("since"),
            svar("__reif"),
        );
        assert!(matches!(tps[1].object, TermPattern::Variable(_)));
    }

    #[test]
    fn annotated_triple_object_is_literal() {
        let tps = annotated_triple(
            var("a"),
            pred("http://ex/KNOWS"),
            var("b"),
            iri("http://ex/since"),
            TermPattern::Literal(Literal::new_simple_literal("2020")),
            svar("__reif"),
        );
        assert!(matches!(tps[1].object, TermPattern::Literal(_)));
    }

    #[test]
    fn all_property_triples_count() {
        let props = vec![
            (iri("http://ex/since"), var("since")),
            (iri("http://ex/weight"), var("weight")),
        ];
        // 1 rdf:reifies + 2 property triples = 3
        let result = all_property_triples(var("a"), pred("http://ex/KNOWS"), var("b"), &props, svar("__reif"));
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn all_property_triples_subjects_are_reifier_var() {
        let props = vec![
            (iri("http://ex/p1"), var("v1")),
            (iri("http://ex/p2"), var("v2")),
        ];
        let result = all_property_triples(var("a"), pred("http://ex/KNOWS"), var("b"), &props, svar("__reif"));
        // Property triples (index 1, 2) have Variable subject (the reifier)
        assert!(result[1..].iter().all(|tp| matches!(tp.subject, TermPattern::Variable(_))));
    }

    #[test]
    fn display_contains_rdf_reifies_and_triple_term() {
        let tps = annotated_triple(
            var("a"),
            pred("http://ex/KNOWS"),
            var("b"),
            iri("http://ex/since"),
            var("since"),
            svar("__reif"),
        );
        let s = tps[0].to_string();
        assert!(s.contains("reifies"), "got: {s}");
        assert!(s.contains("<<") && s.contains(">>"), "got: {s}");
    }
}
