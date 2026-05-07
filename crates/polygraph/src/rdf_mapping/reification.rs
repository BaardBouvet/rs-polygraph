/// Standard RDF reification fallback.
///
/// Edge properties are encoded using the RDF reification vocabulary
/// (`rdf:Statement`, `rdf:subject`, `rdf:predicate`, `rdf:object`) for
/// engines that do not support RDF-star:
///
/// ```sparql
/// ?__r rdf:type       rdf:Statement ;
///       rdf:subject   ?a ;
///       rdf:predicate <base:KNOWS> ;
///       rdf:object    ?b ;
///       <base:since>  ?since .
/// ```
use spargebra::term::{NamedNode, TermPattern, TriplePattern, Variable};

// ── Well-known IRIs ───────────────────────────────────────────────────────────

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_STATEMENT: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#Statement";
const RDF_SUBJECT: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#subject";
const RDF_PREDICATE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#predicate";
const RDF_OBJECT: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#object";

fn rdf(local: &str) -> NamedNode {
    NamedNode::new_unchecked(match local {
        "type" => RDF_TYPE,
        "Statement" => RDF_STATEMENT,
        "subject" => RDF_SUBJECT,
        "predicate" => RDF_PREDICATE,
        "object" => RDF_OBJECT,
        _ => unreachable!("unknown rdf: local {local}"),
    })
}

/// Produce the four structural reification triples for an edge, given a
/// fresh reification node variable.
///
/// Emits:
/// ```sparql
/// ?reif_var rdf:type       rdf:Statement .
/// ?reif_var rdf:subject    ?src .
/// ?reif_var rdf:predicate  <pred> .
/// ?reif_var rdf:object     ?dst .
/// ```
pub fn structural_triples(
    reif_var: &Variable,
    src: TermPattern,
    pred: NamedNode,
    dst: TermPattern,
) -> Vec<TriplePattern> {
    let r: TermPattern = reif_var.clone().into();
    vec![
        TriplePattern {
            subject: r.clone(),
            predicate: rdf("type").into(),
            object: rdf("Statement").into(),
        },
        TriplePattern {
            subject: r.clone(),
            predicate: rdf("subject").into(),
            object: src,
        },
        TriplePattern {
            subject: r.clone(),
            predicate: rdf("predicate").into(),
            object: pred.into(),
        },
        TriplePattern {
            subject: r,
            predicate: rdf("object").into(),
            object: dst,
        },
    ]
}

/// Produce the property triples for a reified edge, i.e.:
/// `?reif_var <prop_iri> ?prop_val` for each `(prop_iri, prop_val)` in `props`.
pub fn property_triples(
    reif_var: &Variable,
    props: &[(NamedNode, TermPattern)],
) -> Vec<TriplePattern> {
    let r: TermPattern = reif_var.clone().into();
    props
        .iter()
        .map(|(prop_iri, prop_val)| TriplePattern {
            subject: r.clone(),
            predicate: prop_iri.clone().into(),
            object: prop_val.clone(),
        })
        .collect()
}

/// Produce all reification triples (structural + property) for a single edge.
pub fn all_triples(
    reif_var: &Variable,
    src: TermPattern,
    pred: NamedNode,
    dst: TermPattern,
    props: &[(NamedNode, TermPattern)],
) -> Vec<TriplePattern> {
    let mut out = structural_triples(reif_var, src, pred, dst);
    out.extend(property_triples(reif_var, props));
    out
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn var(n: &str) -> Variable {
        Variable::new_unchecked(n)
    }
    fn var_tp(n: &str) -> TermPattern {
        Variable::new_unchecked(n).into()
    }
    fn iri(s: &str) -> NamedNode {
        NamedNode::new_unchecked(s)
    }

    #[test]
    fn structural_triples_count() {
        let ts = structural_triples(&var("r"), var_tp("a"), iri("http://ex/KNOWS"), var_tp("b"));
        assert_eq!(ts.len(), 4);
    }

    #[test]
    fn structural_triple_subjects_are_reif_var() {
        let rv = var("r");
        let ts = structural_triples(&rv, var_tp("a"), iri("http://ex/KNOWS"), var_tp("b"));
        let expected_subject = TermPattern::Variable(rv);
        assert!(ts.iter().all(|tp| tp.subject == expected_subject));
    }

    #[test]
    fn structural_triples_first_is_rdf_type() {
        let ts = structural_triples(&var("r"), var_tp("a"), iri("http://ex/KNOWS"), var_tp("b"));
        let pred_str = ts[0].predicate.to_string();
        assert!(
            pred_str.contains("type") || pred_str.contains("#type"),
            "got: {pred_str}"
        );
    }

    #[test]
    fn structural_triples_rdf_statement_object() {
        let ts = structural_triples(&var("r"), var_tp("a"), iri("http://ex/KNOWS"), var_tp("b"));
        let obj_str = ts[0].object.to_string();
        assert!(obj_str.contains("Statement"), "got: {obj_str}");
    }

    #[test]
    fn structural_triples_predicate_triple_contains_edge_pred() {
        let ts = structural_triples(&var("r"), var_tp("a"), iri("http://ex/KNOWS"), var_tp("b"));
        // The rdf:predicate triple's object should be the edge predicate IRI.
        let pred_triple = &ts[2];
        let obj_str = pred_triple.object.to_string();
        assert!(obj_str.contains("KNOWS"), "got: {obj_str}");
    }

    #[test]
    fn property_triples_count() {
        let props = vec![
            (iri("http://ex/since"), var_tp("since")),
            (iri("http://ex/weight"), var_tp("weight")),
        ];
        let ts = property_triples(&var("r"), &props);
        assert_eq!(ts.len(), 2);
    }

    #[test]
    fn property_triples_subjects_are_reif_var() {
        let rv = var("r");
        let props = vec![(iri("http://ex/since"), var_tp("since"))];
        let ts = property_triples(&rv, &props);
        assert_eq!(ts[0].subject, TermPattern::Variable(rv));
    }

    #[test]
    fn all_triples_structural_plus_property() {
        let props = vec![(iri("http://ex/since"), var_tp("since"))];
        let ts = all_triples(
            &var("r"),
            var_tp("a"),
            iri("http://ex/KNOWS"),
            var_tp("b"),
            &props,
        );
        // 4 structural + 1 property
        assert_eq!(ts.len(), 5);
    }

    #[test]
    fn all_triples_display_contains_rdf_subject() {
        let ts = structural_triples(&var("r"), var_tp("a"), iri("http://ex/KNOWS"), var_tp("b"));
        let combined = ts
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            combined.contains("subject") || combined.contains("rdf-syntax"),
            "got: {combined}"
        );
    }
}
