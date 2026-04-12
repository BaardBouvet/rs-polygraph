use pest::iterators::Pair;
use pest::Parser;
use pest_derive::Parser;

use crate::ast::cypher::{
    Clause, CompOp, CypherQuery, Direction, Expression, Ident, Literal, MapLiteral, MatchClause,
    NodePattern, Pattern, PatternElement, PatternList, RangeQuantifier, RelationshipPattern,
    ReturnClause, ReturnItem, ReturnItems, WhereClause, WithClause,
};
use crate::error::PolygraphError;

// The #[grammar] path is relative to the Cargo.toml (crate root).
#[derive(Parser)]
#[grammar = "grammars/cypher.pest"]
struct CypherPestParser;

/// Parse an openCypher query string into a typed [`CypherQuery`] AST.
///
/// # Errors
///
/// Returns [`PolygraphError::Parse`] if the input does not conform to the
/// supported openCypher subset.
pub fn parse(input: &str) -> Result<CypherQuery, PolygraphError> {
    let mut pairs = CypherPestParser::parse(Rule::query, input).map_err(|e| {
        let span = match e.location {
            pest::error::InputLocation::Pos(p) => format!("pos:{p}"),
            pest::error::InputLocation::Span((s, end)) => format!("span:{s}..{end}"),
        };
        PolygraphError::Parse { span, message: e.to_string() }
    })?;
    let query_pair = pairs.next().unwrap(); // Rule::query guaranteed by grammar
    build_query(query_pair)
}

// ── Top-level builders ────────────────────────────────────────────────────────

fn build_query(pair: Pair<Rule>) -> Result<CypherQuery, PolygraphError> {
    // query = { SOI ~ statement ~ EOI }
    let statement = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::statement)
        .expect("grammar guarantees a statement");
    build_statement(statement)
}

fn build_statement(pair: Pair<Rule>) -> Result<CypherQuery, PolygraphError> {
    // statement = { clause+ }
    let mut clauses = Vec::new();
    for clause_pair in pair.into_inner() {
        // Each is Rule::clause, which wraps the concrete clause variant.
        let inner = clause_pair
            .into_inner()
            .next()
            .expect("clause always has an inner rule");
        let clause = match inner.as_rule() {
            Rule::match_clause => Clause::Match(build_match_clause(inner)?),
            Rule::with_clause => Clause::With(build_with_clause(inner)?),
            Rule::return_clause => Clause::Return(build_return_clause(inner)?),
            _ => unreachable!("unexpected clause rule: {:?}", inner.as_rule()),
        };
        clauses.push(clause);
    }
    Ok(CypherQuery { clauses })
}

// ── Clause builders ───────────────────────────────────────────────────────────

fn build_match_clause(pair: Pair<Rule>) -> Result<MatchClause, PolygraphError> {
    // match_clause = { optional_marker? ~ kw_MATCH ~ pattern_list ~ where_clause? }
    let mut optional = false;
    let mut pattern = None;
    let mut where_ = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::optional_marker => optional = true,
            Rule::kw_MATCH => {}
            Rule::pattern_list => pattern = Some(build_pattern_list(inner)?),
            Rule::where_clause => where_ = Some(build_where_clause(inner)?),
            _ => {}
        }
    }
    Ok(MatchClause {
        optional,
        pattern: pattern.expect("grammar guarantees pattern_list"),
        where_,
    })
}

fn build_where_clause(pair: Pair<Rule>) -> Result<WhereClause, PolygraphError> {
    // where_clause = { kw_WHERE ~ expression }
    let expr_pair = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::expression)
        .expect("grammar guarantees expression");
    Ok(WhereClause { expression: build_expression(expr_pair)? })
}

fn build_return_clause(pair: Pair<Rule>) -> Result<ReturnClause, PolygraphError> {
    // return_clause = { kw_RETURN ~ return_body }
    let body_pair = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::return_body)
        .expect("grammar guarantees return_body");
    let (distinct, items) = build_return_body(body_pair)?;
    Ok(ReturnClause { distinct, items })
}

