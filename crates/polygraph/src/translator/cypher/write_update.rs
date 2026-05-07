//! SPARQL 1.1 Update generation for Cypher write clauses.
//!
//! Extracts from `tests/tck/main.rs` the write-clause generation logic and
//! promotes it to a first-class library function parameterised over the base
//! IRI from a [`TargetEngine`].
//!
//! # Usage
//!
//! ```rust
//! use polygraph::{Transpiler, sparql_engine::GenericSparql11};
//!
//! let engine = GenericSparql11;
//! let updates = Transpiler::cypher_to_sparql_update(
//!     "CREATE (n:Person {name: 'Alice', age: 30})",
//!     &engine,
//! ).unwrap();
//! assert!(!updates.is_empty());
//! ```

use std::collections::{HashMap, HashSet};

use crate::{
    ast::cypher::{
        Clause, Direction, Expression, Literal, Pattern, PatternElement, RemoveItem, ReturnItems,
        SetItem,
    },
    error::PolygraphError,
    parser::parse_cypher,
};

// ── Literal serialiser ────────────────────────────────────────────────────────

/// Convert a Cypher `Expression` literal to a SPARQL literal string.
///
/// Returns `None` for expressions that cannot be statically resolved to a
/// literal (variables, temporal constructors, complex subexpressions).
pub(crate) fn expr_to_sparql_lit(expr: &Expression) -> Option<String> {
    match expr {
        Expression::Literal(Literal::Integer(n)) => Some(n.to_string()),
        Expression::Literal(Literal::Float(f)) => Some(format!(
            "\"{}\"^^<http://www.w3.org/2001/XMLSchema#double>",
            f
        )),
        Expression::Literal(Literal::String(s)) => {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            Some(format!("\"{}\"", escaped))
        }
        Expression::Literal(Literal::Boolean(b)) => {
            Some(if *b { "true" } else { "false" }.to_owned())
        }
        Expression::Literal(Literal::Null) => None,
        Expression::List(items) => {
            let parts: Vec<String> = items.iter().filter_map(list_elem_to_str).collect();
            Some(format!("\"[{}]\"", parts.join(", ")))
        }
        // Temporal constructors (date(), time(), datetime(), duration()) require
        // runtime evaluation — not yet supported in the static write path.
        // Callers that need temporal literals should use the LQA write path.
        Expression::FunctionCall { .. } => None,
        _ => None,
    }
}

fn expr_to_sparql_lit_with_bindings(
    expr: &Expression,
    bindings: &HashMap<String, &Expression>,
    node_props: &HashMap<String, HashMap<String, Expression>>,
) -> Option<String> {
    match expr {
        Expression::Variable(v) => {
            if let Some(bound) = bindings.get(v.as_str()) {
                return expr_to_sparql_lit_with_bindings(bound, bindings, node_props);
            }
            None
        }
        Expression::Negate(inner) => {
            if let Expression::Literal(Literal::Integer(n)) = inner.as_ref() {
                return Some((-n).to_string());
            }
            if let Expression::Literal(Literal::Float(f)) = inner.as_ref() {
                return Some(format!(
                    "\"{}\"^^<http://www.w3.org/2001/XMLSchema#double>",
                    -f
                ));
            }
            None
        }
        Expression::Property(object, key) => {
            if let Expression::Variable(v) = object.as_ref() {
                if let Some(props) = node_props.get(v.as_str()) {
                    if let Some(val_expr) = props.get(key.as_str()) {
                        return expr_to_sparql_lit_with_bindings(val_expr, bindings, node_props);
                    }
                }
            }
            None
        }
        _ => expr_to_sparql_lit(expr),
    }
}

/// Convert a SET-clause value expression to a SPARQL expression string for BIND.
fn expr_to_sparql_update_expr(expr: &Expression, _var: &str) -> Option<String> {
    match expr {
        Expression::Literal(Literal::Integer(n)) => Some(format!("{n}")),
        Expression::Literal(Literal::Float(f)) => Some(format!(
            "\"{}\"^^<http://www.w3.org/2001/XMLSchema#double>",
            f
        )),
        Expression::Literal(Literal::String(s)) => {
            let escaped = s.replace('"', "\\\"");
            Some(format!("\"{escaped}\""))
        }
        Expression::Literal(Literal::Boolean(b)) => {
            Some(if *b { "true" } else { "false" }.to_owned())
        }
        Expression::Property(base, key) => {
            if let Expression::Variable(v) = base.as_ref() {
                Some(format!("?{v}_{key}_old"))
            } else {
                None
            }
        }
        Expression::Add(a, b) => {
            let la = expr_to_sparql_update_expr(a, _var)?;
            let ra = expr_to_sparql_update_expr(b, _var)?;
            Some(format!("({la} + {ra})"))
        }
        Expression::Subtract(a, b) => {
            let la = expr_to_sparql_update_expr(a, _var)?;
            let ra = expr_to_sparql_update_expr(b, _var)?;
            Some(format!("({la} - {ra})"))
        }
        Expression::Multiply(a, b) => {
            let la = expr_to_sparql_update_expr(a, _var)?;
            let ra = expr_to_sparql_update_expr(b, _var)?;
            Some(format!("({la} * {ra})"))
        }
        Expression::Divide(a, b) => {
            let la = expr_to_sparql_update_expr(a, _var)?;
            let ra = expr_to_sparql_update_expr(b, _var)?;
            Some(format!("({la} / {ra})"))
        }
        _ => None,
    }
}

