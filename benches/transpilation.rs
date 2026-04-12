use criterion::{criterion_group, criterion_main, Criterion};

fn bench_parse_simple(c: &mut Criterion) {
    let query = "MATCH (n:Person) WHERE n.age > 30 RETURN n.name";
    c.bench_function("parse_simple_match_return", |b| {
        b.iter(|| polygraph::parser::parse_cypher(query).unwrap())
    });
}

criterion_group!(benches, bench_parse_simple);
criterion_main!(benches);
