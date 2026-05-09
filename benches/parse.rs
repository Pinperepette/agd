use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_parse(c: &mut Criterion) {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/api-doc.agd"))
        .unwrap_or_else(|_| String::from("@h1 Placeholder\n@p Body\n"));
    c.bench_function("parse api-doc.agd", |b| {
        b.iter(|| agd::parse(black_box(&src)).unwrap());
    });

    let canonical = agd::canonicalize(&src).unwrap_or(src.clone());
    c.bench_function("serialize api-doc.agd", |b| {
        let doc = agd::parse(&canonical).unwrap();
        b.iter(|| agd::serialize(black_box(&doc)));
    });
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