fn expr_to_create_insert_expr(expr: &Expression) -> Option<(String, Vec<(String, String)>)> {
    match expr {
        Expression::Literal(Literal::Integer(n)) => Some((format!("{n}"), vec![])),
        Expression::Literal(Literal::Float(f)) => Some((
            format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#double>", f),
            vec![],
        )),
        Expression::Literal(Literal::String(s)) => {
            let escaped = s.replace('"', "\\\"");
            Some((format!("\"{escaped}\""), vec![]))
        }
        Expression::Literal(Literal::Boolean(b)) => {
            Some((if *b { "true" } else { "false" }.to_owned(), vec![]))
        }
        Expression::Property(base, key) => {
            if let Expression::Variable(v) = base.as_ref() {
                let var = format!("?{v}_{key}");
                Some((var, vec![(v.clone(), key.clone())]))
            } else {
                None
            }
        }
        Expression::Variable(v) => Some((format!("?{v}"), vec![])),
        Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b) => {
            let (la, mut da) = expr_to_create_insert_expr(a)?;
            let (lb, db) = expr_to_create_insert_expr(b)?;
            da.extend(db);
            let op = match expr {
                Expression::Add(_, _) => "+",
                Expression::Subtract(_, _) => "-",
                Expression::Multiply(_, _) => "*",
                Expression::Divide(_, _) => "/",
                _ => unreachable!(),
            };
            if matches!(expr, Expression::Add(_, _))
                && (matches!(b.as_ref(), Expression::Literal(Literal::String(_)))
                    || matches!(a.as_ref(), Expression::Literal(Literal::String(_))))
            {
                Some((format!("CONCAT(STR({la}), STR({lb}))"), da))
            } else {
                Some((format!("({la} {op} {lb})"), da))
            }
        }
        _ => None,
    }
}

fn list_elem_to_str(expr: &Expression) -> Option<String> {
    match expr {
        Expression::Literal(Literal::Integer(n)) => Some(n.to_string()),
        Expression::Literal(Literal::Float(f)) => Some(f.to_string()),
        Expression::Literal(Literal::String(s)) => Some(format!("'{}'", s)),
        Expression::Literal(Literal::Boolean(b)) => {
            Some(if *b { "true" } else { "false" }.to_owned())
        }
        Expression::Literal(Literal::Null) => Some("null".to_owned()),
        Expression::List(inner) => {
            let parts: Vec<String> = inner.iter().filter_map(list_elem_to_str).collect();
            Some(format!("[{}]", parts.join(", ")))
        }
        _ => None,
    }
}

// ── Blank-node / pattern helpers ──────────────────────────────────────────────

fn assign_node_bnodes(
    elements: &[PatternElement],
    node_map: &mut HashMap<String, String>,
    counter: &mut usize,
) -> Vec<Option<String>> {
    elements
        .iter()
        .map(|elem| match elem {
            PatternElement::Node(n) => {
                let bnode = if let Some(var) = &n.variable {
                    node_map
                        .entry(var.clone())
                        .or_insert_with(|| {
                            let s = format!("_:__n{}", *counter);
                            *counter += 1;
                            s
                        })
                        .clone()
                } else {
                    let s = format!("_:__n{}", *counter);
                    *counter += 1;
                    s
                };
                Some(bnode)
            }
            PatternElement::Relationship(_) => None,
        })
        .collect()
}

