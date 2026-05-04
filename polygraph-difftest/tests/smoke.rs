//! Smoke test that runs the entire curated suite under `cargo test`.

use std::path::PathBuf;

#[test]
fn curated_suite_passes() {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.push("queries");

    let reports = polygraph_difftest::run_curated(&dir);
    assert!(
        !reports.is_empty(),
        "no curated queries found under {dir:?}"
    );

    let mut failures = Vec::new();
    for r in &reports {
        if !r.passed() {
            failures.push(format!(
                "  - {} ({})\n    error: {:?}\n    outcome: {:?}\n    sparql: {}",
                r.name,
                r.spec_ref,
                r.error,
                r.outcome,
                r.sparql.lines().next().unwrap_or("")
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} curated queries failed:\n{}",
        failures.len(),
        reports.len(),
        failures.join("\n")
    );
}