fn build_with_clause(pair: Pair<Rule>) -> Result<WithClause, PolygraphError> {
    // with_clause = { kw_WITH ~ return_body ~ where_clause? }
    let mut distinct = false;
    let mut items = None;
    let mut where_ = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::kw_WITH => {}
            Rule::return_body => {
                let (d, i) = build_return_body(inner)?;
                distinct = d;
                items = Some(i);
            }
            Rule::where_clause => where_ = Some(build_where_clause(inner)?),
            _ => {}
        }
    }
    Ok(WithClause {
        distinct,
        items: items.expect("grammar guarantees return_body"),
        where_,
    })
}

// ── Return body ───────────────────────────────────────────────────────────────

fn build_return_body(pair: Pair<Rule>) -> Result<(bool, ReturnItems), PolygraphError> {
    // return_body = { distinct_marker? ~ return_items }
    let mut distinct = false;
    let mut items = ReturnItems::All;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::distinct_marker => distinct = true,
            Rule::return_items => items = build_return_items(inner)?,
            _ => {}
        }
    }
    Ok((distinct, items))
}

fn build_return_items(pair: Pair<Rule>) -> Result<ReturnItems, PolygraphError> {
    // return_items = { star_projection | explicit_items }
    let inner = pair.into_inner().next().expect("return_items has one child");
    match inner.as_rule() {
        Rule::star_projection => Ok(ReturnItems::All),
        Rule::explicit_items => {
            let items: Result<Vec<_>, _> = inner
                .into_inner()
                .filter(|p| p.as_rule() == Rule::return_item)
                .map(build_return_item)
                .collect();
            Ok(ReturnItems::Explicit(items?))
        }
        _ => unreachable!(),
    }
}

fn build_return_item(pair: Pair<Rule>) -> Result<ReturnItem, PolygraphError> {
    // return_item = { expression ~ (kw_AS ~ variable)? }
    let mut expr = None;
    let mut alias = None;
    let mut saw_as = false;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::expression => expr = Some(build_expression(inner)?),
            Rule::kw_AS => saw_as = true,
            Rule::variable if saw_as => alias = Some(ident_text(&inner)),
            _ => {}
        }
    }
    Ok(ReturnItem {
        expression: expr.expect("grammar guarantees expression"),
        alias,
    })
}

// ── Pattern builders ──────────────────────────────────────────────────────────

fn build_pattern_list(pair: Pair<Rule>) -> Result<PatternList, PolygraphError> {
    // pattern_list = { pattern ~ ("," ~ pattern)* }
    let patterns: Result<Vec<_>, _> = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::pattern)
        .map(build_pattern)
        .collect();
    Ok(PatternList(patterns?))
}

fn build_pattern(pair: Pair<Rule>) -> Result<Pattern, PolygraphError> {
    // pattern = { (variable ~ "=")? ~ node_pattern_chain }
    let mut variable = None;
    let mut chain_pair = None;
    let mut saw_eq = false;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::variable if !saw_eq => {
                // Peek ahead: the `=` sign comes next if this is a named pattern.
                // Because pest doesn't expose consumed-but-not-matched info, we
                // rely on the grammar ordering: variable comes before node_pattern_chain.
                variable = Some(ident_text(&inner));
            }
            Rule::node_pattern_chain => chain_pair = Some(inner),
            _ => {
                // A bare `=` token — once we see the chain we know variable was used.
                saw_eq = true;
            }
        }
    }

    // If no `=` was consumed, the "variable" was actually the start of the chain;
    // but the grammar forces `variable ~ "="` as the prefix, so `variable` field
    // holds the pattern binding if `=` was present, which we detect by the presence
    // of both `variable` AND `node_pattern_chain` pairs.
    // Actually: if `variable` is Some and `chain_pair` is also Some, the variable is
    // a pattern binder. If only `chain_pair` is Some, no binder was parsed (the grammar
    // would not emit a variable pair). This logic is handled correctly by the grammar.
    let chain = chain_pair.expect("grammar guarantees node_pattern_chain");
    let elements = build_node_pattern_chain(chain)?;
    Ok(Pattern { variable, elements })
}

