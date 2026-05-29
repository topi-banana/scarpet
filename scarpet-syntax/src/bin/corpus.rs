//! Stand-alone integration runner: walks every `.sc` file under
//! `example/<org>/<repo>/` and prints a summary. Exits with code 1 if any file
//! parsed unexpectedly (either an unannounced failure or a `KNOWN_BAD` entry
//! that has since started parsing cleanly).
//!
//! Run with `cargo run -p scarpet-syntax --bin corpus`.

use std::process::ExitCode;

use scarpet_syntax::corpus;

fn main() -> ExitCode {
    let outcome = match corpus::run() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    println!(
        "corpus: {} files | ok={} expected_failures={} unexpected_failures={} unexpected_passes={}",
        outcome.total,
        outcome.ok,
        outcome.expected_failures,
        outcome.unexpected_failures.len(),
        outcome.unexpected_passes.len(),
    );

    if !outcome.unexpected_failures.is_empty() {
        println!(
            "\nUnexpected parse failures ({}):",
            outcome.unexpected_failures.len()
        );
        for f in &outcome.unexpected_failures {
            println!("  - {f}");
        }
    }
    if !outcome.unexpected_passes.is_empty() {
        println!(
            "\nFiles that unexpectedly parsed (remove from KNOWN_BAD): {}",
            outcome.unexpected_passes.len()
        );
        for f in &outcome.unexpected_passes {
            println!("  - {f}");
        }
    }

    if outcome.unexpected_failures.is_empty() && outcome.unexpected_passes.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
