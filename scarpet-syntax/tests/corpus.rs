//! Parses every `.sc` file under the workspace-level `example/` directory and
//! asserts the result against the `KNOWN_BAD` allow-list. Fixtures upstream
//! that flip from "known bad" to "parses cleanly" surface as
//! `unexpected_passes` so the list can be pruned.

use scarpet_syntax::corpus;

#[test]
fn corpus_round_trips() {
    let outcome = corpus::run().unwrap_or_else(|e| panic!("{e}"));

    eprintln!(
        "corpus: {} files | ok={} expected_failures={} unexpected_failures={} unexpected_passes={}",
        outcome.total,
        outcome.ok,
        outcome.expected_failures,
        outcome.unexpected_failures.len(),
        outcome.unexpected_passes.len(),
    );

    if !outcome.unexpected_failures.is_empty() {
        eprintln!(
            "\nUnexpected parse failures ({}):",
            outcome.unexpected_failures.len()
        );
        for f in &outcome.unexpected_failures {
            eprintln!("  - {f}");
        }
    }
    if !outcome.unexpected_passes.is_empty() {
        eprintln!(
            "\nFiles that unexpectedly parsed (remove from KNOWN_BAD): {}",
            outcome.unexpected_passes.len()
        );
        for f in &outcome.unexpected_passes {
            eprintln!("  - {f}");
        }
    }
    assert!(
        outcome.unexpected_failures.is_empty() && outcome.unexpected_passes.is_empty(),
        "corpus parse expectations not met"
    );
}