fn emit_create_pattern_with_bindings(
    pattern: &Pattern,
    triples: &mut Vec<String>,
    node_map: &mut HashMap<String, String>,
    counter: &mut usize,
    bindings: &HashMap<String, &Expression>,
    node_props: &HashMap<String, HashMap<String, Expression>>,
    base: &str,
) {
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    let elements = &pattern.elements;
    let node_bnodes = assign_node_bnodes(elements, node_map, counter);

    for (i, elem) in elements.iter().enumerate() {
        match elem {
            PatternElement::Node(n) => {
                let bnode = node_bnodes[i].as_deref().unwrap();
                for label in &n.labels {
                    triples.push(format!("{bnode} <{rdf_type}> <{base}{label}> ."));
                }
                if let Some(props) = &n.properties {
                    for (key, val_expr) in props {
                        if let Some(lit) =
                            expr_to_sparql_lit_with_bindings(val_expr, bindings, node_props)
                        {
                            triples.push(format!("{bnode} <{base}{key}> {lit} ."));
                        }
                    }
                }
                triples.push(format!("{bnode} <{base}__node> <{base}__node> ."));
            }
            PatternElement::Relationship(rel) => {
                let src = node_bnodes[..i]
                    .iter()
                    .filter_map(|x| x.as_deref())
                    .next_back();
                let dst = node_bnodes[i + 1..]
                    .iter()
                    .filter_map(|x| x.as_deref())
                    .next();
                if let (Some(src_b), Some(dst_b)) = (src, dst) {
                    let (s, o) = match rel.direction {
                        Direction::Left => (dst_b, src_b),
                        _ => (src_b, dst_b),
                    };
                    if rel.rel_types.is_empty() {
                        triples.push(format!("{s} <{base}__rel> {o} ."));
                    } else {
                        for rt in &rel.rel_types {
                            triples.push(format!("{s} <{base}{rt}> {o} ."));
                            if let Some(props) = &rel.properties {
                                for (key, val_expr) in props {
                                    if let Some(lit) = expr_to_sparql_lit_with_bindings(
                                        val_expr, bindings, node_props,
                                    ) {
                                        triples.push(format!(
                                            "<< {s} <{base}{rt}> {o} >> <{base}{key}> {lit} ."
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── Core write-clause compiler ────────────────────────────────────────────────

/// Generate SPARQL 1.1 Update statements for the write clauses in `clauses`.
///
/// `base` is the IRI prefix for all labels, relationship types, and property
/// names (from [`TargetEngine::base_iri()`]).  Returns an empty `Vec` for
/// pure-read queries.
pub fn cypher_clauses_to_updates(
    clauses: &[Clause],
    base: &str,
) -> Result<Vec<String>, PolygraphError> {
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

    let mut updates: Vec<String> = Vec::new();
    let mut node_map: HashMap<String, String> = HashMap::new();
    let mut counter: usize = 0;
    let mut loop_values: Vec<Expression> = vec![Expression::Literal(Literal::Null)];
    let mut unwind_var_name: Option<String> = None;
    let mut match_node_triples: HashMap<String, Vec<String>> = HashMap::new();
    let mut with_aliases: HashMap<String, String> = HashMap::new();
    let mut match_connected_node_pairs: Vec<(String, String)> = Vec::new();

    // Pre-pass: coalesce consecutive CREATE clauses so bnode scope is shared.
    let mut merged_clauses: Vec<Clause> = Vec::new();
    for c in clauses {
        match (merged_clauses.last_mut(), c) {
            (Some(Clause::Create(prev)), Clause::Create(curr)) => {
                prev.pattern.0.extend(curr.pattern.0.iter().cloned());
            }
            _ => merged_clauses.push(c.clone()),
        }
    }

    for clause in &merged_clauses {
        match clause {
            Clause::Unwind(u) => {
                if let Expression::List(items) = &u.expression {
                    loop_values = items.clone();
                    unwind_var_name = Some(u.variable.clone());
                }
            }

            Clause::With(w) => {
                if let ReturnItems::Explicit(items) = &w.items {
                    for item in items {
                        if let Some(ref alias) = item.alias {
                            if let Expression::Variable(src_var) = &item.expression {
                                let orig = with_aliases
                                    .get(src_var.as_str())
                                    .cloned()
                                    .unwrap_or_else(|| src_var.clone());
                                let constraints = match_node_triples
                                    .get(orig.as_str())
                                    .cloned()
                                    .or_else(|| match_node_triples.get(src_var.as_str()).cloned())
                                    .unwrap_or_else(|| {
                                        vec![format!("?{alias} <{base}__node> <{base}__node>")]
                                    });
                                let aliased: Vec<String> = constraints
                                    .iter()
                                    .map(|t| {
                                        t.replace(&format!("?{src_var} "), &format!("?{alias} "))
                                            .replace(&format!("?{src_var}>"), &format!("?{alias}>"))
                                            .replace(&format!("?{orig} "), &format!("?{alias} "))
                                            .replace(&format!("?{orig}>"), &format!("?{alias}>"))
                                    })
                                    .collect();
                                match_node_triples.insert(alias.clone(), aliased);
                                with_aliases.insert(alias.clone(), orig);
                            }
                        }
                    }
                }
            }

            Clause::Match(mc) => {
                for pattern in &mc.pattern.0 {
                    let mut prev_node_var: Option<String> = None;
                    for elem in &pattern.elements {
                        match elem {
                            PatternElement::Node(node) => {
                                if let Some(var) = &node.variable {
                                    let mut triples =
                                        vec![format!("?{var} <{base}__node> <{base}__node>")];
                                    for label in &node.labels {
                                        triples
                                            .push(format!("?{var} <{rdf_type}> <{base}{label}>"));
                                    }
                                    if let Some(props) = &node.properties {
                                        for (key, val) in props {
                                            if let Some(lit) = expr_to_sparql_lit(val) {
                                                triples.push(format!("?{var} <{base}{key}> {lit}"));
                                            }
                                        }
                                    }
                                    match_node_triples.insert(var.clone(), triples);
                                }
                                if let Some(ref prev) = prev_node_var {
                                    if let Some(ref curr) = node.variable {
                                        match_connected_node_pairs
                                            .push((prev.clone(), curr.clone()));
                                    }
                                }
                                prev_node_var = node.variable.clone();
                            }
                            PatternElement::Relationship(_) => {}
                        }
                    }
                }
            }

            Clause::Create(c) => {
                fn expr_refs_bound(e: &Expression, bound: &HashMap<String, Vec<String>>) -> bool {
                    match e {
                        Expression::Variable(v) => bound.contains_key(v.as_str()),
                        Expression::Property(b, _) => expr_refs_bound(b, bound),
                        Expression::Add(a, b)
                        | Expression::Subtract(a, b)
                        | Expression::Multiply(a, b)
                        | Expression::Divide(a, b)
                        | Expression::Modulo(a, b)
                        | Expression::Power(a, b)
                        | Expression::Comparison(a, _, b) => {
                            expr_refs_bound(a, bound) || expr_refs_bound(b, bound)
                        }
                        Expression::FunctionCall { args, .. } => {
                            args.iter().any(|a| expr_refs_bound(a, bound))
                        }
                        Expression::List(items) => items.iter().any(|i| expr_refs_bound(i, bound)),
                        Expression::Map(pairs) => {
                            pairs.iter().any(|(_, v)| expr_refs_bound(v, bound))
                        }
                        Expression::Negate(e) | Expression::Not(e) => expr_refs_bound(e, bound),
                        _ => false,
                    }
                }

                let has_bound_vars = c.pattern.0.iter().any(|pat| {
                    pat.elements.iter().any(|elem| {
                        if let PatternElement::Node(n) = elem {
                            let var_bound = n
                                .variable
                                .as_ref()
                                .map(|v| match_node_triples.contains_key(v.as_str()))
                                .unwrap_or(false);
                            let props_bound = n
                                .properties
                                .as_ref()
                                .map(|props| {
                                    props
                                        .iter()
                                        .any(|(_, v)| expr_refs_bound(v, &match_node_triples))
                                })
                                .unwrap_or(false);
                            var_bound || props_bound
                        } else {
                            false
                        }
                    })
                });

                if has_bound_vars {
                    let mut insert_triples: Vec<String> = Vec::new();
                    let mut where_triples: Vec<String> = Vec::new();
                    let mut seen_bound: HashSet<String> = HashSet::new();

                    for pattern in &c.pattern.0 {
                        let elements = &pattern.elements;
                        let mut node_refs: Vec<Option<String>> = Vec::with_capacity(elements.len());

                        for elem in elements {
                            match elem {
                                PatternElement::Node(n) => {
                                    let node_ref = if let Some(var) = &n.variable {
                                        if let Some(constraints) =
                                            match_node_triples.get(var.as_str())
                                        {
                                            if seen_bound.insert(var.clone()) {
                                                for t in constraints {
                                                    if !where_triples.contains(t) {
                                                        where_triples.push(t.clone());
                                                    }
                                                }
                                            }
                                            format!("?{var}")
                                        } else {
                                            let bnode = node_map
                                                .entry(var.clone())
                                                .or_insert_with(|| {
                                                    let s = format!("_:__n{counter}");
                                                    counter += 1;
                                                    s
                                                })
                                                .clone();
                                            insert_triples.push(format!(
                                                "{bnode} <{base}__node> <{base}__node> ."
                                            ));
                                            for label in &n.labels {
                                                insert_triples.push(format!(
                                                    "{bnode} <{rdf_type}> <{base}{label}> ."
                                                ));
                                            }
                                            if let Some(props) = &n.properties {
                                                for (key, val_expr) in props {
                                                    if let Some(lit) = expr_to_sparql_lit(val_expr)
                                                    {
                                                        insert_triples.push(format!(
                                                            "{bnode} <{base}{key}> {lit} ."
                                                        ));
                                                    } else if let Some((sparql_expr, prop_refs)) =
                                                        expr_to_create_insert_expr(val_expr)
                                                    {
                                                        for (ref_var, ref_key) in &prop_refs {
                                                            if seen_bound.insert(ref_var.clone()) {
                                                                if let Some(constraints) =
                                                                    match_node_triples
                                                                        .get(ref_var.as_str())
                                                                {
                                                                    for t in constraints {
                                                                        if !where_triples
                                                                            .contains(t)
                                                                        {
                                                                            where_triples
                                                                                .push(t.clone());
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            let opt = format!(
                                                                "OPTIONAL {{ ?{ref_var} <{base}{ref_key}> ?{ref_var}_{ref_key} }}"
                                                            );
                                                            if !where_triples.contains(&opt) {
                                                                where_triples.push(opt);
                                                            }
                                                        }
                                                        let prop_var =
                                                            format!("?__cprop_{counter}_{key}");
                                                        let bind = format!(
                                                            "BIND({sparql_expr} AS {prop_var})"
                                                        );
                                                        where_triples.push(bind);
                                                        insert_triples.push(format!(
                                                            "{bnode} <{base}{key}> {prop_var} ."
                                                        ));
                                                    }
                                                }
                                            }
                                            bnode
                                        }
                                    } else {
                                        let bnode = format!("_:__n{counter}");
                                        counter += 1;
                                        insert_triples.push(format!(
                                            "{bnode} <{base}__anon_node> <{base}__anon_node> ."
                                        ));
                                        bnode
                                    };
                                    node_refs.push(Some(node_ref));
                                }
                                PatternElement::Relationship(_) => {
                                    node_refs.push(None);
                                }
                            }
                        }

                        for (i, elem) in elements.iter().enumerate() {
                            if let PatternElement::Relationship(rel) = elem {
                                let src_ref = node_refs[..i]
                                    .iter()
                                    .filter_map(|x| x.as_deref())
                                    .next_back();
                                let dst_ref = node_refs[i + 1..]
                                    .iter()
                                    .filter_map(|x| x.as_deref())
                                    .next();
                                if let (Some(src_b), Some(dst_b)) = (src_ref, dst_ref) {
                                    let (s, o) = match rel.direction {
                                        Direction::Left => (dst_b, src_b),
                                        _ => (src_b, dst_b),
                                    };
                                    if rel.rel_types.is_empty() {
                                        insert_triples.push(format!("{s} <{base}__rel> {o} ."));
                                    } else {
                                        for rt in &rel.rel_types {
                                            insert_triples.push(format!("{s} <{base}{rt}> {o} ."));
                                            if let Some(props) = &rel.properties {
                                                for (key, val_expr) in props {
                                                    if let Some(lit) = expr_to_sparql_lit(val_expr)
                                                    {
                                                        insert_triples.push(format!("<< {s} <{base}{rt}> {o} >> <{base}{key}> {lit} ."));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !insert_triples.is_empty() {
                        let insert_body = insert_triples.join("\n  ");
                        let where_body = if where_triples.is_empty() {
                            "{ }".to_string()
                        } else {
                            format!(
                                "{{ {} }}",
                                where_triples
                                    .iter()
                                    .map(|t| format!("{t} ."))
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            )
                        };
                        updates.push(format!("INSERT {{\n  {insert_body}\n}} WHERE {where_body}"));
                    }
                } else {
                    let mut triples: Vec<String> = Vec::new();
                    for pattern in &c.pattern.0 {
                        emit_create_pattern_with_bindings(
                            pattern,
                            &mut triples,
                            &mut node_map,
                            &mut counter,
                            &Default::default(),
                            &Default::default(),
                            base,
                        );
                    }
                    if !triples.is_empty() {
                        updates.push(format!("INSERT DATA {{\n  {}\n}}", triples.join("\n  ")));
                    }
                }

                // Track new named nodes for subsequent MERGE relationship clauses.
                for pattern in &c.pattern.0 {
                    for elem in &pattern.elements {
                        if let PatternElement::Node(node) = elem {
                            if let Some(var) = &node.variable {
                                if !match_node_triples.contains_key(var.as_str()) {
                                    let mut triples =
                                        vec![format!("?{var} <{base}__node> <{base}__node>")];
                                    if let Some(props) = &node.properties {
                                        for (key, val) in props {
                                            if let Some(lit) = expr_to_sparql_lit(val) {
                                                triples.push(format!("?{var} <{base}{key}> {lit}"));
                                            }
                                        }
                                    }
                                    match_node_triples.insert(var.clone(), triples);
                                }
                            }
                        }
                    }
                }
            }

            Clause::Remove(r) => {
                for item in &r.items {
                    match item {
                        RemoveItem::Property { variable, key } => {
                            let prop_iri = format!("{base}{key}");
                            let del_var = format!("?{variable}_{key}_del");
                            let n_var = format!("?{variable}");
                            updates.push(format!(
                                "DELETE {{ {n_var} <{prop_iri}> {del_var} }} WHERE {{ {n_var} <{base}__node> <{base}__node> . OPTIONAL {{ {n_var} <{prop_iri}> {del_var} }} }}"
                            ));
                            let src_var = format!("?{variable}_src");
                            let pred_var = format!("?{variable}_pred");
                            let dst_var = format!("?{variable}_dst");
                            let edge_del = format!("?{variable}_{key}_edel");
                            let reif_var = format!("?{variable}_{key}_reif");
                            let rdf_reifies_iri =
                                "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
                            updates.push(format!(
                                "DELETE {{ {reif_var} <{prop_iri}> {edge_del} }} WHERE {{ {src_var} {pred_var} {dst_var} . {reif_var} <{rdf_reifies_iri}> <<( {src_var} {pred_var} {dst_var} )>> . OPTIONAL {{ {reif_var} <{prop_iri}> {edge_del} }} }}"
                            ));
                        }
                        RemoveItem::Label { variable, labels } => {
                            for label in labels {
                                let label_iri = format!("{base}{label}");
                                let n_var = format!("?{variable}");
                                updates.push(format!(
                                    "DELETE {{ {n_var} <{rdf_type}> <{label_iri}> }} WHERE {{ {n_var} <{rdf_type}> <{label_iri}> }}"
                                ));
                            }
                        }
                    }
                }
            }

            Clause::Set(s) => {
                for item in &s.items {
                    match item {
                        SetItem::Property {
                            variable,
                            key,
                            value,
                        } => {
                            let prop_iri = format!("{base}{key}");
                            let old_var = format!("?{variable}_{key}_old");
                            let new_var = format!("?{variable}_{key}_new");
                            let n_var = format!("?{variable}");
                            if let Some(lit_str) = expr_to_sparql_lit(value) {
                                updates.push(format!(
                                    "DELETE {{ {n_var} <{prop_iri}> {old_var} }} INSERT {{ {n_var} <{prop_iri}> {lit_str} }} WHERE {{ {n_var} <{base}__node> <{base}__node> . OPTIONAL {{ {n_var} <{prop_iri}> {old_var} }} }}"
                                ));
                            } else if let Some(expr_str) =
                                expr_to_sparql_update_expr(value, variable)
                            {
                                updates.push(format!(
                                    "DELETE {{ {n_var} <{prop_iri}> {old_var} }} INSERT {{ {n_var} <{prop_iri}> {new_var} }} WHERE {{ {n_var} <{base}__node> <{base}__node> . {n_var} <{prop_iri}> {old_var} . BIND({expr_str} AS {new_var}) . FILTER(BOUND({new_var})) }}"
                                ));
                                let src_var = format!("?{variable}_src");
                                let pred_var = format!("?{variable}_pred");
                                let dst_var = format!("?{variable}_dst");
                                let reif_var = format!("?{variable}_{key}_reif");
                                let rdf_reifies_iri =
                                    "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
                                updates.push(format!(
                                    "DELETE {{ {reif_var} <{prop_iri}> {old_var} }} INSERT {{ {reif_var} <{prop_iri}> {new_var} }} WHERE {{ {src_var} {pred_var} {dst_var} . {reif_var} <{rdf_reifies_iri}> <<( {src_var} {pred_var} {dst_var} )>> . {reif_var} <{prop_iri}> {old_var} . BIND({expr_str} AS {new_var}) . FILTER(BOUND({new_var})) }}"
                                ));
                            }
                        }
                        SetItem::SetLabel { variable, labels } => {
                            let n_var = format!("?{variable}");
                            let sentinel = format!("<{base}__node>");
                            for label in labels {
                                let label_iri = format!("{base}{label}");
                                updates.push(format!(
                                    "INSERT {{ {n_var} <{rdf_type}> <{label_iri}> }} WHERE {{ {n_var} {sentinel} {sentinel} }}"
                                ));
                            }
                        }
                        SetItem::MergeMap { .. } | SetItem::NodeReplace { .. } => {}
                    }
                }
            }

            Clause::Merge(m) => {
                if m.pattern.elements.len() == 1 {
                    if let PatternElement::Node(node) = &m.pattern.elements[0] {
                        let var_name = node.variable.as_deref().unwrap_or("__merge_n");
                        let n_var = format!("?{var_name}");

                        let loop_count = loop_values.len();
                        for iter in 0..loop_count {
                            let mut bindings_map: HashMap<String, &Expression> = HashMap::new();
                            if let Some(ref lv) = unwind_var_name {
                                if let Some(val) = loop_values.get(iter) {
                                    bindings_map.insert(lv.clone(), val);
                                }
                            }

                            let resolve_val = |val: &Expression,
                                               bindings: &HashMap<String, &Expression>|
                             -> Option<String> {
                                match val {
                                    Expression::Variable(v) => {
                                        bindings.get(v.as_str()).and_then(|e| expr_to_sparql_lit(e))
                                    }
                                    _ => expr_to_sparql_lit(val),
                                }
                            };

                            let has_unresolved_prop =
                                node.properties.as_ref().is_some_and(|props| {
                                    props.iter().any(|(_, val)| {
                                        resolve_val(val, &bindings_map).is_none()
                                            && !matches!(val, Expression::Literal(_))
                                    })
                                });
                            if has_unresolved_prop {
                                continue;
                            }

                            let bnode = format!("_:n{iter}");
                            let mut insert_triples: Vec<String> = Vec::new();
                            insert_triples.push(format!("{bnode} <{base}__node> <{base}__node>"));
                            for label in &node.labels {
                                insert_triples
                                    .push(format!("{bnode} <{rdf_type}> <{base}{label}>"));
                            }
                            if let Some(props) = &node.properties {
                                for (key, val) in props {
                                    if let Some(lit) = resolve_val(val, &bindings_map) {
                                        insert_triples.push(format!("{bnode} <{base}{key}> {lit}"));
                                    }
                                }
                            }

                            // ON CREATE SET: include in INSERT so they only apply on creation.
                            for action in &m.actions {
                                if action.on_create {
                                    for item in &action.items {
                                        match item {
                                            SetItem::Property { key, value, .. } => {
                                                if let Some(lit_str) =
                                                    resolve_val(value, &bindings_map)
                                                {
                                                    insert_triples.push(format!(
                                                        "{bnode} <{base}{key}> {lit_str}"
                                                    ));
                                                }
                                            }
                                            SetItem::SetLabel { labels, .. } => {
                                                for label in labels {
                                                    insert_triples.push(format!(
                                                        "{bnode} <{rdf_type}> <{base}{label}>"
                                                    ));
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }

                            // FILTER NOT EXISTS: check uniqueness.
                            let mut exists_conds: Vec<String> = Vec::new();
                            exists_conds.push(format!("{n_var} <{base}__node> <{base}__node>"));
                            for label in &node.labels {
                                exists_conds.push(format!("{n_var} <{rdf_type}> <{base}{label}>"));
                            }
                            if let Some(props) = &node.properties {
                                for (key, val) in props {
                                    if let Some(lit) = resolve_val(val, &bindings_map) {
                                        exists_conds.push(format!("{n_var} <{base}{key}> {lit}"));
                                    }
                                }
                            }

                            let insert_body = insert_triples.join(" . ");
                            let exists_body = exists_conds.join(" . ");
                            updates.push(format!(
                                "INSERT {{ {insert_body} }} WHERE {{ FILTER NOT EXISTS {{ {exists_body} }} }}"
                            ));

                            // ON MATCH SET: apply when node already exists.
                            let mut match_conds: Vec<String> = Vec::new();
                            match_conds.push(format!("{n_var} <{base}__node> <{base}__node>"));
                            for label in &node.labels {
                                match_conds.push(format!("{n_var} <{rdf_type}> <{base}{label}>"));
                            }
                            if let Some(props) = &node.properties {
                                for (key, val) in props {
                                    if let Some(lit) = resolve_val(val, &bindings_map) {
                                        match_conds.push(format!("{n_var} <{base}{key}> {lit}"));
                                    }
                                }
                            }
                            let match_where = match_conds.join(" . ");
                            for action in &m.actions {
                                if !action.on_create {
                                    for item in &action.items {
                                        match item {
                                            SetItem::Property { key, value, .. } => {
                                                if let Some(lit_str) =
                                                    resolve_val(value, &bindings_map)
                                                {
                                                    let prop_iri = format!("{base}{key}");
                                                    let old_var = format!("?{var_name}_{key}_old");
                                                    updates.push(format!(
                                                        "DELETE {{ {n_var} <{prop_iri}> {old_var} }} INSERT {{ {n_var} <{prop_iri}> {lit_str} }} WHERE {{ {match_where} . OPTIONAL {{ {n_var} <{prop_iri}> {old_var} }} }}"
                                                    ));
                                                }
                                            }
                                            SetItem::SetLabel { labels, .. } => {
                                                for label in labels {
                                                    let label_iri = format!("{base}{label}");
                                                    updates.push(format!(
                                                        "INSERT {{ {n_var} <{rdf_type}> <{label_iri}> }} WHERE {{ {match_where} }}"
                                                    ));
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }

                        // Track this MERGE node for subsequent relationship MERGEs.
                        if let Some(var) = &node.variable {
                            if !match_node_triples.contains_key(var.as_str()) {
                                let mut triples =
                                    vec![format!("?{var} <{base}__node> <{base}__node>")];
                                for label in &node.labels {
                                    triples.push(format!("?{var} <{rdf_type}> <{base}{label}>"));
                                }
                                if let Some(props) = &node.properties {
                                    for (key, val) in props {
                                        // Use first UNWIND binding for property literals.
                                        let lit = if let (Expression::Variable(v), Some(ref lv)) =
                                            (val, &unwind_var_name)
                                        {
                                            if v == lv {
                                                loop_values.first().and_then(expr_to_sparql_lit)
                                            } else {
                                                expr_to_sparql_lit(val)
                                            }
                                        } else {
                                            expr_to_sparql_lit(val)
                                        };
                                        if let Some(lit) = lit {
                                            triples.push(format!("?{var} <{base}{key}> {lit}"));
                                        }
                                    }
                                }
                                match_node_triples.insert(var.clone(), triples);
                            }
                        }
                    }
                } else if m.pattern.elements.len() >= 3 {
                    // Relationship MERGE: (src)-[r:TYPE]->(dst)
                    if let (
                        PatternElement::Node(src_node),
                        PatternElement::Relationship(rel),
                        PatternElement::Node(dst_node),
                    ) = (
                        &m.pattern.elements[0],
                        &m.pattern.elements[1],
                        &m.pattern.elements[2],
                    ) {
                        let src_name = src_node.variable.as_deref().unwrap_or("__src");
                        let dst_name = dst_node.variable.as_deref().unwrap_or("__dst");
                        let default_src =
                            vec![format!("?{src_name} <{base}__node> <{base}__node>")];
                        let default_dst =
                            vec![format!("?{dst_name} <{base}__node> <{base}__node>")];
                        let src_triples = match_node_triples.get(src_name).unwrap_or(&default_src);
                        let dst_triples = match_node_triples.get(dst_name).unwrap_or(&default_dst);

                        let mut src_conds: Vec<String> = src_triples.clone();
                        for label in &src_node.labels {
                            let cond = format!("?{src_name} <{rdf_type}> <{base}{label}>");
                            if !src_conds.contains(&cond) {
                                src_conds.push(cond);
                            }
                        }
                        // Also add property constraints from the MERGE pattern.
                        if let Some(props) = &src_node.properties {
                            for (key, val) in props {
                                if let Some(lit) = expr_to_sparql_lit(val) {
                                    let cond = format!("?{src_name} <{base}{key}> {lit}");
                                    if !src_conds.contains(&cond) {
                                        src_conds.push(cond);
                                    }
                                }
                            }
                        }
                        let mut dst_conds: Vec<String> = dst_triples.clone();
                        for label in &dst_node.labels {
                            let cond = format!("?{dst_name} <{rdf_type}> <{base}{label}>");
                            if !dst_conds.contains(&cond) {
                                dst_conds.push(cond);
                            }
                        }
                        if let Some(props) = &dst_node.properties {
                            for (key, val) in props {
                                if let Some(lit) = expr_to_sparql_lit(val) {
                                    let cond = format!("?{dst_name} <{base}{key}> {lit}");
                                    if !dst_conds.contains(&cond) {
                                        dst_conds.push(cond);
                                    }
                                }
                            }
                        }

                        for rt in &rel.rel_types {
                            let type_iri = format!("{base}{rt}");
                            let (actual_src, actual_dst) = match rel.direction {
                                Direction::Left => (dst_name, src_name),
                                _ => (src_name, dst_name),
                            };

                            let mut insert_parts: Vec<String> =
                                vec![format!("?{actual_src} <{type_iri}> ?{actual_dst}")];
                            if let Some(props) = &rel.properties {
                                for (key, val) in props {
                                    if let Some(lit) = expr_to_sparql_lit(val) {
                                        insert_parts.push(format!(
                                            "<< ?{actual_src} <{type_iri}> ?{actual_dst} >> <{base}{key}> {lit}"
                                        ));
                                    }
                                }
                            }
                            for action in &m.actions {
                                if action.on_create {
                                    for item in &action.items {
                                        if let SetItem::Property { key, value, .. } = item {
                                            if let Some(lit) = expr_to_sparql_lit(value) {
                                                insert_parts.push(format!(
                                                    "<< ?{actual_src} <{type_iri}> ?{actual_dst} >> <{base}{key}> {lit}"
                                                ));
                                            }
                                        }
                                    }
                                }
                            }

                            let insert_body = insert_parts.join(" . ");
                            let mut where_parts = src_conds.clone();
                            where_parts.extend(dst_conds.clone());

                            let mut not_exists_parts: Vec<String> =
                                vec![format!("?{actual_src} <{type_iri}> ?{actual_dst}")];
                            if let Some(props) = &rel.properties {
                                for (key, val) in props {
                                    if let Some(lit) = expr_to_sparql_lit(val) {
                                        not_exists_parts.push(format!(
                                            "<< ?{actual_src} <{type_iri}> ?{actual_dst} >> <{base}{key}> {lit}"
                                        ));
                                    }
                                }
                            }
                            let not_exists_str = if matches!(rel.direction, Direction::Both) {
                                let rev_parts: Vec<String> = not_exists_parts
                                    .iter()
                                    .map(|p| {
                                        p.replace(
                                            &format!("?{actual_src} <{type_iri}> ?{actual_dst}"),
                                            &format!("?{actual_dst} <{type_iri}> ?{actual_src}"),
                                        )
                                    })
                                    .collect();
                                format!(
                                    "{{ {} }} UNION {{ {} }}",
                                    not_exists_parts.join(" . "),
                                    rev_parts.join(" . ")
                                )
                            } else {
                                not_exists_parts.join(" . ")
                            };
                            where_parts.push(format!("FILTER NOT EXISTS {{ {not_exists_str} }}"));

                            let is_connected_pair =
                                match_connected_node_pairs.iter().any(|(a, b)| {
                                    (a.as_str() == src_name && b.as_str() == dst_name)
                                        || (a.as_str() == dst_name && b.as_str() == src_name)
                                });
                            if matches!(rel.direction, Direction::Both) && is_connected_pair {
                                let anyrel = format!("?__anyrel_{}_{}", src_name, dst_name);
                                where_parts.push(format!(
                                    "{{ ?{actual_src} {anyrel} ?{actual_dst} }} UNION {{ ?{actual_dst} {anyrel} ?{actual_src} . FILTER(!(?{actual_src} = ?{actual_dst})) }}"
                                ));
                            }

                            let where_body = where_parts.join(" . ");
                            updates.push(format!(
                                "INSERT {{ {insert_body} }} WHERE {{ {where_body} }}"
                            ));
                        }
                    }
                }

                loop_values = vec![Expression::Literal(Literal::Null)];
                unwind_var_name = None;
            }

            // Delete and other clauses handled by the LQA write path via cypher_to_sparql.
            _ => {}
        }
    }

    Ok(updates)
}

/// Parse a Cypher string and generate SPARQL 1.1 Update statements for its
/// write clauses (`CREATE`, `MERGE`, `SET`, `REMOVE`).
///
/// Pure-read queries return `Ok(vec![])`. `DELETE`/`DETACH DELETE` clauses
/// are handled by the LQA write path (call [`crate::Transpiler::cypher_to_sparql_update`]
/// instead, which dispatches through LQA first).
///
/// Returns [`PolygraphError::UnsupportedFeature`] for DDL constructs
/// (`CREATE CONSTRAINT`, `CREATE INDEX`) that have no SPARQL 1.1 equivalent.
pub fn cypher_to_update_statements(
    cypher: &str,
    base: &str,
) -> Result<Vec<String>, PolygraphError> {
    detect_unsupported_ddl(cypher)?;
    let ast = parse_cypher(cypher)?;
    cypher_clauses_to_updates(&ast.clauses, base)
}

/// Return `Err(UnsupportedFeature)` for `CREATE CONSTRAINT` / `CREATE INDEX`.
pub(crate) fn detect_unsupported_ddl(query: &str) -> Result<(), PolygraphError> {
    let upper = query.trim_start().to_ascii_uppercase();
    if upper.starts_with("CREATE CONSTRAINT") {
        return Err(PolygraphError::UnsupportedFeature {
            feature: "CREATE CONSTRAINT — no SPARQL 1.1 equivalent; skip this statement".into(),
        });
    }
    if upper.starts_with("CREATE INDEX") {
        return Err(PolygraphError::UnsupportedFeature {
            feature: "CREATE INDEX — no SPARQL 1.1 equivalent; skip this statement".into(),
        });
    }
    Ok(())
}
