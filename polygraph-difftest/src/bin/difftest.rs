//! `difftest` CLI — runs the curated suite, optionally cross-checking a live
//! Neo4j (when built with `--features live-neo4j`).

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.push("queries");

    // CLI: --queries DIR
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--queries" => {
                if let Some(p) = args.next() {
                    dir = PathBuf::from(p);
                }
            }
            "-h" | "--help" => {
                println!("difftest [--queries DIR]");
                println!("  Runs every *.toml curated query under DIR (default: queries/).");
                println!("  Exit code 0 on all-pass, 1 on any failure.");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown arg: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let reports = polygraph_difftest::run_curated(&dir);
    let total = reports.len();
    let passed = reports.iter().filter(|r| r.passed()).count();
    let failed = total - passed;

    for r in &reports {
        if r.passed() {
            println!("  ok   {}", r.name);
        } else {
            println!("  FAIL {}", r.name);
            if let Some(err) = &r.error {
                println!("       error: {err}");
            }
            match &r.outcome {
                polygraph_difftest::ComparisonOutcome::Match => {}
                polygraph_difftest::ComparisonOutcome::Mismatch {
                    missing_from_actual,
                    unexpected_in_actual,
                    column_name_diff,
                } => {
                    if let Some((exp, act)) = column_name_diff {
                        println!("       columns expected: {exp:?}");
                        println!("       columns actual:   {act:?}");
                    }
                    if !missing_from_actual.is_empty() {
                        println!("       missing  ({}):", missing_from_actual.len());
                        for row in missing_from_actual.iter().take(5) {
                            println!("         {row:?}");
                        }
                    }
                    if !unexpected_in_actual.is_empty() {
                        println!("       extra    ({}):", unexpected_in_actual.len());
                        for row in unexpected_in_actual.iter().take(5) {
                            println!("         {row:?}");
                        }
                    }
                }
            }
            if !r.sparql.is_empty() {
                let first_lines: Vec<&str> = r.sparql.lines().take(6).collect();
                println!("       SPARQL:\n         {}", first_lines.join("\n         "));
            }
        }
    }

    println!();
    println!("difftest: {passed}/{total} passing, {failed} failing");

    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