fn build_node_pattern_chain(pair: Pair<Rule>) -> Result<Vec<PatternElement>, PolygraphError> {
    // node_pattern_chain = { node_pattern ~ chain_link* }
    let mut elements = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::node_pattern => {
                elements.push(PatternElement::Node(build_node_pattern(inner)?));
            }
            Rule::chain_link => {
                // chain_link = { rel_pattern ~ node_pattern }
                for link_inner in inner.into_inner() {
                    match link_inner.as_rule() {
                        Rule::rel_pattern => {
                            elements.push(PatternElement::Relationship(
                                build_rel_pattern(link_inner)?,
                            ));
                        }
                        Rule::node_pattern => {
                            elements.push(PatternElement::Node(build_node_pattern(link_inner)?));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(elements)
}

fn build_node_pattern(pair: Pair<Rule>) -> Result<NodePattern, PolygraphError> {
    // node_pattern = { "(" ~ variable? ~ node_labels? ~ properties? ~ ")" }
    let mut variable = None;
    let mut labels = Vec::new();
    let mut properties = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::variable => variable = Some(ident_text(&inner)),
            Rule::node_labels => {
                for label_pair in inner.into_inner() {
                    if label_pair.as_rule() == Rule::node_label {
                        // node_label = { ":" ~ (ident_escaped | ident) }
                        let name = label_pair
                            .into_inner()
                            .next()
                            .expect("node_label has ident")
                            .as_str()
                            .trim_matches('`')
                            .to_string();
                        labels.push(name);
                    }
                }
            }
            Rule::properties => properties = Some(build_map_literal(inner)?),
            _ => {}
        }
    }
    Ok(NodePattern { variable, labels, properties })
}

fn build_rel_pattern(pair: Pair<Rule>) -> Result<RelationshipPattern, PolygraphError> {
    // rel_pattern = { left_arrow ~ rel_body ~ rel_dash
    //               | rel_dash ~ rel_body ~ right_arrow
    //               | rel_dash ~ rel_body ~ rel_dash }
    let mut has_left_arrow = false;
    let mut has_right_arrow = false;
    let mut variable = None;
    let mut rel_types = Vec::new();
    let mut range = None;
    let mut properties = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::left_arrow => has_left_arrow = true,
            Rule::right_arrow => has_right_arrow = true,
            Rule::rel_dash => {}
            Rule::rel_body => {
                for rb in inner.into_inner() {
                    match rb.as_rule() {
                        Rule::variable => variable = Some(ident_text(&rb)),
                        Rule::rel_type_list => {
                            for rt in rb.into_inner() {
                                if rt.as_rule() == Rule::rel_type_elem {
                                    rel_types.push(rt.as_str().trim_matches('`').to_string());
                                }
                            }
                        }
                        Rule::range_literal => range = Some(build_range_literal(rb)?),
                        Rule::properties => properties = Some(build_map_literal(rb)?),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let direction = match (has_left_arrow, has_right_arrow) {
        (true, false) => Direction::Left,
        (false, true) => Direction::Right,
        _ => Direction::Both,
    };

    Ok(RelationshipPattern { variable, direction, rel_types, properties, range })
}

fn build_range_literal(pair: Pair<Rule>) -> Result<RangeQuantifier, PolygraphError> {
    // range_literal = { "*" ~ (integer_literal ~ (".." ~ integer_literal?)?)? }
    let text = pair.as_str().trim();
    if text == "*" {
        return Ok(RangeQuantifier { lower: None, upper: None });
    }
    // Strip leading "*"
    let rest = text.trim_start_matches('*').trim();
    if rest.is_empty() {
        return Ok(RangeQuantifier { lower: None, upper: None });
    }
    if let Some((lo, hi)) = rest.split_once("..") {
        let lower = if lo.trim().is_empty() { None } else { Some(lo.trim().parse::<u64>().unwrap_or(0)) };
        let upper = if hi.trim().is_empty() { None } else { Some(hi.trim().parse::<u64>().unwrap_or(0)) };
        Ok(RangeQuantifier { lower, upper })
    } else {
        let n = rest.parse::<u64>().unwrap_or(0);
        Ok(RangeQuantifier { lower: Some(n), upper: Some(n) })
    }
}

// ── Map literal builder ───────────────────────────────────────────────────────

fn build_map_literal(pair: Pair<Rule>) -> Result<MapLiteral, PolygraphError> {
    // properties = { map_literal } OR directly map_literal
    // map_literal = { "{" ~ (map_entry ~ ("," ~ map_entry)*)? ~ "}" }
    let map_pair = if pair.as_rule() == Rule::properties {
        pair.into_inner()
            .find(|p| p.as_rule() == Rule::map_literal)
            .expect("properties wraps map_literal")
    } else {
        pair
    };

    let mut entries = Vec::new();
    for entry in map_pair.into_inner() {
        if entry.as_rule() == Rule::map_entry {
            // map_entry = { prop_name ~ ":" ~ expression }
            let mut key = None;
            let mut val = None;
            for part in entry.into_inner() {
                match part.as_rule() {
                    Rule::prop_name => {
                        key = Some(part.as_str().trim_matches('`').to_string());
                    }
                    Rule::expression => val = Some(build_expression(part)?),
                    _ => {}
                }
            }
            entries.push((
                key.expect("map_entry has prop_name"),
                val.expect("map_entry has expression"),
            ));
        }
    }
    Ok(entries)
}

// ── Expression builder ────────────────────────────────────────────────────────

fn build_expression(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    // expression = { or_expr }
    let inner = pair.into_inner().next().expect("expression wraps or_expr");
    build_or_expr(inner)
}

fn build_or_expr(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    // or_expr = { xor_expr ~ (kw_OR ~ xor_expr)* }
    let mut children: Vec<_> = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::xor_expr)
        .collect();
    let first = build_xor_expr(children.remove(0))?;
    children
        .into_iter()
        .try_fold(first, |acc, p| Ok(Expression::Or(Box::new(acc), Box::new(build_xor_expr(p)?))))
}

fn build_xor_expr(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    let mut children: Vec<_> = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::and_expr)
        .collect();
    let first = build_and_expr(children.remove(0))?;
    children
        .into_iter()
        .try_fold(first, |acc, p| Ok(Expression::Xor(Box::new(acc), Box::new(build_and_expr(p)?))))
}

fn build_and_expr(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    let mut children: Vec<_> = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::not_expr)
        .collect();
    let first = build_not_expr(children.remove(0))?;
    children
        .into_iter()
        .try_fold(first, |acc, p| Ok(Expression::And(Box::new(acc), Box::new(build_not_expr(p)?))))
}

fn build_not_expr(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    // not_expr = { kw_NOT ~ not_expr | comparison_expr }
    let mut children = pair.into_inner();
    let first = children.next().expect("not_expr has child");
    match first.as_rule() {
        Rule::kw_NOT => {
            // The second inner pair is the nested not_expr
            let nested = children.next().expect("NOT is followed by not_expr");
            Ok(Expression::Not(Box::new(build_not_expr(nested)?)))
        }
        Rule::comparison_expr => build_comparison_expr(first),
        _ => unreachable!("unexpected not_expr child: {:?}", first.as_rule()),
    }
}

fn build_comparison_expr(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    // comparison_expr = { add_sub_expr ~ comparison_suffix? }
    let mut inners = pair.into_inner();
    let lhs_pair = inners.next().expect("comparison_expr has add_sub_expr");
    let lhs = build_add_sub_expr(lhs_pair)?;

    if let Some(suffix) = inners.next() {
        // comparison_suffix = { comp_op ~ add_sub_expr | kw_IS ... | kw_IN ... | ... }
        build_comparison_suffix(lhs, suffix)
    } else {
        Ok(lhs)
    }
}

fn build_comparison_suffix(
    lhs: Expression,
    pair: Pair<Rule>,
) -> Result<Expression, PolygraphError> {
    // Peek at the first child to determine which variant.
    let mut children = pair.into_inner().peekable();
    let first = children.next().expect("comparison_suffix has children");

    match first.as_rule() {
        Rule::comp_op => {
            let op = match first.as_str() {
                "=" => CompOp::Eq,
                "<>" => CompOp::Ne,
                "<=" => CompOp::Le,
                ">=" => CompOp::Ge,
                "<" => CompOp::Lt,
                ">" => CompOp::Gt,
                other => {
                    return Err(PolygraphError::Parse {
                        span: String::new(),
                        message: format!("unknown comparison operator: {other}"),
                    })
                }
            };
            let rhs_pair = children.next().expect("comp_op is followed by add_sub_expr");
            let rhs = build_add_sub_expr(rhs_pair)?;
            Ok(Expression::Comparison(Box::new(lhs), op, Box::new(rhs)))
        }
        Rule::kw_IS => {
            // IS NULL or IS NOT NULL
            let next = children.next().expect("IS is followed by something");
            if next.as_rule() == Rule::kw_NOT {
                // IS NOT NULL
                Ok(Expression::IsNotNull(Box::new(lhs)))
            } else {
                // IS NULL (next is kw_NULL)
                Ok(Expression::IsNull(Box::new(lhs)))
            }
        }
        Rule::kw_IN => {
            let rhs_pair = children.next().expect("IN is followed by add_sub_expr");
            let rhs = build_add_sub_expr(rhs_pair)?;
            Ok(Expression::Comparison(Box::new(lhs), CompOp::In, Box::new(rhs)))
        }
        Rule::kw_STARTS => {
            // STARTS WITH expr
            let _kw_with = children.next(); // kw_WITH
            let rhs_pair = children.next().expect("STARTS WITH is followed by add_sub_expr");
            let rhs = build_add_sub_expr(rhs_pair)?;
            Ok(Expression::Comparison(Box::new(lhs), CompOp::StartsWith, Box::new(rhs)))
        }
        Rule::kw_ENDS => {
            // ENDS WITH expr
            let _kw_with = children.next(); // kw_WITH
            let rhs_pair = children.next().expect("ENDS WITH is followed by add_sub_expr");
            let rhs = build_add_sub_expr(rhs_pair)?;
            Ok(Expression::Comparison(Box::new(lhs), CompOp::EndsWith, Box::new(rhs)))
        }
        Rule::kw_CONTAINS => {
            let rhs_pair = children.next().expect("CONTAINS is followed by add_sub_expr");
            let rhs = build_add_sub_expr(rhs_pair)?;
            Ok(Expression::Comparison(Box::new(lhs), CompOp::Contains, Box::new(rhs)))
        }
        _ => unreachable!("unexpected comparison_suffix first child: {:?}", first.as_rule()),
    }
}

fn build_add_sub_expr(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    // add_sub_expr = { mul_div_expr ~ (add_sub_op ~ mul_div_expr)* }
    let mut children = pair.into_inner().peekable();
    let first = children.next().expect("add_sub_expr has mul_div_expr");
    let mut acc = build_mul_div_expr(first)?;

    while let Some(op_pair) = children.next() {
        let operand_pair = children.next().expect("operator is followed by operand");
        let rhs = build_mul_div_expr(operand_pair)?;
        acc = match op_pair.as_str() {
            "+" => Expression::Add(Box::new(acc), Box::new(rhs)),
            "-" => Expression::Subtract(Box::new(acc), Box::new(rhs)),
            _ => unreachable!(),
        };
    }
    Ok(acc)
}

fn build_mul_div_expr(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    // mul_div_expr = { unary_expr ~ (mul_div_op ~ unary_expr)* }
    let mut children = pair.into_inner().peekable();
    let first = children.next().expect("mul_div_expr has unary_expr");
    let mut acc = build_unary_expr(first)?;

    while let Some(op_pair) = children.next() {
        let operand_pair = children.next().expect("operator is followed by operand");
        let rhs = build_unary_expr(operand_pair)?;
        acc = match op_pair.as_str() {
            "*" => Expression::Multiply(Box::new(acc), Box::new(rhs)),
            "/" => Expression::Divide(Box::new(acc), Box::new(rhs)),
            "%" => Expression::Modulo(Box::new(acc), Box::new(rhs)),
            _ => unreachable!(),
        };
    }
    Ok(acc)
}

fn build_unary_expr(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    // unary_expr = { unary_minus | power_expr }
    let inner = pair.into_inner().next().expect("unary_expr has child");
    match inner.as_rule() {
        Rule::unary_minus => {
            // unary_minus = { "-" ~ unary_expr }
            let operand = inner.into_inner().next().expect("unary_minus has unary_expr");
            Ok(Expression::Negate(Box::new(build_unary_expr(operand)?)))
        }
        Rule::power_expr => build_power_expr(inner),
        _ => unreachable!(),
    }
}

fn build_power_expr(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    // power_expr = { prop_expr ~ ("^" ~ unary_expr)? }
    let mut children = pair.into_inner();
    let base = build_prop_expr(children.next().expect("power_expr has prop_expr"))?;
    if let Some(exponent_pair) = children.next() {
        Ok(Expression::Power(Box::new(base), Box::new(build_unary_expr(exponent_pair)?)))
    } else {
        Ok(base)
    }
}

fn build_prop_expr(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    // prop_expr = { atom ~ property_lookup* }
    let mut children = pair.into_inner();
    let atom_pair = children.next().expect("prop_expr has atom");
    let mut acc = build_atom(atom_pair)?;
    for lookup in children {
        // property_lookup = { "." ~ prop_name }
        let key = lookup
            .into_inner()
            .find(|p| p.as_rule() == Rule::prop_name)
            .expect("property_lookup has prop_name")
            .as_str()
            .trim_matches('`')
            .to_string();
        acc = Expression::Property(Box::new(acc), key);
    }
    Ok(acc)
}

fn build_atom(pair: Pair<Rule>) -> Result<Expression, PolygraphError> {
    // atom = { float_literal | integer_literal | string_literal | boolean_literal
    //        | null_literal | list_literal | map_literal | "(" ~ expression ~ ")" | variable }
    let inner = pair.into_inner().next().expect("atom has child");
    match inner.as_rule() {
        Rule::integer_literal => {
            let n: i64 = inner.as_str().parse().map_err(|_| PolygraphError::Parse {
                span: inner.as_str().to_string(),
                message: "integer literal out of range".to_string(),
            })?;
            Ok(Expression::Literal(Literal::Integer(n)))
        }
        Rule::float_literal => {
            let f: f64 = inner.as_str().parse().map_err(|_| PolygraphError::Parse {
                span: inner.as_str().to_string(),
                message: "float literal out of range".to_string(),
            })?;
            Ok(Expression::Literal(Literal::Float(f)))
        }
        Rule::string_literal => {
            let raw = inner.as_str();
            // Strip outer quotes
            let content = &raw[1..raw.len() - 1];
            // Basic escape processing
            let s = unescape_string(content);
            Ok(Expression::Literal(Literal::String(s)))
        }
        Rule::boolean_literal => {
            let b = inner.as_str().to_ascii_lowercase() == "true";
            Ok(Expression::Literal(Literal::Boolean(b)))
        }
        Rule::null_literal => Ok(Expression::Literal(Literal::Null)),
        Rule::list_literal => {
            let items: Result<Vec<_>, _> = inner
                .into_inner()
                .filter(|p| p.as_rule() == Rule::expression)
                .map(build_expression)
                .collect();
            Ok(Expression::List(items?))
        }
        Rule::map_literal => {
            let entries = build_map_literal(inner)?;
            Ok(Expression::Map(entries))
        }
        Rule::expression => build_expression(inner),
        Rule::variable => Ok(Expression::Variable(ident_text(&inner))),
        _ => unreachable!("unexpected atom child: {:?}", inner.as_rule()),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extract the text of a `variable` rule, handling backtick-escaped identifiers.
fn ident_text(pair: &Pair<Rule>) -> Ident {
    // variable = { !(keyword ~ !ident_char) ~ (ident_escaped | ident) }
    let inner = pair
        .clone()
        .into_inner()
        .next()
        .expect("variable has an ident or ident_escaped child");
    inner.as_str().trim_matches('`').to_string()
}

/// Unescape a Cypher string literal body (content between quotes).
fn unescape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(c2) => out.push(c2),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::cypher::*;

    fn parse_ok(q: &str) -> CypherQuery {
        parse(q).unwrap_or_else(|e| panic!("parse failed for {q:?}: {e}"))
    }

    // --- Round-trip tests -------------------------------------------------------

    #[test]
    fn match_return_node() {
        let q = parse_ok("MATCH (n) RETURN n");
        assert_eq!(q.clauses.len(), 2);
        assert!(matches!(q.clauses[0], Clause::Match(_)));
        assert!(matches!(q.clauses[1], Clause::Return(_)));
    }

    #[test]
    fn match_node_with_label() {
        let q = parse_ok("MATCH (n:Person) RETURN n");
        if let Clause::Match(m) = &q.clauses[0] {
            let pat = &m.pattern.0[0];
            if let PatternElement::Node(node) = &pat.elements[0] {
                assert_eq!(node.labels, vec!["Person"]);
            } else {
                panic!("expected node");
            }
        } else {
            panic!("expected match");
        }
    }

    #[test]
    fn match_node_with_multiple_labels() {
        let q = parse_ok("MATCH (n:Person:Employee) RETURN n");
        if let Clause::Match(m) = &q.clauses[0] {
            let pat = &m.pattern.0[0];
            if let PatternElement::Node(node) = &pat.elements[0] {
                assert_eq!(node.labels, vec!["Person", "Employee"]);
            } else {
                panic!("expected node");
            }
        } else {
            panic!("expected match");
        }
    }

    #[test]
    fn match_node_with_property() {
        let q = parse_ok("MATCH (n:Person {name: 'Alice'}) RETURN n");
        if let Clause::Match(m) = &q.clauses[0] {
            let pat = &m.pattern.0[0];
            if let PatternElement::Node(node) = &pat.elements[0] {
                let props = node.properties.as_ref().unwrap();
                assert_eq!(props[0].0, "name");
                assert_eq!(props[0].1, Expression::Literal(Literal::String("Alice".to_string())));
            } else {
                panic!("expected node");
            }
        }
    }

    #[test]
    fn match_relationship_right() {
        let q = parse_ok("MATCH (a)-[:KNOWS]->(b) RETURN a, b");
        if let Clause::Match(m) = &q.clauses[0] {
            let pat = &m.pattern.0[0];
            assert_eq!(pat.elements.len(), 3);
            if let PatternElement::Relationship(r) = &pat.elements[1] {
                assert_eq!(r.direction, Direction::Right);
                assert_eq!(r.rel_types, vec!["KNOWS"]);
            } else {
                panic!("expected relationship");
            }
        }
    }

    #[test]
    fn match_relationship_left() {
        let q = parse_ok("MATCH (a)<-[:KNOWS]-(b) RETURN a");
        if let Clause::Match(m) = &q.clauses[0] {
            let pat = &m.pattern.0[0];
            if let PatternElement::Relationship(r) = &pat.elements[1] {
                assert_eq!(r.direction, Direction::Left);
            } else {
                panic!("expected relationship");
            }
        }
    }

    #[test]
    fn match_relationship_undirected() {
        let q = parse_ok("MATCH (a)-[:KNOWS]-(b) RETURN a");
        if let Clause::Match(m) = &q.clauses[0] {
            let pat = &m.pattern.0[0];
            if let PatternElement::Relationship(r) = &pat.elements[1] {
                assert_eq!(r.direction, Direction::Both);
            } else {
                panic!("expected relationship");
            }
        }
    }

    #[test]
    fn optional_match() {
        let q = parse_ok("OPTIONAL MATCH (n:Person) RETURN n");
        if let Clause::Match(m) = &q.clauses[0] {
            assert!(m.optional);
        } else {
            panic!("expected match");
        }
    }

    #[test]
    fn match_where_return() {
        let q = parse_ok("MATCH (n:Person) WHERE n.age > 30 RETURN n.name");
        assert_eq!(q.clauses.len(), 2);
        if let Clause::Match(m) = &q.clauses[0] {
            assert!(m.where_.is_some());
        }
    }

    #[test]
    fn return_distinct() {
        let q = parse_ok("MATCH (n) RETURN DISTINCT n.name");
        if let Clause::Return(r) = &q.clauses[1] {
            assert!(r.distinct);
        }
    }

    #[test]
    fn return_star() {
        let q = parse_ok("MATCH (n) RETURN *");
        if let Clause::Return(r) = &q.clauses[1] {
            assert!(matches!(r.items, ReturnItems::All));
        }
    }

    #[test]
    fn return_with_alias() {
        let q = parse_ok("MATCH (n) RETURN n.name AS name");
        if let Clause::Return(r) = &q.clauses[1] {
            if let ReturnItems::Explicit(items) = &r.items {
                assert_eq!(items[0].alias.as_deref(), Some("name"));
            }
        }
    }

    #[test]
    fn with_clause() {
        let q = parse_ok("MATCH (n:Person) WITH n WHERE n.age > 18 RETURN n");
        assert_eq!(q.clauses.len(), 3);
        assert!(matches!(q.clauses[1], Clause::With(_)));
        if let Clause::With(w) = &q.clauses[1] {
            assert!(w.where_.is_some());
        }
    }

    #[test]
    fn multi_hop_path() {
        let q = parse_ok("MATCH (a)-[:KNOWS]->(b)-[:KNOWS]->(c) RETURN a, c");
        if let Clause::Match(m) = &q.clauses[0] {
            let pat = &m.pattern.0[0];
            assert_eq!(pat.elements.len(), 5); // node, rel, node, rel, node
        }
    }

    #[test]
    fn return_multiple_items() {
        let q = parse_ok("MATCH (n) RETURN n.name, n.age");
        if let Clause::Return(r) = &q.clauses[1] {
            if let ReturnItems::Explicit(items) = &r.items {
                assert_eq!(items.len(), 2);
            }
        }
    }

    #[test]
    fn expression_and_or() {
        let q = parse_ok("MATCH (n) WHERE n.age > 18 AND n.active = true RETURN n");
        if let Clause::Match(m) = &q.clauses[0] {
            let expr = &m.where_.as_ref().unwrap().expression;
            assert!(matches!(expr, Expression::And(_, _)));
        }
    }

    #[test]
    fn expression_not() {
        let q = parse_ok("MATCH (n) WHERE NOT n.deleted RETURN n");
        if let Clause::Match(m) = &q.clauses[0] {
            let expr = &m.where_.as_ref().unwrap().expression;
            assert!(matches!(expr, Expression::Not(_)));
        }
    }

    #[test]
    fn expression_is_null() {
        let q = parse_ok("MATCH (n) WHERE n.name IS NULL RETURN n");
        if let Clause::Match(m) = &q.clauses[0] {
            let expr = &m.where_.as_ref().unwrap().expression;
            assert!(matches!(expr, Expression::IsNull(_)));
        }
    }

    #[test]
    fn expression_is_not_null() {
        let q = parse_ok("MATCH (n) WHERE n.name IS NOT NULL RETURN n");
        if let Clause::Match(m) = &q.clauses[0] {
            let expr = &m.where_.as_ref().unwrap().expression;
            assert!(matches!(expr, Expression::IsNotNull(_)));
        }
    }

    #[test]
    fn case_insensitive_keywords() {
        let q = parse_ok("match (n:Person) where n.age > 30 return n");
        assert_eq!(q.clauses.len(), 2);
    }

    #[test]
    fn mixed_case_keywords() {
        let q = parse_ok("Match (n) Return n.name As name");
        assert_eq!(q.clauses.len(), 2);
    }

    #[test]
    fn string_literal_double_quoted() {
        let q = parse_ok(r#"MATCH (n {name: "Alice"}) RETURN n"#);
        if let Clause::Match(m) = &q.clauses[0] {
            let pat = &m.pattern.0[0];
            if let PatternElement::Node(node) = &pat.elements[0] {
                let props = node.properties.as_ref().unwrap();
                assert_eq!(
                    props[0].1,
                    Expression::Literal(Literal::String("Alice".to_string()))
                );
            }
        }
    }

    #[test]
    fn integer_literal_in_expression() {
        let q = parse_ok("MATCH (n) WHERE n.age = 42 RETURN n");
        if let Clause::Match(m) = &q.clauses[0] {
            if let Expression::Comparison(_, CompOp::Eq, rhs) = &m.where_.as_ref().unwrap().expression {
                assert_eq!(**rhs, Expression::Literal(Literal::Integer(42)));
            }
        }
    }

    #[test]
    fn relationship_variable() {
        let q = parse_ok("MATCH (a)-[r:KNOWS]->(b) RETURN r");
        if let Clause::Match(m) = &q.clauses[0] {
            let pat = &m.pattern.0[0];
            if let PatternElement::Relationship(r) = &pat.elements[1] {
                assert_eq!(r.variable.as_deref(), Some("r"));
            }
        }
    }

    #[test]
    fn parse_error_returns_err() {
        assert!(parse("NOT VALID CYPHER %%%").is_err());
    }

    #[test]
    fn empty_input_returns_err() {
        assert!(parse("").is_err());
    }
}
